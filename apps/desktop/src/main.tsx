import { installNativeMirror } from "./lib/nativeMirror";
import React, { useEffect } from "react";
import type { ReactNode } from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { SettingsPage } from "./pages/SettingsPage";
import type { AppConfig } from "./ipc";
import {
  ThemeHarness,
  parseHarnessPro,
  parseHarnessSource,
  parseHarnessSpectrum,
  parseHarnessState,
  qaProStatus,
} from "./qa/ThemeHarness";
import { initTheme, THEMES } from "./themes";
import "./index.css";

installNativeMirror();

const params = new URLSearchParams(window.location.search);
const requestedTheme = import.meta.env.DEV ? params.get("qa-theme") : null;
const harnessTheme = THEMES.some((theme) => theme.id === requestedTheme)
  ? requestedTheme
  : null;
const harnessPro = parseHarnessPro(params.get("qa-pro"));
const requestedSettings = import.meta.env.DEV ? params.get("qa-settings") : null;
const qaDictionaryReady = params.get("qa-dictionary-state") === "ready";
const qaConfig: AppConfig = {
  config_version: 4,
  selected_engine: "fluidaudio",
  fluid_audio_model_version: "v2",
  selected_whisper_model_path: null,
  models_directory: null,
  audio_input_device_id: null,
  whisper_language: "en",
  translate_to_english: false,
  suppress_blank_audio: true,
  show_timestamps: false,
  temperature: 0,
  no_speech_threshold: 0.6,
  initial_prompt: "",
  custom_dictionary: "Retex = retext\nUltraVox = Ultra Box\nMarisol Vega",
  retex_dictionary: qaDictionaryReady ? "Cloudflare\nOpenAI\nRetex\nUltraVox" : "",
  retex_vault_paths: qaDictionaryReady ? ["/Users/example/Retex Vault"] : [],
  retex_vault_identities: qaDictionaryReady ? ["qa:example"] : [],
  retex_auto_refresh: qaDictionaryReady,
  retex_last_refresh_at: qaDictionaryReady ? "2026-09-03T20:00:00Z" : null,
  use_beam_search: false,
  beam_size: 5,
  debug_mode: false,
  play_sound_on_record_start: true,
  use_asian_autocorrect: false,
  modifier_only_hotkey: "none",
  key_combination: "Option+Backtick",
  hold_to_record: false,
  meeting_key_combination: "Control+M",
  meeting_detection_enabled: true,
  add_space_after_sentence: true,
  auto_copy_to_clipboard: true,
  auto_paste_transcription: true,
  onboarding_completed: true,
  model_language: "english",
  theme: "midnight",
  media_panel_enabled: false,
  reactive_visuals_enabled: false,
  show_meeting_mode: true,
  show_lecture_mode: true,
  show_transcribe_url: false,
};
const harnessReactive = params.get("qa-reactive") !== "0";
const requestedRecordingLevel = Number(params.get("qa-recording-level") ?? 0);
const harnessRecordingLevel = Number.isFinite(requestedRecordingLevel)
  ? Math.min(1, Math.max(0, requestedRecordingLevel))
  : 0;
const requestedTranscriptionDuration = Number(params.get("qa-transcription-ms") ?? 0);
const harnessTranscriptionDuration = Number.isFinite(requestedTranscriptionDuration)
  ? Math.max(0, requestedTranscriptionDuration)
  : 0;

// macOS draws the traffic lights inside the themed surface once the window
// uses the overlay title bar; reserve the inset via CSS chrome hooks.
if (/Macintosh|MacIntel/.test(navigator.platform || navigator.userAgent)) {
  document.documentElement.dataset.chrome = "traffic-lights";
}

if (harnessTheme) {
  document.documentElement.dataset.theme = harnessTheme;
} else {
  void initTheme();
}

function BootReady({ children }: { children: ReactNode }) {
  useEffect(() => {
    document.getElementById("boot-status")?.remove();
  }, []);
  return children;
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <BootReady>
      {requestedSettings === "dictionary" ? (
        <SettingsPage
          initialConfig={qaConfig}
          initialTab="dictionary"
          pro={harnessPro ?? qaProStatus("unlocked")}
          build={harnessPro?.available === false ? "open-source" : "official"}
          qaMode
          onClose={() => undefined}
        />
      ) : harnessTheme ? (
        <ThemeHarness
          drop={params.get("qa-drop") === "1"}
          pro={harnessPro}
          reactive={harnessReactive}
          recordingLevel={harnessRecordingLevel}
          transcriptionDurationMs={harnessTranscriptionDuration}
          state={parseHarnessState(params.get("qa-state"))}
          source={parseHarnessSource(params.get("qa-source"))}
          spectrum={parseHarnessSpectrum(params.get("qa-spectrum"))}
        />
      ) : (
        <App />
      )}
    </BootReady>
  </React.StrictMode>,
);
