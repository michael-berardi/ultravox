import React, { useEffect, type ReactNode } from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import type { AppConfig } from "./ipc";
import { SettingsPage } from "./pages/SettingsPage";
import { initTheme } from "./themes";
import "./index.css";

const params = new URLSearchParams(window.location.search);
const requestedSettings = import.meta.env.DEV ? params.get("qa-settings") : null;

if (/Macintosh|MacIntel/.test(navigator.platform || navigator.userAgent)) {
  document.documentElement.dataset.chrome = "traffic-lights";
}
void initTheme();

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
  custom_dictionary: "Retex = retext\nUltraVox = Ultra Box\nKatherina Lucero",
  retex_dictionary: "",
  retex_vault_paths: [],
  retex_vault_identities: [],
  retex_auto_refresh: false,
  retex_last_refresh_at: null,
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
  show_meeting_mode: false,
  show_lecture_mode: false,
  show_transcribe_url: true,
};

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
          qaMode
          onClose={() => undefined}
        />
      ) : (
        <App />
      )}
    </BootReady>
  </React.StrictMode>,
);
