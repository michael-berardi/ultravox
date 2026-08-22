import { useMemo } from "react";
import { MediaPanel, type MediaPanelServices } from "../components/MediaPanel";
import type { MediaState } from "../ipc";

const BASE_MEDIA_STATE: MediaState = {
  active: true,
  appName: "Music",
  bundleId: "com.apple.Music",
  title: "Midnight City",
  artist: "M83",
  isPlaying: true,
  volume: 0.62,
  muted: false,
  volumeAvailable: true,
  transportAvailable: true,
  previousAvailable: false,
  nextAvailable: false,
};

type HarnessState = "playing" | "paused" | "unknown" | "volume-unavailable";

function fixtureState(state: HarnessState): MediaState {
  if (state === "paused") return { ...BASE_MEDIA_STATE, isPlaying: false };
  if (state === "unknown") return { ...BASE_MEDIA_STATE, isPlaying: null };
  if (state === "volume-unavailable") {
    return {
      ...BASE_MEDIA_STATE,
      volume: null,
      muted: null,
      volumeAvailable: false,
    };
  }
  return { ...BASE_MEDIA_STATE };
}

export function ThemeHarness({ state }: { state: HarnessState }) {
  const services = useMemo<MediaPanelServices>(() => {
    const sample = fixtureState(state);
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
  }, [state]);

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
              <path d="M19.4 15a1.7 1.7 0 0 0 .34 1.88l.06.06-2.83 2.83-.06-.06a1.7 1.7 0 0 0-1.88-.34 1.7 1.7 0 0 0-1.03 1.56V21h-4v-.09A1.7 1.7 0 0 0 9 19.36a1.7 1.7 0 0 0-1.88.34l-.06.06-2.83-2.83.06-.06A1.7 1.7 0 0 0 4.63 15 1.7 1.7 0 0 0 3.09 14H3v-4h.09A1.7 1.7 0 0 0 4.64 9a1.7 1.7 0 0 0-.34-1.88l-.06-.06 2.83-2.83.06.06A1.7 1.7 0 0 0 9 4.63 1.7 1.7 0 0 0 10 3.09V3h4v.09A1.7 1.7 0 0 0 15 4.64a1.7 1.7 0 0 0 1.88-.34l.06-.06 2.83 2.83-.06.06A1.7 1.7 0 0 0 19.37 9 1.7 1.7 0 0 0 20.91 10H21v4h-.09A1.7 1.7 0 0 0 19.4 15Z" />
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
  return value === "paused" || value === "unknown" || value === "volume-unavailable"
    ? value
    : "playing";
}
