import type { UpdateCheckStatus, UpdateCheckTrigger, UpdateInfo, UpdateNotice, UpdatePreferences } from "../ipc";

/** hidden: nothing newer · available: the header shows Update · updating: spinner · failed: Retry with the cause. */
export type UpdatePhase = "hidden" | "available" | "updating" | "failed";

export type UpdaterState = {
  phase: UpdatePhase;
  info: UpdateInfo | null;
  error: string | null;
  /** Install updates automatically (opt-in, off by default). */
  automatic: boolean;
  /** Check for updates automatically (on by default, at most daily). */
  checkAutomatically: boolean;
  /** A Check Now request is in flight. */
  checking: boolean;
  /** The last check found nothing newer (only set after a completed check). */
  upToDate: boolean;
  checkError: string | null;
  preferenceError: string | null;
  /** Why the environment turns update checks off (env opt-out, CI, agent host, mirror build). */
  blockedReason: string | null;
  /** Unix seconds of the last network check, from the native record. */
  lastCheckedAt: number | null;
  /** The update notice (banner) is showing for `info`. */
  notice: boolean;
  /** The user chose Skip This Version for `info`; nothing is offered for it. */
  skipped: boolean;
};

export type UpdaterDeps = {
  check: (trigger: UpdateCheckTrigger) => Promise<UpdateInfo | null>;
  install: (info: UpdateInfo) => Promise<void>;
  getPreferences: () => Promise<UpdatePreferences>;
  setPreferences: (preferences: UpdatePreferences) => Promise<void>;
  getStatus?: () => Promise<UpdateCheckStatus>;
  /** Native once-a-day-per-version notice gate; records a `show`. */
  claimNotice?: (version: string) => Promise<UpdateNotice>;
  /** Persists Skip This Version. */
  skipVersion?: (version: string) => Promise<void>;
  setTimer: (run: () => void, ms: number) => unknown;
  clearTimer: (handle: unknown) => void;
  log?: (message: string) => void;
};

/** Matches `UPDATE_BUSY_PREFIX` in src-tauri/src/update.rs. */
export const UPDATE_BUSY_PREFIX = "Update waits:";
/**
 * How often the app asks the native side whether a check is due. The native
 * gate (update.rs `gated_check`) makes at most one network request per 24 h;
 * every other tick is answered from its record without a request.
 */
export const CHECK_EVERY_MS = 60 * 60 * 1000;
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
    automatic: false,
    checkAutomatically: true,
    checking: false,
    upToDate: false,
    checkError: null,
    preferenceError: null,
    blockedReason: null,
    lastCheckedAt: null,
    notice: false,
    skipped: false,
  };
  const listeners = new Set<() => void>();
  let checkTimer: unknown = null;
  let retryTimer: unknown = null;
  let started = false;
  /** Remind Me Later in this session, so a failed native record never re-shows the notice hourly. */
  let remindedVersion: string | null = null;

  const set = (patch: Partial<UpdaterState>) => {
    state = { ...state, ...patch };
    listeners.forEach((listener) => listener());
  };

  const clearRetry = () => {
    if (retryTimer !== null) deps.clearTimer(retryTimer);
    retryTimer = null;
  };

  const refreshStatus = async () => {
    if (!deps.getStatus) return;
    try {
      const status = await deps.getStatus();
      set({
        blockedReason: status.blocked_reason,
        lastCheckedAt: status.last_checked_at,
        // The native record keeps the cause of the last failed check.
        checkError: status.last_error ?? state.checkError,
      });
    } catch {
      // Status is informational; the check result already reached the UI.
    }
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

  const check = async (trigger: UpdateCheckTrigger = "automatic") => {
    // A blocked environment (mirror build, CI, agent host or explicit opt-out)
    // cannot make requests. The native status already explains why; do not
    // call the checker at all, so blocked builds never surface an IPC failure.
    if (state.blockedReason) return;
    // A saved opt-out stops the automatic requests; Check Now stays available.
    if (trigger === "automatic" && !state.checkAutomatically) return;
    if (trigger === "manual") set({ checking: true, checkError: null });
    try {
      const info = await deps.check(trigger);
      if (state.phase === "updating") return;
      if (!info) {
        clearRetry();
        set({ info: null, phase: "hidden", error: null, checkError: null, upToDate: true, notice: false, skipped: false });
        return;
      }
      const sameVersion = state.info?.latest_version === info.latest_version;
      // Check Now is explicit: it offers the release even if it was skipped.
      // Automatic checks honour Skip This Version (no notice, no automatic
      // install) and show the notice at most once a day per version; with
      // automatic install on, the update installs instead of asking.
      let notice: UpdateNotice = "recent";
      if (trigger === "automatic") notice = await claimNotice(info.latest_version);
      if (trigger === "automatic" && sameVersion && state.skipped) notice = "skipped";
      // An install may have started while the notice state was read.
      if ((state.phase as UpdatePhase) === "updating") return;
      if (notice === "skipped") {
        clearRetry();
        set({ info, phase: "hidden", error: null, checkError: null, upToDate: false, notice: false, skipped: true });
        return;
      }
      set({
        info,
        checkError: null,
        upToDate: false,
        skipped: false,
        notice: !state.automatic && (notice === "show" || (sameVersion && state.notice)),
        phase: sameVersion && state.phase === "failed" ? "failed" : "available",
      });
      if (state.automatic && state.phase === "available") await run(true);
    } catch (cause) {
      const error = String(cause);
      deps.log?.(`UltraVox update check failed: ${error}`);
      set({ checkError: error, upToDate: false });
    } finally {
      if (trigger === "manual") set({ checking: false });
      await refreshStatus();
    }
  };

  const claimNotice = async (version: string): Promise<UpdateNotice> => {
    if (!deps.claimNotice) return "show";
    try {
      return await deps.claimNotice(version);
    } catch (cause) {
      deps.log?.(`UltraVox could not record the update notice: ${String(cause)}`);
      return remindedVersion === version ? "recent" : "show";
    }
  };

  const schedule = () => {
    checkTimer = deps.setTimer(() => {
      void check("automatic").finally(() => {
        if (started) schedule();
      });
    }, options.checkEveryMs);
  };

  const savePreferences = async (patch: Partial<UpdatePreferences>) => {
    await deps.setPreferences({
      automatic: state.automatic,
      check_automatically: state.checkAutomatically,
      ...patch,
    });
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
        set({
          automatic: preferences.automatic,
          checkAutomatically: preferences.check_automatically,
        });
      } catch (cause) {
        // An unreadable preference file may hold an opt-out: fail closed until the user chooses again.
        set({
          automatic: false,
          checkAutomatically: false,
          preferenceError: `Could not read the update preference: ${String(cause)}`,
        });
      }
      await refreshStatus();
      // Never on the startup path: the caller does not await this, and the native check has a 4 s timeout.
      await check("automatic");
      schedule();
    },
    stop() {
      started = false;
      if (checkTimer !== null) deps.clearTimer(checkTimer);
      checkTimer = null;
      clearRetry();
    },
    check,
    /** Settings → Updates → Check Now. */
    checkNow: () => check("manual"),
    install: () => run(false),
    /** Remind Me Later: hide the notice; the header Update button stays. */
    later() {
      remindedVersion = state.info?.latest_version ?? null;
      set({ notice: false });
    },
    /** Skip This Version: no notice or Update button for it until a newer release. */
    async skip() {
      const info = state.info;
      if (!info || state.phase === "updating") return;
      try {
        await deps.skipVersion?.(info.latest_version);
      } catch (cause) {
        set({ error: `Could not skip this version: ${String(cause)}` });
        return;
      }
      clearRetry();
      set({ notice: false, skipped: true, phase: "hidden", error: null });
    },
    /** `installNow`: when turned on with an update waiting, install it now (Settings). The notice's
     *  "in the future" checkbox passes false. */
    async setAutomatic(enabled: boolean, installNow = true) {
      const previous = state.automatic;
      set({ automatic: enabled, preferenceError: null });
      try {
        await savePreferences({ automatic: enabled });
      } catch (cause) {
        set({ automatic: previous, preferenceError: `Could not save the update preference: ${String(cause)}` });
        return;
      }
      if (!enabled) clearRetry();
      if (enabled && installNow && state.phase === "available") await run(true);
    },
    async setCheckAutomatically(enabled: boolean) {
      const previous = state.checkAutomatically;
      set({ checkAutomatically: enabled, preferenceError: null });
      try {
        await savePreferences({ check_automatically: enabled });
      } catch (cause) {
        set({ checkAutomatically: previous, preferenceError: `Could not save the update preference: ${String(cause)}` });
        return;
      }
      if (!enabled) {
        clearRetry();
        return;
      }
      if (!previous) await check("automatic");
    },
  };
}
