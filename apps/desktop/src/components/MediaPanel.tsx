import { useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import {
  getMediaState,
  getSystemAudioSpectrum,
  mediaTransport,
  setSystemAudioMeterEnabled,
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

/** Backend contract: poll media state at most 1 Hz. */
const POLL_INTERVAL_MS = 1000;
/** Native system-output spectrum sampling cadence. */
const AUDIO_METER_POLL_INTERVAL_MS = 80;
/** Backend contract: exactly eleven normalized bands, low to high frequency. */
const SPECTRUM_BAND_COUNT = 11;

/**
 * Clamp an untrusted IPC payload to exactly eleven finite [0,1] bands.
 * Short, long, or malformed payloads degrade to silence instead of throwing.
 */
function sanitizeSpectrum(raw: unknown): number[] {
  const bands = Array.isArray(raw) && raw.length === SPECTRUM_BAND_COUNT ? raw : [];
  return Array.from({ length: SPECTRUM_BAND_COUNT }, (_, index) => {
    const value = bands[index];
    return typeof value === "number" && Number.isFinite(value)
      ? Math.min(1, Math.max(0, value))
      : 0;
  });
}

const SILENT_SPECTRUM = sanitizeSpectrum(null);

/**
 * Eleven bars map one-to-one onto the live low-to-high spectrum bands.
 * There is deliberately no weight table and no CSS animation: only the
 * sampled spectrum can move a bar.
 */
export function MediaEqualizer({
  bands,
  mirror = false,
}: {
  bands: readonly number[];
  mirror?: boolean;
}) {
  return (
    <div
      className={`media-eq${mirror ? " media-eq-mirror" : ""}`}
      aria-hidden="true"
    >
      {Array.from({ length: SPECTRUM_BAND_COUNT }, (_, index) => (
        <span
          key={index}
          className="media-eq-bar"
          style={{ "--meter-level": bands[index] ?? 0 } as CSSProperties}
        />
      ))}
    </div>
  );
}

/** Trailing throttle so a slider drag collapses into few IPC calls. */
const VOLUME_SEND_DELAY_MS = 120;

export interface MediaPanelServices {
  getState: () => Promise<MediaState>;
  setVolume: (volume: number) => Promise<void>;
  setMuted: (muted: boolean) => Promise<void>;
  setAudioMeterEnabled: (enabled: boolean) => Promise<void>;
  getAudioSpectrum: () => Promise<number[]>;
  sendTransport: (command: MediaTransportCommand) => Promise<void>;
}

const DEFAULT_SERVICES: MediaPanelServices = {
  getState: getMediaState,
  setVolume: setSystemVolume,
  setMuted: setSystemMuted,
  setAudioMeterEnabled: setSystemAudioMeterEnabled,
  getAudioSpectrum: getSystemAudioSpectrum,
  sendTransport: mediaTransport,
};

interface MediaPanelProps {
  /** Persisted AppConfig.media_panel_enabled. */
  enabled: boolean;
  /** True when theme meters and reactive visual chrome are enabled. */
  reactive?: boolean;
  /** True while UltraVox records or sits in a meeting; forces the panel hidden immediately. */
  suppressed: boolean;
  /** Injectable only for deterministic dev/test harnesses. */
  services?: MediaPanelServices;
}

export function MediaPanel({
  enabled,
  reactive = false,
  suppressed,
  services = DEFAULT_SERVICES,
}: MediaPanelProps) {
  const [sample, setSample] = useState<MediaState | null>(null);
  const [sampleAt, setSampleAt] = useState(0);
  const [clockNow, setClockNow] = useState(() => Date.now());
  const [visible, setVisible] = useState(false);
  const [spectrum, setSpectrum] = useState<number[]>(SILENT_SPECTRUM);
  const [reducedMotion, setReducedMotion] = useState(
    () => window.matchMedia("(prefers-reduced-motion: reduce)").matches,
  );
  const [pageVisible, setPageVisible] = useState(
    () => document.visibilityState === "visible",
  );
  const [volume, setVolume] = useState(0);
  const [muted, setMuted] = useState(false);
  const visibilityRef = useRef(initialMediaVisibility(Date.now()));
  const volumeSendTimerRef = useRef<number | null>(null);
  const pendingVolumeRef = useRef<number | null>(null);
  const meterCommandRef = useRef<Promise<void>>(Promise.resolve());

  useEffect(() => {
    const query = window.matchMedia("(prefers-reduced-motion: reduce)");
    const update = () => setReducedMotion(query.matches);
    query.addEventListener("change", update);
    return () => query.removeEventListener("change", update);
  }, []);

  useEffect(() => {
    const update = () => setPageVisible(document.visibilityState === "visible");
    document.addEventListener("visibilitychange", update);
    return () => document.removeEventListener("visibilitychange", update);
  }, []);
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
          setSampleAt(Date.now());
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

  useEffect(() => {
    const shouldSample =
      enabled && !suppressed && reactive && visible && !reducedMotion && pageVisible;
    let cancelled = false;
    let timer: number | null = null;

    const setMeterEnabled = (nextEnabled: boolean) => {
      const command = meterCommandRef.current
        .catch(() => undefined)
        .then(() => services.setAudioMeterEnabled(nextEnabled));
      meterCommandRef.current = command;
      return command;
    };
    const disableMeter = () => {
      void setMeterEnabled(false).catch(() => undefined);
    };

    if (!shouldSample) {
      setSpectrum(SILENT_SPECTRUM);
      disableMeter();
      return;
    }

    async function poll() {
      if (cancelled) return;
      try {
        const bands = await services.getAudioSpectrum();
        if (!cancelled) setSpectrum(sanitizeSpectrum(bands));
      } catch {
        if (!cancelled) setSpectrum(SILENT_SPECTRUM);
      } finally {
        if (!cancelled) {
          timer = window.setTimeout(() => void poll(), AUDIO_METER_POLL_INTERVAL_MS);
        }
      }
    }

    void (async () => {
      try {
        await setMeterEnabled(true);
        if (cancelled) {
          disableMeter();
          return;
        }
        await poll();
      } catch {
        if (!cancelled) setSpectrum(SILENT_SPECTRUM);
      }
    })();

    return () => {
      cancelled = true;
      if (timer != null) window.clearTimeout(timer);
      setSpectrum(SILENT_SPECTRUM);
      disableMeter();
    };
  }, [enabled, pageVisible, reactive, reducedMotion, services, suppressed, visible]);

  useEffect(() => {
    const root = document.documentElement;
    const clear = () => {
      delete root.dataset.mediaReactive;
      for (const property of ["--media-audio-low", "--media-audio-mid", "--media-audio-high", "--media-audio-peak"]) {
        root.style.removeProperty(property);
      }
    };
    if (!enabled || suppressed || !reactive || !visible || reducedMotion || !pageVisible) {
      clear();
      return;
    }
    const average = (from: number, to: number) =>
      spectrum.slice(from, to + 1).reduce((sum, band) => sum + band, 0) / (to - from + 1);
    root.dataset.mediaReactive = "true";
    root.style.setProperty("--media-audio-low", String(average(0, 2)));
    root.style.setProperty("--media-audio-mid", String(average(3, 7)));
    root.style.setProperty("--media-audio-high", String(average(8, 10)));
    root.style.setProperty("--media-audio-peak", String(Math.max(...spectrum)));
    return clear;
  }, [enabled, pageVisible, reactive, reducedMotion, spectrum, suppressed, visible]);

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
  const artworkDataUrl =
    sample.artworkDataUrl?.startsWith("data:image/") &&
    sample.artworkDataUrl.length <= 4 * 1024 * 1024
      ? sample.artworkDataUrl
      : null;
  const playback = mediaPlaybackKind(sample.isPlaying);
  const playingClock = playback === "playing" && sample.active;
  useEffect(() => {
    if (!playingClock) return;
    const timer = window.setInterval(() => setClockNow(Date.now()), 250);
    return () => window.clearInterval(timer);
  }, [playingClock]);

  const playing = playback === "playing";
  const playbackLabel =
    playback === "unknown" ? "Play or pause" : playing ? "Pause" : "Play";

  // Interpolate between IPC polls so seconds advance smoothly instead of in
  // 2s lurches (media fetch can take up to ~600ms inside a 1s interval).
  const baseElapsed = sample.elapsedSeconds ?? 0;
  const interpolatedElapsed =
    playingClock && sampleAt > 0
      ? baseElapsed + Math.max(0, clockNow - sampleAt) / 1000
      : baseElapsed;
  const hasElapsed = sample.elapsedSeconds != null;
  const elapsedClock = hasElapsed ? formatMediaClock(interpolatedElapsed) : null;
  const durationClock = formatMediaClock(sample.durationSeconds);
  const progressRatio = mediaProgressRatio(sample.elapsedSeconds, sample.durationSeconds);
  const timeSummary = mediaTimeSummary(sample.elapsedSeconds, sample.durationSeconds);
  const showProgress = elapsedClock != null || durationClock != null;

  const volumeReady = sample.volumeAvailable;
  const muteReady = sample.muted != null;

  // Band groups exposed to theme CSS: lows 0–2, mids 3–7, highs 8–10.
  const bandAverage = (from: number, to: number) =>
    spectrum.slice(from, to + 1).reduce((sum, band) => sum + band, 0) / (to - from + 1);
  const reactiveStyle = reactive
    ? ({
        "--audio-low": bandAverage(0, 2),
        "--audio-mid": bandAverage(3, 7),
        "--audio-high": bandAverage(8, 10),
        "--audio-peak": Math.max(...spectrum),
      } as CSSProperties)
    : undefined;

  return (
    <section
      className="media-panel"
      aria-label="Now playing"
      data-playback={playback}
      data-reactive={reactive || undefined}
      data-has-artwork={artworkDataUrl ? true : undefined}
      style={reactiveStyle}
    >
      <div className="media-display">
        {reactive && (
          <div className="media-atmosphere" aria-hidden="true">
            <span className="media-atmosphere-low" />
            <span className="media-atmosphere-mid" />
            <span className="media-atmosphere-high" />
            <span className="media-atmosphere-peak" />
          </div>
        )}
        {reactive && <MediaEqualizer bands={spectrum} mirror />}
        <div className="media-preview">
          <span className="media-artwork" aria-hidden="true">
            {artworkDataUrl ? (
              <img src={artworkDataUrl} alt="" decoding="async" />
            ) : (
              <span className="media-artwork-fallback">{title?.slice(0, 1) || "♪"}</span>
            )}
          </span>
          <div className="media-meta">
            <span className="media-source">{source}</span>
            <span className={`media-title${title ? "" : " is-unknown"}`}>
              {title ?? "System media session"}
            </span>
            {artist && <span className="media-artist">{artist}</span>}
            {album && <span className="media-album">{album}</span>}
          </div>
        </div>
        {reactive && <MediaEqualizer bands={spectrum} />}
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
