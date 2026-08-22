import { useMemo } from "react";
import { MediaPanel, type MediaPanelServices } from "../components/MediaPanel";
import type { MediaState } from "../ipc";

/**
 * Deterministic now-playing fixtures per source. Apple Music exposes the
 * richest metadata (album + position + full transport), YouTube Music keeps
 * album and duration, and plain YouTube in a browser tab has no previous
 * track and no album — the honest coverage matrix for every layout.
 */
const SOURCE_FIXTURES: Record<HarnessSource, MediaState> = {
  music: {
    active: true,
    appName: "Apple Music",
    bundleId: "com.apple.Music",
    title: "Midnight City",
    artist: "M83",
    album: "Hurry Up, We're Dreaming",
    isPlaying: true,
    elapsedSeconds: 83,
    durationSeconds: 222,
    volume: 0.62,
    muted: false,
    volumeAvailable: true,
    transportAvailable: true,
    previousAvailable: true,
    nextAvailable: true,
  },
  "youtube-music": {
    active: true,
    appName: "YouTube Music",
    bundleId: "com.google.Chrome",
    title: "Dreams",
    artist: "Fleetwood Mac",
    album: "Rumours",
    isPlaying: true,
    elapsedSeconds: 61,
    durationSeconds: 257,
    volume: 0.48,
    muted: false,
    volumeAvailable: true,
    transportAvailable: true,
    previousAvailable: true,
    nextAvailable: true,
  },
  youtube: {
    active: true,
    appName: "YouTube",
    bundleId: "com.google.Chrome",
    title: "UltraVox live build session",
    artist: "Liberty Design Studio",
    album: null,
    isPlaying: true,
    elapsedSeconds: 245,
    durationSeconds: 913,
    volume: 0.8,
    muted: false,
    volumeAvailable: true,
    transportAvailable: true,
    previousAvailable: false,
    nextAvailable: true,
  },
};

type HarnessSource = "music" | "youtube-music" | "youtube";
type HarnessState = "playing" | "paused" | "unknown" | "volume-unavailable";

function fixtureState(source: HarnessSource, state: HarnessState): MediaState {
  const base = { ...SOURCE_FIXTURES[source] };
  if (state === "paused") return { ...base, isPlaying: false };
  if (state === "unknown") return { ...base, isPlaying: null };
  if (state === "volume-unavailable") {
    return { ...base, volume: null, muted: null, volumeAvailable: false };
  }
  return base;
}

export function ThemeHarness({
  state,
  source,
}: {
  state: HarnessState;
  source: HarnessSource;
}) {
  const services = useMemo<MediaPanelServices>(() => {
    const sample = fixtureState(source, state);
    return {
      getState: async () => sample,
      setVolume: async (volume) => {
        sample.volume = volume;
      },
      setMuted: async (muted) => {
        sample.muted = muted;
      },
      sendTransport: async () => undefined,
    };
  }, [state, source]);

  return (
    <div className="app" data-qa-harness="media-theme">
      <main className="main">
        <section className="hero" aria-labelledby="activity-status">
          <button
            className="icon-button frame-settings"
            type="button"
            title="Settings"
            aria-label="Open settings"
          >
            <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
              <circle cx="12" cy="12" r="3" />
              <path d="M19.4 15a1.7 1.7 0 0 0 .34 1.88l.06.06-2.83 2.83-.06-.06a1.7 1.7 0 0 0-1.88-.34 1.7 1.7 0 0 0-1.03 1.56V21h-4v-.09A1.7 1.7 0 0 0 9 19.36a1.7 1.7 0 0 0-1.88.34l-.06-.06-2.83-2.83.06-.06A1.7 1.7 0 0 0 4.63 15 1.7 1.7 0 0 0 3.09 14H3v-4h.09A1.7 1.7 0 0 0 4.64 9a1.7 1.7 0 0 0-.34-1.88l-.06-.06 2.83-2.83.06.06A1.7 1.7 0 0 0 9 4.63 1.7 1.7 0 0 0 10 3.09V3h4v.09A1.7 1.7 0 0 0 15 4.64a1.7 1.7 0 0 0 1.88-.34l.06-.06 2.83 2.83-.06.06A1.7 1.7 0 0 0 19.37 9 1.7 1.7 0 0 0 20.91 10H21v4h-.09A1.7 1.7 0 0 0 19.4 15Z" />
            </svg>
          </button>
          <div id="activity-status" className="status-label ready">
            Ready
          </div>
          <button className="record-button" type="button" aria-label="Start recording">
            <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
              <rect x="9" y="2" width="6" height="12" rx="3" />
              <path d="M5 10a7 7 0 0 0 14 0M12 17v5" />
            </svg>
          </button>
          <div className="secondary-actions">
            <button type="button" className="secondary-action">Meeting mode</button>
            <button type="button" className="secondary-action">Transcribe URL</button>
          </div>
        </section>

        <section className="history-section" aria-labelledby="latest-message-title">
          <div className="history-heading">
            <span id="latest-message-title">Latest message</span>
            <button className="history-link" type="button">View all</button>
          </div>
          <div className="empty-state compact">
            <p>Your latest transcription will appear here.</p>
          </div>
          <MediaPanel enabled suppressed={false} services={services} />
        </section>
      </main>
    </div>
  );
}

export function parseHarnessState(value: string | null): HarnessState {
  // "unavailable" is accepted as shorthand for the volume-unavailable fixture.
  if (value === "paused" || value === "unknown" || value === "volume-unavailable") {
    return value;
  }
  return value === "unavailable" ? "volume-unavailable" : "playing";
}

export function parseHarnessSource(value: string | null): HarnessSource {
  return value === "youtube-music" || value === "youtube" ? value : "music";
}
