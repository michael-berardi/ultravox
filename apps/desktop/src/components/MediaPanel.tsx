import { useEffect, useRef, useState } from "react";
import {
  getMediaState,
  mediaTransport,
  setSystemMuted,
  setSystemVolume,
  type MediaState,
  type MediaTransportCommand,
} from "../ipc";
import {
  formatMediaClock,
  initialMediaVisibility,
  mediaPlaybackKind,
  mediaProgressRatio,
  mediaTimeSummary,
  nextMediaVisibility,
  normalizeVolumePercent,
} from "./mediaActivity";

/** Backend contract: poll at most 1 Hz. */
const POLL_INTERVAL_MS = 1000;
/** Trailing throttle so a slider drag collapses into few IPC calls. */
const VOLUME_SEND_DELAY_MS = 120;

export interface MediaPanelServices {
  getState: () => Promise<MediaState>;
  setVolume: (volume: number) => Promise<void>;
  setMuted: (muted: boolean) => Promise<void>;
  sendTransport: (command: MediaTransportCommand) => Promise<void>;
}

const DEFAULT_SERVICES: MediaPanelServices = {
  getState: getMediaState,
  setVolume: setSystemVolume,
  setMuted: setSystemMuted,
  sendTransport: mediaTransport,
};

interface MediaPanelProps {
  /** Persisted AppConfig.media_panel_enabled. */
  enabled: boolean;
  /** True while UltraVox records or sits in a meeting; forces the panel hidden immediately. */
  suppressed: boolean;
  /** Injectable only for deterministic dev/test harnesses. */
  services?: MediaPanelServices;
}

/**
 * Purely decorative playback-state animation. UltraVox never captures PCM,
 * so these bars are NOT a measured spectrum: they idle low when paused or
 * unknown and sway only while playback is confirmed. Themes restyle the
 * motif; the reduced-motion media query freezes it entirely.
 */
const EQUALIZER_BAR_COUNT = 11;

export function MediaEqualizer({
  playing,
  mirror = false,
}: {
  playing: boolean;
  mirror?: boolean;
}) {
  return (
    <div
      className={`media-eq${mirror ? " media-eq-mirror" : ""}`}
      aria-hidden="true"
      data-active={playing || undefined}
    >
      {Array.from({ length: EQUALIZER_BAR_COUNT }, (_, index) => (
        <span key={index} className="media-eq-bar" />
      ))}
    </div>
  );
}

export function MediaPanel({
  enabled,
  suppressed,
  services = DEFAULT_SERVICES,
}: MediaPanelProps) {
  const [sample, setSample] = useState<MediaState | null>(null);
  const [visible, setVisible] = useState(false);
  const [volume, setVolume] = useState(0);
  const [muted, setMuted] = useState(false);
  const visibilityRef = useRef(initialMediaVisibility(Date.now()));
  const volumeSendTimerRef = useRef<number | null>(null);
  const pendingVolumeRef = useRef<number | null>(null);

  useEffect(() => {
    if (!enabled || suppressed) {
      visibilityRef.current = initialMediaVisibility(Date.now());
      setVisible(false);
      setSample(null);
      return;
    }
    let cancelled = false;

    async function tick() {
      try {
        const state = await services.getState();
        if (cancelled) return;
        const nextVisibility = nextMediaVisibility(
          visibilityRef.current,
          state.active,
          Date.now(),
        );
        visibilityRef.current = nextVisibility;
        if (state.active) {
          setSample(state);
        } else if (!nextVisibility.visible) {
          setSample(null);
        }
        setVisible(nextVisibility.visible);
        if (state.volume != null && volumeSendTimerRef.current == null) {
          setVolume(normalizeVolumePercent(state.volume));
        }
        if (state.muted != null) setMuted(state.muted);
      } catch {
        if (cancelled) return;
        const nextVisibility = nextMediaVisibility(visibilityRef.current, false, Date.now());
        visibilityRef.current = nextVisibility;
        setVisible(nextVisibility.visible);
        if (!nextVisibility.visible) setSample(null);
        // The panel is optional chrome; repeated backend failures age it out.
      }
    }

    void tick();
    const interval = window.setInterval(() => void tick(), POLL_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, [enabled, services, suppressed]);

  useEffect(
    () => () => {
      if (volumeSendTimerRef.current != null) {
        window.clearTimeout(volumeSendTimerRef.current);
        volumeSendTimerRef.current = null;
      }
    },
    [],
  );

  const changeVolume = (raw: number) => {
    const percent = Math.round(Math.min(100, Math.max(0, raw)));
    setVolume(percent);
    pendingVolumeRef.current = percent;
    if (volumeSendTimerRef.current != null) return;
    volumeSendTimerRef.current = window.setTimeout(() => {
      volumeSendTimerRef.current = null;
      const pending = pendingVolumeRef.current;
      pendingVolumeRef.current = null;
      // Wire contract: CoreAudio 0..1 scalar.
      if (pending != null) void services.setVolume(pending / 100).catch(() => undefined);
    }, VOLUME_SEND_DELAY_MS);
  };

  const toggleMute = async () => {
    const next = !muted;
    try {
      await services.setMuted(next);
      setMuted(next);
    } catch {
      // The next poll retains the authoritative device state.
    }
  };

  const sendTransport = async (command: MediaTransportCommand) => {
    try {
      await services.sendTransport(command);
      if (command === "play_pause") {
        setSample((current) => {
          if (current?.isPlaying == null) return current;
          return { ...current, isPlaying: !current.isPlaying };
        });
      }
    } catch {
      // The next poll retains the authoritative now-playing state.
    }
  };

  if (!enabled || suppressed || !visible || !sample) return null;

  const source = sample.appName?.trim() || "System audio";
  const title = sample.title?.trim() || null;
  const artist = sample.artist?.trim() || null;
  const album = sample.album?.trim() || null;
  const playback = mediaPlaybackKind(sample.isPlaying);
  const playing = playback === "playing";
  const playbackLabel =
    playback === "unknown" ? "Play or pause" : playing ? "Pause" : "Play";

  const elapsedClock = formatMediaClock(sample.elapsedSeconds);
  const durationClock = formatMediaClock(sample.durationSeconds);
  const progressRatio = mediaProgressRatio(sample.elapsedSeconds, sample.durationSeconds);
  const timeSummary = mediaTimeSummary(sample.elapsedSeconds, sample.durationSeconds);
  const showProgress = elapsedClock != null || durationClock != null;

  const volumeReady = sample.volumeAvailable;
  const muteReady = sample.muted != null;

  return (
    <section className="media-panel" aria-label="Now playing" data-playback={playback}>
      <div className="media-display">
        <MediaEqualizer playing={playing} mirror />
        <div className="media-meta">
          <span className="media-source">{source}</span>
          <span className={`media-title${title ? "" : " is-unknown"}`}>
            {title ?? "Unknown track"}
          </span>
          {artist && <span className="media-artist">{artist}</span>}
          {album && <span className="media-album">{album}</span>}
        </div>
        <MediaEqualizer playing={playing} />
      </div>

      {showProgress && (
        <div className="media-progress">
          <span className="media-time">{elapsedClock ?? "—:—"}</span>
          {progressRatio != null ? (
            <div
              className="media-progress-track"
              role="progressbar"
              aria-label="Track progress"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={Math.round(progressRatio * 100)}
              aria-valuetext={timeSummary ?? undefined}
            >
              <div
                className="media-progress-fill"
                style={{ width: `${progressRatio * 100}%` }}
              />
            </div>
          ) : (
            <div className="media-progress-track is-indeterminate" aria-hidden="true" />
          )}
          <span className="media-time media-time-end">{durationClock ?? "—:—"}</span>
        </div>
      )}

      <div className="media-controls">
        <div className="media-transport" role="group" aria-label="Playback controls">
          <button
            type="button"
            className="icon-button media-button"
            title="Previous track"
            aria-label="Previous track"
            disabled={!sample.previousAvailable}
            onClick={() => void sendTransport("previous")}
          >
            <SkipBackIcon />
          </button>
          <button
            type="button"
            className="icon-button media-button media-play"
            title={playbackLabel}
            aria-label={playbackLabel}
            disabled={!sample.transportAvailable}
            onClick={() => void sendTransport("play_pause")}
          >
            {playing ? <PauseIcon /> : <PlayIcon />}
          </button>
          <button
            type="button"
            className="icon-button media-button"
            title="Next track"
            aria-label="Next track"
            disabled={!sample.nextAvailable}
            onClick={() => void sendTransport("next")}
          >
            <SkipForwardIcon />
          </button>
        </div>

        {/* Decorative rotary-dial chrome; themes that want a knob reveal it. */}
        <span className="media-dial" aria-hidden="true" />

        <div className="media-volume">
          <span className="media-volume-label" id="media-volume-label">
            Volume
          </span>
          <input
            className="media-volume-slider"
            type="range"
            id="media-volume-slider"
            min={0}
            max={100}
            step={1}
            value={volume}
            aria-labelledby="media-volume-label"
            aria-valuetext={volumeReady ? `${volume}%` : "Unavailable"}
            disabled={!volumeReady}
            onChange={(event) => changeVolume(event.target.valueAsNumber)}
          />
          <output className="media-volume-value" htmlFor="media-volume-slider">
            {volumeReady ? `${volume}%` : "—"}
          </output>
          <button
            type="button"
            className="icon-button media-button"
            title={muted ? "Unmute system audio" : "Mute system audio"}
            aria-label={muted ? "Unmute system audio" : "Mute system audio"}
            aria-pressed={muted}
            disabled={!muteReady}
            onClick={() => void toggleMute()}
          >
            {muted ? <VolumeMutedIcon /> : <VolumeIcon />}
          </button>
        </div>
      </div>
    </section>
  );
}

function PlayIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <path d="M8 5.5v13a1 1 0 0 0 1.54.84l10-6.5a1 1 0 0 0 0-1.68l-10-6.5A1 1 0 0 0 8 5.5Z" />
    </svg>
  );
}

function PauseIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <rect x="6" y="5" width="4" height="14" rx="1" />
      <rect x="14" y="5" width="4" height="14" rx="1" />
    </svg>
  );
}

function SkipBackIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M19 20 9 12l10-8v16Z" fill="currentColor" stroke="none" />
      <path d="M5 19V5" />
    </svg>
  );
}

function SkipForwardIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="m5 4 10 8-10 8V4Z" fill="currentColor" stroke="none" />
      <path d="M19 5v14" />
    </svg>
  );
}

function VolumeIcon() {
  return (
    <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M11 5 6 9H2v6h4l5 4V5Z" fill="currentColor" stroke="none" />
      <path d="M15.5 8.5a5 5 0 0 1 0 7" />
      <path d="M18.5 5.5a9 9 0 0 1 0 13" />
    </svg>
  );
}

function VolumeMutedIcon() {
  return (
    <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M11 5 6 9H2v6h4l5 4V5Z" fill="currentColor" stroke="none" />
      <path d="m16 9 6 6" />
      <path d="m22 9-6 6" />
    </svg>
  );
}
