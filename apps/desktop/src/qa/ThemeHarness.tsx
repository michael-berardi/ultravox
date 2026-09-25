import { useEffect, useMemo, useState } from "react";
import { BrandMark, proBrandSuffix } from "../components/BrandMark";
import { MediaPanel, type MediaPanelServices } from "../components/MediaPanel";
import { DropOverlay } from "../components/DropOverlay";
import { ProLockBadge, ProLockPrompt } from "../components/ProLock";
import { RecordingMeter } from "../components/ReactiveMeter";
import {
  TRANSCRIBING_INDICATOR_DELAY_MS,
  useDelayedVisibility,
} from "../components/useDelayedVisibility";
import type { MediaState } from "../ipc";
import { mapProStatus, type ProStatus } from "../lib/proStatus";

const QA_ARTWORK =
  "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 96 96'%3E%3Crect width='96' height='96' rx='14' fill='%23091222'/%3E%3Ccircle cx='48' cy='48' r='30' fill='%231a315b'/%3E%3Cpath d='M18 58c13-20 22 17 34-4s18 9 28-16' fill='none' stroke='%237ceaff' stroke-width='7' stroke-linecap='round'/%3E%3Ccircle cx='69' cy='30' r='8' fill='%23ff71ce'/%3E%3C/svg%3E";
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
    artworkDataUrl: QA_ARTWORK,
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
    title: "UltraVox media controls demo",
    artist: "Open Source Preview",
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
export type HarnessPro = "unlocked" | "locked" | "unavailable";

/** The three Pro states the harness can stage: `?qa-pro=…`. */
export function qaProStatus(mode: HarnessPro): ProStatus {
  return mapProStatus({
    available: mode !== "unavailable",
    unlocked: mode === "unlocked",
    state: mode === "unlocked" ? "paid" : mode === "unavailable" ? "unavailable" : "none",
  });
}

export function parseHarnessPro(value: string | null): ProStatus | null {
  return value === "unlocked" || value === "locked" || value === "unavailable"
    ? qaProStatus(value)
    : null;
}
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

/**
 * Stable deterministic spectrum fixtures, low to high. These prove that the
 * same theme scene responds differently to bass-, mid-, and high-led audio
 * without random or time-derived motion.
 */
const SPECTRUM_FIXTURES = {
  default: [0.88, 0.66, 0.42, 0.24, 0.54, 0.76, 0.6, 0.34, 0.22, 0.13, 0.07],
  low: [0.96, 0.82, 0.64, 0.28, 0.18, 0.12, 0.08, 0.05, 0.03, 0.02, 0.01],
  mid: [0.1, 0.18, 0.3, 0.58, 0.88, 0.96, 0.78, 0.5, 0.22, 0.1, 0.05],
  high: [0.04, 0.06, 0.09, 0.14, 0.22, 0.38, 0.56, 0.74, 0.9, 0.98, 0.84],
} as const;
type HarnessSpectrum = keyof typeof SPECTRUM_FIXTURES;

export function ThemeHarness({
  drop,
  pro,
  reactive,
  recordingLevel,
  transcriptionDurationMs,
  state,
  source,
  spectrum,
}: {
  drop: boolean;
  pro: ProStatus | null;
  reactive: boolean;
  recordingLevel: number;
  transcriptionDurationMs: number;
  state: HarnessState;
  source: HarnessSource;
  spectrum: HarnessSpectrum;
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
      setAudioMeterEnabled: async () => undefined,
      getAudioSpectrum: async () => [...SPECTRUM_FIXTURES[spectrum]],
      sendTransport: async () => undefined,
    };
  }, [spectrum, state, source]);
  const [transcriptionActive, setTranscriptionActive] = useState(transcriptionDurationMs > 0);
  useEffect(() => {
    setTranscriptionActive(transcriptionDurationMs > 0);
    if (transcriptionDurationMs <= 0) return;
    const timer = window.setTimeout(() => setTranscriptionActive(false), transcriptionDurationMs);
    return () => window.clearTimeout(timer);
  }, [transcriptionDurationMs]);
  const showTranscribing = useDelayedVisibility(
    transcriptionActive ? "qa-transcription" : null,
    TRANSCRIBING_INDICATOR_DELAY_MS,
  );

  return (
    <div
      className="app"
      data-qa-harness="media-theme"
      data-qa-transcribing-visible={showTranscribing}
    >
      {pro && (
        <header className="header">
          <BrandMark suffix={proBrandSuffix(pro)} />
        </header>
      )}
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
          <div
            id="activity-status"
            className={`status-label ${showTranscribing ? "transcribing" : "ready"}`}
          >
            {showTranscribing ? "Transcribing" : "Ready"}
          </div>
          {reactive && (
            <RecordingMeter active={recordingLevel > 0} level={recordingLevel} />
          )}
          <button className="record-button" type="button" aria-label="Start recording">
            <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
              <rect x="9" y="2" width="6" height="12" rx="3" />
              <path d="M5 10a7 7 0 0 0 14 0M12 17v5" />
            </svg>
          </button>
          <div className="secondary-actions">
            {(!pro || pro.available) && (
              <>
                <button type="button" className={`secondary-action${pro && !pro.unlocked ? " pro-locked" : ""}`}>
                  {pro && !pro.unlocked && <ProLockBadge />}
                  Meeting mode
                </button>
                <button type="button" className={`secondary-action${pro && !pro.unlocked ? " pro-locked" : ""}`}>
                  {pro && !pro.unlocked && <ProLockBadge />}
                  Lecture mode
                </button>
              </>
            )}
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
          {!pro || pro.unlocked ? (
            <MediaPanel enabled reactive={reactive} suppressed={false} services={services} />
          ) : (
            pro.available && (
              <ProLockPrompt
                copy="The media console is part of UltraVox Pro."
                onOpenPro={() => undefined}
              />
            )
          )}
        </section>
      </main>
      {drop && <DropOverlay />}
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

export function parseHarnessSpectrum(value: string | null): HarnessSpectrum {
  return value === "low" || value === "mid" || value === "high" ? value : "default";
}
