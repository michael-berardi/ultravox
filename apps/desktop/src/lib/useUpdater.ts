import { useEffect, useSyncExternalStore } from "react";
import { checkForUpdate, getUpdatePreferences, installUpdate, setUpdatePreferences } from "../ipc";
import { createUpdater, type UpdaterState } from "./updater";

declare global { interface Window { __ULTRAVOX_MIRROR_UPDATER__?: (patch: Partial<UpdaterState> | null) => void } }

/** One updater per window: the header button and the Settings toggle share it. */
export const updater = createUpdater({
  check: checkForUpdate,
  install: installUpdate,
  getPreferences: getUpdatePreferences,
  setPreferences: setUpdatePreferences,
  setTimer: (run, ms) => window.setTimeout(run, ms),
  clearTimer: (handle) => window.clearTimeout(handle as number),
  log: (message) => console.error(message),
});

let mirrorFixture: UpdaterState | null = null;
const mirrorListeners = new Set<() => void>();
if (import.meta.env.VITE_MIRROR_DEBUG === "1") {
  // Mirror-only visual fixtures for the update states; never in release bundles.
  window.__ULTRAVOX_MIRROR_UPDATER__ = (patch) => {
    mirrorFixture = patch ? { ...updater.getState(), ...patch } : null;
    mirrorListeners.forEach((listener) => listener());
  };
}

function subscribe(listener: () => void) {
  const off = updater.subscribe(listener);
  mirrorListeners.add(listener);
  return () => { off(); mirrorListeners.delete(listener); };
}
const snapshot = () => mirrorFixture ?? updater.getState();

export function useUpdater(): UpdaterState {
  return useSyncExternalStore(subscribe, snapshot);
}

/** Starts launch and periodic checks; mount once in the main window. */
export function useUpdaterLifecycle(enabled: boolean): void {
  useEffect(() => {
    if (!enabled) return;
    void updater.start();
    return () => updater.stop();
  }, [enabled]);
}
