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
  initialMediaVisibility,
  mediaSubtitle,
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

test("subtitle joins title and artist and tolerates blanks", () => {
  assert.equal(mediaSubtitle("Song", "Artist"), "Song — Artist");
  assert.equal(mediaSubtitle("Song", null), "Song");
  assert.equal(mediaSubtitle(undefined, "Artist"), "Artist");
  assert.equal(mediaSubtitle("  ", ""), null);
});
