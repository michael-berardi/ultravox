/**
 * Focused, dependency-free checks for the Pro status mapping helper.
 * Run with: node --experimental-strip-types src/lib/proStatus.test.ts
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
  isProLockedError,
  mapProStatus,
  proLockedMessage,
  proStatusSummary,
} from "./proStatus.ts";

test("a missing payload maps to an unavailable, locked status", () => {
  const status = mapProStatus(null);
  assert.equal(status.available, false);
  assert.equal(status.unlocked, false);
  assert.equal(status.state, "unavailable");
  assert.equal(status.trialUntil, null);
  assert.equal(status.plan, null);
});

test("an unlocked trial keeps its contract fields", () => {
  const status = mapProStatus({
    available: true,
    unlocked: true,
    state: "trial",
    trialUntil: "2026-09-17T00:00:00Z",
    updatesUntil: null,
    plan: "pro",
    label: "Trial",
    error: null,
  });
  assert.equal(status.available, true);
  assert.equal(status.unlocked, true);
  assert.equal(status.state, "trial");
  assert.equal(status.trialUntil, "2026-09-17T00:00:00Z");
  assert.equal(status.plan, "pro");
});

test("unlock never survives an unavailable build or an unknown state", () => {
  assert.equal(mapProStatus({ available: false, unlocked: true, state: "paid" }).unlocked, false);
  assert.equal(mapProStatus({ available: false, unlocked: true, state: "paid" }).state, "unavailable");
  assert.equal(mapProStatus({ available: true, unlocked: true, state: "sideways" }).state, "none");
  assert.equal(mapProStatus("garbage").available, false);
});

test("pro-locked command errors are recognised and never quoted raw", () => {
  assert.equal(isProLockedError("pro-locked: meeting needs a license"), true);
  assert.equal(isProLockedError(new Error("pro-locked: studio")), true);
  assert.equal(isProLockedError("disk full"), false);
  assert.equal(isProLockedError(undefined), false);
  assert.equal(proLockedMessage("Meeting mode").includes("pro-locked:"), false);
});

test("state summary reads as plain product copy", () => {
  assert.equal(proStatusSummary(mapProStatus({ available: true, state: "paid" })), "Licensed on this device");
  assert.equal(proStatusSummary(mapProStatus({ available: true, state: "none" })), "Not activated on this device");
  assert.equal(proStatusSummary(mapProStatus({ available: true, state: "expired" })), "License expired");
  assert.equal(proStatusSummary(mapProStatus(null)), "Pro is not available in this build");
  const trial = mapProStatus({ available: true, state: "trial", trialUntil: "2026-09-17T00:00:00Z" });
  assert.equal(proStatusSummary(trial).startsWith("Free trial until "), true);
});

test("status errors drop the IPC prefix and stay quiet when Pro is simply not activated", () => {
  const none = mapProStatus({ available: true, state: "none", error: "pro-locked: this device has no UltraVox Pro entitlement." });
  assert.equal(none.error, null);
  const expired = mapProStatus({ available: true, state: "expired", error: "pro-locked: the Pro entitlement has expired." });
  assert.equal(expired.error, "The Pro entitlement has expired.");
});
