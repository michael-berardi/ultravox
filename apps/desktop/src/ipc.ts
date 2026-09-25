import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen, type Event, type UnlistenFn } from "@tauri-apps/api/event";
import { wrapIpcForMirror } from "./lib/nativeMirror";
import { mapProStatus, PRO_STATUS_EVENT, type ProState, type ProStatus } from "./lib/proStatus";

export type { ProState, ProStatus };

// Mirror builds route every app command through the mirror wrapper (license
// simulation + suppression); consumer builds pass straight through.
const invoke = wrapIpcForMirror(tauriInvoke);

export type Engine = "whisper" | "fluidaudio";

export type Language = string;

export type AppConfig = {
  config_version: number;
  selected_engine: Engine;
  fluid_audio_model_version: string;
  selected_whisper_model_path: string | null;
  models_directory: string | null;
  audio_input_device_id: string | null;
  whisper_language: Language;
  translate_to_english: boolean;
  suppress_blank_audio: boolean;
  show_timestamps: boolean;
  temperature: number;
  no_speech_threshold: number;
  initial_prompt: string;
  custom_dictionary: string;
  retex_dictionary: string;
  retex_vault_paths: string[];
  retex_vault_identities: string[];
  retex_auto_refresh: boolean;
  retex_last_refresh_at: string | null;
  use_beam_search: boolean;
  beam_size: number;
  debug_mode: boolean;
  play_sound_on_record_start: boolean;
  use_asian_autocorrect: boolean;
  modifier_only_hotkey: string;
  key_combination: string;
  hold_to_record: boolean;
  meeting_key_combination: string;
  meeting_detection_enabled: boolean;
  add_space_after_sentence: boolean;
  auto_copy_to_clipboard: boolean;
  auto_paste_transcription: boolean;
  onboarding_completed: boolean;
  model_language: string;
  theme: string;
  media_panel_enabled: boolean;
  reactive_visuals_enabled: boolean;
  show_meeting_mode: boolean;
  show_lecture_mode: boolean;
  show_transcribe_url: boolean;
};
export type MediaTransportCommand = "play_pause" | "previous" | "next";

export type MediaState = {
  active: boolean;
  appName?: string | null;
  bundleId?: string | null;
  title?: string | null;
  artist?: string | null;
  album?: string | null;
  /** Bounded in-memory `data:image/…` artwork from the canonical adapter.
   *  The renderer must reject anything else; remote URLs are never sent. */
  artworkDataUrl?: string | null;
  elapsedSeconds?: number | null;
  durationSeconds?: number | null;
  isPlaying?: boolean | null;
  volume?: number | null;
  muted?: boolean | null;
  volumeAvailable: boolean;
  transportAvailable: boolean;
  previousAvailable: boolean;
  nextAvailable: boolean;
};

/** Which flavor of the one product this binary is: the public source build
 * (no Pro module) or the official build we sign and distribute. */
export type BuildFlavor = "open-source" | "official";

export type AppInfoResponse = {
  name: string;
  version: string;
  identifier: string;
  build: BuildFlavor;
};

export type LicenseStatus = {
  configured: boolean;
  active: boolean;
  entitlementId?: string | null;
  label?: string | null;
  plan?: string | null;
  expiresAt?: string | null;
  maxDevices?: number | null;
  unlimitedDevices: boolean;
  licenseState?: string | null;
  trialUntil?: string | null;
  updatesUntil?: string | null;
  promotion?: string | null;
  organizationId?: string | null;
  product?: string | null;
  error?: string | null;
};

export type AppStatusResponse = {
  status: "ready" | "loading" | "error";
  recording: boolean;
  meeting: boolean;
  transcription: string;
};

export type PermissionState = "granted" | "denied" | "not_determined" | "unavailable";

export type PermissionStatus = {
  microphone: PermissionState;
  accessibility: PermissionState;
  screen_recording: PermissionState;
};

export type PermissionKind = "microphone" | "accessibility" | "screen_recording";

export type RetexScanStatus = {
  vault_count: number;
  record_count: number;
  term_count: number;
  last_refresh_at: string;
};

export type UpdatePreferences = { automatic: boolean };
export type UpdateInfo = {
  current_version: string;
  latest_version: string;
  release_url: string;
};

export type TelemetryStatus = { consent: "undecided" | "accepted" | "declined"; enabled: boolean };

export type CaretPosition = {
  x: number;
  y: number;
  found: boolean;
};

export type TranscriptionResult = {
  text: string;
  success: boolean;
};

export type BridgeVersion = {
  version: string;
};

export type ModelFamily = "whisper" | "fluidaudio";
export type ModelVersion = "v2" | "v3";

export type ModelEntry = {
  id: string;
  family: ModelFamily;
  version: ModelVersion;
  name: string;
  description: string;
  url: string;
  filename: string;
  size_bytes: number | null;
  is_default: boolean;
};

export type ModelCatalog = {
  models: ModelEntry[];
};

export type DownloadState = "queued" | "downloading" | "completed" | "cancelled" | "failed";

export type DownloadProgress = {
  id: string;
  model_id: string;
  state: DownloadState;
  bytes_total: number | null;
  bytes_received: number;
};

export type ModelDownload = {
  id: string;
  model_id: string;
  url: string;
  destination: string;
};

export type RecordingStatus =
  | "pending"
  | "converting"
  | "transcribing"
  | "completed"
  | "failed"
  | "cancelled";

export type RecordingRow = {
  id: string;
  timestamp: string;
  file_name: string;
  title: string;
  preview: string;
  transcription: string;
  language: string;
  duration_seconds: number;
  status: RecordingStatus;
  progress: number;
  source_file_url: string | null;
};

export type ModifierKey = "none" | "leftOption" | "rightOption" | "rightCommand";

export type ShortcutSettings = {
  modifier_only_hotkey: ModifierKey;
  hold_to_record: boolean;
  key_combination: string | null;
  meeting_key_combination: string;
};

export type AudioRecording = {
  id: string;
  output_path: string;
  start_time_ms: number;
  duration_ms: number | null;
};

export type AudioDeviceInfo = {
  id: string;
  name: string;
  is_default: boolean;
};

export type SampleFormat = "i16" | "f32";

export type AudioInputConfig = {
  sample_rate: number;
  channels: number;
  sample_format: SampleFormat;
  device_id: string | null;
};

export type Segment = {
  start_ms: number;
  end_ms: number;
  text: string;
};

export type TranscriptionResultCore = {
  text: string;
  segments: Segment[];
  language: string | null;
};

export type TranscriptionRequest = {
  audio_path: string;
  language: string | null;
  translate_to_english: boolean;
  initial_prompt: string | null;
  temperature: number | null;
  suppress_blank_audio: boolean | null;
  show_timestamps: boolean | null;
  use_beam_search: boolean | null;
  beam_size: number | null;
};

// Event payloads
export type RecordingStartedPayload = {
  recording_id: string;
  start_time_ms: number;
};

export type RecordingStoppedPayload = {
  recording_id: string;
  output_path: string;
  duration_ms: number | null;
};

export type TranscriptionProgressPayload = {
  recording_id: string;
  progress: number;
  status: string;
};

export type UrlImportProgressPayload = {
  progress: number;
  status: string;
};

export type TranscriptionCompletedPayload = {
  recording_id: string;
  text: string;
  language: string | null;
};

export type ShortcutTriggeredPayload = {
  shortcut: string;
};

export type IndicatorShowPayload = {
  x: number;
  y: number;
};

export type IndicatorHidePayload = void;

export type SettingsChangedPayload = {
  config: AppConfig;
};

export type RecordingAddedPayload = {
  recording: RecordingRow;
};

export type RecordingDeletedPayload = {
  id: string;
};

export type MeetingProvider = "google_meet" | "zoom";

export type MeetingDetectionPendingPayload = {
  version: 1;
  detection_id: string;
  provider: MeetingProvider;
  detected_at_ms: number;
  expires_at_ms: number;
};

export type EventName =
  | "recording-started"
  | "recording-stopped"
  | "transcription-progress"
  | "transcription-completed"
  | "shortcut-triggered"
  | "indicator-show"
  | "recording-added"
  | "meeting-state-changed"
  | "meeting-detection-pending"
  | "url-import-progress"
  | "recording-deleted";

export async function getAppInfo(): Promise<AppInfoResponse> {
  return await invoke<AppInfoResponse>("get_app_info");
}

/** Verified Pro entitlement as reported by the Pro module. */
export async function getProStatus(): Promise<ProStatus> {
  return mapProStatus(await invoke<unknown>("get_pro_status"));
}

export async function onProStatusChanged(
  handler: (payload: ProStatus) => void
): Promise<UnlistenFn> {
  return listen<ProStatus>(PRO_STATUS_EVENT, (event: Event<ProStatus>) =>
    handler(mapProStatus(event.payload))
  );
}

export async function getAppStatus(): Promise<AppStatusResponse> {
  return await invoke<AppStatusResponse>("get_app_status");
}

export async function getPermissionStatus(): Promise<PermissionStatus> {
  return await invoke<PermissionStatus>("get_permission_status");
}

export async function requestPermission(kind: PermissionKind): Promise<PermissionStatus> {
  return await invoke<PermissionStatus>("request_permission", { kind });
}

export async function openPermissionSettings(kind: PermissionKind): Promise<void> {
  return await invoke("open_permission_settings", { kind });
}
export async function getUpdatePreferences(): Promise<UpdatePreferences> {
  return await invoke<UpdatePreferences>("get_update_preferences");
}

export async function setUpdatePreferences(preferences: UpdatePreferences): Promise<void> {
  return await invoke("set_update_preferences", { preferences });
}

export async function checkForUpdate(): Promise<UpdateInfo | null> {
  return await invoke<UpdateInfo | null>("check_for_update");
}

export async function installUpdate(info: UpdateInfo): Promise<void> {
  return await invoke("install_update", { info });
}
export async function getAppTelemetryStatus(): Promise<TelemetryStatus> {
  return await invoke<TelemetryStatus>("get_app_telemetry_status");
}

export async function setAppTelemetryEnabled(enabled: boolean): Promise<TelemetryStatus> {
  return await invoke<TelemetryStatus>("set_app_telemetry_enabled", { enabled });
}


export async function getSettings(): Promise<AppConfig> {
  return await invoke<AppConfig>("get_settings");
}

export async function setThemeMaterial(theme: string): Promise<void> {
  return await invoke("set_theme_material", { theme });
}

export async function getLicenseStatus(): Promise<LicenseStatus> {
  return await invoke<LicenseStatus>("distribution_access_status");
}

export async function activateLicense(accessKey: string): Promise<LicenseStatus> {
  return await invoke<LicenseStatus>("activate_distribution_access", { accessKey });
}

/** Registers this install's revocable device grant; idempotent. */
export async function registerDeviceLicense(): Promise<LicenseStatus | null> {
  return await invoke<LicenseStatus | null>("register_device_license");
}

/** Consumes a one-time organization access file before trial registration. */
export async function bootstrapDistributionAccess(): Promise<LicenseStatus | null> {
  return await invoke<LicenseStatus | null>("bootstrap_distribution_access");
}

export async function setSettings(config: AppConfig): Promise<void> {
  return await invoke("set_settings", { config });
}

export async function chooseRetexVaults(): Promise<void> {
  return await invoke("choose_retex_vaults");
}

export async function revokeRetexVault(path: string): Promise<void> {
  return await invoke("revoke_retex_vault", { path });
}

export async function setRetexAutoRefresh(enabled: boolean): Promise<void> {
  return await invoke("set_retex_auto_refresh", { enabled });
}

export async function scanRetexDictionary(): Promise<RetexScanStatus> {
  return await invoke<RetexScanStatus>("scan_retex_dictionary");
}

export async function getModelCatalog(): Promise<ModelCatalog> {
  return await invoke<ModelCatalog>("get_model_catalog");
}

export async function getMediaState(): Promise<MediaState> {
  return await invoke<MediaState>("get_media_state");
}


export async function getInputLevel(): Promise<number> {
  return await invoke<number>("get_input_level");
}
export async function setSystemAudioMeterEnabled(enabled: boolean): Promise<void> {
  return await invoke("set_system_audio_meter_enabled", { enabled });
}

/** Eleven normalized [0,1] bands, low to high frequency. */
export async function getSystemAudioSpectrum(): Promise<number[]> {
  return await invoke<number[]>("get_system_audio_spectrum");
}

export async function setSystemVolume(volume: number): Promise<void> {
  return await invoke("set_system_volume", { volume });
}

export async function setSystemMuted(muted: boolean): Promise<void> {
  return await invoke("set_system_muted", { muted });
}

export async function mediaTransport(command: MediaTransportCommand): Promise<void> {
  return await invoke("media_transport", { command });
}

export async function getDownloadProgress(id: string): Promise<DownloadProgress> {
  return await invoke<DownloadProgress>("get_download_progress", { id });
}

export async function getDownloads(): Promise<DownloadProgress[]> {
  return await invoke<DownloadProgress[]>("get_downloads");
}

export async function startDownload(request: ModelDownload): Promise<DownloadProgress> {
  return await invoke<DownloadProgress>("start_download", { request });
}

export async function prepareModel(modelId: string): Promise<boolean> {
  return await invoke<boolean>("prepare_model", { modelId });
}

export async function isModelDownloaded(modelId: string): Promise<boolean> {
  return await invoke<boolean>("is_model_downloaded", { modelId });
}

export async function getModelProgress(modelId: string): Promise<number> {
  return await invoke<number>("get_model_progress", { modelId });
}

export async function cancelDownload(id: string): Promise<void> {
  return await invoke("cancel_download", { id });
}

export async function listRecordings(): Promise<RecordingRow[]> {
  return await invoke<RecordingRow[]>("list_recordings");
}

export async function searchRecordings(query: string): Promise<RecordingRow[]> {
  return await invoke<RecordingRow[]>("search_recordings", { query });
}

export async function getRecording(id: string): Promise<RecordingRow | null> {
  return await invoke<RecordingRow | null>("get_recording", { id });
}

export async function updateRecording(row: RecordingRow): Promise<RecordingRow> {
  return await invoke<RecordingRow>("update_recording", { row });
}

export async function deleteRecording(id: string): Promise<void> {
  return await invoke("delete_recording", { id });
}

export async function deleteAllRecordings(): Promise<number> {
  return await invoke<number>("delete_all_recordings");
}

export async function startRecording(): Promise<string> {
  return await invoke<string>("start_recording");
}

export async function stopRecording(): Promise<AudioRecording> {
  return await invoke<AudioRecording>("stop_recording");
}

export async function importUrl(url: string): Promise<string> {
  return await invoke<string>("import_url", { url });
}

export async function importFile(path: string): Promise<string> {
  return await invoke<string>("import_file", { path });
}

export async function startMeeting(): Promise<string> {
  return await invoke<string>("start_meeting");
}


export async function startLecture(): Promise<string> {
  return await invoke<string>("start_lecture");
}
export type MeetingDetectionDecision = "accept" | "decline";

export async function respondMeetingDetection(
  detectionId: string,
  decision: MeetingDetectionDecision,
): Promise<string> {
  return await invoke<string>("respond_meeting_detection", {
    detectionId,
    decision,
  });
}
export async function getPendingMeetingDetection(): Promise<MeetingDetectionPendingPayload | null> {
  return await invoke<MeetingDetectionPendingPayload | null>("get_pending_meeting_detection");
}
export async function stopMeeting(): Promise<AudioRecording> {
  return await invoke<AudioRecording>("stop_meeting");
}

export async function getTranscriptionStatus(): Promise<string> {
  return await invoke<string>("get_transcription_status");
}

export async function retryTranscription(id: string): Promise<string> {
  return await invoke<string>("retry_transcription", { id });
}

export async function getShortcutSettings(): Promise<ShortcutSettings> {
  return await invoke<ShortcutSettings>("get_shortcut_settings");
}

export async function setShortcutSettings(settings: ShortcutSettings): Promise<void> {
  return await invoke("set_shortcut_settings", { settings });
}

export async function getAudioDevices(): Promise<AudioDeviceInfo[]> {
  return await invoke<AudioDeviceInfo[]>("get_audio_devices");
}

export async function getAudioInputConfig(): Promise<AudioInputConfig> {
  return await invoke<AudioInputConfig>("get_audio_input_config");
}

export async function exportRecording(id: string, destination: string): Promise<string> {
  return await invoke<string>("export_recording", { id, destination });
}

export async function bridgeVersion(): Promise<BridgeVersion> {
  return await invoke<BridgeVersion>("bridge_version");
}

export async function getCaretPosition(): Promise<CaretPosition> {
  return await invoke<CaretPosition>("get_caret_position");
}

export async function pasteText(text: string): Promise<number> {
  return await invoke<number>("paste_text", { text });
}

export async function copyToClipboard(text: string): Promise<void> {
  return await invoke("copy_to_clipboard", { text });
}

export type DictionarySkillInfo = {
  path: string;
  exists: boolean;
  text: string;
};

export async function dictionarySkillInfo(): Promise<DictionarySkillInfo> {
  return await invoke<DictionarySkillInfo>("dictionary_skill_info");
}

export async function revealDictionarySkill(): Promise<void> {
  return await invoke("reveal_dictionary_skill");
}

export async function startModifierHotkey(modifier: string): Promise<number> {
  return await invoke<number>("start_modifier_hotkey", { modifier });
}

export async function startKeyCombinationHotkey(
  combo: string,
  holdToRecord: boolean
): Promise<number> {
  return await invoke<number>("start_key_combination_hotkey", { combo, holdToRecord });
}

export async function stopKeyCombinationHotkey(): Promise<number> {
  return await invoke<number>("stop_key_combination_hotkey");
}

export async function showIndicator(x: number, y: number): Promise<number> {
  return await invoke<number>("show_indicator", { x, y });
}

export async function hideIndicator(): Promise<number> {
  return await invoke<number>("hide_indicator");
}

export async function transcribeFile(path: string): Promise<TranscriptionResult> {
  return await invoke<TranscriptionResult>("transcribe_file", { path });
}

// Event listener helpers
export async function onRecordingStarted(
  handler: (payload: RecordingStartedPayload) => void
): Promise<UnlistenFn> {
  return listen<RecordingStartedPayload>("recording-started", (event: Event<RecordingStartedPayload>) =>
    handler(event.payload)
  );
}

export async function onRecordingStopped(
  handler: (payload: RecordingStoppedPayload) => void
): Promise<UnlistenFn> {
  return listen<RecordingStoppedPayload>("recording-stopped", (event: Event<RecordingStoppedPayload>) =>
    handler(event.payload)
  );
}

export async function onTranscriptionProgress(
  handler: (payload: TranscriptionProgressPayload) => void
): Promise<UnlistenFn> {
  return listen<TranscriptionProgressPayload>("transcription-progress", (event: Event<TranscriptionProgressPayload>) =>
    handler(event.payload)
  );
}

export async function onTranscriptionCompleted(
  handler: (payload: TranscriptionCompletedPayload) => void
): Promise<UnlistenFn> {
  return listen<TranscriptionCompletedPayload>("transcription-completed", (event: Event<TranscriptionCompletedPayload>) =>
    handler(event.payload)
  );
}

export async function onShortcutTriggered(
  handler: (payload: ShortcutTriggeredPayload) => void
): Promise<UnlistenFn> {
  return listen<ShortcutTriggeredPayload>("shortcut-triggered", (event: Event<ShortcutTriggeredPayload>) =>
    handler(event.payload)
  );
}

export async function onIndicatorShow(
  handler: (payload: IndicatorShowPayload) => void
): Promise<UnlistenFn> {
  return listen<IndicatorShowPayload>("indicator-show", (event: Event<IndicatorShowPayload>) =>
    handler(event.payload)
  );
}

export async function onIndicatorHide(
  handler: () => void
): Promise<UnlistenFn> {
  return listen<IndicatorHidePayload>("indicator-hide", () => handler());
}

export async function onSettingsChanged(
  handler: (payload: SettingsChangedPayload) => void
): Promise<UnlistenFn> {
  return listen<SettingsChangedPayload>("settings-changed", (event: Event<SettingsChangedPayload>) =>
    handler(event.payload)
  );
}

export async function onMeetingDetectionPending(
  handler: (payload: MeetingDetectionPendingPayload) => void,
): Promise<UnlistenFn> {
  return listen<MeetingDetectionPendingPayload>(
    "meeting-detection-pending",
    (event: Event<MeetingDetectionPendingPayload>) => handler(event.payload),
  );
}

export async function onRecordingAdded(
  handler: (payload: RecordingAddedPayload) => void
): Promise<UnlistenFn> {
  return listen<RecordingAddedPayload>("recording-added", (event: Event<RecordingAddedPayload>) =>
    handler(event.payload)
  );
}

export async function onRecordingDeleted(
  handler: (payload: RecordingDeletedPayload) => void
): Promise<UnlistenFn> {
  return listen<RecordingDeletedPayload>("recording-deleted", (event: Event<RecordingDeletedPayload>) =>
    handler(event.payload)
  );
}


export async function onMeetingStateChanged(
  handler: (active: boolean) => void
): Promise<UnlistenFn> {
  return listen<boolean>("meeting-state-changed", (event: Event<boolean>) =>
    handler(event.payload)
  );
}

export async function onUrlImportProgress(
  handler: (payload: UrlImportProgressPayload) => void
): Promise<UnlistenFn> {
  return listen<UrlImportProgressPayload>(
    "url-import-progress",
    (event: Event<UrlImportProgressPayload>) => handler(event.payload)
  );
}

// Voice Studio: UUIDs and RFC3339 dates cross IPC as strings.
export type VoiceProfile = {
  id: string; name: string; created_at: string; language: string;
  source_recording_ids: string[]; source_paths: string[];
  reference_seconds: number; embeddings_path: string; builtin: boolean;
};
export type CorpusCandidate = {
  recording_id: string; path: string; duration_seconds: number; selected: boolean;
};
export type GenerationEntry = {
  id: string; path: string; kind: string; text: string | null;
  created_at: number; size: number;
};
export async function voiceGenerationsList(): Promise<GenerationEntry[]> {
  return await invoke("voice_generations_list");
}
export async function voiceGenerationDelete(pathOrId: string): Promise<void> {
  return await invoke("voice_generation_delete", { pathOrId });
}
export async function voiceGenerationsDeleteAll(): Promise<void> {
  return await invoke("voice_generations_delete_all");
}
export type SpeakResult = { wav_path: string; seconds: number };
export type TtsStatus = { models_ready: boolean; voice_count: number };
export async function vsListVoices(): Promise<VoiceProfile[]> {
  return await invoke("vs_list_voices");
}
export async function vsCorpusCandidates(): Promise<CorpusCandidate[]> {
  return await invoke("vs_corpus_candidates");
}
export async function vsCreateVoice(name: string, recordingIds: string[] | null, importPath: string | null, language: string): Promise<VoiceProfile> {
  return await invoke("vs_create_voice", { name, recordingIds, importPath, language });
}
export async function vsDeleteVoice(id: string): Promise<void> {
  return await invoke("vs_delete_voice", { id });
}
export async function vsRenameVoice(id: string, name: string): Promise<void> {
  return await invoke("vs_rename_voice", { id, name });
}
export async function vsSpeak(voiceId: string | null, builtin: string | null, text: string, language: string): Promise<SpeakResult> {
  return await invoke("vs_speak", { voiceId, builtin, text, language, useInt8: false });
}
export async function vsTtsStatus(): Promise<TtsStatus> {
  return await invoke("vs_tts_status");
}
export async function voiceStudioAudioUrl(path: string): Promise<string> {
  const bytes = await invoke<number[]>("vs_read_audio", { path });
  return URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: "audio/wav" }));
}
