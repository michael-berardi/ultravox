import { normalizeVoiceLevel, settleVoiceLevel } from "./voiceGlowLifecycle.ts";

// The native meter (crates/ultravox-core audio.rs compute_audio_level) is the
// RMS of the latest capture buffer in dBFS, mapped linearly from
// [-60 dB, -3 dB] to [0, 1] and hard-gated to 0 at or below -60 dB. It is a
// snapshot, not an envelope, so decibels are recovered here and everything
// perceptual (floor, gate, curve, timing) lives in this one module.
export const NATIVE_LEVEL_FLOOR_DB = -60;
export const NATIVE_LEVEL_CEILING_DB = -3;

// The first CALIBRATION_MS of listening estimate the room's noise floor (its
// lower quartile, so speech that starts at once does not become the floor).
export const CALIBRATION_MS = 500;
export const CALIBRATION_QUANTILE = 0.25;
// The floor is bounded: a mic never measures quieter than FLOOR_MIN_DB once
// the native gate flickers, and nothing louder than FLOOR_MAX_DB is room noise.
export const FLOOR_MIN_DB = -54;
export const FLOOR_MAX_DB = -44;
// After calibration the floor follows the quietest audio of the last
// FLOOR_WINDOW_MS (minimum statistics): it drops quickly to anything quieter,
// and only rises, slowly, when nothing in the whole window was that quiet. Speech
// therefore never lifts it while the speaker pauses now and then.
export const FLOOR_WINDOW_MS = 10000;
export const FLOOR_BUCKET_MS = 1000;
export const FLOOR_FALL_MS = 300;
export const FLOOR_RISE_MS = 2500;

// Level above floor + GATE_DB is speech; it reaches 1 at GATE_DB + SPAN_DB
// above the floor, then bends through a perceptual (compressive) power curve.
export const GATE_DB = 5;
export const SPAN_DB = 24;
export const CURVE_EXPONENT = 0.8;

// Envelope timing is settleVoiceLevel's VOICE_ATTACK_MS / VOICE_RELEASE_MS. The
// beam library adds its own short smoothing (attack 0.02 s, release 0.06 s).

// Below this the light is in its calm resting state.
export const CALM_LEVEL = 0.06;

export function nativeLevelToDb(level: number): number {
  return NATIVE_LEVEL_FLOOR_DB + normalizeVoiceLevel(level) * (NATIVE_LEVEL_CEILING_DB - NATIVE_LEVEL_FLOOR_DB);
}

export function clampFloorDb(db: number): number {
  return Number.isFinite(db) ? Math.max(FLOOR_MIN_DB, Math.min(FLOOR_MAX_DB, db)) : FLOOR_MIN_DB;
}

// Decibels above the floor -> [0, 1] speech amplitude.
export function speechLevel(db: number, floorDb: number): number {
  if (!Number.isFinite(db) || !Number.isFinite(floorDb)) return 0;
  const linear = Math.max(0, Math.min(1, (db - floorDb - GATE_DB) / SPAN_DB));
  return Math.pow(linear, CURVE_EXPONENT);
}

function quantile(values: readonly number[], q: number): number {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))];
}

export class VoiceLevelTracker {
  private clockMs = 0;
  private calibration: number[] = [];
  private floor = FLOOR_MIN_DB;
  private smoothed = 0;
  // Minimum dB per FLOOR_BUCKET_MS bucket, oldest first, for the rolling window.
  private buckets: { startMs: number; minDb: number }[] = [];

  get floorDb(): number { return this.floor; }
  get calibrating(): boolean { return this.clockMs < CALIBRATION_MS; }
  get level(): number { return this.smoothed; }
  get calm(): boolean { return this.smoothed < CALM_LEVEL; }

  // One native sample. Returns the smoothed envelope in [0, 1].
  update(nativeLevel: number, elapsedMs: number): number {
    const elapsed = Number.isFinite(elapsedMs) ? Math.max(0, elapsedMs) : 0;
    const db = nativeLevelToDb(nativeLevel);
    const calibrating = this.calibrating;
    this.clockMs += elapsed;
    this.remember(db);
    if (calibrating) {
      this.calibration.push(db);
      this.floor = clampFloorDb(quantile(this.calibration, CALIBRATION_QUANTILE));
    } else {
      this.trackFloor(elapsed);
    }
    this.smoothed = settleVoiceLevel(this.smoothed, speechLevel(db, this.floor), elapsed);
    return this.smoothed;
  }

  // No usable sample (stalled or failed read): the light releases, and the
  // floor learns nothing from a silence it did not hear.
  release(elapsedMs: number): number {
    this.smoothed = settleVoiceLevel(this.smoothed, 0, elapsedMs);
    return this.smoothed;
  }

  private remember(db: number): void {
    const last = this.buckets[this.buckets.length - 1];
    if (last && this.clockMs - last.startMs < FLOOR_BUCKET_MS) last.minDb = Math.min(last.minDb, db);
    else this.buckets.push({ startMs: this.clockMs, minDb: db });
    while (this.buckets.length > 1 && this.clockMs - this.buckets[0].startMs >= FLOOR_WINDOW_MS) this.buckets.shift();
  }

  private trackFloor(elapsedMs: number): void {
    const windowMin = Math.min(...this.buckets.map((bucket) => bucket.minDb));
    const gap = windowMin - this.floor;
    // Rising needs a full window of evidence; falling needs only one sample.
    if (gap > 0 && this.clockMs < FLOOR_WINDOW_MS) return;
    const timeConstant = gap < 0 ? FLOOR_FALL_MS : FLOOR_RISE_MS;
    this.floor = clampFloorDb(this.floor + gap * (1 - Math.exp(-elapsedMs / timeConstant)));
  }
}
