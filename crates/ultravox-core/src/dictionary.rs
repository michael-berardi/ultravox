//! In-memory custom vocabulary parsing and transcript post-processing.
//!
//! The format is deliberately small: one canonical term per line, optionally
//! followed by ` = alias one, alias two`. Blank lines and `#` comments are
//! ignored. A compiled dictionary performs no I/O or network access.

use std::collections::{HashMap, HashSet};
use thiserror::Error;

pub const MAX_DICTIONARY_BYTES: usize = 128 * 1024;
pub const MAX_DICTIONARY_ENTRIES: usize = 512;
pub const MAX_DICTIONARY_FIELD_BYTES: usize = 128;
pub const MAX_ALIASES_PER_ENTRY: usize = 16;
const MAX_PROMPT_VOCABULARY_BYTES: usize = 2_048;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictionaryEntry {
    pub canonical: String,
    pub aliases: Vec<String>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DictionaryError {
    #[error("dictionary is larger than {MAX_DICTIONARY_BYTES} bytes")]
    SourceTooLarge,
    #[error("dictionary has more than {MAX_DICTIONARY_ENTRIES} entries")]
    TooManyEntries,
    #[error("dictionary line {line} has an empty canonical term")]
    EmptyCanonical { line: usize },
    #[error(
        "dictionary line {line} contains a field larger than {MAX_DICTIONARY_FIELD_BYTES} bytes"
    )]
    FieldTooLong { line: usize },
    #[error("dictionary line {line} has more than {MAX_ALIASES_PER_ENTRY} aliases")]
    TooManyAliases { line: usize },
}

#[derive(Debug, Clone)]
struct Replacement {
    source: String,
    canonical: String,
}

#[derive(Debug, Clone)]
struct FuzzyTerm {
    canonical: String,
    normalized: String,
    word_count: usize,
    distinctive: bool,
}

/// Parsed and compiled custom vocabulary. It is cheap to reuse for every
/// transcript and contains no external resources.
#[derive(Debug, Clone, Default)]
pub struct CustomDictionary {
    entries: Vec<DictionaryEntry>,
    exact: Vec<Replacement>,
    fuzzy: Vec<FuzzyTerm>,
}

impl CustomDictionary {
    pub fn parse(source: &str) -> Result<Self, DictionaryError> {
        if source.len() > MAX_DICTIONARY_BYTES {
            return Err(DictionaryError::SourceTooLarge);
        }

        let mut entries: Vec<DictionaryEntry> = Vec::new();
        let mut canonical_indices = HashMap::<String, usize>::new();
        for (line_index, raw_line) in source.lines().enumerate() {
            let line_number = line_index + 1;
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let (canonical, aliases) = line
                .split_once('=')
                .map(|(canonical, aliases)| (canonical.trim(), Some(aliases)))
                .unwrap_or((line, None));
            if canonical.is_empty() {
                return Err(DictionaryError::EmptyCanonical { line: line_number });
            }
            validate_field(canonical, line_number)?;

            let aliases = aliases
                .map(|value| {
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|alias| !alias.is_empty())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if aliases.len() > MAX_ALIASES_PER_ENTRY {
                return Err(DictionaryError::TooManyAliases { line: line_number });
            }
            for alias in &aliases {
                validate_field(alias, line_number)?;
            }

            let canonical_key = canonical.to_lowercase();
            if let Some(index) = canonical_indices.get(&canonical_key).copied() {
                let entry = &mut entries[index];
                let mut known = entry
                    .aliases
                    .iter()
                    .map(|alias| alias.to_lowercase())
                    .collect::<HashSet<_>>();
                for alias in aliases {
                    if !alias.eq_ignore_ascii_case(&entry.canonical)
                        && known.insert(alias.to_lowercase())
                    {
                        if entry.aliases.len() >= MAX_ALIASES_PER_ENTRY {
                            return Err(DictionaryError::TooManyAliases { line: line_number });
                        }
                        entry.aliases.push(alias.to_string());
                    }
                }
                continue;
            }

            if entries.len() >= MAX_DICTIONARY_ENTRIES {
                return Err(DictionaryError::TooManyEntries);
            }
            let mut deduped_aliases = Vec::new();
            let mut known = HashSet::new();
            known.insert(canonical_key.clone());
            for alias in aliases {
                if known.insert(alias.to_lowercase()) {
                    deduped_aliases.push(alias.to_string());
                }
            }
            canonical_indices.insert(canonical_key, entries.len());
            entries.push(DictionaryEntry {
                canonical: canonical.to_string(),
                aliases: deduped_aliases,
            });
        }

        Ok(Self::compile(entries))
    }

    /// Merge manual and generated vocabulary. Manual entries win when the same
    /// alias is present in both sources; generated entries never overwrite the
    /// source text that the user maintains.
    pub fn from_sources(manual: &str, generated: &str) -> Result<Self, DictionaryError> {
        let manual = Self::parse(manual)?;
        let generated = Self::parse(generated)?;
        let mut entries = manual.entries;
        let mut canonical_indices = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.canonical.to_lowercase(), index))
            .collect::<HashMap<_, _>>();
        for generated_entry in generated.entries {
            let key = generated_entry.canonical.to_lowercase();
            if let Some(index) = canonical_indices.get(&key).copied() {
                let entry = &mut entries[index];
                let mut known = entry
                    .aliases
                    .iter()
                    .map(|alias| alias.to_lowercase())
                    .collect::<HashSet<_>>();
                for alias in generated_entry.aliases {
                    if entry.aliases.len() >= MAX_ALIASES_PER_ENTRY {
                        break;
                    }
                    if known.insert(alias.to_lowercase()) {
                        entry.aliases.push(alias);
                    }
                }
            } else if entries.len() < MAX_DICTIONARY_ENTRIES {
                canonical_indices.insert(key, entries.len());
                entries.push(generated_entry);
            } else {
                // Manual entries have priority; generated terms beyond the
                // combined in-memory bound are deliberately omitted.
                break;
            }
        }
        Ok(Self::compile(entries))
    }

    fn compile(entries: Vec<DictionaryEntry>) -> Self {
        let mut exact = Vec::new();
        let mut seen_sources = HashSet::new();
        let mut fuzzy = Vec::new();
        for entry in &entries {
            for source in std::iter::once(&entry.canonical).chain(entry.aliases.iter()) {
                if seen_sources.insert(source.to_lowercase()) {
                    exact.push(Replacement {
                        source: source.clone(),
                        canonical: entry.canonical.clone(),
                    });
                }
            }
            let normalized = normalize_alphanumeric(&entry.canonical);
            if normalized.len() >= 5 {
                fuzzy.push(FuzzyTerm {
                    canonical: entry.canonical.clone(),
                    distinctive: fuzzy_is_distinctive(&entry.canonical, &normalized),
                    normalized,
                    word_count: entry
                        .canonical
                        .split_whitespace()
                        .filter(|word| !word.is_empty())
                        .count()
                        .max(1),
                });
            }
        }
        exact.sort_by(|left, right| right.source.len().cmp(&left.source.len()));
        fuzzy.sort_by(|left, right| right.normalized.len().cmp(&left.normalized.len()));
        Self {
            entries,
            exact,
            fuzzy,
        }
    }

    pub fn entries(&self) -> &[DictionaryEntry] {
        &self.entries
    }

    pub fn canonical_terms(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|entry| entry.canonical.as_str())
    }

    /// Combine the existing Whisper prompt with a bounded vocabulary hint.
    /// The existing prompt is retained verbatim; only the generated suffix is
    /// bounded.
    pub fn initial_prompt(&self, existing: &str) -> String {
        if self.entries.is_empty() {
            return existing.to_string();
        }
        let mut suffix = String::from("Custom vocabulary: ");
        for term in self.canonical_terms() {
            let separator = if suffix.ends_with(": ") { "" } else { ", " };
            if suffix.len() + separator.len() + term.len() + 1 > MAX_PROMPT_VOCABULARY_BYTES {
                break;
            }
            suffix.push_str(separator);
            suffix.push_str(term);
        }
        suffix.push('.');
        if existing.trim().is_empty() {
            suffix
        } else {
            format!("{}\n{}", existing.trim_end(), suffix)
        }
    }

    /// Apply exact aliases case-insensitively and conservative fuzzy matching
    /// to canonical custom terms. Matches must occupy complete word boundaries;
    /// punctuation outside a match is copied unchanged.
    pub fn apply(&self, text: &str) -> String {
        if text.is_empty() || self.entries.is_empty() {
            return text.to_string();
        }

        let mut output = String::with_capacity(text.len());
        let mut cursor = 0;
        while cursor < text.len() {
            let mut best: Option<(usize, &str, bool, usize)> = None;
            if is_start_boundary(text, cursor) {
                for replacement in &self.exact {
                    if let Some(end) = exact_match_end(text, cursor, &replacement.source) {
                        choose_match(&mut best, end, &replacement.canonical, true, 0, cursor);
                    }
                }

                for (end, normalized) in fuzzy_candidates(text, cursor, 3) {
                    for term in &self.fuzzy {
                        let candidate_word_count = normalized_word_count(&text[cursor..end]);
                        if candidate_word_count > term.word_count + 1 {
                            continue;
                        }
                        if term.normalized.len() < 7
                            && candidate_word_count > term.word_count
                            && normalized != term.normalized
                        {
                            continue;
                        }
                        if let Some(distance) =
                            conservative_fuzzy_distance(&normalized, &term.normalized)
                                .filter(|distance| *distance == 0 || term.distinctive)
                        {
                            choose_match(&mut best, end, &term.canonical, false, distance, cursor);
                        }
                    }
                }
            }

            if let Some((end, canonical, _, _)) = best {
                output.push_str(canonical);
                cursor = end;
            } else {
                let character = text[cursor..].chars().next().expect("cursor is in bounds");
                output.push(character);
                cursor += character.len_utf8();
            }
        }
        output
    }
}

fn validate_field(value: &str, line: usize) -> Result<(), DictionaryError> {
    if value.len() > MAX_DICTIONARY_FIELD_BYTES || value.contains(['\r', '\n', '\0']) {
        return Err(DictionaryError::FieldTooLong { line });
    }
    Ok(())
}

fn case_insensitive_equal(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right) || left.to_lowercase() == right.to_lowercase()
}

fn exact_match_end(text: &str, start: usize, source: &str) -> Option<usize> {
    let maximum_characters = source.chars().count().saturating_add(2);
    for (count, (offset, character)) in text[start..].char_indices().enumerate() {
        if count >= maximum_characters {
            break;
        }
        let end = start + offset + character.len_utf8();
        if is_end_boundary(text, end) && case_insensitive_equal(&text[start..end], source) {
            return Some(end);
        }
    }
    None
}

fn normalize_alphanumeric(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn fuzzy_is_distinctive(canonical: &str, normalized: &str) -> bool {
    if normalized.len() < 5 {
        return false;
    }
    let has_internal_uppercase = canonical
        .chars()
        .skip(1)
        .any(|character| character.is_uppercase());
    let has_digit = canonical.chars().any(|character| character.is_numeric());
    let has_multiple_words = canonical.split_whitespace().nth(1).is_some();
    let has_distinctive_ending = normalized
        .chars()
        .next_back()
        .is_some_and(|character| matches!(character, 'q' | 'x' | 'z'));
    has_internal_uppercase || has_digit || has_multiple_words || has_distinctive_ending
}

fn conservative_fuzzy_distance(candidate: &str, canonical: &str) -> Option<usize> {
    if candidate == canonical {
        return Some(0);
    }
    if candidate.is_empty()
        || canonical.is_empty()
        || candidate.chars().next() != canonical.chars().next()
    {
        return None;
    }

    if canonical.len() >= 7 {
        return edit_distance_at_most_one(candidate, canonical).then_some(1);
    }

    // Short terms are especially collision-prone. Permit only one trailing
    // insertion/deletion, which covers a common repeated-final-sound error such
    // as `retext` -> `Retex` without correcting arbitrary nearby words.
    ((candidate.len() + 1 == canonical.len() && canonical.starts_with(candidate))
        || (canonical.len() + 1 == candidate.len() && candidate.starts_with(canonical)))
    .then_some(1)
}

fn edit_distance_at_most_one(left: &str, right: &str) -> bool {
    let left = left.chars().collect::<Vec<_>>();
    let right = right.chars().collect::<Vec<_>>();
    if left.len().abs_diff(right.len()) > 1 {
        return false;
    }
    if left.len() == right.len() {
        return left
            .iter()
            .zip(&right)
            .filter(|(left, right)| left != right)
            .take(2)
            .count()
            <= 1;
    }
    let (shorter, longer) = if left.len() < right.len() {
        (&left, &right)
    } else {
        (&right, &left)
    };
    let (mut short_index, mut long_index, mut skipped) = (0, 0, false);
    while short_index < shorter.len() && long_index < longer.len() {
        if shorter[short_index] == longer[long_index] {
            short_index += 1;
            long_index += 1;
        } else if skipped {
            return false;
        } else {
            skipped = true;
            long_index += 1;
        }
    }
    true
}

fn is_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn is_start_boundary(text: &str, index: usize) -> bool {
    if index == 0 {
        return text[index..].chars().next().is_some_and(is_word_character);
    }
    !text[..index]
        .chars()
        .next_back()
        .is_some_and(is_word_character)
        && text[index..].chars().next().is_some_and(is_word_character)
}

fn is_end_boundary(text: &str, index: usize) -> bool {
    index == text.len() || !text[index..].chars().next().is_some_and(is_word_character)
}

fn fuzzy_candidates(text: &str, start: usize, max_words: usize) -> Vec<(usize, String)> {
    let mut candidates = Vec::new();
    let mut index = start;
    let mut words = 0;
    while index < text.len() && words < max_words {
        let Some(first) = text[index..].chars().next() else {
            break;
        };
        if !first.is_alphanumeric() {
            break;
        }
        while index < text.len() {
            let character = text[index..].chars().next().expect("index is in bounds");
            if !character.is_alphanumeric() {
                break;
            }
            index += character.len_utf8();
        }
        words += 1;
        candidates.push((index, normalize_alphanumeric(&text[start..index])));

        let separator_start = index;
        while index < text.len() {
            let character = text[index..].chars().next().expect("index is in bounds");
            if character.is_whitespace() || character == '-' {
                index += character.len_utf8();
            } else {
                break;
            }
        }
        if index == separator_start || index == text.len() {
            break;
        }
    }
    candidates
}

fn normalized_word_count(value: &str) -> usize {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .count()
}

fn choose_match<'a>(
    best: &mut Option<(usize, &'a str, bool, usize)>,
    end: usize,
    canonical: &'a str,
    exact: bool,
    distance: usize,
    start: usize,
) {
    let replace = match best {
        None => true,
        Some((best_end, _, best_exact, best_distance)) => {
            end - start > *best_end - start
                || (end == *best_end
                    && ((exact && !*best_exact)
                        || (exact == *best_exact && distance < *best_distance)))
        }
    };
    if replace {
        *best = Some((end, canonical, exact, distance));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comments_aliases_and_deduplicates() {
        let dictionary = CustomDictionary::parse(
            "# products\nUltraVox = Ultra Box, ultra box\nRetex\nultravox = Ultra Vox\n",
        )
        .unwrap();
        assert_eq!(dictionary.entries().len(), 2);
        assert_eq!(dictionary.entries()[0].canonical, "UltraVox");
        assert_eq!(
            dictionary.entries()[0].aliases,
            vec!["Ultra Box", "Ultra Vox"]
        );
    }

    #[test]
    fn applies_exact_and_conservative_fuzzy_matches_with_punctuation() {
        let dictionary = CustomDictionary::parse("Retex\nUltraVox = ultra box\n").unwrap();
        assert_eq!(
            dictionary.apply("Try retext, then Ultra Box! ultravox works."),
            "Try Retex, then UltraVox! UltraVox works."
        );
        assert_eq!(
            dictionary.apply("pretext and ultra boxing"),
            "pretext and ultra boxing"
        );
    }

    #[test]
    fn closest_fuzzy_match_wins_for_the_same_span() {
        let dictionary = CustomDictionary::parse("OpenAPI\nOpenAI\nCloudflare\nApple\n").unwrap();
        assert_eq!(
            dictionary.apply("Open AI uses Cloud flare; apply it."),
            "OpenAI uses Cloudflare; apply it."
        );
    }

    #[test]
    fn longest_match_wins() {
        let dictionary = CustomDictionary::parse("Vox = box\nUltraVox = Ultra Box\n").unwrap();
        assert_eq!(dictionary.apply("Ultra Box, box."), "UltraVox, Vox.");
    }

    #[test]
    fn short_terms_do_not_merge_multiple_words_for_a_typo() {
        let dictionary = CustomDictionary::parse("Aprox\nRetex\n").unwrap();
        assert_eq!(dictionary.apply("a pro and retext"), "a pro and Retex");
    }

    #[test]
    fn common_terms_are_not_fuzzy_corrected() {
        let dictionary = CustomDictionary::parse("Cat\nName\nHello\nBusiness\nApple\n").unwrap();
        assert_eq!(
            dictionary.apply("cap names name, hellos, busyness, apply, cello"),
            "cap names Name, hellos, busyness, apply, cello"
        );
    }

    #[test]
    fn aliases_are_case_insensitive_for_unicode() {
        let dictionary = CustomDictionary::parse("Éclair = élán\nẞeta = ßeta").unwrap();
        assert_eq!(dictionary.apply("ÉLÁN and ßeta!"), "Éclair and ẞeta!");
    }

    #[test]
    fn combines_existing_whisper_prompt_with_canonical_terms() {
        let dictionary = CustomDictionary::parse("Retex = retext\nUltraVox\n").unwrap();
        assert_eq!(
            dictionary.initial_prompt("Use sentence case."),
            "Use sentence case.\nCustom vocabulary: Retex, UltraVox."
        );
    }

    #[test]
    fn rejects_oversized_fields_and_entry_counts() {
        let oversized = "x".repeat(MAX_DICTIONARY_FIELD_BYTES + 1);
        assert!(matches!(
            CustomDictionary::parse(&oversized),
            Err(DictionaryError::FieldTooLong { line: 1 })
        ));
        let source = (0..=MAX_DICTIONARY_ENTRIES)
            .map(|index| format!("Term{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(matches!(
            CustomDictionary::parse(&source),
            Err(DictionaryError::TooManyEntries)
        ));
    }

    #[test]
    fn alias_limit_applies_across_repeated_canonical_lines() {
        let first = (0..8)
            .map(|index| format!("a{index}"))
            .collect::<Vec<_>>()
            .join(",");
        let second = (8..17)
            .map(|index| format!("a{index}"))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(
            CustomDictionary::parse(&format!("Retex = {first}\nretex = {second}")).unwrap_err(),
            DictionaryError::TooManyAliases { line: 2 }
        );
    }
}
