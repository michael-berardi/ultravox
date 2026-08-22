import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { MainWindow } from "./pages/MainWindow";
import { SettingsPage } from "./pages/SettingsPage";
import {
  checkForUpdate,
  getAppStatus,
  getAppTelemetryStatus,
  getPendingMeetingDetection,
  getPermissionStatus,
  getSettings,
  getUpdatePreferences,
  installUpdate,
  onMeetingDetectionPending,
  openPermissionSettings,
  requestPermission,
  respondMeetingDetection,
  setAppTelemetryEnabled,
  setUpdatePreferences,
  type AppConfig,
  type AppStatusResponse,
  type MeetingDetectionPendingPayload,
  type PermissionKind,
  type PermissionStatus,
  type TelemetryStatus,
  type UpdateInfo,

} from "./ipc";
const PERMISSIONS: Array<{ kind: PermissionKind; label: string; copy: string }> = [

  { kind: "microphone", label: "Microphone", copy: "Record speech for local transcription." },
  { kind: "accessibility", label: "Accessibility", copy: "Place the transcription in the focused text field." },
  { kind: "screen_recording", label: "Screen Recording", copy: "Capture meeting system audio with ScreenCaptureKit." },
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
function UpdatePrompt({
  info,
  automatic,
  busy,
  error,
  onAutomaticChange,
  onInstall,
  onLater,
}: {
  info: UpdateInfo;
  automatic: boolean;
  busy: boolean;
  error: string | null;
  onAutomaticChange: (enabled: boolean) => Promise<void>;
  onInstall: () => Promise<void>;
  onLater: () => void;
}) {
  return (
    <div className="compact-modal-backdrop" role="presentation">
      <section className="compact-modal permission-panel" role="dialog" aria-modal="true" aria-labelledby="update-title">
        <span className="eyebrow">Update available</span>
        <h2 id="update-title">UltraVox {info.latest_version}</h2>
        <p className="compact-modal-copy">
          The update is downloaded from the stable GitHub release, then its checksum,
          Developer ID, bundle identity, sealed code, and notarization are verified
          before the current app is replaced.
        </p>
        <label className="settings-row update-automatic-choice">
          <input
            type="checkbox"
            checked={automatic}
            disabled={busy}
            onChange={(event) => void onAutomaticChange(event.target.checked)}
          />
          <span>
            <strong>Install updates automatically</strong>
            <small>Opt in to verified stable updates on future launch and daily checks.</small>
          </span>
        </label>
        {error && <p className="settings-error" role="alert">{error}</p>}
        <div className="compact-modal-actions">
          <button className="btn" type="button" disabled={busy} onClick={onLater}>Later</button>
          <button className="btn btn-primary" type="button" disabled={busy} onClick={() => void onInstall()}>
            {busy ? "Verifying…" : "Update now"}
          </button>
        </div>
      </section>
    </div>
  );
}


function PermissionPanel({
  status,
  onStatus,
}: {
  status: PermissionStatus;
  onStatus: (status: PermissionStatus) => void;
}) {
  const [busy, setBusy] = useState<PermissionKind | null>(null);
  const run = async (kind: PermissionKind, action: "request" | "settings") => {
    setBusy(kind);
    try {
      const next = action === "request"
        ? await requestPermission(kind)
        : (await openPermissionSettings(kind), await getPermissionStatus());
      onStatus(next);
    } finally {
      setBusy(null);
    }
  };
  return (
    <div className="compact-modal-backdrop" role="presentation">
      <section className="compact-modal permission-panel" role="dialog" aria-modal="true" aria-labelledby="permission-title">
        <span className="eyebrow">First-run setup</span>
        <h2 id="permission-title">Allow UltraVox to work</h2>
        <p className="compact-modal-copy">
          UltraVox checks each permission at runtime. Old rows in macOS settings are only diagnostics and are never treated as granted.
        </p>
        <div className="permission-list">
          {PERMISSIONS.map(({ kind, label, copy }) => {
            const state = status[kind];
            return (
              <div className="permission-row" key={kind}>
                <div><strong>{label}</strong><span>{copy}</span></div>
                <span className={`permission-state permission-${state}`}>{state === "granted" ? "Granted" : state === "not_determined" ? "Needs access" : state}</span>
                {state !== "granted" && state !== "unavailable" && (
                  <div className="permission-actions">
                    <button className="btn btn-primary" type="button" disabled={busy !== null} onClick={() => void run(kind, "request")}>
                      {busy === kind ? "Checking…" : "Allow"}
                    </button>
                    <button className="btn" type="button" disabled={busy !== null} onClick={() => void run(kind, "settings")}>Open Settings</button>
                  </div>
                )}
              </div>
            );
          })}
        </div>
        <p className="permission-footnote">After changing a permission, restart UltraVox if macOS asks, then choose Recheck. You do not need to delete the permission row.</p>
        <button className="btn" type="button" onClick={() => void getPermissionStatus().then(onStatus)}>Recheck permissions</button>
      </section>
    </div>
  );
}

export type AppStatus = AppStatusResponse["status"];

export default function App() {
  const [showSettings, setShowSettings] = useState(false);
  const [settingsConfig, setSettingsConfig] = useState<AppConfig | null>(null);
  const [status, setStatus] = useState<AppStatus>("loading");
  const [initialRecording, setInitialRecording] = useState(false);
  const [permissionStatus, setPermissionStatus] = useState<PermissionStatus | null>(null);
  const [telemetryStatus, setTelemetryStatus] = useState<TelemetryStatus | null>(null);
  const [availableUpdate, setAvailableUpdate] = useState<UpdateInfo | null>(null);
  const [automaticUpdates, setAutomaticUpdates] = useState(false);
  const [updateBusy, setUpdateBusy] = useState(false);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [initialMeeting, setInitialMeeting] = useState(false);
  const [detection, setDetection] = useState<MeetingDetectionPendingPayload | null>(null);
  const [decisionError, setDecisionError] = useState<string | null>(null);
  const promptRef = useRef<HTMLDivElement>(null);
  const returnFocusRef = useRef<HTMLElement | null>(null);
  const isReminderWindow = getCurrentWindow().label === "meeting-reminder";

  useEffect(() => {
    let cancelled = false;

    async function load() {
      const [appStatusResult, settingsResult, permissionResult, telemetryResult] = await Promise.allSettled([
        getAppStatus(),
        getSettings(),
        getPermissionStatus(),
        getAppTelemetryStatus(),
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
      if (permissionResult.status === "fulfilled") {
        setPermissionStatus(permissionResult.value);
      } else {
        console.error("Failed to load permission status:", permissionResult.reason);
      }
      if (telemetryResult.status === "fulfilled") {
        setTelemetryStatus(telemetryResult.value);
      } else {
        console.error("Failed to load telemetry status:", telemetryResult.reason);
      }
    }
    void load();
    return () => {
      cancelled = true;
    };
  }, []);
  useEffect(() => {
    if (isReminderWindow) return;
    let cancelled = false;
    const check = async () => {
      try {
        const [preferences, candidate] = await Promise.all([
          getUpdatePreferences(),
          checkForUpdate(),
        ]);
        if (cancelled) return;
        setAutomaticUpdates(preferences.automatic);
        if (!candidate) {
          setAvailableUpdate(null);
          return;
        }
        if (!preferences.automatic) {
          setAvailableUpdate(candidate);
          return;
        }
        setUpdateBusy(true);
        try {
          await installUpdate(candidate);
        } catch (error) {
          if (!cancelled) {
            setAvailableUpdate(candidate);
            setUpdateError(`Automatic update was not installed: ${String(error)}`);
          }
        } finally {
          if (!cancelled) setUpdateBusy(false);
        }
      } catch (error) {
        if (!cancelled) console.error("Update check unavailable:", error);
      }
    };
    void check();
    const interval = window.setInterval(() => void check(), 24 * 60 * 60 * 1000);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, [isReminderWindow]);

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

  const openSettings = useCallback(async () => {
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

  const needsPermission = permissionStatus
    ? Object.values(permissionStatus).some((value) => value !== "granted" && value !== "unavailable")
    : false;
  const needsConsent = telemetryStatus?.consent === "undecided";
  const decideTelemetry = useCallback(async (enabled: boolean) => {
    setTelemetryStatus(await setAppTelemetryEnabled(enabled));
  }, []);
  const changeAutomaticUpdates = useCallback(async (enabled: boolean) => {
    setAutomaticUpdates(enabled);
    try {
      await setUpdatePreferences({ automatic: enabled });
    } catch (error) {
      setAutomaticUpdates(!enabled);
      setUpdateError(`Could not save the update preference: ${String(error)}`);
    }
  }, []);
  const installAvailableUpdate = useCallback(async () => {
    if (!availableUpdate) return;
    setUpdateBusy(true);
    setUpdateError(null);
    try {
      await installUpdate(availableUpdate);
    } catch (error) {
      setUpdateError(`Update was not installed: ${String(error)}`);
      setUpdateBusy(false);
    }
  }, [availableUpdate]);

  return (
    <div className="app-shell">
      {!isReminderWindow && (
        <>
          <div className="view-layer" hidden={showSettings || needsConsent || needsPermission} aria-hidden={showSettings || needsConsent || needsPermission}>
            <MainWindow
              status={status}
              initialRecording={initialRecording}
              initialMeeting={initialMeeting}
              onOpenSettings={() => void openSettings()}
            />
          </div>
          {settingsConfig && (
            <div className="view-layer" hidden={!showSettings || needsConsent || needsPermission} aria-hidden={!showSettings || needsConsent || needsPermission}>
              <SettingsPage
                initialConfig={settingsConfig}
                onClose={() => setShowSettings(false)}
              />
            </div>
          )}
        </>
      )}
      {!isReminderWindow && needsConsent && (
        <TelemetryConsent onDecision={decideTelemetry} />
      )}
      {!isReminderWindow && !needsConsent && needsPermission && permissionStatus && (
        <PermissionPanel status={permissionStatus} onStatus={setPermissionStatus} />
      )}
      {!isReminderWindow && !needsConsent && !needsPermission && availableUpdate && (
        <UpdatePrompt
          info={availableUpdate}
          automatic={automaticUpdates}
          busy={updateBusy}
          error={updateError}
          onAutomaticChange={changeAutomaticUpdates}
          onInstall={installAvailableUpdate}
          onLater={() => setAvailableUpdate(null)}
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
