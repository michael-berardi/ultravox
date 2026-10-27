/**
 * Synthetic envelope/noise-floor checks only — no native audio, visual,
 * motion, or performance proof. Run from apps/desktop with:
 * node --experimental-strip-types src/components/voiceLevel.test.ts
 */
import {
  CALIBRATION_MS,
  CALM_LEVEL,
  FLOOR_MAX_DB,
  FLOOR_MIN_DB,
  GATE_DB,
  SPAN_DB,
  VoiceLevelTracker,
  nativeLevelToDb,
  speechLevel,
} from "./voiceLevel.ts";

function check(condition: boolean, message: string): void {
  if (!condition) throw new Error(`Synthetic voice level: ${message}`);
}

// Inverse of the native meter: dBFS in [-60, -3] -> [0, 1].
const native = (db: number) => Math.max(0, Math.min(1, (db + 60) / 57));
// The beam library's own saturating curve (threshold 0), for end-to-end bounds.
const libraryCurve = (x: number) => (1 - Math.exp(-3 * x)) / (1 - Math.exp(-3));
const STEP_MS = 33;

check(nativeLevelToDb(0) === -60 && nativeLevelToDb(1) === -3, "native meter endpoints are -60 and -3 dBFS");
check(Math.abs(nativeLevelToDb(0.5) + 31.5) < 1e-9, "native meter is linear in dB");
check(Math.abs(nativeLevelToDb(native(-30)) + 30) < 1e-9, "dB round-trips through the native mapping");
check(nativeLevelToDb(NaN) === -60 && nativeLevelToDb(9) === -3, "invalid native levels stay bounded");

// Curve: nothing inside the gate, monotonic, bounded, full at gate + span.
check(speechLevel(-50, -50) === 0 && speechLevel(-50 + GATE_DB, -50) === 0, "room noise within the gate maps to 0");
check(speechLevel(-50 + GATE_DB + SPAN_DB, -50) === 1 && speechLevel(0, -50) === 1, "level saturates at 1");
let previous = -1;
for (let excess = -10; excess <= 40; excess += 1) {
  const value = speechLevel(-50 + excess, -50);
  check(value >= previous && value >= 0 && value <= 1, "curve is monotonic and bounded");
  previous = value;
}
check(speechLevel(-50 + GATE_DB + SPAN_DB / 2, -50) > 0.5, "curve is perceptually compressive (mid-span reads above half)");
check(speechLevel(NaN, -50) === 0 && speechLevel(-30, NaN) === 0, "invalid dB never leaks NaN");
// Quiet speech (+12 dB over the floor) already clears 0.55 after the library curve.
check(libraryCurve(speechLevel(-50 + 12, -50)) > 0.55, "quiet speech is clearly visible");
check(libraryCurve(speechLevel(-50 + 20, -50)) > 0.8, "normal speech reaches the top of the range");

function run(tracker: VoiceLevelTracker, ms: number, db: (t: number) => number, onSample?: (t: number, level: number) => void, from = 0) {
  for (let t = 0; t < ms; t += STEP_MS) {
    const level = tracker.update(native(db(from + t)), STEP_MS);
    onSample?.(from + t, level);
  }
}

// Silence: jittering room noise (-48 +/- 2.5 dB) stays calm throughout, floor
// calibrates near the noise and the light never leaves its resting state.
{
  const tracker = new VoiceLevelTracker();
  let peak = 0;
  run(tracker, 6000, (t) => -48 + 2.5 * Math.sin(t / 37) * Math.cos(t / 11), (_t, level) => { peak = Math.max(peak, level); });
  check(peak < CALM_LEVEL, `room noise stays calm (peak ${peak.toFixed(3)})`);
  check(tracker.floorDb > -52 && tracker.floorDb < -45, "floor calibrates to the room noise");
  check(tracker.calm, "silent tracker reports calm");
}

// Native gate: a room quieter than -60 dBFS reads exactly 0 and never moves the light.
{
  const tracker = new VoiceLevelTracker();
  run(tracker, 3000, () => -70);
  check(tracker.level === 0 && tracker.floorDb === FLOOR_MIN_DB, "gated silence clamps the floor at the minimum");
}

// Speech after silence: syllabic bursts (-28 dBFS) with gaps (-44 dBFS).
{
  const tracker = new VoiceLevelTracker();
  const noise = (t: number) => -52 + 1.5 * Math.sin(t / 53);
  run(tracker, 1500, noise);
  const speech = (t: number) => ((t % 260) < 170 ? -28 : -44);
  const levels: number[] = [];
  run(tracker, 3000, speech, (_t, level) => levels.push(level), 1500);
  const sorted = [...levels].sort((a, b) => a - b);
  const median = sorted[Math.floor(sorted.length / 2)];
  check(median >= 0.6 && Math.max(...levels) <= 1, `speech envelope sits in 0.6-1.0 (median ${median.toFixed(2)})`);
  check(Math.min(...levels.slice(5)) > 0.25, "envelope bridges syllable gaps instead of flickering to zero");
  // Release: back to room noise, the light is calm again within ~1.2 s.
  let calmAt = -1;
  run(tracker, 2000, noise, (t, level) => { if (calmAt < 0 && level < CALM_LEVEL) calmAt = t - 4500; }, 4500);
  check(calmAt > 0 && calmAt < 1200, `silence after speech returns to calm quickly (${calmAt} ms)`);
}

// Timing: fast attack, slower release, all elapsed-time based.
{
  const tracker = new VoiceLevelTracker();
  run(tracker, 1000, () => -52);
  const attack = tracker.update(native(-26), 40);
  check(attack > 0.6 * speechLevel(-26, tracker.floorDb), "attack reaches most of the target within ~40 ms");
  for (let i = 0; i < 20; i++) tracker.update(native(-26), STEP_MS);
  const peak = tracker.level;
  tracker.update(native(-70), 200);
  check(tracker.level > 0.2 * peak && tracker.level < 0.5 * peak, "release takes a few hundred milliseconds, not one frame");
  check(tracker.release(0) === tracker.level, "zero elapsed time does not move the envelope");
}

// Speech from the very first sample must not become the floor.
{
  const tracker = new VoiceLevelTracker();
  const levels: number[] = [];
  run(tracker, CALIBRATION_MS, () => -30, (_t, level) => levels.push(level));
  check(tracker.floorDb <= FLOOR_MAX_DB, "floor never exceeds the room-noise ceiling");
  check(levels[0] > 0.3 && levels[levels.length - 1] > 0.3, "immediate speech is visible during calibration");
  run(tracker, 2000, () => -53, undefined, CALIBRATION_MS);
  check(tracker.floorDb < -50, "floor falls quickly once the room is heard to be quieter");
}

// Slow tracking: bursts and ordinary speech never lift the floor; a sustained
// noise increase is absorbed once a whole window has stayed that loud.
{
  const tracker = new VoiceLevelTracker();
  run(tracker, 1500, () => -52);
  const floorBefore = tracker.floorDb;
  run(tracker, 1000, () => -30, undefined, 1500);
  check(tracker.floorDb - floorBefore < 0.5, "a one-second burst does not move the floor");
  let last = 0;
  run(tracker, 40000, () => -42, (_t, level) => { last = level; }, 2500);
  check(tracker.floorDb > -46 && last < CALM_LEVEL, "a sustained noise increase is eventually treated as room noise");
}

// A minute of dictation with a short pause between sentences keeps its response.
{
  const tracker = new VoiceLevelTracker();
  const room = (t: number) => -52 + 1.5 * Math.sin(t / 53);
  run(tracker, 1500, room);
  const dictation = (t: number) => ((t % 4000) > 3500 ? room(t) : ((t % 260) < 170 ? -28 : -44));
  const levels: number[] = [];
  run(tracker, 60000, dictation, (t, level) => { if ((t % 4000) < 3300 && t > 6000) levels.push(level); }, 1500);
  const sorted = [...levels].sort((a, b) => a - b);
  check(tracker.floorDb < -49, `floor stays near the room across a minute of speech (${tracker.floorDb.toFixed(1)} dB)`);
  check(sorted[Math.floor(sorted.length / 2)] >= 0.6, "dictation keeps its amplitude over a minute");
}

// Failed reads release the light and teach the floor nothing.
{
  const tracker = new VoiceLevelTracker();
  run(tracker, 1000, () => -50);
  const floor = tracker.floorDb;
  run(tracker, 1000, () => -26, undefined, 1000);
  const loud = tracker.level;
  for (let i = 0; i < 40; i++) tracker.release(STEP_MS);
  check(tracker.floorDb >= floor - 0.5 && tracker.level < loud * 0.05, "failed reads decay the light without touching the floor");
}

check(Number.isFinite(new VoiceLevelTracker().update(NaN, NaN)), "invalid samples never leak NaN");
console.log("Synthetic voice level checks passed (not native/visual evidence).");
