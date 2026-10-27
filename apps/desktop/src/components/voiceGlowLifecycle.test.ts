/**
 * Synthetic lifecycle/level checks only — no native audio, visual, motion,
 * accessibility, or performance proof. Run from apps/desktop with:
 * node --experimental-strip-types src/components/voiceGlowLifecycle.test.ts
 */
import {
  beamVoiceLevel,
  normalizeVoiceLevel,
  settleVoiceLevel,
  voiceAnimationEnabled,
  voicePalette,
  voicePhase,
  VOICE_PALETTES,
} from "./voiceGlowLifecycle.ts";

function check(condition: boolean, message: string): void {
  if (!condition) throw new Error(`Synthetic voice lifecycle: ${message}`);
}

const listening = voicePhase({ recording: true, processing: false });
check(listening === "listening", "Record starts listening before any sample");
check(beamVoiceLevel(listening, 0) === 0, "silence never becomes fabricated speech amplitude");
check(voiceAnimationEnabled(listening, true, false), "silence still mounts the library's idle breathing");
check(voicePhase({ recording: false, processing: true }) === "processing", "stop uses actual processing state without a delay");
check(beamVoiceLevel("processing", 0.9) === 0, "processing discards the prior microphone level");
check(voicePhase({ recording: false, processing: false }) === "idle", "completion returns to idle");
check(voicePhase({ recording: true, processing: true }) === "listening", "recording takes precedence over prior processing");

for (const phase of ["idle", "listening", "processing"] as const) {
  check(!voiceAnimationEnabled(phase, false, false), `${phase}: hidden decoration is unmounted`);
  check(!voiceAnimationEnabled(phase, true, true), `${phase}: reduced-motion decoration is unmounted`);
}
check(!voiceAnimationEnabled("idle", true, false), "idle decoration is unmounted");
check(voiceAnimationEnabled("listening", true, false), "reopened active view can animate again");
for (const invalid of [NaN, Infinity, -Infinity, -1]) {
  check(normalizeVoiceLevel(invalid) === 0, "invalid/negative levels normalize to zero");
}
check(normalizeVoiceLevel(2) === 1, "hot input remains bounded");
check(beamVoiceLevel("listening", 0.4) === 0.4, "manual input remains quantitative");
check(settleVoiceLevel(0, 0.8) > settleVoiceLevel(0, 0.2), "larger RMS produces a larger response");
check(settleVoiceLevel(0, 1) > 1 - settleVoiceLevel(1, 0), "attack is faster than release");
check(settleVoiceLevel(0.4, 0.9, 0) === 0.4, "zero elapsed time does not jump");
const oneStep = settleVoiceLevel(0, 0.8, 160);
const twoSteps = settleVoiceLevel(settleVoiceLevel(0, 0.8, 80), 0.8, 80);
check(Math.abs(oneStep - twoSteps) < 1e-12, "response is elapsed-time based, not sample-count based");
let silence = 1;
for (let i = 0; i < 60; i++) silence = settleVoiceLevel(silence, 0);
check(silence < 1e-6, "silence settles without disabling listening presence");
check(Number.isFinite(settleVoiceLevel(NaN, Infinity, NaN)), "invalid input never leaks NaN");

check(Object.keys(VOICE_PALETTES).length === 14, "all fourteen theme palettes exist");
for (const [theme, palette] of Object.entries(VOICE_PALETTES)) {
  check(/^#[0-9a-f]{6}$/i.test(palette.core), `${theme}: finite core color`);
  check(/^#[0-9a-f]{6}$/i.test(palette.halo), `${theme}: finite halo color`);
}
check(voicePalette("unknown-theme") === VOICE_PALETTES.midnight, "unknown theme has a finite fallback");
console.log("Synthetic voice lifecycle checks passed (not native/visual evidence).");
