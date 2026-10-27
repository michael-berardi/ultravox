import type { UpdateInfo, UpdatePreferences } from "../ipc";

/** hidden: nothing newer · available: the header shows Update · updating: spinner · failed: Retry with the cause. */
export type UpdatePhase = "hidden" | "available" | "updating" | "failed";

export type UpdaterState = {
  phase: UpdatePhase;
  info: UpdateInfo | null;
  error: string | null;
  automatic: boolean;
  checkError: string | null;
  preferenceError: string | null;
};

export type UpdaterDeps = {
  check: () => Promise<UpdateInfo | null>;
  install: (info: UpdateInfo) => Promise<void>;
  getPreferences: () => Promise<UpdatePreferences>;
  setPreferences: (preferences: UpdatePreferences) => Promise<void>;
  setTimer: (run: () => void, ms: number) => unknown;
  clearTimer: (handle: unknown) => void;
  log?: (message: string) => void;
};

/** Matches `UPDATE_BUSY_PREFIX` in src-tauri/src/update.rs. */
export const UPDATE_BUSY_PREFIX = "Update waits:";
export const CHECK_EVERY_MS = 4 * 60 * 60 * 1000;
export const BUSY_RETRY_MS = 60 * 1000;

export function isBusyRefusal(error: string): boolean {
  return error.startsWith(UPDATE_BUSY_PREFIX);
}

export type Updater = ReturnType<typeof createUpdater>;

export function createUpdater(deps: UpdaterDeps, options = { checkEveryMs: CHECK_EVERY_MS, busyRetryMs: BUSY_RETRY_MS }) {
  let state: UpdaterState = {
    phase: "hidden",
    info: null,
    error: null,
    automatic: true,
    checkError: null,
    preferenceError: null,
  };
  const listeners = new Set<() => void>();
  let checkTimer: unknown = null;
  let retryTimer: unknown = null;
  let started = false;

  const set = (patch: Partial<UpdaterState>) => {
    state = { ...state, ...patch };
    listeners.forEach((listener) => listener());
  };

  const clearRetry = () => {
    if (retryTimer !== null) deps.clearTimer(retryTimer);
    retryTimer = null;
  };

  const run = async (automatic: boolean) => {
    const info = state.info;
    if (!info || state.phase === "updating") return;
    clearRetry();
    set({ phase: "updating", error: null });
    try {
      // On success the app quits and the new version relaunches.
      await deps.install(info);
    } catch (cause) {
      const error = String(cause);
      if (automatic && isBusyRefusal(error)) {
        // Never interrupt a recording: wait for an idle moment and try again.
        set({ phase: "available", error: null });
        retryTimer = deps.setTimer(() => {
          retryTimer = null;
          if (state.automatic) void run(true);
        }, options.busyRetryMs);
        return;
      }
      deps.log?.(`UltraVox update failed: ${error}`);
      set({ phase: "failed", error });
    }
  };

  const check = async () => {
    try {
      const info = await deps.check();
      if (state.phase === "updating") return;
      if (!info) {
        clearRetry();
        set({ info: null, phase: "hidden", error: null, checkError: null });
        return;
      }
      const sameVersion = state.info?.latest_version === info.latest_version;
      set({
        info,
        checkError: null,
        phase: sameVersion && state.phase === "failed" ? "failed" : "available",
      });
      if (state.automatic && state.phase === "available") await run(true);
    } catch (cause) {
      const error = String(cause);
      deps.log?.(`UltraVox update check failed: ${error}`);
      set({ checkError: error });
    }
  };

  const schedule = () => {
    checkTimer = deps.setTimer(() => {
      void check().finally(schedule);
    }, options.checkEveryMs);
  };

  return {
    getState: () => state,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    async start() {
      if (started) return;
      started = true;
      try {
        const preferences = await deps.getPreferences();
        set({ automatic: preferences.automatic });
      } catch (cause) {
        set({ preferenceError: `Could not read the update preference: ${String(cause)}` });
      }
      await check();
      schedule();
    },
    stop() {
      started = false;
      if (checkTimer !== null) deps.clearTimer(checkTimer);
      checkTimer = null;
      clearRetry();
    },
    check,
    install: () => run(false),
    async setAutomatic(enabled: boolean) {
      const previous = state.automatic;
      set({ automatic: enabled, preferenceError: null });
      try {
        await deps.setPreferences({ automatic: enabled });
      } catch (cause) {
        set({ automatic: previous, preferenceError: `Could not save the update preference: ${String(cause)}` });
        return;
      }
      if (!enabled) clearRetry();
      if (enabled && state.phase === "available") await run(true);
    },
  };
}
