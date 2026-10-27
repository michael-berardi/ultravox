import { useCallback, useEffect, useRef, useState, type CSSProperties } from "react";
import { VoiceBeam, type VoiceBeamMotion } from "voice-glow";
import { beamVoiceLevel, normalizeVoiceLevel, voiceAnimationEnabled, voicePalette, type VoicePhase } from "./voiceGlowLifecycle";
import { VoiceLevelTracker } from "./voiceLevel";

const BEAM_FADE_MS = 450;
// The native meter is a snapshot of the latest capture buffer, so it is read
// often; the tracker turns the snapshots into an envelope.
const SAMPLE_MS = 33;
const STALE_MS = 200;

interface RecordingSurroundProps {
  phase: VoicePhase;
  readLevel: () => Promise<number>;
  theme: string;
}

export function RecordingSurround({ phase, readLevel, theme }: RecordingSurroundProps) {
  const host = useRef<HTMLDivElement>(null);
  const level = useRef(0);
  const measuredAt = useRef(0);
  const phaseStartedAt = useRef(0);
  const [visible, setVisible] = useState(false);
  const [reduced, setReduced] = useState(true);
  const [radius, setRadius] = useState(16);
  const palette = voicePalette(theme);
  const light = theme === "frutiger-aero" || theme === "nord-frost" || theme === "crystal";
  const running = voiceAnimationEnabled(phase, visible, reduced);
  // Keep the beam mounted while the panel's 420 ms opacity fade carries it out;
  // unmounting at once cut the light off instead of letting it fade.
  const [lingering, setLingering] = useState(false);
  useEffect(() => {
    if (running) {
      setLingering(true);
      return;
    }
    const timer = window.setTimeout(() => setLingering(false), BEAM_FADE_MS);
    return () => window.clearTimeout(timer);
  }, [running]);
  const beamMounted = running || (lingering && !reduced);

  useEffect(() => {
    const parent = host.current?.parentElement;
    if (!parent) return;
    const value = parseFloat(window.getComputedStyle(parent).borderTopLeftRadius);
    setRadius(Number.isFinite(value) ? Math.max(0, value) : 16);
  }, [theme]);

  // Visibility changes are infrequent React state; RMS samples never are.
  // Ancestor hidden/inert changes matter because Tauri keeps views mounted.
  useEffect(() => {
    const element = host.current;
    if (!element) return;
    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    let intersecting = true;
    const update = () => {
      setReduced(media.matches);
      setVisible(!document.hidden && intersecting &&
        !element.parentElement?.closest("[hidden], [inert], [aria-hidden='true']") &&
        element.getClientRects().length > 0);
    };
    const intersection = new IntersectionObserver(([entry]) => {
      intersecting = entry?.isIntersecting ?? false;
      update();
    });
    intersection.observe(element);
    const mutation = new MutationObserver(update);
    for (let ancestor = element.parentElement; ancestor; ancestor = ancestor.parentElement) {
      mutation.observe(ancestor, { attributes: true, attributeFilter: ["hidden", "inert", "aria-hidden", "class", "style"] });
    }
    document.addEventListener("visibilitychange", update);
    media.addEventListener("change", update);
    update();
    return () => {
      intersection.disconnect();
      mutation.disconnect();
      document.removeEventListener("visibilitychange", update);
      media.removeEventListener("change", update);
    };
  }, []);

  useEffect(() => {
    phaseStartedAt.current = performance.now();
    level.current = 0;
    measuredAt.current = 0;
    if (phase !== "listening" || !visible || reduced) return;
    let cancelled = false;
    let timer: number | undefined;
    const tracker = new VoiceLevelTracker();
    let lastSampleAt = performance.now() - SAMPLE_MS;
    let warned = false;
    const sample = async () => {
      let value: number | null = null;
      try {
        value = await readLevel();
        warned = false;
      } catch (cause) {
        if (!warned) console.warn("UltraVox input level unavailable", cause);
        warned = true;
      }
      if (cancelled) return;
      const now = performance.now();
      level.current = value === null ? tracker.release(now - lastSampleAt) : tracker.update(value, now - lastSampleAt);
      measuredAt.current = now;
      lastSampleAt = now;
      timer = window.setTimeout(() => void sample(), SAMPLE_MS);
    };
    void sample();
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
      level.current = 0;
    };
  }, [phase, visible, reduced, readLevel]);

  const readBeamLevel = useCallback(() => {
    // A stalled IPC read cannot hold stale speech energy. The library retains
    // its subtle listening motion; this getter never invents microphone input.
    const fresh = performance.now() - measuredAt.current <= STALE_MS;
    return beamVoiceLevel(phase, fresh ? level.current : 0);
  }, [phase]);
  const readMotion = useCallback((): VoiceBeamMotion | null => {
    if (phase !== "processing") return null;
    // Public library motion API, separate from the measured speech level.
    // Processing gathers the lobes into a gently travelling light; no fake RMS.
    const seconds = (performance.now() - phaseStartedAt.current) / 1000;
    return { gather: 1, offset: Math.sin(seconds * 0.85) * 0.8, stretch: 0.2, heldLevel: 0.24, cornerFollow: 0.5 };
  }, [phase]);

  // The library hands back its smoothed level every frame; the ambient halo
  // behind the controls follows it through one CSS variable, no React render.
  const lastHalo = useRef(-1);
  const followLevel = useCallback((value: number) => {
    const rounded = Math.round(normalizeVoiceLevel(value) * 100) / 100;
    if (rounded === lastHalo.current) return;
    lastHalo.current = rounded;
    host.current?.style.setProperty("--voice-level", String(rounded));
  }, []);
  useEffect(() => {
    lastHalo.current = -1;
    host.current?.style.setProperty("--voice-level", "0");
  }, [phase, running]);

  return (
    <div
      ref={host}
      className="recording-surround"
      data-phase={phase}
      data-running={running || undefined}
      data-reduced={(reduced && phase !== "idle") || undefined}
      aria-hidden="true"
      // WebKit does not clip composited, blurred layers to an ancestor's
      // rounded overflow; an explicit rounded clip keeps the panel corners.
      style={{ "--voice-core": palette.core, "--voice-halo": palette.halo, clipPath: `inset(0 round ${radius}px)` } as CSSProperties}
    >
      <div className="recording-surround-halo" />
      {beamMounted && (
        <VoiceBeam
          className="recording-surround-beam"
          level={readBeamLevel}
          motion={readMotion}
          onLevel={followLevel}
          theme={light ? "light" : "dark"}
          colors={[palette.core, palette.halo, palette.core, palette.halo, palette.core, palette.halo, palette.core]}
          bandColors={{ core: palette.core, above: palette.halo, mid: palette.core, below: palette.halo }}
          staticColors
          borderRadius={radius}
          idle={phase === "processing" ? 0.5 : 0.12}
          breatheDuration={phase === "processing" ? 2.8 : 7.5}
          threshold={0}
          attack={0.02}
          release={0.06}
          flow={phase === "processing" ? 18 : 40}
          bend={36}
          bandStrength={light ? 0.9 : 1.1}
          bandAberration={0.4}
          distortion={0}
          coreLight={0}
          strokeOpacity={0}
          innerOpacity={1}
          brightness={light ? 1 : 1.05}
          saturation={light ? 1.5 : 1.2}
          scale={1.25}
          reach={1.6}
          spread={1}
          rangeWidth={1.1}
          bloomOpacity={light ? 0.3 : 0.8}
          glowSize={1.1}
        >
          <div className="recording-surround-surface" />
        </VoiceBeam>
      )}
    </div>
  );
}
