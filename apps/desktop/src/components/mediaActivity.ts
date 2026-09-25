/**
 * Pure decision logic for the conditional media panel.
 *
 * The panel appears only while another process sustains audio output:
 * activity must hold for MEDIA_SHOW_DELAY_MS before the panel shows and
 * stay absent for MEDIA_HIDE_DELAY_MS before it hides, which filters both
 * directions of flicker (sub-second blips never toggle the panel).
 */

export const MEDIA_SHOW_DELAY_MS = 1000;
export const MEDIA_HIDE_DELAY_MS = 1500;

export type MediaVisibilityState = {
  /** Latest observed external-activity sample. */
  active: boolean;
  /** Timestamp of the last flip of `active`. */
  changedAt: number;
  /** Whether the panel may currently be shown. */
  visible: boolean;
};

export function initialMediaVisibility(now: number): MediaVisibilityState {
  return { active: false, changedAt: now, visible: false };
}

export function nextMediaVisibility(
  state: MediaVisibilityState,
  active: boolean,
  now: number,
): MediaVisibilityState {
  const changedAt = active === state.active ? state.changedAt : now;
  const heldFor = now - changedAt;
  const visible = active
    ? state.visible || heldFor >= MEDIA_SHOW_DELAY_MS
    : state.visible && heldFor < MEDIA_HIDE_DELAY_MS;
  return { active, changedAt, visible };
}

/**
 * Backend `volume` mirrors CoreAudio's 0..1 device scalar. Returns integer 0..100.
 */
export function normalizeVolumePercent(value: number | null | undefined): number {
  if (value == null || !Number.isFinite(value)) return 0;
  return Math.round(Math.min(1, Math.max(0, value)) * 100);
}

export type MediaPlaybackKind = "playing" | "paused" | "unknown";

/** Tri-state playback: MediaRemote may legitimately not know. */
export function mediaPlaybackKind(
  isPlaying: boolean | null | undefined,
): MediaPlaybackKind {
  if (isPlaying === true) return "playing";
  if (isPlaying === false) return "paused";
  return "unknown";
}

/** Seconds → "m:ss" for elapsed/duration readouts; null when not a real time. */
export function formatMediaClock(seconds: number | null | undefined): string | null {
  if (seconds == null || !Number.isFinite(seconds) || seconds < 0) return null;
  const whole = Math.floor(seconds);
  const minutes = Math.floor(whole / 60);
  const rest = whole % 60;
  return `${minutes}:${String(rest).padStart(2, "0")}`;
}

/** 0..1 progress; null unless both elapsed and a positive duration are known. */
export function mediaProgressRatio(
  elapsedSeconds: number | null | undefined,
  durationSeconds: number | null | undefined,
): number | null {
  if (
    elapsedSeconds == null ||
    durationSeconds == null ||
    !Number.isFinite(elapsedSeconds) ||
    !Number.isFinite(durationSeconds) ||
    durationSeconds <= 0 ||
    elapsedSeconds < 0
  ) {
    return null;
  }
  return Math.min(1, elapsedSeconds / durationSeconds);
}

/** Spoken progress summary for the progressbar's aria-valuetext. */
export function mediaTimeSummary(
  elapsedSeconds: number | null | undefined,
  durationSeconds: number | null | undefined,
): string | null {
  const elapsed = formatMediaClock(elapsedSeconds);
  const duration = formatMediaClock(durationSeconds);
  if (elapsed && duration) return `${elapsed} of ${duration}`;
  if (duration) return `Length ${duration}`;
  if (elapsed) return `${elapsed} elapsed`;
  return null;
}
