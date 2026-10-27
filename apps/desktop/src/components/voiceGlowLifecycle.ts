export type VoicePhase = "idle" | "listening" | "processing";

// Phase is native activity/request state, never an amplitude threshold.
export function voicePhase({ recording, processing }: { recording: boolean; processing: boolean }): VoicePhase {
  return recording ? "listening" : processing ? "processing" : "idle";
}

export function voiceAnimationEnabled(phase: VoicePhase, visible: boolean, reduced: boolean): boolean {
  return phase !== "idle" && visible && !reduced;
}

// Finite colors only: the renderer must not resolve CSS variables. Light
// materials deliberately use an ink-colored core instead of white glare.
export const VOICE_PALETTES = {
  midnight: { core: "#8eb7ff", halo: "#b794ff" },
  "frutiger-aero": { core: "#166a94", halo: "#218968" },
  "frutiger-dark": { core: "#35d0ff", halo: "#6ee87a" },
  winamp: { core: "#7eff54", halo: "#ff9f2e" },
  "winamp-mmd3": { core: "#9fc4ff", halo: "#ffb347" },
  "winamp-bento": { core: "#ff8c1a", halo: "#ffb45d" },
  "winamp-hifi": { core: "#ffb347", halo: "#7ef0a0" },
  pioneer: { core: "#00a8ff", halo: "#8ff7ff" },
  olive: { core: "#a3b86b", halo: "#d96a5f" },
  "nord-frost": { core: "#365f86", halo: "#367c8a" },
  "solar-dusk": { core: "#ffb86b", halo: "#f0719a" },
  vapor: { core: "#01cdfe", halo: "#ff71ce" },
  "obsidian-rite": { core: "#a06bff", halo: "#8fe8ff" },
  crystal: { core: "#936014", halo: "#526c84" },
} as const;

export function voicePalette(theme: string) {
  return VOICE_PALETTES[theme as keyof typeof VOICE_PALETTES] ?? VOICE_PALETTES.midnight;
}

export function normalizeVoiceLevel(level: number): number {
  return Number.isFinite(level) ? Math.max(0, Math.min(1, level)) : 0;
}

// Fast attack, slower release: onsets read at once, decay stays visible.
export const VOICE_ATTACK_MS = 25;
export const VOICE_RELEASE_MS = 160;

export function settleVoiceLevel(current: number, sample: number, elapsedMs = 80): number {
  const previous = normalizeVoiceLevel(current);
  const target = normalizeVoiceLevel(sample);
  // Time-based attack/release stays quantitative even if IPC timing varies.
  const elapsed = Number.isFinite(elapsedMs) ? Math.max(0, elapsedMs) : 80;
  const timeConstant = target > previous ? VOICE_ATTACK_MS : VOICE_RELEASE_MS;
  return previous + (target - previous) * (1 - Math.exp(-elapsed / timeConstant));
}

export function beamVoiceLevel(phase: VoicePhase, measuredLevel: number): number {
  // Library free motion supplies listening presence at zero RMS. Neither
  // silence nor processing is represented as invented speech amplitude.
  return phase === "listening" ? normalizeVoiceLevel(measuredLevel) : 0;
}
