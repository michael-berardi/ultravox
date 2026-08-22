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

/** "Title — Artist" for glanceable metadata; null when there is nothing to show. */
export function mediaSubtitle(
  title: string | null | undefined,
  artist: string | null | undefined,
): string | null {
  const cleanTitle = title?.trim();
  const cleanArtist = artist?.trim();
  if (cleanTitle && cleanArtist) return `${cleanTitle} — ${cleanArtist}`;
  return cleanTitle || cleanArtist || null;
}
