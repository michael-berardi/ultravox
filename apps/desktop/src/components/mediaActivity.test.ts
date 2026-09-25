/**
 * Focused, dependency-free checks for the media panel's pure decision logic.
 * Run with: node --experimental-strip-types src/components/mediaActivity.test.ts
 */
function test(_name: string, check: () => void): void {
  check();
}

const assert = {
  equal(actual: unknown, expected: unknown, message?: string): void {
    if (!Object.is(actual, expected)) {
      throw new Error(message ?? `Expected ${String(expected)}, received ${String(actual)}`);
    }
  },
};

import {
  MEDIA_HIDE_DELAY_MS,
  MEDIA_SHOW_DELAY_MS,
  formatMediaClock,
  initialMediaVisibility,
  mediaPlaybackKind,
  mediaProgressRatio,
  mediaTimeSummary,
  nextMediaVisibility,
  normalizeVolumePercent,
} from "./mediaActivity.ts";

test("panel stays hidden until activity is sustained for the show delay", () => {
  let state = initialMediaVisibility(0);
  state = nextMediaVisibility(state, true, 0);
  assert.equal(state.visible, false);
  state = nextMediaVisibility(state, true, MEDIA_SHOW_DELAY_MS - 1);
  assert.equal(state.visible, false);
  state = nextMediaVisibility(state, true, MEDIA_SHOW_DELAY_MS);
  assert.equal(state.visible, true);
});

test("brief activity blips never show the panel", () => {
  let state = initialMediaVisibility(0);
  state = nextMediaVisibility(state, true, 0);
  state = nextMediaVisibility(state, false, 400);
  state = nextMediaVisibility(state, true, 800);
  state = nextMediaVisibility(state, false, 1200);
  assert.equal(state.visible, false);
});

test("panel hides only after the hide delay and survives short gaps", () => {
  let state = initialMediaVisibility(0);
  state = nextMediaVisibility(state, true, 0);
  state = nextMediaVisibility(state, true, MEDIA_SHOW_DELAY_MS);
  assert.equal(state.visible, true);

  // Sub-hide-delay silence keeps the panel up.
  state = nextMediaVisibility(state, false, MEDIA_SHOW_DELAY_MS + 100);
  assert.equal(state.visible, true);
  state = nextMediaVisibility(state, true, MEDIA_SHOW_DELAY_MS + 900);
  assert.equal(state.visible, true);

  // Silence held for just under the hide delay keeps it up...
  const silenceStart = MEDIA_SHOW_DELAY_MS + 900;
  state = nextMediaVisibility(state, false, silenceStart);
  assert.equal(state.visible, true);
  state = nextMediaVisibility(state, false, silenceStart + MEDIA_HIDE_DELAY_MS - 1);
  assert.equal(state.visible, true);

  // ...but reaching the hide delay drops it.
  state = nextMediaVisibility(state, false, silenceStart + MEDIA_HIDE_DELAY_MS);
  assert.equal(state.visible, false);
});
test("visibility timestamps reset on each observed flip", () => {
  let state = initialMediaVisibility(0);
  state = nextMediaVisibility(state, true, 5_000);
  assert.equal(state.changedAt, 5_000);
  state = nextMediaVisibility(state, true, 9_000);
  assert.equal(state.changedAt, 5_000, "same-polarity samples keep the original timestamp");
});

test("volume normalizes the 0..1 CoreAudio scalar to integer percent", () => {
  assert.equal(normalizeVolumePercent(0.42), 42);
  assert.equal(normalizeVolumePercent(1), 100);
  assert.equal(normalizeVolumePercent(0), 0);
  assert.equal(normalizeVolumePercent(0.004), 0);
  assert.equal(normalizeVolumePercent(1.4), 100);
  assert.equal(normalizeVolumePercent(-0.3), 0);
  assert.equal(normalizeVolumePercent(null), 0);
  assert.equal(normalizeVolumePercent(Number.NaN), 0);
});

test("playback kind keeps the unknown state distinct from paused", () => {
  assert.equal(mediaPlaybackKind(true), "playing");
  assert.equal(mediaPlaybackKind(false), "paused");
  assert.equal(mediaPlaybackKind(null), "unknown");
  assert.equal(mediaPlaybackKind(undefined), "unknown");
});

test("media clock formats m:ss and rejects non-times", () => {
  assert.equal(formatMediaClock(0), "0:00");
  assert.equal(formatMediaClock(83), "1:23");
  assert.equal(formatMediaClock(83.9), "1:23");
  assert.equal(formatMediaClock(913), "15:13");
  assert.equal(formatMediaClock(3600), "60:00");
  assert.equal(formatMediaClock(null), null);
  assert.equal(formatMediaClock(Number.NaN), null);
  assert.equal(formatMediaClock(-4), null);
  assert.equal(formatMediaClock(Number.POSITIVE_INFINITY), null);
});

test("progress ratio needs elapsed and a positive duration", () => {
  assert.equal(mediaProgressRatio(0, 200), 0);
  assert.equal(mediaProgressRatio(200, 200), 1);
  assert.equal(mediaProgressRatio(300, 200), 1, "elapsed past duration clamps to full");
  assert.equal(mediaProgressRatio(null, 200), null);
  assert.equal(mediaProgressRatio(50, null), null);
  assert.equal(mediaProgressRatio(50, 0), null);
  assert.equal(mediaProgressRatio(50, -10), null);
  assert.equal(mediaProgressRatio(-1, 200), null);
  const ratio = mediaProgressRatio(83, 222);
  if (ratio == null || Math.abs(ratio - 83 / 222) > 1e-9) {
    throw new Error(`Expected 83/222 ratio, received ${String(ratio)}`);
  }
});

test("time summary speaks only what is known", () => {
  assert.equal(mediaTimeSummary(83, 222), "1:23 of 3:42");
  assert.equal(mediaTimeSummary(null, 222), "Length 3:42");
  assert.equal(mediaTimeSummary(83, null), "1:23 elapsed");
  assert.equal(mediaTimeSummary(null, null), null);
});
