import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { MainWindow } from "./pages/MainWindow";
import { SettingsPage } from "./pages/SettingsPage";
import {
  getAppStatus,
  getPendingMeetingDetection,
  getSettings,
  onMeetingDetectionPending,
  respondMeetingDetection,
  type AppConfig,
  type AppStatusResponse,
  type MeetingDetectionPendingPayload,
} from "./ipc";

export type AppStatus = AppStatusResponse["status"];

export default function App() {
  const [showSettings, setShowSettings] = useState(false);
  const [settingsConfig, setSettingsConfig] = useState<AppConfig | null>(null);
  const [status, setStatus] = useState<AppStatus>("loading");
  const [initialRecording, setInitialRecording] = useState(false);
  const [initialMeeting, setInitialMeeting] = useState(false);
  const [detection, setDetection] = useState<MeetingDetectionPendingPayload | null>(null);
  const [decisionError, setDecisionError] = useState<string | null>(null);
  const promptRef = useRef<HTMLDivElement>(null);
  const returnFocusRef = useRef<HTMLElement | null>(null);
  const isReminderWindow = getCurrentWindow().label === "meeting-reminder";

  useEffect(() => {
    let cancelled = false;

    async function load() {
      const [appStatusResult, settingsResult] = await Promise.allSettled([
        getAppStatus(),
        getSettings(),
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
    }

    void load();
    return () => {
      cancelled = true;
    };
  }, []);
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
    void getCurrentWindow().setSize(new LogicalSize(450, showSettings ? 520 : 650));
  }, [isReminderWindow, showSettings]);

  useEffect(() => {
    if (isReminderWindow) return;
    const unlistenPromise = listen<string>("navigate-to", ({ payload }) => {
      if (payload === "settings") void openSettings();
    });
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [isReminderWindow, openSettings]);

  return (
    <div className="app-shell">
      {!isReminderWindow && (
        <>
          <div className="view-layer" hidden={showSettings} aria-hidden={showSettings}>
            <MainWindow
              status={status}
              initialRecording={initialRecording}
              initialMeeting={initialMeeting}
              onOpenSettings={() => void openSettings()}
            />
          </div>
          {settingsConfig && (
            <div className="view-layer" hidden={!showSettings} aria-hidden={!showSettings}>
              <SettingsPage
                initialConfig={settingsConfig}
                onClose={() => setShowSettings(false)}
              />
            </div>
          )}
        </>
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
