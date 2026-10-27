import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { MainWindow } from "./pages/MainWindow";
import { VoiceStudioPage } from "@pro";
import { SettingsPage, type SettingsTab } from "./pages/SettingsPage";
import { useUpdaterLifecycle } from "./lib/useUpdater";
import { useProStatus } from "./lib/useProStatus";
import {
  canRequestPermission,
  createPermissionReadinessController,
  dictationAccess,
  microphonePromptNote,
  nextMicrophonePromptState,
  permissionActionFailure,
  permissionStateLabel,
  type MicrophonePromptState,
  type PermissionReadinessController,
} from "./lib/permissionReadiness";
import {
  bootstrapDistributionAccess,
  getAppInfo,
  getAppStatus,
  getAppTelemetryStatus,
  getLegacyMigrationNotice,
  getPendingMeetingDetection,
  getPermissionStatus,
  getSettings,
  onMeetingDetectionPending,
  openPermissionSettings,
  registerDeviceLicense,
  requestPermission,
  respondMeetingDetection,
  setAppTelemetryEnabled,
  type AppConfig,
  type AppStatusResponse,
  type BuildFlavor,
  type MeetingDetectionPendingPayload,
  type PermissionKind,
  type PermissionStatus,
  type TelemetryStatus,

} from "./ipc";
const PERMISSIONS: Array<{ kind: PermissionKind; label: string; copy: string }> = [

  { kind: "microphone", label: "Microphone", copy: "Record speech for local transcription." },
  { kind: "accessibility", label: "Accessibility (optional)", copy: "Place the transcription in the focused text field." },
  { kind: "screen_recording", label: "Screen Recording (optional)", copy: "Capture meeting system audio with ScreenCaptureKit." },
];
function TelemetryConsent({
  onDecision,
}: {
  onDecision: (enabled: boolean) => Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const decide = async (enabled: boolean) => {
    setBusy(true);
    setError(null);
    try {
      await onDecision(enabled);
    } catch {
      setError("The privacy choice could not be saved. Try again.");
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="compact-modal-backdrop" role="presentation">
      <section className="compact-modal permission-panel" role="dialog" aria-modal="true" aria-labelledby="telemetry-title">
        <span className="eyebrow">Privacy choice</span>
        <h2 id="telemetry-title">Help improve UltraVox?</h2>
        <p className="compact-modal-copy">
          Optional telemetry is off by default. If you accept, UltraVox sends a random
          app-install ID, app version, platform, architecture, UTC day, and daily counts
          of recordings, transcriptions, and model downloads. It never sends transcripts,
          audio, prompts, recordings, URLs, paths, errors, or hardware identifiers.
          Identifier rows expire within 34 UTC days; ID-free daily totals within 360 days.
        </p>
        <p className="permission-footnote">You can change this later in Settings → Privacy. Declining creates no app-install ID.</p>
        {error && <p className="settings-error" role="alert">{error}</p>}
        <div className="compact-modal-actions">
          <button className="btn" type="button" disabled={busy} onClick={() => void decide(false)}>No thanks</button>
          <button className="btn btn-primary" type="button" disabled={busy} onClick={() => void decide(true)}>Allow optional telemetry</button>
        </div>
      </section>
    </div>
  );
}
function PermissionPanel({
  status,
  busy,
  error,
  requesting,
  microphonePrompt,
  onAction,
  onRecheck,
  onOpenSettings,
  migrationNotice,
}: {
  status: PermissionStatus | null;
  busy: boolean;
  error: string | null;
  requesting: PermissionKind | null;
  microphonePrompt: MicrophonePromptState;
  onAction: (kind: PermissionKind, action: "request" | "settings") => Promise<void>;
  onRecheck: () => Promise<void>;
  onOpenSettings: () => void;
  migrationNotice: string | null;
}) {
  return (
    <div className="compact-modal-backdrop" role="presentation">
      <section className="compact-modal permission-panel" role="dialog" aria-modal="true" aria-labelledby="permission-title">
        <h2 id="permission-title">Microphone access for dictation</h2>
        {migrationNotice && <p className="compact-modal-copy" role="status">{migrationNotice}</p>}
        <div className="permission-list">
          {PERMISSIONS.map(({ kind, label, copy }) => {
            const state = status?.[kind];
            const prompt = kind === "microphone" ? microphonePrompt : null;
            const note = microphonePromptNote(prompt);
            const stateLabel = requesting === kind
              ? "Waiting for your answer"
              : prompt === "timed_out"
                ? "Timed out"
                : prompt === "pending"
                  ? "Waiting for macOS"
                  : permissionStateLabel(state, !status && !!error);
            const canRequest = canRequestPermission(kind, state, prompt);
            return (
              <div className="permission-row" key={kind}>
                <div><strong>{label}</strong><span>{copy}</span>{note && <span>{note}</span>}</div>
                <span className={`permission-state permission-${state ?? "unknown"}`}>{stateLabel}</span>
                {(state === "denied" || state === "not_determined") && (
                  <div className="permission-actions">
                    {canRequest && (
                      <button className="btn btn-primary" type="button" disabled={busy} onClick={() => void onAction(kind, "request")}>Allow</button>
                    )}
                    <button className={canRequest ? "btn" : "btn btn-primary"} type="button" disabled={busy} onClick={() => void onAction(kind, "settings")}>Open macOS Settings</button>
                  </div>
                )}
              </div>
            );
          })}
        </div>
        {error && <p className="settings-error" role="alert">{error}</p>}
        <div className="compact-modal-actions">
          <button className="btn" type="button" disabled={busy} onClick={() => void onRecheck()}>{busy && !requesting ? "Checking…" : "Recheck permissions"}</button>
          <button className="btn" type="button" onClick={onOpenSettings}>App Settings</button>
        </div>
      </section>
    </div>
  );
}

export type AppStatus = AppStatusResponse["status"];

export default function App() {
  const [showSettings, setShowSettings] = useState(false);
  const [settingsTab, setSettingsTab] = useState<SettingsTab>("shortcut");
  const [mode, setMode] = useState<"dictation" | "studio">("dictation");
  const [studioVisited, setStudioVisited] = useState(false);
  const [studioBlocked, setStudioBlocked] = useState(false);
  const studioReturnFocus = useRef<HTMLElement | null>(null);
  const openStudio = useCallback(() => {
    studioReturnFocus.current = document.activeElement as HTMLElement | null;
    setStudioVisited(true);
    setMode("studio");
  }, []);
  const closeStudio = useCallback(() => {
    setMode("dictation");
    window.setTimeout(() => studioReturnFocus.current?.focus(), 0);
  }, []);
  const [settingsConfig, setSettingsConfig] = useState<AppConfig | null>(null);
  const [status, setStatus] = useState<AppStatus>("loading");
  const [initialRecording, setInitialRecording] = useState(false);
  const [permissionStatus, setPermissionStatus] = useState<PermissionStatus | null>(null);
  const [permissionError, setPermissionError] = useState<string | null>(null);
  const [permissionBusy, setPermissionBusy] = useState(false);
  const [requestingPermission, setRequestingPermission] = useState<PermissionKind | null>(null);
  const [microphonePrompt, setMicrophonePrompt] = useState<MicrophonePromptState>(null);
  const permissionController = useRef<PermissionReadinessController | null>(null);
  const [telemetryStatus, setTelemetryStatus] = useState<TelemetryStatus | null>(null);
  const [migrationNotice, setMigrationNotice] = useState<string | null>(null);
  const [initialMeeting, setInitialMeeting] = useState(false);
  const [detection, setDetection] = useState<MeetingDetectionPendingPayload | null>(null);
  const [decisionError, setDecisionError] = useState<string | null>(null);
  const promptRef = useRef<HTMLDivElement>(null);
  const returnFocusRef = useRef<HTMLElement | null>(null);
  const isReminderWindow = getCurrentWindow().label === "meeting-reminder";

  const [pro, refreshPro] = useProStatus();
  const [build, setBuild] = useState<BuildFlavor | null>(null);

  useEffect(() => {
    if (isReminderWindow) return;
    const controller = createPermissionReadinessController({
      readStatus: getPermissionStatus,
      onStatus: (next) => {
        setPermissionStatus(next);
        setMicrophonePrompt((previous) => nextMicrophonePromptState(previous, next));
      },
      onError: setPermissionError,
      onBusy: setPermissionBusy,
    });
    permissionController.current = controller;
    void controller.refresh();
    const onFocus = () => {
      if (document.visibilityState === "visible") controller.scheduleRefresh();
    };
    const onVisibility = () => {
      if (document.visibilityState === "visible") controller.scheduleRefresh();
    };
    window.addEventListener("focus", onFocus);
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      window.removeEventListener("focus", onFocus);
      document.removeEventListener("visibilitychange", onVisibility);
      controller.dispose();
      if (permissionController.current === controller) permissionController.current = null;
    };
  }, [isReminderWindow]);

  const recheckPermissions = useCallback(() => {
    return permissionController.current?.refresh() ?? Promise.resolve();
  }, []);
  const permissionAction = useCallback(async (kind: PermissionKind, action: "request" | "settings") => {
    const controller = permissionController.current;
    if (!controller) return;
    if (action === "request") setRequestingPermission(kind);
    try {
      await controller.runAction(
        permissionActionFailure(kind, action),
        async () => action === "request"
          ? await requestPermission(kind)
          : (await openPermissionSettings(kind), await getPermissionStatus()),
      );
    } finally {
      if (action === "request") setRequestingPermission(null);
    }
  }, []);

  // Register this install's revocable Pareto license grant (idempotent) and
  // learn which flavor of UltraVox this binary is. Both fail open.
  useEffect(() => {
    if (isReminderWindow) return;
    let cancelled = false;
    (async () => {
      try {
        const info = await getAppInfo();
        if (!cancelled) setBuild(info.build);
      } catch (error) {
        console.error("Failed to read app info:", error);
      }
      try {
        await bootstrapDistributionAccess();
      } catch (error) {
        console.error("Bundled license activation skipped:", error);
      }
      try {
        await registerDeviceLicense();
      } catch (error) {
        console.error("License registration skipped:", error);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [isReminderWindow]);

  useEffect(() => {
    let cancelled = false;

    async function load() {
      const [appStatusResult, settingsResult, telemetryResult, migrationResult] = await Promise.allSettled([
        getAppStatus(),
        getSettings(),
        getAppTelemetryStatus(),
        getLegacyMigrationNotice(),
      ]);
      if (cancelled) return;


      if (appStatusResult.status === "fulfilled") {
        setStatus(appStatusResult.value.status);
        setInitialRecording(appStatusResult.value.recording);
        setInitialMeeting(appStatusResult.value.meeting);
      } else {
        console.error("Failed to load app status:", appStatusResult.reason);
        setStatus("error");
      }

      if (settingsResult.status === "fulfilled") {
        setSettingsConfig(settingsResult.value);
      } else {
        console.error("Failed to preload settings:", settingsResult.reason);
      }
      if (telemetryResult.status === "fulfilled") {
        setTelemetryStatus(telemetryResult.value);
      } else {
        console.error("Failed to load telemetry status:", telemetryResult.reason);
      }
      if (migrationResult.status === "fulfilled") {
        setMigrationNotice(migrationResult.value);
      } else {
        console.error("Failed to load migration notice:", migrationResult.reason);
      }
    }
    void load();
    return () => {
      cancelled = true;
    };
  }, []);
  useUpdaterLifecycle(!isReminderWindow);

  useEffect(() => {
    if (!isReminderWindow) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    const showDetection = (payload: MeetingDetectionPendingPayload) => {
      if (cancelled) return;
      returnFocusRef.current = document.activeElement as HTMLElement | null;
      setDecisionError(null);
      setDetection(payload);
      window.setTimeout(() => promptRef.current?.focus(), 0);
    };
    void onMeetingDetectionPending(showDetection).then(async (stop) => {
      if (cancelled) {
        stop();
        return;
      }
      unlisten = stop;
      const pending = await getPendingMeetingDetection();
      if (pending) showDetection(pending);
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [isReminderWindow]);

  const decideMeeting = useCallback(async (decision: "accept" | "decline") => {
    if (!detection) return;
    try {
      await respondMeetingDetection(detection.detection_id, decision);
      setDetection(null);
      returnFocusRef.current?.focus();
    } catch (error) {
      setDecisionError(String(error));
    }
  }, [detection]);

  useEffect(() => {
    if (!detection) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        void decideMeeting("decline");
        return;
      }
      if (event.key !== "Tab") return;
      const focusable = promptRef.current?.querySelectorAll<HTMLElement>(
        "button:not([disabled]), [href], input, select, textarea, [tabindex]:not([tabindex='-1'])",
      );
      if (!focusable?.length) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [detection, decideMeeting]);

  const openSettings = useCallback(async (tab: SettingsTab = "shortcut") => {
    setSettingsTab(tab);
    if (settingsConfig) {
      setShowSettings(true);
      return;
    }
    try {
      setSettingsConfig(await getSettings());
      setShowSettings(true);
    } catch (error) {
      console.error("Failed to open settings:", error);
    }
  }, [settingsConfig]);


  useEffect(() => {
    if (isReminderWindow) return;
    const unlistenPromise = listen<string>("navigate-to", ({ payload }) => {
      if (payload === "settings") void openSettings();
    });
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [isReminderWindow, openSettings]);

  // "loading" (no status or error yet) must not show the panel.
  const needsPermission = dictationAccess(permissionStatus, permissionError) === "blocked";
  const permissionPanelVisible = needsPermission && !showSettings;
  const needsConsent = telemetryStatus?.consent === "undecided";
  const decideTelemetry = useCallback(async (enabled: boolean) => {
    setTelemetryStatus(await setAppTelemetryEnabled(enabled));
  }, []);

  const studioActive = mode === "studio" && !showSettings && !needsConsent && !needsPermission;
  useEffect(() => {
    if (isReminderWindow) return;
    const keydown = (event: KeyboardEvent) => {
      // Do not intercept shortcut capture in Settings or any open modal.
      if (!pro.available || showSettings || needsConsent || needsPermission || studioBlocked || document.querySelector('[role="dialog"]') || event.repeat || event.isComposing) return;
      if ((event.metaKey || event.ctrlKey) && event.shiftKey && !event.altKey && event.code === "KeyV") {
        event.preventDefault(); event.stopImmediatePropagation();
        if (!pro.unlocked) { void openSettings("pro"); return; }
        if (mode === "studio") closeStudio(); else openStudio();
      }
    };
    window.addEventListener("keydown", keydown, true);
    return () => window.removeEventListener("keydown", keydown, true);
  }, [isReminderWindow, pro, openSettings, showSettings, needsConsent, needsPermission, studioBlocked, mode, closeStudio, openStudio]);

  return (
    <div className="app-shell">
      {!isReminderWindow && (
        <>
          {permissionError && !needsConsent && !permissionPanelVisible && (
            <div className="settings-error" role="alert">
              {permissionError}{" "}
              <button className="btn" type="button" disabled={permissionBusy} onClick={() => void recheckPermissions()}>Recheck permissions</button>
            </div>
          )}
          <div className="view-layer" ref={(node) => node?.toggleAttribute("inert", mode === "studio" || showSettings || needsConsent || needsPermission)} hidden={mode === "studio" || showSettings || needsConsent || needsPermission} aria-hidden={mode === "studio" || showSettings || needsConsent || needsPermission}>
            <MainWindow
              status={status}
              initialRecording={initialRecording}
              initialMeeting={initialMeeting}
              pro={pro}
              onOpenPro={() => void openSettings("pro")}
              onOpenStudio={openStudio}
              onOpenSettings={() => void openSettings()}
            />
          </div>
          {studioVisited && pro.unlocked && (
            <div className="view-layer" hidden={!studioActive} aria-hidden={!studioActive} ref={(node) => node?.toggleAttribute("inert", !studioActive)}>
              <VoiceStudioPage active={studioActive} onClose={closeStudio} onBlockingChange={setStudioBlocked} />
            </div>
          )}
          {settingsConfig && (
            <div className="view-layer" hidden={!showSettings || needsConsent} aria-hidden={!showSettings || needsConsent}>
              <SettingsPage
                initialConfig={settingsConfig}
                initialTab={settingsTab}
                pro={pro}
                build={build}
                onProRefresh={refreshPro}
                onClose={() => setShowSettings(false)}
              />
            </div>
          )}
        </>
      )}
      {!isReminderWindow && needsConsent && (
        <TelemetryConsent onDecision={decideTelemetry} />
      )}
      {!isReminderWindow && !needsConsent && permissionPanelVisible && (
        <PermissionPanel
          status={permissionStatus}
          busy={permissionBusy}
          error={permissionError}
          requesting={requestingPermission}
          microphonePrompt={microphonePrompt}
          onAction={permissionAction}
          onRecheck={recheckPermissions}
          onOpenSettings={() => void openSettings()}
          migrationNotice={migrationNotice}
        />
      )}
      {detection && (
        <div className="compact-modal-backdrop meeting-reminder-backdrop" role="presentation">
          <div
            ref={promptRef}
            className="compact-modal meeting-reminder"
            role="dialog"
            aria-modal="true"
            aria-labelledby="meeting-reminder-title"
            aria-describedby="meeting-reminder-copy"
            tabIndex={-1}
          >
            <div className="compact-modal-heading">
              <div>
                <span className="eyebrow">Browser signal</span>
                <h2 id="meeting-reminder-title">
                  {detection.provider === "google_meet" ? "Google Meet" : "Zoom"} meeting detected
                </h2>
              </div>
            </div>
            <p id="meeting-reminder-copy" className="compact-modal-copy">
              UltraVox detected a meeting locally. Nothing is recorded unless you choose to start recording.
            </p>
            {decisionError && <p className="settings-error" role="alert">{decisionError}</p>}
            <div className="compact-modal-actions">
              <button type="button" className="btn" onClick={() => void decideMeeting("decline")}>
                Not now
              </button>
              <button
                type="button"
                className="btn btn-primary"
                autoFocus
                onClick={() => void decideMeeting("accept")}
              >
                Start recording
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
