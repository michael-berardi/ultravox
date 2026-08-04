import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { MainWindow } from "./pages/MainWindow";
import { SettingsPage } from "./pages/SettingsPage";
import {
  getAppStatus,
  getSettings,
  type AppConfig,
  type AppStatusResponse,
} from "./ipc";


export type AppStatus = AppStatusResponse["status"];

export default function App() {
  const [showSettings, setShowSettings] = useState(false);
  const [settingsConfig, setSettingsConfig] = useState<AppConfig | null>(null);
  const [status, setStatus] = useState<AppStatus>("loading");
  const [initialRecording, setInitialRecording] = useState(false);
  const [initialMeeting, setInitialMeeting] = useState(false);

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
    void getCurrentWindow().setSize(new LogicalSize(450, showSettings ? 520 : 650));
  }, [showSettings]);

  useEffect(() => {
    const unlistenPromise = listen<string>("navigate-to", ({ payload }) => {
      if (payload === "settings") void openSettings();
    });
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [openSettings]);

  return (
    <div className="app-shell">
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
    </div>
  );
}
