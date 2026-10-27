import type { MicrophoneRequestOutcome, PermissionKind, PermissionState, PermissionStatus } from "../ipc";

// Optional paste/hotkey and system-audio access do not gate in-app dictation.
// Windows and Linux have no microphone gate and always report "unavailable";
// that means there is nothing to wait for, so it counts as ready.
export function isDictationReady(status: PermissionStatus | null): boolean {
  return status?.microphone === "granted" || status?.microphone === "unavailable";
}

export type DictationAccess = "loading" | "ready" | "blocked";

// Until the first status or error arrives the answer is unknown, not "blocked".
// Treating it as blocked would flash the permission panel on every launch.
export function dictationAccess(status: PermissionStatus | null, error: string | null): DictationAccess {
  if (status) return isDictationReady(status) ? "ready" : "blocked";
  return error ? "blocked" : "loading";
}

// Set once the one-shot macOS microphone prompt has been used without an answer.
export type MicrophonePromptState = "pending" | "timed_out" | null;

// A plain recheck never carries a request outcome, so the used prompt stays
// remembered until macOS reports an answer. A timeout outranks a later "pending".
export function nextMicrophonePromptState(
  previous: MicrophonePromptState,
  status: PermissionStatus,
): MicrophonePromptState {
  if (status.microphone !== "not_determined") return null;
  const outcome: MicrophoneRequestOutcome | undefined = status.microphone_request;
  if (outcome === "timed_out") return "timed_out";
  if (outcome === "pending") return previous === "timed_out" ? previous : "pending";
  return previous;
}

export function permissionStateLabel(state: PermissionState | undefined, failed = false): string {
  switch (state) {
    case "granted":
      return "Granted";
    case "denied":
      return "Denied";
    case "not_determined":
      return "Not allowed yet";
    case "unavailable":
      return "Not available on this system";
    default:
      return failed ? "Couldn't check" : "Checking…";
  }
}

export function microphonePromptNote(prompt: MicrophonePromptState): string | null {
  if (prompt === "pending") return "macOS hasn't recorded an answer. Turn on UltraVox in Settings; a relaunch may be needed.";
  if (prompt === "timed_out") return "The macOS prompt timed out. Turn on UltraVox in Settings; a relaunch may be needed.";
  return null;
}

// macOS shows its microphone prompt once per launch and never again after a
// denial. Accessibility and Screen Recording are requested while denied.
export function canRequestPermission(
  kind: PermissionKind,
  state: PermissionState | undefined,
  prompt: MicrophonePromptState,
): boolean {
  if (kind === "microphone") return state === "not_determined" && prompt === null;
  return state === "denied";
}

const PERMISSION_NOUN: Record<PermissionKind, string> = {
  microphone: "microphone",
  accessibility: "Accessibility",
  screen_recording: "Screen Recording",
};

export function permissionActionFailure(kind: PermissionKind, action: "request" | "settings"): string {
  return action === "request"
    ? `Couldn't request ${PERMISSION_NOUN[kind]} access`
    : `Couldn't open ${PERMISSION_NOUN[kind]} settings`;
}

export const PERMISSION_CHECK_FAILURE = "Couldn't check permissions";

export function permissionValidationError(failure: string, cause: unknown): string {
  const detail = cause instanceof Error ? cause.message : typeof cause === "string" ? cause : "Unknown error";
  return `${failure}: ${detail.trim().slice(0, 240) || "Unknown error"}`;
}

export type PermissionReadinessController = {
  refresh: () => Promise<void>;
  scheduleRefresh: () => void;
  runAction: (failure: string, operation: () => Promise<PermissionStatus>) => Promise<void>;
  dispose: () => void;
};

// All reads and explicit actions share a queue: an older read cannot overwrite a
// newer request result. An in-flight read permits one coalesced trailing read.
// No authorization request is ever triggered by refresh or by a lifecycle event.
export function createPermissionReadinessController({
  readStatus,
  onStatus,
  onError,
  onBusy,
  debounceMs = 150,
}: {
  readStatus: () => Promise<PermissionStatus>;
  onStatus: (status: PermissionStatus) => void;
  onError: (error: string | null) => void;
  onBusy: (busy: boolean) => void;
  debounceMs?: number;
}): PermissionReadinessController {
  let disposed = false;
  let tail = Promise.resolve();
  let pending = 0;
  let queuedRefresh: Promise<void> | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;

  function enqueue(failure: string, operation: () => Promise<PermissionStatus>): Promise<void> {
    if (disposed) return Promise.resolve();
    pending += 1;
    onBusy(true);
    const result = tail.then(async () => {
      if (disposed) return;
      try {
        const status = await operation();
        if (!disposed) {
          onStatus(status);
          onError(null);
        }
      } catch (cause) {
        // Preserve the last known OS status; a validation error is not a denial.
        if (!disposed) onError(permissionValidationError(failure, cause));
      } finally {
        pending -= 1;
        if (!disposed) onBusy(pending > 0);
      }
    });
    tail = result;
    return result;
  }

  function refresh(): Promise<void> {
    if (timer !== undefined) {
      clearTimeout(timer);
      timer = undefined;
    }
    if (disposed) return Promise.resolve();
    if (queuedRefresh) return queuedRefresh;
    const result = enqueue(PERMISSION_CHECK_FAILURE, () => {
      queuedRefresh = null;
      return readStatus();
    });
    queuedRefresh = result;
    return result;
  }

  return {
    refresh,
    scheduleRefresh() {
      if (disposed) return;
      if (timer !== undefined) clearTimeout(timer);
      timer = setTimeout(() => {
        timer = undefined;
        void refresh();
      }, debounceMs);
    },
    runAction: enqueue,
    dispose() {
      disposed = true;
      if (timer !== undefined) clearTimeout(timer);
    },
  };
}
