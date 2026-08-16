import { useCallback, useEffect, useRef, useState } from "react";
import { ThinkingOrb } from "thinking-orbs";
import type { AppStatus } from "../App";
import {
  listRecordings,
  searchRecordings,
  deleteRecording,
  deleteAllRecordings,
  startRecording,
  stopRecording,
  importUrl,
  startMeeting,
  stopMeeting,
  exportRecording,
  retryTranscription,
  copyToClipboard,
  prepareModel,
  isModelDownloaded,
  getModelCatalog,
  getModelProgress,
  getSettings,
  setSettings,
  onRecordingAdded,
  onRecordingDeleted,
  onRecordingStarted,
  onRecordingStopped,
  onSettingsChanged,
  onMeetingStateChanged,
  onUrlImportProgress,
  type RecordingRow,
  type AppConfig,
  type ModelEntry,
} from "../ipc";
import type { UnlistenFn } from "@tauri-apps/api/event";

interface MainWindowProps {
  status: AppStatus;
  initialRecording: boolean;
  initialMeeting: boolean;
  onOpenSettings: () => void;
}

export function MainWindow({
  status,
  initialRecording,
  initialMeeting,
  onOpenSettings,
}: MainWindowProps) {
  const [recording, setRecording] = useState(initialRecording);
  const [meeting, setMeeting] = useState(initialMeeting);
  const [meetingPending, setMeetingPending] = useState(false);
  const [urlOpen, setUrlOpen] = useState(false);
  const [urlValue, setUrlValue] = useState("");
  const [urlImporting, setUrlImporting] = useState(false);
  const [urlProgress, setUrlProgress] = useState(0);
  const [urlStatus, setUrlStatus] = useState("");
  const [urlError, setUrlError] = useState<string | null>(null);
  const [recordings, setRecordings] = useState<RecordingRow[]>([]);
  const [search, setSearch] = useState("");
  const [loading, setLoading] = useState(true);
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [downloaded, setDownloaded] = useState<Record<string, boolean>>({});
  const [catalog, setCatalog] = useState<ModelEntry[]>([]);
  const [modelStatusLoading, setModelStatusLoading] = useState(true);
  const [preparingModelId, setPreparingModelId] = useState<string | null>(null);
  const [modelProgress, setModelProgress] = useState<Record<string, number>>({});
  const [modelError, setModelError] = useState<string | null>(null);
  const [activityError, setActivityError] = useState<string | null>(null);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [deletingAll, setDeletingAll] = useState(false);
  const [confirmingDeleteAll, setConfirmingDeleteAll] = useState(false);
  const [deleteAllError, setDeleteAllError] = useState<string | null>(null);
  const searchRef = useRef(search);
  const configRef = useRef<AppConfig | null>(null);
  const recordingsRequestRef = useRef(0);

  const mainRef = useRef<HTMLElement>(null);
  const historyModalRef = useRef<HTMLDivElement>(null);
  const viewAllRef = useRef<HTMLButtonElement>(null);
  const urlModalRef = useRef<HTMLElement>(null);
  const transcribeUrlRef = useRef<HTMLButtonElement>(null);
  const urlStatusRef = useRef<HTMLDivElement>(null);
  const deleteAllButtonRef = useRef<HTMLButtonElement>(null);
  const cancelDeleteAllRef = useRef<HTMLButtonElement>(null);
  const wasHistoryOpenRef = useRef(false);
  const wasConfirmingDeleteAllRef = useRef(false);
  const wasUrlOpenRef = useRef(false);

  const closeHistory = () => {
    setHistoryOpen(false);
    setSearch("");
    setConfirmingDeleteAll(false);
    setDeleteAllError(null);
    setHistoryError(null);
  };

  const refreshRecordings = useCallback(async () => {
    const requestId = ++recordingsRequestRef.current;
    const query = searchRef.current.trim();
    try {
      const rows = query ? await searchRecordings(query) : await listRecordings();
      if (requestId === recordingsRequestRef.current) {
        setRecordings(rows);
        setHistoryError(null);
      }
    } catch (loadError) {
      if (requestId === recordingsRequestRef.current) {
        setHistoryError(`Could not load messages: ${String(loadError)}`);
      }
    } finally {
      if (requestId === recordingsRequestRef.current) {
        setLoading(false);
      }
    }
  }, []);

  const refreshCatalog = useCallback(async () => {
    setModelStatusLoading(true);
    try {
      const modelCatalog = await getModelCatalog();
      const visibleCatalog = ["fluidaudio-en-v2", "fluidaudio-multilingual-v3"]
        .map((id) => modelCatalog.models.find((model) => model.id === id))
        .filter((model): model is ModelEntry => model !== undefined);
      const statuses = Object.fromEntries(
        await Promise.all(
          visibleCatalog.map(async (model) => [
            model.id,
            await isModelDownloaded(model.id),
          ] as const),
        ),
      );
      setCatalog(visibleCatalog);
      setDownloaded(statuses);
      setModelError(
        visibleCatalog.length === 2
          ? null
          : "The model catalog is incomplete. Restart UltraVox and try again.",
      );
    } catch (catalogError) {
      setModelError(`Could not load model status: ${String(catalogError)}`);
    } finally {
      setModelStatusLoading(false);
    }
  }, []);

  useEffect(() => {
    setRecording(initialRecording);
  }, [initialRecording]);

  useEffect(() => {
    setMeeting(initialMeeting);
  }, [initialMeeting]);

  useEffect(() => {
    searchRef.current = search;
    setLoading(true);
    void refreshRecordings();
  }, [search, refreshRecordings]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: UnlistenFn[] = [];

    async function loadConfig() {
      try {
        const loadedConfig = await getSettings();
        if (!configRef.current) {
          configRef.current = loadedConfig;
          setConfig(loadedConfig);
        }
      } catch (configError) {
        setActivityError(`Could not load settings: ${String(configError)}`);
      }
    }

    async function setupListeners() {
      const results = await Promise.allSettled([
        onRecordingAdded(() => void refreshRecordings()),
        onRecordingDeleted(() => void refreshRecordings()),
        onRecordingStarted(() => setRecording(true)),
        onRecordingStopped(() => setRecording(false)),
        onMeetingStateChanged(setMeeting),
        onUrlImportProgress((payload) => {
          setUrlProgress(payload.progress);
          setUrlStatus(payload.status);
        }),
        onSettingsChanged((payload) => {
          const directoryChanged =
            configRef.current?.models_directory !== payload.config.models_directory;
          configRef.current = payload.config;
          setConfig(payload.config);
          if (directoryChanged) void refreshCatalog();
        }),
      ]);
      const listeners = results.flatMap((result) =>
        result.status === "fulfilled" ? [result.value] : [],
      );
      const failed = results.find((result) => result.status === "rejected");
      if (failed?.status === "rejected") {
        setActivityError(`Could not connect to app events: ${String(failed.reason)}`);
      }
      if (cancelled) {
        listeners.forEach((stopListening) => stopListening());
      } else {
        unlisten = listeners;
      }
    }

    void loadConfig();
    void refreshCatalog();
    void setupListeners();
    return () => {
      cancelled = true;
      unlisten.forEach((stopListening) => stopListening());
    };
  }, [refreshCatalog, refreshRecordings]);

  useEffect(() => {
    if (!historyOpen) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (confirmingDeleteAll) {
        setConfirmingDeleteAll(false);
      } else {
        closeHistory();
      }
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [confirmingDeleteAll, historyOpen]);

  useEffect(() => {
    if (!urlOpen || urlImporting) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setUrlOpen(false);
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [urlImporting, urlOpen]);

  useEffect(() => {
    const main = mainRef.current;
    if (!main) return;
    main.toggleAttribute("inert", historyOpen || urlOpen);
  }, [historyOpen, urlOpen]);

  useEffect(() => {
    if (!historyOpen) return;
    const modal = historyModalRef.current;
    if (!modal) return;
    const focusableSelector =
      'a[href], button:not([disabled]), input:not([disabled]), textarea, select, [tabindex]:not([tabindex="-1"])';
    const trap = (event: KeyboardEvent) => {
      if (event.key !== "Tab") return;
      const focusable = Array.from(
        modal.querySelectorAll<HTMLElement>(focusableSelector),
      );
      if (focusable.length === 0) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement as HTMLElement | null;
      if (event.shiftKey) {
        if (!active || active === first || !focusable.includes(active)) {
          event.preventDefault();
          last.focus();
        }
      } else {
        if (!active || active === last || !focusable.includes(active)) {
          event.preventDefault();
          first.focus();
        }
      }
    };
    modal.addEventListener("keydown", trap);
    return () => modal.removeEventListener("keydown", trap);
  }, [historyOpen]);

  useEffect(() => {
    if (!urlOpen) return;
    const modal = urlModalRef.current;
    if (!modal) return;
    const focusableSelector =
      'a[href], button:not([disabled]), input:not([disabled]), textarea, select, [tabindex]:not([tabindex="-1"])';
    const trap = (event: KeyboardEvent) => {
      if (event.key !== "Tab") return;
      const focusable = Array.from(
        modal.querySelectorAll<HTMLElement>(focusableSelector),
      );
      if (focusable.length === 0) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement as HTMLElement | null;
      if (event.shiftKey && (!active || active === first || !focusable.includes(active))) {
        event.preventDefault();
        last.focus();
      } else if (
        !event.shiftKey &&
        (!active || active === last || !focusable.includes(active))
      ) {
        event.preventDefault();
        first.focus();
      }
    };
    modal.addEventListener("keydown", trap);
    return () => modal.removeEventListener("keydown", trap);
  }, [urlOpen]);

  useEffect(() => {
    if (confirmingDeleteAll) {
      cancelDeleteAllRef.current?.focus();
    } else if (wasConfirmingDeleteAllRef.current && historyOpen) {
      deleteAllButtonRef.current?.focus();
    }
    wasConfirmingDeleteAllRef.current = confirmingDeleteAll;
  }, [confirmingDeleteAll, historyOpen]);

  useEffect(() => {
    if (wasHistoryOpenRef.current && !historyOpen) {
      viewAllRef.current?.focus();
    }
    wasHistoryOpenRef.current = historyOpen;
  }, [historyOpen]);

  useEffect(() => {
    if (wasUrlOpenRef.current && !urlOpen) {
      transcribeUrlRef.current?.focus();
    }
    wasUrlOpenRef.current = urlOpen;
  }, [urlOpen]);

  useEffect(() => {
    if (urlImporting) {
      urlStatusRef.current?.focus();
    }
  }, [urlImporting]);

  const toggleRecording = async () => {
    setActivityError(null);
    try {
      if (recording) {
        await stopRecording();
      } else {
        await startRecording();
      }
    } catch (recordingError) {
      setActivityError(
        `${recording ? "Could not stop recording" : "Could not start recording"}: ${String(recordingError)}`,
      );
    }
  };

  const toggleMeeting = async () => {
    if (meetingPending) return;
    setMeetingPending(true);
    setActivityError(null);
    try {
      if (meeting) {
        await stopMeeting();
        setMeeting(false);
        await refreshRecordings();
      } else {
        await startMeeting();
        setMeeting(true);
      }
    } catch (meetingError) {
      setActivityError(
        `${meeting ? "Could not stop meeting mode" : "Could not start meeting mode"}: ${String(meetingError)}`,
      );
    } finally {
      setMeetingPending(false);
    }
  };

  const openUrlImport = () => {
    setUrlValue("");
    setUrlProgress(0);
    setUrlStatus("");
    setUrlError(null);
    setUrlOpen(true);
  };

  const submitUrlImport = async () => {
    const value = urlValue.trim();
    if (!value) {
      setUrlError("Enter a YouTube or direct media URL.");
      return;
    }
    setUrlImporting(true);
    setUrlProgress(0);
    setUrlStatus("Starting download");
    setUrlError(null);
    try {
      await importUrl(value);
      setUrlProgress(1);
      setUrlStatus("Transcription started");
      await refreshRecordings();
    } catch (importError) {
      setUrlError(String(importError));
      setUrlStatus("");
    } finally {
      setUrlImporting(false);
    }
  };

  const onDelete = async (id: string) => {
    setHistoryError(null);
    try {
      await deleteRecording(id);
    } catch (deleteError) {
      setHistoryError(`Could not delete the message: ${String(deleteError)}`);
    }
  };

  const onDeleteAll = async () => {
    setDeletingAll(true);
    setDeleteAllError(null);
    try {
      await deleteAllRecordings();
      setRecordings([]);
      setSearch("");
      setConfirmingDeleteAll(false);
    } catch (deleteError) {
      setDeleteAllError(String(deleteError));
    } finally {
      setDeletingAll(false);
    }
  };

  const onRetry = async (id: string) => {
    setHistoryError(null);
    try {
      await retryTranscription(id);
    } catch (retryError) {
      setHistoryError(`Could not retry transcription: ${String(retryError)}`);
    }
  };

  const onCopy = async (text: string) => {
    setHistoryError(null);
    try {
      await copyToClipboard(text);
    } catch (copyError) {
      setHistoryError(`Could not copy the transcript: ${String(copyError)}`);
    }
  };

  const onExport = async (row: RecordingRow) => {
    setHistoryError(null);
    try {
      const destination = `~/Downloads/${row.file_name}`;
      await exportRecording(row.id, destination);
    } catch (exportError) {
      setHistoryError(`Could not export the recording: ${String(exportError)}`);
    }
  };

  const selectedModelId =
    config?.selected_engine === "fluidaudio" && config.fluid_audio_model_version === "v3"
      ? "fluidaudio-multilingual-v3"
      : "fluidaudio-en-v2";
  const selectLanguage = async (mode: "english" | "multilingual"): Promise<boolean> => {
    try {
      const current = configRef.current ?? config ?? (await getSettings());
      const next: AppConfig = {
        ...current,
        selected_engine: "fluidaudio",
        fluid_audio_model_version: mode === "english" ? "v2" : "v3",
        model_language: mode,
        onboarding_completed: true,
      };
      configRef.current = next;
      setConfig(next);
      await setSettings(next);
      return true;
    } catch (settingsError) {
      setModelError(`Could not save the model choice: ${String(settingsError)}`);
      return false;
    }
  };

  useEffect(() => {
    if (!preparingModelId) return;
    let cancelled = false;
    const update = async () => {
      try {
        const fraction = await getModelProgress(preparingModelId);
        if (!cancelled) {
          setModelProgress((current) => ({ ...current, [preparingModelId]: fraction }));
        }
      } catch {
        // The preparation call remains the source of truth for failure.
      }
    };
    void update();
    const interval = window.setInterval(update, 250);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, [preparingModelId]);

  const startModelDownload = async (model: ModelEntry) => {
    const mode = model.id === "fluidaudio-multilingual-v3" ? "multilingual" : "english";
    if (!(await selectLanguage(mode))) return;

    setModelError(null);
    setPreparingModelId(model.id);
    setModelProgress((current) => ({ ...current, [model.id]: 0 }));
    try {
      const prepared = await prepareModel(model.id);
      if (!prepared) {
        throw new Error("The download did not complete.");
      }
      setModelProgress((current) => ({ ...current, [model.id]: 1 }));
      setDownloaded((current) => ({ ...current, [model.id]: true }));
    } catch (downloadError) {
      setModelError(`Could not download the model: ${String(downloadError)}`);
    } finally {
      setPreparingModelId(null);
    }
  };

  const selectedDownloaded = downloaded[selectedModelId] === true;
  const needsOnboarding = !modelStatusLoading && !selectedDownloaded;
  const latestRecording = recordings[0];
  const transcribing = ["pending", "converting", "transcribing"].includes(
    latestRecording?.status ?? "",
  );
  let statusKind: AppStatus | "recording" | "meeting" | "transcribing" = status;
  if (recording) {
    statusKind = "recording";
  } else if (meeting || meetingPending) {
    statusKind = "meeting";
  } else if (transcribing) {
    statusKind = "transcribing";
  } else if (modelStatusLoading) {
    statusKind = "loading";
  }

  let statusText = "Unavailable";
  if (recording) {
    statusText = "Recording";
  } else if (meetingPending) {
    statusText = meeting ? "Finishing meeting…" : "Starting meeting…";
  } else if (meeting) {
    statusText = "Meeting mode";
  } else if (transcribing) {
    if (latestRecording?.status === "converting") {
      statusText = "Preparing audio";
    } else {
      statusText = "Transcribing";
    }
  } else if (modelStatusLoading) {
    statusText = "Checking model…";
  } else if (status === "ready") {
    statusText = "Ready";
  } else if (status === "loading") {
    statusText = "Loading…";
  }

  return (
    <div className="app">

      <main ref={mainRef} className="main">
        {needsOnboarding ? (
          <section className="onboarding-card" aria-labelledby="model-setup-title">
            <button
              className="icon-button frame-settings"
              type="button"
              title="Settings"
              onClick={onOpenSettings}
              aria-label="Open settings"
            >
              <SettingsIcon />
            </button>
            <div className="onboarding-heading">
              <div>
                <span className="eyebrow">One-time setup</span>
                <h1 id="model-setup-title">Choose a transcription model</h1>
              </div>
            </div>
            <p className="description">Models run privately on your Mac and remain available offline.</p>
            {catalog.length === 0 && !modelError ? (
              <p className="setup-loading" aria-live="polite">Checking model status…</p>
            ) : (
              <div className="model-choice" role="radiogroup" aria-label="Transcription model">
                {catalog.map((model) => {
                  const mode =
                    model.id === "fluidaudio-multilingual-v3" ? "multilingual" : "english";
                  return (
                    <ModelCard
                      key={model.id}
                      model={model}
                      title={mode === "english" ? "English" : "Multilingual"}
                      selected={model.id === selectedModelId}
                      downloaded={downloaded[model.id] === true}
                      preparing={preparingModelId === model.id}
                      progress={modelProgress[model.id] ?? 0}
                      onSelect={() => void selectLanguage(mode)}
                      onDownload={() => void startModelDownload(model)}
                    />
                  );
                })}
              </div>
            )}
            {modelError && <p className="setup-error" role="alert">{modelError}</p>}
          </section>
        ) : (
          <section className="hero" aria-labelledby="activity-status">
            <button
              className="icon-button frame-settings"
              type="button"
              title="Settings"
              onClick={onOpenSettings}
              aria-label="Open settings"
            >
              <SettingsIcon />
            </button>
            <div
              id="activity-status"
              className={`status-label ${statusKind}`}
              aria-live="polite"
            >
              {statusKind === "ready" && <SmallMicIcon />}
              {statusKind === "loading" && (
                <ThinkingOrb state="working" size={20} theme="dark" aria-hidden="true" />
              )}
              {statusKind === "recording" && (
                <ThinkingOrb state="listening" size={20} theme="dark" aria-hidden="true" />
              )}
              {statusKind === "meeting" && (
                <ThinkingOrb state="listening" size={20} theme="dark" aria-hidden="true" />
              )}
              {statusKind === "transcribing" && (
                <ThinkingOrb state="composing" size={20} theme="dark" aria-hidden="true" />
              )}
              {statusText}
            </div>

            <button
              className={`record-button ${recording || meeting ? "recording" : ""}`}
              onClick={() => void (meeting ? toggleMeeting() : toggleRecording())}
              aria-label={meeting ? "Stop meeting" : recording ? "Stop recording" : "Start recording"}
              disabled={
                meetingPending ||
                (!recording &&
                  !meeting &&
                  (modelStatusLoading || status !== "ready" || transcribing))
              }
              aria-describedby={activityError ? "activity-error" : undefined}
            >
              {recording || meeting ? <StopIcon /> : <MicIcon />}
            </button>

            <div className="secondary-actions">
              {meeting ? (
                <span className="capture-note">System audio + microphone</span>
              ) : (
                <>
                  <button
                    type="button"
                    className="secondary-action"
                    onClick={() => void toggleMeeting()}
                    disabled={
                      meetingPending ||
                      recording ||
                      transcribing ||
                      modelStatusLoading ||
                      status !== "ready"
                    }
                    aria-pressed={false}
                  >
                    {meetingPending ? "Starting…" : "Meeting mode"}
                  </button>
                  <button
                    type="button"
                    className="secondary-action"
                    ref={transcribeUrlRef}
                    onClick={openUrlImport}
                    disabled={recording || meetingPending || transcribing}
                  >
                    Transcribe URL
                  </button>
                </>
              )}
            </div>
            {activityError && <p id="activity-error" className="activity-error" role="alert">{activityError}</p>}
          </section>
        )}

        <section className="history-section" aria-labelledby="latest-message-title">
          <div className="history-heading">
            <span id="latest-message-title">Latest message</span>
            <button
              ref={viewAllRef}
              className="history-link"
              onClick={() => setHistoryOpen(true)}
            >
              View all
            </button>
          </div>

          {historyError ? (
            <div className="inline-error" role="alert">
              <span>{historyError}</span>
              <button className="text-button" type="button" onClick={() => void refreshRecordings()}>
                Try again
              </button>
            </div>
          ) : loading ? (
            <div className="empty-state compact">
              <WaveIcon />
              <p>Loading messages…</p>
            </div>
          ) : recordings.length === 0 ? (
            <div className="empty-state compact">
              <WaveIcon />
              <p>Your latest transcription will appear here.</p>
            </div>
          ) : (
            <RecordingCard
              row={recordings[0]}
              context="latest"
              onCopy={onCopy}
              onRetry={onRetry}
              retryDisabled={meeting}
              onExport={onExport}
              onDelete={onDelete}
            />
          )}
        </section>
      </main>

      {historyOpen && (
        <div
          ref={historyModalRef}
          className="history-modal"
          role="dialog"
          aria-modal="true"
          aria-labelledby="history-title"
        >
          <header className="history-modal-header">
            <div>
              <h2 id="history-title">All messages</h2>
            </div>
            <button
              className="icon-button"
              onClick={closeHistory}
              title="Close"
              aria-label="Close message history"
            >
              <CloseIcon />
            </button>
          </header>
          <div className="history-modal-toolbar">
            {confirmingDeleteAll ? (
              <div className="delete-all-confirmation" role="group" aria-label="Confirm delete all">
                <span>Delete every saved message and its audio file?</span>
                <div>
                  <button
                    ref={cancelDeleteAllRef}
                    type="button"
                    className="btn btn-small"
                    onClick={() => setConfirmingDeleteAll(false)}
                    disabled={deletingAll}
                  >
                    Cancel
                  </button>
                  <button
                    type="button"
                    className="btn btn-danger history-delete-all"
                    onClick={() => void onDeleteAll()}
                    disabled={deletingAll || recording || transcribing}
                  >
                    {deletingAll ? "Deleting…" : "Delete everything"}
                  </button>
                </div>
              </div>
            ) : (
              <>
                <input
                  type="search"
                  className="input search-input"
                  placeholder="Search messages…"
                  value={search}
                  onChange={(event) => setSearch(event.target.value)}
                  aria-label="Search messages"
                  autoFocus
                />
                <button
                  ref={deleteAllButtonRef}
                  type="button"
                  className="btn btn-danger history-delete-all"
                  onClick={() => setConfirmingDeleteAll(true)}
                  disabled={deletingAll || recording || transcribing}
                >
                  Delete all
                </button>
              </>
            )}
          </div>
          {(deleteAllError || historyError) && (
            <p className="history-modal-error" role="alert">{deleteAllError ?? historyError}</p>
          )}
          <div className="history-modal-content">
            {loading ? (
              <div className="empty-state">
                <WaveIcon />
                <p>Loading messages…</p>
              </div>
            ) : recordings.length === 0 ? (
              <div className="empty-state">
                <WaveIcon />
                <p>{search ? "Try a different search." : "New transcriptions will appear here."}</p>
              </div>
            ) : (
              <ul className="recording-list" aria-label="Message history">
                {recordings.map((row) => (
                  <li key={row.id}>
                    <RecordingCard
                      row={row}
                      context="history"
                      onCopy={onCopy}
                      onRetry={onRetry}
                      retryDisabled={meeting}
                      onExport={onExport}
                      onDelete={onDelete}
                    />
                  </li>
                ))}
              </ul>
            )}
          </div>
        </div>
      )}

      {urlOpen && (
        <div className="compact-modal-backdrop">
          <section
            className="compact-modal"
            ref={urlModalRef}
            role="dialog"
            aria-modal="true"
            aria-labelledby="url-import-title"
          >
            <div className="compact-modal-heading">
              <div>
                <span className="eyebrow">Local transcription</span>
                <h2 id="url-import-title">Transcribe from URL</h2>
              </div>
              <button
                type="button"
                className="icon-button"
                onClick={() => setUrlOpen(false)}
                disabled={urlImporting}
                aria-label="Close URL transcription"
              >
                <CloseIcon />
              </button>
            </div>
            <p className="compact-modal-copy">
              Paste a YouTube link or direct media URL. UltraVox downloads only the audio, then uses your selected on-device model.
            </p>
            <form
              className="url-import-form"
              onSubmit={(event) => {
                event.preventDefault();
                void submitUrlImport();
              }}
            >
              <input
                className="input"
                type="url"
                value={urlValue}
                onChange={(event) => setUrlValue(event.target.value)}
                placeholder="https://…"
                aria-label="Media URL"
                autoFocus
                disabled={urlImporting}
              />
              {(urlImporting || urlStatus) && (
                <div
                  className="url-import-progress"
                  ref={urlStatusRef}
                  role="status"
                  aria-live="polite"
                  tabIndex={urlImporting ? 0 : -1}
                >
                  <progress value={urlProgress} max={1} />
                  <span>{urlStatus}</span>
                </div>
              )}
              {urlError && <p className="activity-error" role="alert">{urlError}</p>}
              <div className="compact-modal-actions">
                <button
                  type="button"
                  className="btn"
                  onClick={() => setUrlOpen(false)}
                  disabled={urlImporting}
                >
                  Close
                </button>
                <button
                  type="submit"
                  className="btn btn-primary"
                  disabled={urlImporting || !urlValue.trim()}
                >
                  {urlImporting ? "Downloading…" : "Transcribe"}
                </button>
              </div>
            </form>
          </section>
        </div>
      )}

    </div>
  );
}

function formatDuration(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}

function formatTimestamp(timestamp: string): string {
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return timestamp;
  return date.toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

type RecordingCardProps = {
  row: RecordingRow;
  context: "latest" | "history";
  onCopy: (text: string) => void;
  onRetry: (id: string) => void;
  retryDisabled: boolean;
  onExport: (row: RecordingRow) => void;
  onDelete: (id: string) => void;
};

function RecordingCard({
  row,
  context,
  onCopy,
  onRetry,
  onExport,
  retryDisabled,
  onDelete,
}: RecordingCardProps) {
  const showStatus = context === "history" || row.status !== "completed";
  const isInProgress = row.status === "pending"
    || row.status === "converting"
    || row.status === "transcribing";

  return (
    <article className={`recording-card ${row.status} ${context}`}>
      <div className="recording-header">
        <div className="recording-title-row">
          <time className="recording-title" dateTime={row.timestamp}>
            {formatTimestamp(row.timestamp)}
          </time>
          <span className="recording-duration">{formatDuration(row.duration_seconds)}</span>
        </div>
        {showStatus && (
          <span className={`recording-status-badge ${row.status}`}>{row.status}</span>
        )}
      </div>

      <p className="recording-preview">{row.preview}</p>

      {row.status !== "completed" && row.status !== "failed" && (
        <progress
          className="recording-progress"
          value={row.progress}
          max={1}
          aria-label={`${row.status} progress`}
        />
      )}

      <div className="recording-actions">
        {row.status === "completed" && row.transcription && (
          <button className="btn btn-small" onClick={() => onCopy(row.transcription!)} title="Copy transcript">
            Copy
          </button>
        )}
        {row.status === "failed" && (
          <button
            className="btn btn-small"
            onClick={() => onRetry(row.id)}
            title={retryDisabled ? "Stop meeting mode before retrying" : "Retry transcription"}
            disabled={retryDisabled}
          >
            Retry
          </button>
        )}
        <button className="btn btn-small" onClick={() => onExport(row)} title="Export recording">
          Export
        </button>
        <button
          className="btn btn-small btn-danger"
          onClick={() => onDelete(row.id)}
          title={isInProgress ? "Wait for transcription to finish before deleting" : "Delete recording"}
          disabled={isInProgress}
        >
          Delete
        </button>
      </div>
    </article>
  );
}

function SmallMicIcon() {
  return (
    <svg className="status-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z" />
      <path d="M19 10v2a7 7 0 0 1-14 0v-2" />
    </svg>
  );
}

function CloseIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
      <path d="M18 6 6 18M6 6l12 12" />
    </svg>
  );
}

function MicIcon() {
  return (
    <svg width="36" height="36" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z" />
      <path d="M19 10v2a7 7 0 0 1-14 0v-2" />
      <line x1="12" x2="12" y1="19" y2="22" />
    </svg>
  );
}

function StopIcon() {
  return (
    <svg width="32" height="32" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <rect x="6" y="6" width="12" height="12" rx="2" />
    </svg>
  );
}

function SettingsIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.1a2 2 0 0 1-1-1.72v-.51a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" />
      <circle cx="12" cy="12" r="3" />
    </svg>
  );
}

function ModelCard({
  model,
  title,
  selected,
  downloaded,
  preparing,
  progress,
  onSelect,
  onDownload,
}: {
  model: ModelEntry;
  title: string;
  selected: boolean;
  downloaded: boolean;
  preparing: boolean;
  progress: number;
  onSelect: () => void;
  onDownload: () => void;
}) {
  const fraction = Math.max(0, Math.min(1, progress));
  const size = model.size_bytes
    ? `${Math.round(model.size_bytes / 1024 / 1024)} MB`
    : "Size unavailable";
  let status = "Not downloaded";
  if (downloaded) {
    status = "Downloaded";
  } else if (preparing) {
    status = `${Math.round(fraction * 100)}% downloaded`;
  }

  return (
    <div className={`model-card${selected ? " selected" : ""}`}>
      <button
        type="button"
        className="model-card-select"
        role="radio"
        aria-checked={selected}
        onClick={onSelect}
      >
        <span className="check" aria-hidden="true">{selected ? "✓" : ""}</span>
        <span className="model-card-copy">
          <strong>{title}</strong>
          <span className={downloaded ? "downloaded" : ""} aria-live="polite">{size} · {status}</span>
          {preparing && (
            <progress
              value={fraction}
              max={1}
              aria-label={`${title} download progress, ${Math.round(fraction * 100)} percent`}
            />
          )}
        </span>
      </button>
      {!downloaded && (
        <button
          type="button"
          className="btn btn-small btn-primary"
          onClick={onDownload}
          disabled={preparing}
          aria-label={`Download ${title} model, ${size}`}
        >
          {preparing ? "Downloading…" : "Download"}
        </button>
      )}
    </div>
  );
}

function WaveIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M2 10v4" />
      <path d="M6 6v12" />
      <path d="M10 3v18" />
      <path d="M14 8v8" />
      <path d="M18 5v14" />
      <path d="M22 10v4" />
    </svg>
  );
}
