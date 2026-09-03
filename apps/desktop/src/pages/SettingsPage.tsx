import { useCallback, useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { open as openExternal } from "@tauri-apps/plugin-shell";
import {
  setSettings,
  getSettings,
  getModelCatalog,
  prepareModel,
  isModelDownloaded,
  getModelProgress,
  setShortcutSettings,
  checkForUpdate,
  getUpdatePreferences,
  installUpdate,
  setUpdatePreferences,
  onSettingsChanged,
  type AppConfig,
  type ModelEntry,
  type ModifierKey,
  type ShortcutSettings as ShortcutSettingsPayload,
  type UpdateInfo,
} from "../ipc";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { THEMES } from "../themes";
import { BrandMark, startHeaderDrag } from "../components/BrandMark";
export type SettingsTab = "shortcut" | "model" | "transcription" | "dictionary" | "privacy" | "appearance" | "support";

interface SettingsPageProps {
  initialConfig: AppConfig;
  initialTab?: SettingsTab;
  qaMode?: boolean;
  onClose: () => void;
}

type ShortcutConfigPatch = Partial<
  Pick<
    AppConfig,
    "modifier_only_hotkey" | "key_combination" | "hold_to_record"
  >
>;

function normalizeModifier(value: string): ModifierKey {
  switch (value) {
    case "leftOption":
    case "rightOption":
    case "rightCommand":
    case "none":
      return value;
    case "option":
      return "leftOption";
    case "command":
      return "rightCommand";
    default:
      return "none";
  }
}


export function SettingsPage({ initialConfig, initialTab = "shortcut", qaMode = false, onClose }: SettingsPageProps) {
  const [activeTab, setActiveTab] = useState<SettingsTab>(initialTab);
  const [config, setConfig] = useState<AppConfig>(initialConfig);
  const [error, setError] = useState<string | null>(null);
  const configRef = useRef(initialConfig);
  const pendingConfigRef = useRef<AppConfig | null>(null);
  const saveQueueRef = useRef<Promise<void>>(Promise.resolve());

  useEffect(() => setActiveTab(initialTab), [initialTab]);
  useEffect(() => {
    if (qaMode) return;
    let cancelled = false;
    let unlisten: UnlistenFn | undefined;

    async function setup() {
      try {
        const stopListening = await onSettingsChanged((payload) => {
          const pending = pendingConfigRef.current;
          if (pending && JSON.stringify(payload.config) !== JSON.stringify(pending)) {
            return;
          }
          pendingConfigRef.current = null;
          configRef.current = payload.config;
          setConfig(payload.config);
        });
        if (cancelled) {
          stopListening();
        } else {
          unlisten = stopListening;
        }
      } catch (listenerError) {
        if (!cancelled) setError(`Could not observe settings changes: ${String(listenerError)}`);
      }
    }

    void setup();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [qaMode]);

  const persist = useCallback((next: AppConfig, operation: () => Promise<void>) => {
    configRef.current = next;
    pendingConfigRef.current = next;
    setConfig(next);
    setError(null);
    saveQueueRef.current = saveQueueRef.current
      .catch(() => undefined)
      .then(operation)
      .catch(async (saveError) => {
        setError(String(saveError));
        if (pendingConfigRef.current !== next) return;
        pendingConfigRef.current = null;
        try {
          const persisted = await getSettings();
          configRef.current = persisted;
          setConfig(persisted);
        } catch (reloadError) {
          setError(`${String(saveError)}; could not reload settings: ${String(reloadError)}`);
        }
      });
  }, []);

  const updateConfig = useCallback((patch: Partial<AppConfig>) => {
    const next = { ...configRef.current, ...patch };
    persist(next, () => setSettings(next));
  }, [persist]);

  const updateShortcut = useCallback((patch: ShortcutConfigPatch) => {
    const next = {
      ...configRef.current,
      ...patch,
      modifier_only_hotkey: normalizeModifier(
        patch.modifier_only_hotkey ?? configRef.current.modifier_only_hotkey,
      ),
    };
    const settings: ShortcutSettingsPayload = {
      modifier_only_hotkey: normalizeModifier(next.modifier_only_hotkey),
      key_combination: next.key_combination,
      hold_to_record: next.hold_to_record,
    };
    persist(next, () => setShortcutSettings(settings));
  }, [persist]);

  const cfg = config;

  return (
    <div className="settings">
      <header className="header" onMouseDown={startHeaderDrag}>
        <BrandMark suffix="Settings" />
        <div className="header-actions">
          <button
            className="icon-button"
            title="Close settings"
            onClick={onClose}
            aria-label="Close settings"
          >
            <CloseIcon />
          </button>
        </div>
      </header>

      <div className="settings-tabs" role="tablist" aria-label="Settings tabs">
        <TabButton id="shortcut" label="Shortcut" active={activeTab} onClick={setActiveTab} />
        <TabButton id="model" label="Model" active={activeTab} onClick={setActiveTab} />
        <TabButton id="transcription" label="Transcription" active={activeTab} onClick={setActiveTab} />
        <TabButton id="dictionary" label="Dictionary" active={activeTab} onClick={setActiveTab} />
        <TabButton id="privacy" label="Privacy" active={activeTab} onClick={setActiveTab} />
        <TabButton id="appearance" label="Appearance" active={activeTab} onClick={setActiveTab} />
        <TabButton id="support" label="Support" active={activeTab} onClick={setActiveTab} />
      </div>
      {error && <div className="settings-error" role="alert">{error}</div>}

      <div className="settings-body">
        {activeTab === "shortcut" && <ShortcutSettings config={cfg} onChange={updateShortcut} />}
        {activeTab === "model" && <ModelSettings config={cfg} onChange={updateConfig} />}
        {activeTab === "transcription" && <TranscriptionSettings config={cfg} onChange={updateConfig} />}
        {activeTab === "dictionary" && <DictionarySettings config={cfg} onChange={updateConfig} />}
        {activeTab === "privacy" && <PrivacySettings config={cfg} onChange={updateConfig} />}
        {activeTab === "appearance" && <AppearanceSettings config={cfg} onChange={updateConfig} />}
        {activeTab === "support" && <SupportSettings />}
      </div>

    </div>
  );
}

function TabButton({
  id,
  label,
  active,
  onClick,
}: {
  id: SettingsTab;
  label: string;
  active: SettingsTab;
  onClick: (id: SettingsTab) => void;
}) {
  const isActive = active === id;
  return (
    <button
      className={`settings-tab ${isActive ? "active" : ""}`}
      onClick={() => onClick(id)}
      role="tab"
      aria-selected={isActive}
      id={`tab-${id}`}
      aria-controls={`panel-${id}`}
    >
      {label}
    </button>
  );
}

interface SettingsSectionProps {
  config: AppConfig;
  onChange: (patch: Partial<AppConfig>) => void;
}

function keyNameFromKeyboardEvent(event: KeyboardEvent): string | null {
  if (event.code.startsWith("Key")) {
    return event.code.slice(3);
  }

  if (event.code.startsWith("Digit")) {
    return event.code.slice(5);
  }

  switch (event.code) {
    case "Backquote":
      return "Backtick";
    case "Escape":
      return "Escape";
    case "Space":
      return "Space";
    case "Enter":
      return "Enter";
    default:
      return null;
  }
}

function shortcutFromKeyboardEvent(event: KeyboardEvent, allowBareSpace = false): string | null {
  const modifiers = [
    event.metaKey ? "Command" : null,
    event.altKey ? "Option" : null,
    event.ctrlKey ? "Control" : null,
    event.shiftKey ? "Shift" : null,
  ].filter((modifier): modifier is string => modifier !== null);
  if (modifiers.length === 0 && allowBareSpace && event.code === "Space") return "Space";
  if (modifiers.length !== 1) return null;
  const key = keyNameFromKeyboardEvent(event);
  return key ? `${modifiers[0]}+${key}` : null;
}

function formatShortcut(shortcut: string): string {
  const [modifier, key] = shortcut.split("+");
  const modifierSymbol: Record<string, string> = {
    Command: "⌘",
    Option: "⌥",
    Control: "⌃",
    Shift: "⇧",
  };
  const keyLabel = key === "Backtick" ? "`" : key;
  return `${modifierSymbol[modifier] ?? modifier}  ${keyLabel ?? ""}`.trim();
}


function ShortcutSettings({
  config,
  onChange,
}: {
  config: AppConfig;
  onChange: (patch: ShortcutConfigPatch) => void;
}) {
  const [capturing, setCapturing] = useState<"recording" | null>(null);
  const [captureMessage, setCaptureMessage] = useState("Esc to cancel");
  const modifierValue = normalizeModifier(config.modifier_only_hotkey);
  useEffect(() => {
    if (!capturing) return;
    const handleKeyDown = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopImmediatePropagation();
      if (event.repeat) return;
      const hasModifier = event.metaKey || event.altKey || event.ctrlKey || event.shiftKey;
      if (event.code === "Escape" && !hasModifier) {
        setCapturing(null);
        setCaptureMessage("Esc to cancel");
        return;
      }
      const shortcut = shortcutFromKeyboardEvent(event, config.hold_to_record);
      if (!shortcut) {
        if (event.code === "Space" && !config.hold_to_record) {
          setCaptureMessage("Turn on Hold shortcut to use Space alone");
        } else if (!["MetaLeft", "MetaRight", "AltLeft", "AltRight", "ControlLeft", "ControlRight", "ShiftLeft", "ShiftRight"].includes(event.code)) {
          setCaptureMessage("Use one modifier and one supported key");
        }
        return;
      }
      onChange({ key_combination: shortcut });
      setCapturing(null);
      setCaptureMessage("Esc to cancel");
    };
    window.addEventListener("keydown", handleKeyDown, true);
    return () => window.removeEventListener("keydown", handleKeyDown, true);
  }, [capturing, config.hold_to_record, onChange]);

  return (
    <div className="settings-group" role="tabpanel" id="panel-shortcut" aria-labelledby="tab-shortcut">
      <div className="settings-card shortcut-card">
        <div className="settings-card-heading">
          <div>
            <h3>Recording trigger</h3>
            <p>Changes take effect as soon as you choose them.</p>
          </div>
        </div>
        <SettingsRow label="Keyboard shortcut" description="Use one modifier with a supported key, or turn on Hold shortcut to use Space alone.">
          <button
            type="button"
            className={`shortcut-capture${capturing === "recording" ? " is-capturing" : ""}`}
            onClick={() => {
              setCaptureMessage("Esc to cancel");
              setCapturing("recording");
            }}
            aria-label={
              capturing === "recording"
                ? "Press a new recording shortcut"
                : `Recording shortcut ${config.key_combination}`
            }
            aria-pressed={capturing === "recording"}
          >
            <span>
              {capturing === "recording"
                ? "Press keys…"
                : formatShortcut(config.key_combination)}
            </span>
            <small aria-live="polite">
              {capturing === "recording" ? captureMessage : "Change"}
            </small>
          </button>
        </SettingsRow>
        <SettingsRow label="Hold modifier" description="Record while holding one physical modifier key.">
          <select
            className="select modifier-select"
            value={modifierValue}
            onChange={(event) => onChange({ modifier_only_hotkey: event.target.value })}
            aria-label="Modifier-only hotkey"
          >
            <option value="none">Off</option>
            <option value="leftOption">Left Option</option>
            <option value="rightOption">Right Option</option>
            <option value="rightCommand">Right Command</option>
          </select>
        </SettingsRow>
        <ToggleRow
          label="Hold shortcut to record"
          description="Otherwise, press once to start and again to stop."
          checked={config.hold_to_record}
          onChange={(checked) => onChange({ hold_to_record: checked })}
        />
      </div>
    </div>
  );
}

function ModelSettings({ config, onChange }: SettingsSectionProps) {
  const [catalog, setCatalog] = useState<ModelEntry[]>([]);
  const [downloaded, setDownloaded] = useState<Record<string, boolean>>({});
  const [preparing, setPreparing] = useState<Record<string, boolean>>({});
  const [progress, setProgress] = useState<Record<string, number>>({});
  const [loading, setLoading] = useState(true);
  const [modelError, setModelError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function load() {
      try {
        const modelCatalog = await getModelCatalog();
        const visibleCatalog = modelCatalog.models;
        const statuses = Object.fromEntries(
          await Promise.all(
            visibleCatalog.map(async (model) => [
              model.id,
              await isModelDownloaded(model.id),
            ] as const),
          ),
        );
        if (!cancelled) {
          setCatalog(visibleCatalog);
          setDownloaded(statuses);
          if (visibleCatalog.length !== 2) {
            setModelError("The model catalog is incomplete. Restart UltraVox and try again.");
          }
        }
      } catch (loadError) {
        if (!cancelled) {
          setModelError(`Could not load model status: ${String(loadError)}`);
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    }

    void load();
    const interval = window.setInterval(load, 2000);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, []);

  useEffect(() => {
    const activeModelIds = Object.entries(preparing)
      .filter(([, active]) => active)
      .map(([id]) => id);
    if (activeModelIds.length === 0) return;

    let cancelled = false;
    const update = async () => {
      const entries = await Promise.all(
        activeModelIds.map(async (id) => {
          try {
            return [id, await getModelProgress(id)] as const;
          } catch {
            return [id, null] as const;
          }
        }),
      );
      if (!cancelled) {
        setProgress((current) => {
          const next = { ...current };
          entries.forEach(([id, value]) => {
            if (value !== null) next[id] = value;
          });
          return next;
        });
      }
    };
    void update();
    const interval = window.setInterval(update, 250);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, [preparing]);

  const mode: "english" | "multilingual" =
    config.model_language === "multilingual" ? "multilingual" : "english";
  const selectedModelId = catalog.find((model) =>
    model.id.includes(mode === "multilingual" ? "multilingual" : "-en"),
  )?.id;

  const setMode = (next: "english" | "multilingual") => {
    const usesWhisper = catalog.some((model) => model.family === "whisper");
    onChange({
      selected_engine: usesWhisper ? "whisper" : "fluidaudio",
      fluid_audio_model_version: next === "english" ? "v2" : "v3",
      whisper_language: next === "english" ? "en" : "auto",
      model_language: next,
    });
  };

  const onDownload = async (model: ModelEntry) => {
    const modelMode = model.id.includes("multilingual") ? "multilingual" : "english";
    setMode(modelMode);
    setModelError(null);
    setProgress((current) => ({ ...current, [model.id]: 0 }));
    setPreparing((current) => ({ ...current, [model.id]: true }));
    try {
      const prepared = await prepareModel(model.id);
      if (!prepared) {
        throw new Error("The download did not complete.");
      }
      setProgress((current) => ({ ...current, [model.id]: 1 }));
      setDownloaded((current) => ({ ...current, [model.id]: true }));
    } catch (downloadError) {
      setModelError(`Could not download the model: ${String(downloadError)}`);
    } finally {
      setPreparing((current) => ({ ...current, [model.id]: false }));
    }
  };

  const chooseModelsDirectory = async () => {
    setModelError(null);
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Choose Models Directory",
        defaultPath: config.models_directory ?? undefined,
      });
      if (typeof selected === "string") {
        onChange({ models_directory: selected });
      }
    } catch (directoryError) {
      setModelError(`Could not change the models directory: ${String(directoryError)}`);
    }
  };

  const defaultModelsDirectory = "UltraVox app data / models";

  return (
    <div className="settings-group" role="tabpanel" id="panel-model" aria-labelledby="tab-model">
      <div className="settings-card model-settings-card">
        <div className="settings-card-heading">
          <div>
            <h3>Transcription model</h3>
            <p>Private, on-device transcription. Download a model once to use it offline.</p>
          </div>
        </div>
        {loading ? (
          <p className="placeholder-text model-loading" aria-live="polite">
            Checking model status…
          </p>
        ) : (
          <div className="model-option-list" role="radiogroup" aria-label="Transcription model">
            {catalog.map((model) => {
              const modelMode =
                model.id.includes("multilingual") ? "multilingual" : "english";
              const title = modelMode === "english" ? "English" : "Multilingual";
              const isSelected = model.id === selectedModelId;
              const isDownloaded = downloaded[model.id];
              const isPreparing = preparing[model.id];
              const fraction = Math.max(0, Math.min(1, progress[model.id] ?? 0));
              let statusText = "Not downloaded";
              if (isDownloaded) {
                statusText = "Downloaded";
              } else if (isPreparing) {
                statusText = `${Math.round(fraction * 100)}% downloaded`;
              }

              return (
                <div
                  key={model.id}
                  className={`model-option-card${isSelected ? " selected" : ""}`}
                >
                  <button
                    type="button"
                    className="model-option-select"
                    role="radio"
                    aria-checked={isSelected}
                    onClick={() => setMode(modelMode)}
                  >
                    <span className="model-option-check" aria-hidden="true">
                      {isSelected ? "✓" : ""}
                    </span>
                    <span className="model-option-copy">
                      <strong>{title}</strong>
                      <span className={isDownloaded ? "downloaded" : ""} aria-live="polite">
                        {formatBytes(model.size_bytes)} · {statusText}
                      </span>
                      {isPreparing && (
                        <progress
                          value={fraction}
                          max={1}
                          aria-label={`${title} download progress, ${Math.round(fraction * 100)} percent`}
                        />
                      )}
                    </span>
                  </button>
                  {!isDownloaded && (
                    <button
                      type="button"
                      className="btn btn-small btn-primary"
                      onClick={() => void onDownload(model)}
                      disabled={isPreparing}
                      aria-label={`Download ${title} model, ${formatBytes(model.size_bytes)}`}
                    >
                      {isPreparing ? "Downloading…" : "Download"}
                    </button>
                  )}
                </div>
              );
            })}
          </div>
        )}
        {modelError && (
          <p className="model-error" role="alert">
            {modelError}
          </p>
        )}
      </div>

      <div className="settings-card directory-card">
        <div className="settings-card-heading">
          <div>
            <h3>Models directory</h3>
            <p>Choose where downloaded model data is stored.</p>
          </div>
        </div>
        <button
          type="button"
          className="path-row path-button"
          onClick={() => void chooseModelsDirectory()}
          aria-label="Change models directory"
        >
          <span>{config.models_directory ?? defaultModelsDirectory}</span>
          <span className="path-action">Change…</span>
        </button>
        {config.models_directory && (
          <button
            type="button"
            className="text-button reset-path"
            onClick={() => onChange({ models_directory: null })}
          >
            Use default location
          </button>
        )}
      </div>
    </div>
  );
}

function formatBytes(bytes: number | null): string {
  if (!bytes) return "Size unavailable";
  return `${Math.round(bytes / 1024 / 1024)} MB`;
}


function TranscriptionSettings({ config, onChange }: SettingsSectionProps) {
  return (
    <div className="settings-group" role="tabpanel" id="panel-transcription" aria-labelledby="tab-transcription">
      <section className="settings-card">
        <div className="settings-card-heading">
          <div>
            <h3>Secondary actions</h3>
            <p>Choose which quick actions sit under the record button.</p>
          </div>
        </div>
        <ToggleRow
          label="Show Transcribe URL"
          description="Free feature: add a one-click action to transcribe audio from a URL."
          checked={config.show_transcribe_url}
          onChange={(checked) => onChange({ show_transcribe_url: checked })}
        />
      </section>
      <div className="settings-card">
        <h3>Language Settings</h3>
        <SettingsRow
          label="Transcription Language"
          description="Whisper language code (e.g. en, es, fr)."
        >
          <input
            type="text"
            value={config.whisper_language}
            onChange={(e) => onChange({ whisper_language: e.target.value })}
            placeholder="en"
            className="input"
            aria-label="Transcription language"
          />
        </SettingsRow>
        <ToggleRow
          label="Translate to English"
          description="Translate non-English speech into English output."
          checked={config.translate_to_english}
          onChange={(checked) => onChange({ translate_to_english: checked })}
        />
      </div>

      <div className="settings-card">
        <h3>Output Options</h3>
        <ToggleRow
          label="Show Timestamps"
          description="Include timestamps in the transcription output."
          checked={config.show_timestamps}
          onChange={(checked) => onChange({ show_timestamps: checked })}
        />
        <ToggleRow
          label="Suppress Blank Audio"
          description="Filter out segments with no detected speech."
          checked={config.suppress_blank_audio}
          onChange={(checked) => onChange({ suppress_blank_audio: checked })}
        />
      </div>

      <div className="settings-card">
        <h3>Clipboard & Paste</h3>
        <ToggleRow
          label="Auto-copy transcription"
          description="Copy completed transcriptions to the clipboard automatically."
          checked={config.auto_copy_to_clipboard}
          onChange={(checked) => onChange({ auto_copy_to_clipboard: checked })}
        />
        <ToggleRow
          label="Auto-paste transcription"
          description="Paste completed transcriptions into the active field automatically."
          checked={config.auto_paste_transcription}
          onChange={(checked) => onChange({ auto_paste_transcription: checked })}
        />
        <ToggleRow
          label="Add space after sentence"
          description="Insert a space before pasting so text does not run together."
          checked={config.add_space_after_sentence}
          onChange={(checked) => onChange({ add_space_after_sentence: checked })}
        />
      </div>
    </div>
  );
}

const MAX_DICTIONARY_BYTES = 128 * 1024;

function DictionarySettings({ config, onChange }: SettingsSectionProps) {
  const byteCount = new TextEncoder().encode(config.custom_dictionary).length;
  const updateDictionary = (value: string) => {
    if (new TextEncoder().encode(value).length <= MAX_DICTIONARY_BYTES) {
      onChange({ custom_dictionary: value });
    }
  };

  return (
    <div className="settings-group" role="tabpanel" id="panel-dictionary" aria-labelledby="tab-dictionary">
      <section className="settings-card dictionary-card">
        <div className="settings-card-heading">
          <div>
            <h3>Custom dictionary</h3>
            <p id="dictionary-local-help">Manual and local. Corrections run on this device before history, copy, or paste.</p>
          </div>
        </div>
        <label className="dictionary-label" htmlFor="custom-dictionary">Terms and aliases</label>
        <textarea
          id="custom-dictionary"
          className="input dictionary-editor"
          value={config.custom_dictionary}
          onChange={(event) => updateDictionary(event.target.value)}
          maxLength={MAX_DICTIONARY_BYTES}
          rows={10}
          spellCheck={false}
          aria-describedby="dictionary-local-help dictionary-format-help dictionary-limit"
          placeholder={"Retex = retext\nUltraVox = Ultra Box"}
        />
        <p className="dictionary-help" id="dictionary-format-help">
          One entry per line: <code>Canonical term</code> or <code>Canonical term = alias one, alias two</code>. Blank lines and lines beginning with # or // are ignored. Exact aliases ignore case; distinctive terms also receive conservative typo correction.
        </p>
        <p className="dictionary-limit" id="dictionary-limit" aria-live="polite">
          {byteCount.toLocaleString()} / {MAX_DICTIONARY_BYTES.toLocaleString()} bytes
        </p>
        <p className="dictionary-help">
          UltraVox Light never scans Retex, contacts, files, or other apps for vocabulary.
        </p>
      </section>
    </div>
  );
}

function PrivacySettings({}: SettingsSectionProps) {
  const [automatic, setAutomatic] = useState(false);
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [updateMessage, setUpdateMessage] = useState<string | null>(null);
  const [updateBusy, setUpdateBusy] = useState(false);
  useEffect(() => { let cancelled = false; void Promise.all([getUpdatePreferences(), checkForUpdate()]).then(([preferences, candidate]) => { if (!cancelled) { setAutomatic(preferences.automatic); setUpdate(candidate); } }).catch(error => { if (!cancelled) setUpdateMessage(`Update check unavailable: ${String(error)}`); }); return () => { cancelled = true; }; }, []);
  const updateAutomatic = async (enabled: boolean) => { const previous = automatic; setAutomatic(enabled); try { await setUpdatePreferences({ automatic: enabled }); } catch (error) { setAutomatic(previous); setUpdateMessage(`Could not update automatic updates: ${String(error)}`); } };
  const installCandidate = async () => { if (!update) return; setUpdateBusy(true); try { await installUpdate(update); } catch (error) { setUpdateMessage(`Update was not installed: ${String(error)}`); } finally { setUpdateBusy(false); } };
  return <div className="settings-group" role="tabpanel" id="panel-privacy" aria-labelledby="tab-privacy"><div className="settings-card"><h3>Updates</h3><p className="description">Stable public releases are checked at launch and daily. Updates are verified with SHA-256 before installation.</p><ToggleRow label="Install updates automatically" description="Opt in to automatic public updates." checked={automatic} onChange={checked => void updateAutomatic(checked)} />{update && <div className="settings-row"><div className="settings-row-label"><label>UltraVox Light {update.latest_version} is available</label><p className="description">A failed check leaves the current app untouched.</p></div><button className="btn btn-primary" type="button" disabled={updateBusy} onClick={() => void installCandidate()}>{updateBusy ? "Verifying…" : "Update now"}</button></div>}{updateMessage && <p className="description" role="status">{updateMessage}</p>}</div></div>;
}

function SettingsRow({
  label,
  description,
  children,
}: {
  label: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <div className="settings-row">
      <div className="settings-row-label">
        <label>{label}</label>
        {description && <p className="description">{description}</p>}
      </div>
      {children}
    </div>
  );
}

function ToggleRow({
  label,
  description,
  checked,
  onChange,
}: {
  label: string;
  description?: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <div className="settings-row">
      <div className="settings-row-label">
        <label>{label}</label>
        {description && <p className="description">{description}</p>}
      </div>
      <label className="toggle" aria-label={label}>
        <input
          type="checkbox"
          checked={checked}
          onChange={(e) => onChange(e.target.checked)}
        />
        <span className="toggle-track">
          <span className="toggle-thumb" />
        </span>
      </label>
    </div>
  );
}

function AppearanceSettings({ config, onChange }: SettingsSectionProps) {
  return <div className="settings-group" role="tabpanel" id="panel-appearance" aria-labelledby="tab-appearance">
    <div className="settings-card"><div className="settings-card-heading"><h3>Theme</h3></div><ul className="theme-grid">{THEMES.map(theme => <li key={theme.id}><button type="button" className={`theme-card ${config.theme === theme.id ? "selected" : ""}`} onClick={() => onChange({ theme: theme.id })} aria-pressed={config.theme === theme.id}><span className="theme-card-preview" style={{ background: `linear-gradient(135deg, ${theme.swatch[1]}, ${theme.swatch[2]}) bottom / 100% 12px no-repeat, ${theme.swatch[0]}` }} /><span className="theme-card-name">{theme.name}</span><span className="theme-card-tagline">{theme.tagline}</span></button></li>)}</ul></div>
    <div className="settings-card"><div className="settings-card-heading"><h3>Recording meter</h3></div><ToggleRow label="Reactive microphone meter" description="Show the live microphone level while recording." checked={config.reactive_visuals_enabled} onChange={checked => onChange({ reactive_visuals_enabled: checked })} /></div>
  </div>;
}

const STORE_URL = "https://implosecybernetics.com/software/?product=ultravox";

async function openStorefront(): Promise<void> {
  try {
    await openExternal(STORE_URL);
  } catch {
    window.open(STORE_URL, "_blank");
  }
}

function SupportSettings() {
  return (
    <div className="settings-group" role="tabpanel" id="panel-support" aria-labelledby="tab-support">
      <section className="settings-card support-mission">
        <span className="eyebrow">Sustainable independent software</span>
        <h3>Help keep UltraVox maintained</h3>
        <p>
          UltraVox Light is free and open source. A one-time Pro license directly funds
          code signing, Windows and Linux packaging, compatibility work, support, and
          continued improvements to both editions.
        </p>
        <p>
          Light stays fully usable whether you upgrade or not. Pro is the practical way
          to support long-term development if UltraVox saves you time.
        </p>
        <button type="button" className="btn btn-primary" onClick={() => void openStorefront()}>
          Get a Pro license — $25
        </button>
        <span className="description">One-time purchase · perpetual license · updates included</span>
      </section>
      <section className="settings-card">
        <div className="settings-card-heading">
          <h3>What Pro adds</h3>
          <p>The extras are secondary to keeping the project healthy.</p>
        </div>
        <ul className="support-perks">
          <li>Meeting mode for mixed system and microphone audio, plus system-only Lecture mode</li>
          <li>The complete signature theme collection</li>
          <li>The now-playing media console with live transport and spectrum</li>
          <li>Authenticated releases through the Implose distribution channel</li>
        </ul>
      </section>
    </div>
  );
}

function CloseIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M18 6 6 18" />
      <path d="m6 6 12 12" />
    </svg>
  );
}

