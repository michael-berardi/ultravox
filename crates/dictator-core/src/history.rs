use chrono::{DateTime, Utc};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;
use uuid::Uuid;

/// Lifecycle status of a recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordingStatus {
    Pending,
    Converting,
    Transcribing,
    Completed,
    Failed,
    Cancelled,
}

impl RecordingStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RecordingStatus::Pending => "pending",
            RecordingStatus::Converting => "converting",
            RecordingStatus::Transcribing => "transcribing",
            RecordingStatus::Completed => "completed",
            RecordingStatus::Failed => "failed",
            RecordingStatus::Cancelled => "cancelled",
        }
    }
}

impl TryFrom<&str> for RecordingStatus {
    type Error = HistoryError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "pending" => Ok(RecordingStatus::Pending),
            "converting" => Ok(RecordingStatus::Converting),
            "transcribing" => Ok(RecordingStatus::Transcribing),
            "completed" => Ok(RecordingStatus::Completed),
            "failed" => Ok(RecordingStatus::Failed),
            "cancelled" => Ok(RecordingStatus::Cancelled),
            other => Err(HistoryError::UnknownStatus(other.to_string())),
        }
    }
}

/// A single persisted recording row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingRow {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub file_name: String,
    pub title: String,
    pub preview: String,
    pub transcription: String,
    pub language: String,
    pub duration_seconds: f64,
    pub status: RecordingStatus,
    pub progress: f32,
    pub source_file_url: Option<String>,
}

impl RecordingRow {
    /// Recompute title and preview from the current status and transcription.
    pub fn refresh_display(&mut self) {
        self.title = format_title(&self.transcription, &self.timestamp, &self.file_name);
        self.preview = format_preview(self.status, &self.transcription);
    }
}

fn format_title(transcription: &str, timestamp: &DateTime<Utc>, file_name: &str) -> String {
    let clean = transcription.trim();
    if !clean.is_empty() {
        let first_sentence = clean
            .split(|c| c == '.' || c == '!' || c == '?')
            .next()
            .unwrap_or(clean)
            .replace('\n', " ");
        let trimmed = first_sentence.trim();
        if !trimmed.is_empty() {
            let words: Vec<&str> = trimmed.split_whitespace().collect();
            let short = if words.len() > 6 {
                words[..6].join(" ") + " …"
            } else {
                trimmed.to_string()
            };
            return truncate(&short, 60);
        }
    }
    let fallback = file_name
        .trim_end_matches(".wav")
        .replace(|c: char| !c.is_alphanumeric() && c != ' ', " ");
    if !fallback.trim().is_empty() {
        return fallback;
    }
    timestamp.format("Recording %Y-%m-%d %H:%M").to_string()
}

fn format_preview(status: RecordingStatus, transcription: &str) -> String {
    match status {
        RecordingStatus::Pending | RecordingStatus::Converting | RecordingStatus::Transcribing => {
            "Transcription in progress…".to_string()
        }
        RecordingStatus::Failed => "Transcription failed. Tap Retry to try again.".to_string(),
        RecordingStatus::Cancelled => "Transcription cancelled.".to_string(),
        RecordingStatus::Completed => {
            let clean = transcription.trim().replace('\n', " ");
            if clean.is_empty() {
                "No speech detected.".to_string()
            } else {
                truncate(&clean, 140)
            }
        }
    }
}

fn truncate(text: &str, max_len: usize) -> String {
    if text.chars().count() <= max_len {
        text.to_string()
    } else {
        text.chars().take(max_len).collect::<String>() + "…"
    }
}

/// Errors from the recording history subsystem.
#[derive(Debug, Error)]
pub enum HistoryError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("missing application directory")]
    MissingAppDir,
    #[error("unknown status: {0}")]
    UnknownStatus(String),
    #[error("missing source file")]
    MissingSourceFile,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// SQLite-backed recording history.
#[derive(Debug)]
pub struct RecordingHistory {
    conn: rusqlite::Connection,
}

impl RecordingHistory {
    const DB_FILE_NAME: &'static str = "recordings.sqlite";

    /// Open the history database in the given data directory and run migrations.
    pub fn new(dir: PathBuf) -> Result<Self, HistoryError> {
        std::fs::create_dir_all(&dir).map_err(|e| {
            HistoryError::Database(rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
        })?;
        let db_path = dir.join(Self::DB_FILE_NAME);
        let conn = rusqlite::Connection::open(db_path)?;
        let mut history = Self { conn };
        history.migrate()?;
        Ok(history)
    }

    /// Create an in-memory history database for tests.
    pub fn new_in_memory() -> Result<Self, HistoryError> {
        let conn = rusqlite::Connection::open_in_memory().map_err(HistoryError::Database)?;
        let mut history = Self { conn };
        history.migrate()?;
        Ok(history)
    }

    fn migrate(&mut self) -> Result<(), HistoryError> {
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS recordings (
                id TEXT PRIMARY KEY,
                timestamp TEXT NOT NULL,
                file_name TEXT NOT NULL,
                title TEXT NOT NULL DEFAULT '',
                preview TEXT NOT NULL DEFAULT '',
                transcription TEXT NOT NULL,
                language TEXT NOT NULL DEFAULT 'en',
                duration_seconds REAL NOT NULL,
                status TEXT NOT NULL,
                progress REAL NOT NULL,
                source_file_url TEXT
            )",
            [],
        )?;
        // v2 migration: ensure status/progress/source_file_url exist.
        self.add_column_if_missing("recordings", "status", "TEXT NOT NULL DEFAULT 'completed'")?;
        self.add_column_if_missing("recordings", "progress", "REAL NOT NULL DEFAULT 1.0")?;
        self.add_column_if_missing("recordings", "source_file_url", "TEXT")?;
        // v3 migration: ensure title/preview/language exist.
        self.add_column_if_missing("recordings", "title", "TEXT NOT NULL DEFAULT ''")?;
        self.add_column_if_missing("recordings", "preview", "TEXT NOT NULL DEFAULT ''")?;
        self.add_column_if_missing("recordings", "language", "TEXT NOT NULL DEFAULT 'en'")?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_recordings_timestamp ON recordings(timestamp)",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_recordings_status ON recordings(status)",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_recordings_title ON recordings(title)",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_recordings_preview ON recordings(preview)",
            [],
        )?;
        Ok(())
    }

    fn add_column_if_missing(
        &self,
        table: &str,
        column: &str,
        def: &str,
    ) -> Result<(), HistoryError> {
        if self.column_exists(table, column)? {
            return Ok(());
        }
        let sql = format!("ALTER TABLE {table} ADD COLUMN {column} {def}");
        self.conn.execute(&sql, []).map_err(HistoryError::from)?;
        Ok(())
    }

    fn column_exists(&self, table: &str, column: &str) -> Result<bool, HistoryError> {
        let mut stmt = self
            .conn
            .prepare("SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2")?;
        let found: Option<i32> = stmt
            .query_row([table, column], |row| row.get(0))
            .optional()?;
        Ok(found.is_some())
    }

    /// Insert a new recording row.
    pub fn insert(&mut self, row: &RecordingRow) -> Result<(), HistoryError> {
        self.conn.execute(
            "INSERT INTO recordings
             (id, timestamp, file_name, title, preview, transcription, language, duration_seconds, status, progress, source_file_url)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
             timestamp = excluded.timestamp,
             file_name = excluded.file_name,
             title = excluded.title,
             preview = excluded.preview,
             transcription = excluded.transcription,
             language = excluded.language,
             duration_seconds = excluded.duration_seconds,
             status = excluded.status,
             progress = excluded.progress,
             source_file_url = excluded.source_file_url",
            (
                row.id.to_string(),
                row.timestamp.to_rfc3339(),
                &row.file_name,
                &row.title,
                &row.preview,
                &row.transcription,
                &row.language,
                row.duration_seconds,
                row.status.as_str(),
                row.progress,
                row.source_file_url.as_deref(),
            ),
        )?;
        Ok(())
    }

    /// Fetch a recording by id.
    pub fn get(&self, id: Uuid) -> Result<Option<RecordingRow>, HistoryError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, file_name, title, preview, transcription, language, duration_seconds, status, progress, source_file_url
             FROM recordings WHERE id = ?1"
        )?;
        Ok(stmt
            .query_row([id.to_string()], |row| self.map_row(row))
            .optional()?)
    }

    /// List recordings ordered by timestamp descending.
    pub fn list(&self, limit: usize, offset: usize) -> Result<Vec<RecordingRow>, HistoryError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, file_name, title, preview, transcription, language, duration_seconds, status, progress, source_file_url
             FROM recordings ORDER BY timestamp DESC LIMIT ?1 OFFSET ?2"
        )?;
        let rows = stmt.query_map([limit as i64, offset as i64], |row| self.map_row(row))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(HistoryError::from)
    }

    /// Delete a recording by id.
    pub fn delete(&mut self, id: Uuid) -> Result<usize, HistoryError> {
        let count = self
            .conn
            .execute("DELETE FROM recordings WHERE id = ?1", [id.to_string()])?;
        Ok(count)
    }

    /// List every recording ordered by timestamp descending.
    pub fn list_all(&self) -> Result<Vec<RecordingRow>, HistoryError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, file_name, title, preview, transcription, language, duration_seconds, status, progress, source_file_url
             FROM recordings ORDER BY timestamp DESC"
        )?;
        let rows = stmt.query_map([], |row| self.map_row(row))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(HistoryError::from)
    }

    /// Delete every persisted recording row.
    pub fn delete_all(&mut self) -> Result<usize, HistoryError> {
        self.conn
            .execute("DELETE FROM recordings", [])
            .map_err(HistoryError::from)
    }

    /// Find recordings whose title, preview, file name, or transcription contains the query (case-insensitive).
    pub fn search(
        &self,
        query: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<RecordingRow>, HistoryError> {
        let pattern = format!("%{query}%");
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, file_name, title, preview, transcription, language, duration_seconds, status, progress, source_file_url
             FROM recordings
             WHERE file_name LIKE ?1 OR title LIKE ?1 OR preview LIKE ?1 OR transcription LIKE ?1
             ORDER BY timestamp DESC LIMIT ?2 OFFSET ?3"
        )?;
        let rows = stmt.query_map(
            rusqlite::params![pattern, limit as i64, offset as i64],
            |row| self.map_row(row),
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(HistoryError::from)
    }

    /// Update the transcription and status of an existing recording row.
    pub fn update(
        &mut self,
        id: Uuid,
        transcription: String,
        status: RecordingStatus,
        progress: f32,
    ) -> Result<(), HistoryError> {
        self.conn.execute(
            "UPDATE recordings
             SET transcription = ?2, status = ?3, progress = ?4
             WHERE id = ?1",
            (id.to_string(), transcription, status.as_str(), progress),
        )?;
        Ok(())
    }

    /// Retry a failed recording by resetting it to pending.
    pub fn retry(&mut self, id: Uuid) -> Result<(), HistoryError> {
        self.conn.execute(
            "UPDATE recordings
             SET status = ?2, progress = 0.0
             WHERE id = ?1",
            (id.to_string(), RecordingStatus::Pending.as_str()),
        )?;
        Ok(())
    }

    /// Export the recording's source file to the given destination path.
    /// Returns the destination path on success.
    pub fn export(&self, id: Uuid, destination: PathBuf) -> Result<PathBuf, HistoryError> {
        let row = self
            .get(id)?
            .ok_or_else(|| HistoryError::UnknownStatus(id.to_string()))?;
        if let Some(source) = row.source_file_url {
            let source_path = PathBuf::from(source);
            std::fs::copy(&source_path, &destination)?;
            Ok(destination)
        } else {
            Err(HistoryError::MissingSourceFile)
        }
    }
    pub fn pending(&self) -> Result<Vec<RecordingRow>, HistoryError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, file_name, title, preview, transcription, language, duration_seconds, status, progress, source_file_url
             FROM recordings
             WHERE status IN ('pending', 'converting', 'transcribing')
             ORDER BY timestamp ASC"
        )?;
        let rows = stmt.query_map([], |row| self.map_row(row))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(HistoryError::from)
    }

    fn map_row(&self, row: &rusqlite::Row) -> Result<RecordingRow, rusqlite::Error> {
        Ok(RecordingRow {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
            timestamp: DateTime::parse_from_rfc3339(&row.get::<_, String>(1)?)
                .map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?
                .with_timezone(&Utc),
            file_name: row.get(2)?,
            title: row.get(3)?,
            preview: row.get(4)?,
            transcription: row.get(5)?,
            language: row.get(6)?,
            duration_seconds: row.get(7)?,
            status: RecordingStatus::try_from(row.get::<_, String>(8)?.as_str()).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    8,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
            progress: row.get(9)?,
            source_file_url: row.get(10)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_memory_history() -> RecordingHistory {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let mut history = RecordingHistory { conn };
        history.migrate().unwrap();
        history
    }

    fn sample_row() -> RecordingRow {
        RecordingRow {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            file_name: "test.wav".to_string(),
            title: "Test recording".to_string(),
            preview: "hello world".to_string(),
            transcription: "hello world".to_string(),
            language: "en".to_string(),
            duration_seconds: 1.5,
            status: RecordingStatus::Completed,
            progress: 1.0,
            source_file_url: None,
        }
    }

    #[test]
    fn insert_and_get_roundtrip() {
        let mut history = in_memory_history();
        let row = sample_row();
        history.insert(&row).unwrap();
        let fetched = history.get(row.id).unwrap().unwrap();
        assert_eq!(fetched.file_name, row.file_name);
        assert_eq!(fetched.transcription, row.transcription);
    }

    #[test]
    fn pending_filtering() {
        let mut history = in_memory_history();
        let mut row = sample_row();
        row.status = RecordingStatus::Transcribing;
        row.progress = 0.5;
        history.insert(&row).unwrap();
        let pending = history.pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, row.id);
    }

    #[test]
    fn search_finds_by_file_name() {
        let mut history = in_memory_history();
        let row = sample_row();
        history.insert(&row).unwrap();
        let found = history.search("smoke", 10, 0).unwrap();
        assert_eq!(found.len(), 0);
        let found = history.search("test", 10, 0).unwrap();
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn update_changes_transcription() {
        let mut history = in_memory_history();
        let row = sample_row();
        history.insert(&row).unwrap();
        history
            .update(row.id, "updated".to_string(), RecordingStatus::Failed, 0.0)
            .unwrap();
        let fetched = history.get(row.id).unwrap().unwrap();
        assert_eq!(fetched.transcription, "updated");
        assert_eq!(fetched.status, RecordingStatus::Failed);
    }

    #[test]
    fn retry_resets_to_pending() {
        let mut history = in_memory_history();
        let mut row = sample_row();
        row.status = RecordingStatus::Failed;
        history.insert(&row).unwrap();
        history.retry(row.id).unwrap();
        let fetched = history.get(row.id).unwrap().unwrap();
        assert_eq!(fetched.status, RecordingStatus::Pending);
    }

    #[test]
    fn search_finds_by_title_and_preview() {
        let mut history = in_memory_history();
        let mut row = sample_row();
        row.title = "meeting notes".to_string();
        row.preview = "action items from today".to_string();
        row.transcription = "action items from today".to_string();
        history.insert(&row).unwrap();

        let by_title = history.search("meeting", 10, 0).unwrap();
        assert_eq!(by_title.len(), 1);

        let by_preview = history.search("action items", 10, 0).unwrap();
        assert_eq!(by_preview.len(), 1);
    }

    #[test]
    fn title_and_preview_generated_from_transcription() {
        let mut row = sample_row();
        row.transcription = "Hello world this is a test of the transcription preview.".to_string();
        row.status = RecordingStatus::Completed;
        row.refresh_display();

        assert!(row.title.starts_with("Hello world this is a"));
        assert!(row.preview.contains("transcription preview"));
    }

    #[test]
    fn pending_preview_has_fallback() {
        let mut row = sample_row();
        row.transcription = "".to_string();
        row.status = RecordingStatus::Pending;
        row.refresh_display();
        assert_eq!(row.preview, "Transcription in progress…");
    }

    #[test]
    fn cancelled_status_round_trip() {
        let mut history = in_memory_history();
        let mut row = sample_row();
        row.status = RecordingStatus::Cancelled;
        row.transcription = "".to_string();
        history.insert(&row).unwrap();
        let fetched = history.get(row.id).unwrap().unwrap();
        assert_eq!(fetched.status, RecordingStatus::Cancelled);
        assert_eq!(fetched.status.as_str(), "cancelled");
    }

    #[test]
    fn cancelled_preview_has_fallback() {
        let mut row = sample_row();
        row.transcription = "".to_string();
        row.status = RecordingStatus::Cancelled;
        row.refresh_display();
        assert_eq!(row.preview, "Transcription cancelled.");
    }

    #[test]
    fn cancelled_status_parses_from_str() {
        assert_eq!(
            RecordingStatus::try_from("cancelled").unwrap(),
            RecordingStatus::Cancelled
        );
    }
    #[test]
    fn delete_all_removes_every_recording() {
        let mut history = in_memory_history();
        history.insert(&sample_row()).unwrap();
        history.insert(&sample_row()).unwrap();

        assert_eq!(history.list_all().unwrap().len(), 2);
        assert_eq!(history.delete_all().unwrap(), 2);
        assert!(history.list_all().unwrap().is_empty());
    }
}
