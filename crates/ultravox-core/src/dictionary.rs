//! Small, in-memory custom dictionary parsing and transcript correction.
//!
//! The format is deliberately manual: one canonical term per line, optionally
//! followed by ` = alias one, alias two`. Blank lines and comment lines are
//! ignored. No files, contacts, or external services are inspected.

use std::collections::{HashMap, HashSet};
use thiserror::Error;

pub const MAX_DICTIONARY_BYTES: usize = 128 * 1024;
pub const MAX_DICTIONARY_ENTRIES: usize = 512;
pub const MAX_DICTIONARY_FIELD_BYTES: usize = 128;
pub const MAX_ALIASES_PER_ENTRY: usize = 16;
const MAX_PROMPT_VOCABULARY_BYTES: usize = 2_048;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DictionaryError {
    #[error("dictionary is larger than {MAX_DICTIONARY_BYTES} bytes")]
    SourceTooLarge,
    #[error("dictionary has more than {MAX_DICTIONARY_ENTRIES} entries")]
    TooManyEntries,
    #[error("dictionary line {line} has an empty canonical term")]
    EmptyCanonical { line: usize },
    #[error(
        "dictionary line {line} contains a field larger than {MAX_DICTIONARY_FIELD_BYTES} bytes or an invalid control character"
    )]
    FieldTooLong { line: usize },
    #[error("dictionary line {line} has more than {MAX_ALIASES_PER_ENTRY} aliases")]
    TooManyAliases { line: usize },
}

#[derive(Debug, Clone)]
struct Entry {
    canonical: String,
    aliases: Vec<String>,
}

#[derive(Debug, Clone)]
struct Pattern {
    literal: String,
    canonical: String,
    character_count: usize,
}

#[derive(Debug, Clone, Default)]
pub struct CustomDictionary {
    entries: Vec<Entry>,
    patterns: Vec<Pattern>,
    patterns_by_initial: HashMap<char, Vec<usize>>,
}

#[derive(Debug, Clone)]
struct Replacement {
    start: usize,
    end: usize,
    canonical: String,
    exact: bool,
    distance: usize,
}

#[derive(Debug, Clone, Copy)]
struct WordSpan {
    start: usize,
    end: usize,
}

impl CustomDictionary {
    pub fn parse(source: &str) -> Result<Self, DictionaryError> {
        if source.len() > MAX_DICTIONARY_BYTES {
            return Err(DictionaryError::SourceTooLarge);
        }

        let mut entries: Vec<Entry> = Vec::new();
        let mut entry_indices = HashMap::<String, usize>::new();

        for (index, raw_line) in source.lines().enumerate() {
            let line_number = index + 1;
            if raw_line.contains(['\r', '\0']) {
                return Err(DictionaryError::FieldTooLong { line: line_number });
            }
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
                continue;
            }

            let (canonical, aliases) = match line.split_once('=') {
                Some((canonical, aliases)) => (canonical.trim(), Some(aliases)),
                None => (line, None),
            };
            if canonical.is_empty() {
                return Err(DictionaryError::EmptyCanonical { line: line_number });
            }
            validate_field(canonical, line_number)?;
            let canonical_key = canonical.to_lowercase();
            let entry_index = if let Some(existing) = entry_indices.get(&canonical_key) {
                *existing
            } else {
                if entries.len() >= MAX_DICTIONARY_ENTRIES {
                    return Err(DictionaryError::TooManyEntries);
                }
                let next = entries.len();
                entries.push(Entry {
                    canonical: canonical.to_string(),
                    aliases: Vec::new(),
                });
                entry_indices.insert(canonical_key, next);
                next
            };

            if let Some(aliases) = aliases {
                let mut seen = entries[entry_index]
                    .aliases
                    .iter()
                    .map(|alias| alias.to_lowercase())
                    .collect::<HashSet<_>>();
                for alias in aliases.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    validate_field(alias, line_number)?;
                    let key = alias.to_lowercase();
                    if key == entries[entry_index].canonical.to_lowercase() || !seen.insert(key) {
                        continue;
                    }
                    if entries[entry_index].aliases.len() >= MAX_ALIASES_PER_ENTRY {
                        return Err(DictionaryError::TooManyAliases { line: line_number });
                    }
                    entries[entry_index].aliases.push(alias.to_string());
                }
            }
        }

        // Canonical spellings take precedence over an identical alias assigned
        // elsewhere. The first occurrence of any other duplicate alias wins.
        let mut patterns = Vec::new();
        let mut seen_patterns = HashSet::new();
        for entry in &entries {
            let key = entry.canonical.to_lowercase();
            if seen_patterns.insert(key) {
                patterns.push(Pattern {
                    literal: entry.canonical.clone(),
                    canonical: entry.canonical.clone(),
                    character_count: entry.canonical.chars().count(),
                });
            }
        }
        for entry in &entries {
            for alias in &entry.aliases {
                let key = alias.to_lowercase();
                if seen_patterns.insert(key) {
                    patterns.push(Pattern {
                        literal: alias.clone(),
                        canonical: entry.canonical.clone(),
                        character_count: alias.chars().count(),
                    });
                }
            }
        }

        let mut patterns_by_initial = HashMap::<char, Vec<usize>>::new();
        for (index, pattern) in patterns.iter().enumerate() {
            if let Some(initial) = folded_initial(&pattern.literal) {
                patterns_by_initial.entry(initial).or_default().push(index);
            }
        }

        Ok(Self {
            entries,
            patterns,
            patterns_by_initial,
        })
    }

    pub fn canonical_terms(&self) -> Vec<&str> {
        self.entries
            .iter()
            .map(|entry| entry.canonical.as_str())
            .collect()
    }

    /// Combine the user's existing Whisper prompt with a bounded vocabulary
    /// hint. FluidAudio does not expose a prompt, but uses the same corrections.
    pub fn combined_initial_prompt(&self, existing: &str) -> String {
        let existing = existing.trim();
        if self.entries.is_empty() {
            return existing.to_string();
        }
        let mut suffix = String::from("Preferred terms: ");
        for entry in &self.entries {
            let separator = if suffix.ends_with(": ") { "" } else { ", " };
            if suffix.len() + separator.len() + entry.canonical.len() + 1
                > MAX_PROMPT_VOCABULARY_BYTES
            {
                break;
            }
            suffix.push_str(separator);
            suffix.push_str(&entry.canonical);
        }
        suffix.push('.');
        if existing.is_empty() {
            suffix
        } else {
            format!("{existing}\n{suffix}")
        }
    }

    pub fn apply(&self, text: &str) -> String {
        if text.is_empty() || self.entries.is_empty() {
            return text.to_string();
        }

        let mut replacements = self.exact_replacements(text);
        replacements.extend(self.fuzzy_replacements(text));
        replacements.sort_by(|left, right| {
            left.start
                .cmp(&right.start)
                .then_with(|| (right.end - right.start).cmp(&(left.end - left.start)))
                .then_with(|| right.exact.cmp(&left.exact))
                .then_with(|| left.distance.cmp(&right.distance))
        });

        let mut output = String::with_capacity(text.len());
        let mut cursor = 0;
        for replacement in replacements {
            if replacement.start < cursor {
                continue;
            }
            output.push_str(&text[cursor..replacement.start]);
            output.push_str(&replacement.canonical);
            cursor = replacement.end;
        }
        output.push_str(&text[cursor..]);
        output
    }

    fn exact_replacements(&self, text: &str) -> Vec<Replacement> {
        let mut matches = Vec::new();
        for (start, character) in text.char_indices() {
            let initial = character.to_lowercase().next().unwrap_or(character);
            let Some(pattern_indices) = self.patterns_by_initial.get(&initial) else {
                continue;
            };
            for pattern_index in pattern_indices {
                let pattern = &self.patterns[*pattern_index];
                let Some(end) = end_after_characters(text, start, pattern.character_count) else {
                    continue;
                };
                let candidate = &text[start..end];
                if case_insensitive_eq(candidate, &pattern.literal)
                    && has_term_boundaries(text, start, end, &pattern.literal)
                {
                    matches.push(Replacement {
                        start,
                        end,
                        canonical: pattern.canonical.clone(),
                        exact: true,
                        distance: 0,
                    });
                }
            }
        }
        matches
    }

    fn fuzzy_replacements(&self, text: &str) -> Vec<Replacement> {
        let words = word_spans(text);
        let mut matches = Vec::new();
        for entry in &self.entries {
            let canonical = alphanumeric_key(&entry.canonical);
            if canonical.len() < 5 {
                continue;
            }
            let canonical_words = word_spans(&entry.canonical).len().max(1);
            let max_words = (canonical_words + 1).min(3);

            for start_index in 0..words.len() {
                let mut candidate = String::new();
                for count in 1..=max_words {
                    let end_index = start_index + count - 1;
                    let Some(word) = words.get(end_index) else {
                        break;
                    };
                    if end_index > start_index {
                        let previous = words[end_index - 1];
                        if !text[previous.end..word.start]
                            .chars()
                            .all(|character| character.is_whitespace() || character == '-')
                        {
                            break;
                        }
                    }
                    candidate.extend(
                        text[word.start..word.end]
                            .chars()
                            .flat_map(char::to_lowercase),
                    );
                    let canonical_len = canonical.len();
                    let candidate_len = candidate.len();
                    if candidate_len + 1 < canonical_len {
                        continue;
                    }
                    if candidate_len > canonical_len + 1 {
                        break;
                    }
                    if canonical_len < 7 && count > canonical_words && candidate != canonical {
                        continue;
                    }
                    if let Some(distance) = conservative_fuzzy_distance(&candidate, &canonical)
                        .filter(|distance| {
                            *distance == 0
                                || is_distinctive_fuzzy_term(&entry.canonical, &canonical)
                        })
                    {
                        matches.push(Replacement {
                            start: words[start_index].start,
                            end: word.end,
                            canonical: entry.canonical.clone(),
                            exact: false,
                            distance,
                        });
                    }
                }
            }
        }
        matches
    }
}

fn folded_initial(value: &str) -> Option<char> {
    value.chars().next()?.to_lowercase().next()
}

fn end_after_characters(text: &str, start: usize, count: usize) -> Option<usize> {
    let mut end = start;
    let mut characters = text[start..].chars();
    for _ in 0..count {
        end += characters.next()?.len_utf8();
    }
    Some(end)
}

fn case_insensitive_eq(left: &str, right: &str) -> bool {
    left.chars()
        .flat_map(char::to_lowercase)
        .eq(right.chars().flat_map(char::to_lowercase))
}

fn validate_field(value: &str, line: usize) -> Result<(), DictionaryError> {
    if value.len() > MAX_DICTIONARY_FIELD_BYTES || value.contains(['\r', '\n', '\0']) {
        return Err(DictionaryError::FieldTooLong { line });
    }
    Ok(())
}

fn has_term_boundaries(text: &str, start: usize, end: usize, term: &str) -> bool {
    let begins_with_word = term.chars().next().is_some_and(char::is_alphanumeric);
    let ends_with_word = term.chars().next_back().is_some_and(char::is_alphanumeric);
    let left_is_word = text[..start]
        .chars()
        .next_back()
        .is_some_and(char::is_alphanumeric);
    let right_is_word = text[end..]
        .chars()
        .next()
        .is_some_and(char::is_alphanumeric);
    (!begins_with_word || !left_is_word) && (!ends_with_word || !right_is_word)
}

fn word_spans(text: &str) -> Vec<WordSpan> {
    let mut spans = Vec::new();
    let mut start = None;
    for (index, character) in text.char_indices() {
        if character.is_alphanumeric() {
            start.get_or_insert(index);
        } else if let Some(word_start) = start.take() {
            spans.push(WordSpan {
                start: word_start,
                end: index,
            });
        }
    }
    if let Some(word_start) = start {
        spans.push(WordSpan {
            start: word_start,
            end: text.len(),
        });
    }
    spans
}

fn alphanumeric_key(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
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
        return edit_distance_at_most_one(candidate.as_bytes(), canonical.as_bytes()).then_some(1);
    }

    // Short terms collide easily. Only permit one trailing insertion or
    // deletion, such as `retext` -> `Retex`.
    ((candidate.len() + 1 == canonical.len() && canonical.starts_with(candidate))
        || (canonical.len() + 1 == candidate.len() && candidate.starts_with(canonical)))
    .then_some(1)
}

fn edit_distance_at_most_one(left: &[u8], right: &[u8]) -> bool {
    if left.len().abs_diff(right.len()) > 1 {
        return false;
    }
    if left.len() == right.len() {
        return left
            .iter()
            .zip(right)
            .filter(|(left, right)| left != right)
            .take(2)
            .count()
            <= 1;
    }
    let (shorter, longer) = if left.len() < right.len() {
        (left, right)
    } else {
        (right, left)
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

fn is_distinctive_fuzzy_term(source: &str, canonical: &str) -> bool {
    let has_internal_uppercase = source
        .chars()
        .skip(1)
        .any(|character| character.is_uppercase());
    let has_digit = source.chars().any(|character| character.is_numeric());
    let has_multiple_words = source.split_whitespace().nth(1).is_some();
    let has_distinctive_ending = canonical
        .chars()
        .next_back()
        .is_some_and(|character| matches!(character, 'q' | 'x' | 'z'));
    has_internal_uppercase || has_digit || has_multiple_words || has_distinctive_ending
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comments_blanks_and_deduplicates() {
        let dictionary = CustomDictionary::parse(
            "# local terms\nRetex = re tex, re tex\n\nretex = Retext\n// ignored\nUltraVox = Ultra Box\n",
        )
        .unwrap();
        assert_eq!(dictionary.canonical_terms(), vec!["Retex", "UltraVox"]);
        assert_eq!(
            dictionary.apply("re tex, Retext, and ultra box."),
            "Retex, Retex, and UltraVox."
        );
    }

    #[test]
    fn applies_case_insensitively_with_longest_match_and_keeps_punctuation() {
        let dictionary = CustomDictionary::parse("Vox\nUltraVox = Ultra Box").unwrap();
        assert_eq!(
            dictionary.apply("ULTRA BOX, vox! ultraboxed?"),
            "UltraVox, Vox! ultraboxed?"
        );
    }

    #[test]
    fn fuzzy_matching_is_conservative_for_distinctive_terms() {
        let dictionary = CustomDictionary::parse("Retex\nUltraVox\nModel007\nHello").unwrap();
        assert_eq!(
            dictionary.apply("retext, Ultra Box, Model008, cello, hellos, and hello."),
            "Retex, UltraVox, Model007, cello, hellos, and Hello."
        );
    }

    #[test]
    fn fuzzy_distinctiveness_matches_pro_rules() {
        assert!(is_distinctive_fuzzy_term("UltraVox", "ultravox"));
        assert!(is_distinctive_fuzzy_term("Model007", "model007"));
        assert!(is_distinctive_fuzzy_term("Alpha Beta", "alphabeta"));
        assert!(is_distinctive_fuzzy_term("Retex", "retex"));
        assert!(!is_distinctive_fuzzy_term("Business", "business"));
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
    fn short_terms_do_not_merge_multiple_words_for_a_typo() {
        let dictionary = CustomDictionary::parse("Aprox\nRetex\n").unwrap();
        assert_eq!(dictionary.apply("a pro and retext"), "a pro and Retex");
    }

    #[test]
    fn common_long_terms_are_not_fuzzy_corrected() {
        let dictionary = CustomDictionary::parse("Business\n").unwrap();
        assert_eq!(
            dictionary.apply("busyness and business"),
            "busyness and Business"
        );
    }

    #[test]
    fn multiword_terms_are_distinctive_but_exact_alias_spacing_is_preserved() {
        let dictionary =
            CustomDictionary::parse("Alpha Beta\nAcme = Acme  Incorporated\n").unwrap();
        assert_eq!(
            dictionary.apply("Alpha Beto; Acme Incorporated; Acme  Incorporated"),
            "Alpha Beta; Acme Incorporated; Acme"
        );
    }

    #[test]
    fn aliases_are_case_insensitive_for_unicode() {
        let dictionary = CustomDictionary::parse("Éclair = élán").unwrap();
        assert_eq!(dictionary.apply("ÉLÁN!"), "Éclair!");
    }

    #[test]
    fn combines_canonical_terms_with_existing_prompt() {
        let dictionary = CustomDictionary::parse("Retex = retext\nUltraVox").unwrap();
        assert_eq!(
            dictionary.combined_initial_prompt("Use sentence case."),
            "Use sentence case.\nPreferred terms: Retex, UltraVox."
        );
    }

    #[test]
    fn enforces_pro_compatible_source_field_and_entry_bounds() {
        let max_source = format!("#{}", "x".repeat(MAX_DICTIONARY_BYTES - 1));
        CustomDictionary::parse(&max_source).unwrap();
        assert_eq!(
            CustomDictionary::parse(&(max_source + "x")).unwrap_err(),
            DictionaryError::SourceTooLarge
        );

        CustomDictionary::parse(&"x".repeat(MAX_DICTIONARY_FIELD_BYTES)).unwrap();
        assert!(matches!(
            CustomDictionary::parse(&"x".repeat(MAX_DICTIONARY_FIELD_BYTES + 1)),
            Err(DictionaryError::FieldTooLong { line: 1 })
        ));
        assert!(matches!(
            CustomDictionary::parse(&"é".repeat(MAX_DICTIONARY_FIELD_BYTES / 2 + 1)),
            Err(DictionaryError::FieldTooLong { line: 1 })
        ));

        let max_entries = (0..MAX_DICTIONARY_ENTRIES)
            .map(|index| format!("Term{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        CustomDictionary::parse(&max_entries).unwrap();
        assert!(matches!(
            CustomDictionary::parse(&format!("{max_entries}\nOneTooMany")),
            Err(DictionaryError::TooManyEntries)
        ));
    }

    #[test]
    fn rejects_empty_or_control_character_fields() {
        assert_eq!(
            CustomDictionary::parse("= alias").unwrap_err(),
            DictionaryError::EmptyCanonical { line: 1 }
        );
        assert_eq!(
            CustomDictionary::parse("Bad\0Term").unwrap_err(),
            DictionaryError::FieldTooLong { line: 1 }
        );
        assert_eq!(
            CustomDictionary::parse("Bad\rTerm").unwrap_err(),
            DictionaryError::FieldTooLong { line: 1 }
        );
        assert_eq!(
            CustomDictionary::parse("# ignored\0control").unwrap_err(),
            DictionaryError::FieldTooLong { line: 1 }
        );
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

    #[test]
    fn generated_prompt_vocabulary_is_bounded() {
        let source = (0..MAX_DICTIONARY_ENTRIES)
            .map(|index| format!("Term{index:03}{}", "x".repeat(80)))
            .collect::<Vec<_>>()
            .join("\n");
        let prompt = CustomDictionary::parse(&source)
            .unwrap()
            .combined_initial_prompt("");
        assert!(prompt.len() <= MAX_PROMPT_VOCABULARY_BYTES);
    }
}
