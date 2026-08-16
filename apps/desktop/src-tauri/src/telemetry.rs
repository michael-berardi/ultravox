use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;
use uuid::Uuid;

const ENDPOINT: &str = "https://analytics.libertydesign.studio/api/app-telemetry/event";
const SCHEMA: &str = "lds.app-telemetry.event.v2";
const APP: &str = "ultravox";
const HTTP_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_COUNTER: u64 = 1_000_000;
const STATE_FILE: &str = "app-telemetry.json";

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConsentState {
    Undecided,
    Accepted,
    Declined,
}

impl Default for ConsentState {
    fn default() -> Self {
        Self::Undecided
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TelemetryStatus {
    pub consent: ConsentState,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct UsageCounters {
    pub recordings_started: u64,
    pub recordings_completed: u64,
    pub recordings_failed: u64,
    pub transcriptions_completed: u64,
    pub transcriptions_failed: u64,
    pub model_downloads_completed: u64,
    pub model_downloads_failed: u64,
}

impl UsageCounters {
    fn is_zero(self) -> bool {
        self == Self::default()
    }

    fn checked(self) -> Result<Self, String> {
        let values = [
            self.recordings_started,
            self.recordings_completed,
            self.recordings_failed,
            self.transcriptions_completed,
            self.transcriptions_failed,
            self.model_downloads_completed,
            self.model_downloads_failed,
        ];
        if values.into_iter().any(|value| value > MAX_COUNTER) {
            return Err("Telemetry counters must be between 0 and 1,000,000.".to_string());
        }
        Ok(self)
    }

    fn add(&mut self, delta: Self) {
        self.recordings_started = bounded_add(self.recordings_started, delta.recordings_started);
        self.recordings_completed =
            bounded_add(self.recordings_completed, delta.recordings_completed);
        self.recordings_failed = bounded_add(self.recordings_failed, delta.recordings_failed);
        self.transcriptions_completed = bounded_add(
            self.transcriptions_completed,
            delta.transcriptions_completed,
        );
        self.transcriptions_failed =
            bounded_add(self.transcriptions_failed, delta.transcriptions_failed);
        self.model_downloads_completed = bounded_add(
            self.model_downloads_completed,
            delta.model_downloads_completed,
        );
        self.model_downloads_failed =
            bounded_add(self.model_downloads_failed, delta.model_downloads_failed);
    }
}

fn bounded_add(current: u64, delta: u64) -> u64 {
    current.saturating_add(delta).min(MAX_COUNTER)
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct StoredState {
    consent: ConsentState,
    install_id: Option<Uuid>,
    last_heartbeat_day: Option<String>,
    pending_usage_by_day: BTreeMap<String, UsageCounters>,
}

fn sanitize_loaded_state(state: &mut StoredState) -> bool {
    let original = serde_json::to_vec(state).ok();
    if state.consent == ConsentState::Accepted {
        if !state.install_id.is_some_and(|id| {
            id.get_version_num() == 4 && id.get_variant() == uuid::Variant::RFC4122
        }) {
            state.install_id = Some(Uuid::new_v4());
        }
    } else {
        state.install_id = None;
        state.last_heartbeat_day = None;
        state.pending_usage_by_day.clear();
    }
    for usage in state.pending_usage_by_day.values_mut() {
        usage.recordings_started = usage.recordings_started.min(MAX_COUNTER);
        usage.recordings_completed = usage.recordings_completed.min(MAX_COUNTER);
        usage.recordings_failed = usage.recordings_failed.min(MAX_COUNTER);
        usage.transcriptions_completed = usage.transcriptions_completed.min(MAX_COUNTER);
        usage.transcriptions_failed = usage.transcriptions_failed.min(MAX_COUNTER);
        usage.model_downloads_completed = usage.model_downloads_completed.min(MAX_COUNTER);
        usage.model_downloads_failed = usage.model_downloads_failed.min(MAX_COUNTER);
    }
    original != serde_json::to_vec(state).ok()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Event<'a> {
    schema: &'static str,
    app: &'static str,
    event: &'a str,
    install_id: Uuid,
    version: &'static str,
    platform: &'static str,
    arch: &'static str,
    day: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<UsageCounters>,
}

#[derive(Debug)]
pub struct Telemetry {
    state: Mutex<StoredState>,
    path: PathBuf,
    launch_sent_this_run: AtomicBool,
}

impl Telemetry {
    pub fn new(app: AppHandle) -> Result<Self, String> {
        let directory = app
            .path()
            .app_data_dir()
            .map_err(|error| error.to_string())?;
        let path = directory.join(STATE_FILE);
        let mut state = match fs::read(&path) {
            Ok(raw) => serde_json::from_slice(&raw).unwrap_or_default(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => StoredState::default(),
            Err(error) => return Err(error.to_string()),
        };
        let changed = sanitize_loaded_state(&mut state);
        let persisted_state = changed.then(|| state.clone());
        let telemetry = Self {
            state: Mutex::new(state),
            path,
            launch_sent_this_run: AtomicBool::new(false),
        };
        if let Some(persisted_state) = persisted_state.as_ref() {
            telemetry.persist(persisted_state)?;
        }
        Ok(telemetry)
    }

    fn persist(&self, state: &StoredState) -> Result<(), String> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| "Telemetry state path has no parent directory.".to_string())?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let temporary = self.path.with_extension("json.tmp");
        fs::write(
            &temporary,
            serde_json::to_vec(state).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        fs::rename(temporary, &self.path).map_err(|error| error.to_string())
    }

    pub async fn status(&self) -> TelemetryStatus {
        let state = self.state.lock().await;
        status(&state)
    }

    pub async fn set_enabled(&self, enabled: bool) -> Result<TelemetryStatus, String> {
        let mut state = self.state.lock().await;
        if enabled {
            state.consent = ConsentState::Accepted;
            if state.install_id.is_none() {
                state.install_id = Some(Uuid::new_v4());
            }
        } else {
            state.consent = ConsentState::Declined;
            state.install_id = None;
            state.last_heartbeat_day = None;
            state.pending_usage_by_day.clear();
        }
        self.persist(&state)?;
        Ok(status(&state))
    }

    pub async fn launch(&self) -> Result<(), String> {
        let state = self.state.lock().await;
        if !enabled(&state) || self.launch_sent_this_run.load(Ordering::Relaxed) {
            return Ok(());
        }
        let today = utc_day();
        if post(&state, "launch", &today, None).await {
            self.launch_sent_this_run.store(true, Ordering::Relaxed);
        }
        Ok(())
    }

    pub async fn heartbeat(&self) -> Result<(), String> {
        let mut state = self.state.lock().await;
        if !enabled(&state) {
            return Ok(());
        }
        let today = utc_day();
        if state.last_heartbeat_day.as_deref() != Some(&today)
            && post(&state, "heartbeat", &today, None).await
        {
            state.last_heartbeat_day = Some(today.clone());
            self.persist(&state)?;
        }
        self.send_pending_usage(&mut state, &today).await
    }

    pub async fn usage(&self, counters: UsageCounters) -> Result<(), String> {
        let counters = counters.checked()?;
        let mut state = self.state.lock().await;
        if !enabled(&state) {
            return Ok(());
        }
        add_pending_usage(&mut state, &utc_day(), counters);
        self.persist(&state)
    }

    async fn send_pending_usage(&self, state: &mut StoredState, today: &str) -> Result<(), String> {
        let completed_days = state
            .pending_usage_by_day
            .iter()
            .filter(|(day, usage)| day.as_str() < today && !usage.is_zero())
            .map(|(day, usage)| (day.clone(), *usage))
            .collect::<Vec<_>>();
        let mut changed = false;
        for (day, usage) in completed_days {
            if post(state, "usage", &day, Some(usage)).await {
                state.pending_usage_by_day.remove(&day);
                changed = true;
            }
        }
        if changed {
            self.persist(state)?;
        }
        Ok(())
    }
}

fn add_pending_usage(state: &mut StoredState, day: &str, counters: UsageCounters) {
    state
        .pending_usage_by_day
        .entry(day.to_string())
        .or_default()
        .add(counters);
}

fn status(state: &StoredState) -> TelemetryStatus {
    TelemetryStatus {
        consent: state.consent,
        enabled: enabled(state),
    }
}

fn enabled(state: &StoredState) -> bool {
    matches!(state.consent, ConsentState::Accepted) && state.install_id.is_some()
}

fn utc_day() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

async fn post(state: &StoredState, event: &str, day: &str, usage: Option<UsageCounters>) -> bool {
    let Some(install_id) = state.install_id else {
        return false;
    };
    let payload = Event {
        schema: SCHEMA,
        app: APP,
        event,
        install_id,
        version: env!("CARGO_PKG_VERSION"),
        platform: platform(),
        arch: arch(),
        day,
        usage,
    };
    let Ok(client) = reqwest::Client::builder()
        .user_agent(concat!("ultravox-telemetry/", env!("CARGO_PKG_VERSION")))
        .timeout(HTTP_TIMEOUT)
        .build()
    else {
        return false;
    };
    client
        .post(ENDPOINT)
        .json(&payload)
        .send()
        .await
        .is_ok_and(|response| response.status().is_success())
}

fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    }
}

fn arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "x86") {
        "x86"
    } else {
        "unknown"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_payload_has_only_the_v2_allowlist() {
        let payload = Event {
            schema: SCHEMA,
            app: APP,
            event: "usage",
            install_id: Uuid::parse_str("00000000-0000-4000-8000-000000000000").unwrap(),
            version: "0.2.2",
            platform: "macos",
            arch: "arm64",
            day: "2026-08-16",
            usage: Some(UsageCounters {
                recordings_started: 2,
                transcriptions_completed: 1,
                ..UsageCounters::default()
            }),
        };
        let value = serde_json::to_value(payload).unwrap();
        let keys = value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            [
                "app",
                "arch",
                "day",
                "event",
                "installId",
                "platform",
                "schema",
                "usage",
                "version",
            ]
        );
        assert!(value.get("timestamp").is_none());
        assert!(value.get("osVersion").is_none());
    }

    #[test]
    fn counters_reject_unknown_fields_and_out_of_range_values() {
        assert!(
            serde_json::from_value::<UsageCounters>(serde_json::json!({"unknown": 1})).is_err()
        );
        assert!(serde_json::from_value::<UsageCounters>(
            serde_json::json!({"recordingsStarted": -1})
        )
        .is_err());
        assert!(UsageCounters {
            recordings_started: MAX_COUNTER + 1,
            ..UsageCounters::default()
        }
        .checked()
        .is_err());
    }

    #[test]
    fn pending_counters_saturate_at_the_privacy_bound() {
        let mut counters = UsageCounters {
            recordings_started: MAX_COUNTER,
            ..UsageCounters::default()
        };
        counters.add(UsageCounters {
            recordings_started: 1,
            transcriptions_completed: 2,
            ..UsageCounters::default()
        });
        assert_eq!(counters.recordings_started, MAX_COUNTER);
        assert_eq!(counters.transcriptions_completed, 2);
    }

    #[test]
    fn pending_usage_remains_attributed_to_its_utc_day() {
        let mut state = StoredState::default();
        add_pending_usage(
            &mut state,
            "2026-08-16",
            UsageCounters {
                recordings_started: 1,
                ..UsageCounters::default()
            },
        );
        add_pending_usage(
            &mut state,
            "2026-08-17",
            UsageCounters {
                recordings_completed: 1,
                ..UsageCounters::default()
            },
        );
        assert_eq!(state.pending_usage_by_day.len(), 2);
        assert_eq!(
            state.pending_usage_by_day["2026-08-16"].recordings_started,
            1
        );
        assert_eq!(
            state.pending_usage_by_day["2026-08-17"].recordings_completed,
            1
        );
    }

    #[test]
    fn disabling_erases_identifier_and_pending_state() {
        let mut pending_usage_by_day = BTreeMap::new();
        pending_usage_by_day.insert(
            "2026-08-16".to_string(),
            UsageCounters {
                recordings_started: 1,
                ..UsageCounters::default()
            },
        );
        let mut state = StoredState {
            consent: ConsentState::Accepted,
            install_id: Some(Uuid::new_v4()),
            last_heartbeat_day: Some("2026-08-16".to_string()),
            pending_usage_by_day,
        };
        state.consent = ConsentState::Declined;
        state.install_id = None;
        state.last_heartbeat_day = None;
        state.pending_usage_by_day.clear();
        assert!(!enabled(&state));
        assert!(state.install_id.is_none());
        assert!(state.pending_usage_by_day.is_empty());
    }
    #[test]
    fn loaded_state_preserves_consent_but_repairs_private_state() {
        let mut pending_usage_by_day = BTreeMap::new();
        pending_usage_by_day.insert(
            "2026-08-16".to_string(),
            UsageCounters {
                recordings_started: 3,
                ..UsageCounters::default()
            },
        );
        let mut declined = StoredState {
            consent: ConsentState::Declined,
            install_id: Some(Uuid::new_v4()),
            pending_usage_by_day,
            ..StoredState::default()
        };
        assert!(sanitize_loaded_state(&mut declined));
        assert_eq!(declined.consent, ConsentState::Declined);
        assert!(declined.install_id.is_none());
        assert!(declined.pending_usage_by_day.is_empty());

        let mut accepted = StoredState {
            consent: ConsentState::Accepted,
            install_id: Some(Uuid::parse_str("00000000-0000-1000-8000-000000000000").unwrap()),
            ..StoredState::default()
        };
        assert!(sanitize_loaded_state(&mut accepted));
        let repaired = accepted.install_id.unwrap();
        assert_eq!(repaired.get_version_num(), 4);
        assert_eq!(repaired.get_variant(), uuid::Variant::RFC4122);
    }
}
