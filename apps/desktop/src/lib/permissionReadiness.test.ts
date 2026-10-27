import type { PermissionStatus } from "../ipc";
import {
  canRequestPermission,
  createPermissionReadinessController,
  dictationAccess,
  isDictationReady,
  microphonePromptNote,
  nextMicrophonePromptState,
  permissionActionFailure,
  permissionStateLabel,
  permissionValidationError,
} from "./permissionReadiness.ts";

function equal(actual: unknown, expected: unknown): void {
  if (!Object.is(actual, expected)) throw new Error(`Expected ${String(expected)}, received ${String(actual)}`);
}
function ok(value: boolean): void {
  if (!value) throw new Error("Assertion failed");
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (cause: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0));
const granted: PermissionStatus = { microphone: "granted", accessibility: "denied", screen_recording: "denied" };
const denied: PermissionStatus = { ...granted, microphone: "denied" };

function harness(readStatus: () => Promise<PermissionStatus>, debounceMs = 5) {
  const statuses: PermissionStatus[] = [];
  const errors: Array<string | null> = [];
  const busy: boolean[] = [];
  const controller = createPermissionReadinessController({
    readStatus,
    onStatus: (status) => statuses.push(status),
    onError: (error) => errors.push(error),
    onBusy: (value) => busy.push(value),
    debounceMs,
  });
  return { controller, statuses, errors, busy };
}
const cases: Array<[string, () => void | Promise<void>]> = [];
function test(name: string, run: () => void | Promise<void>) { cases.push([name, run]); }

test("dictation requires only a microphone grant, or no gate on this platform", () => {
  for (const accessibility of ["denied", "not_determined", "unavailable", "granted"] as const) {
    for (const screen_recording of ["denied", "not_determined", "unavailable", "granted"] as const) {
      equal(isDictationReady({ microphone: "granted", accessibility, screen_recording }), true);
      // Windows and Linux always report the microphone as unavailable (no OS gate).
      equal(isDictationReady({ microphone: "unavailable", accessibility, screen_recording }), true);
    }
  }
  equal(isDictationReady(null), false);
  for (const microphone of ["denied", "not_determined"] as const) {
    equal(isDictationReady({ ...granted, microphone }), false);
  }
});

test("synthetic Windows/Linux status (everything unavailable) is ready, not blocked", () => {
  const status: PermissionStatus = { microphone: "unavailable", accessibility: "unavailable", screen_recording: "unavailable" };
  equal(dictationAccess(status, null), "ready");
});

test("no status and no error is loading; the panel waits for the first status or error", () => {
  equal(dictationAccess(null, null), "loading");
  equal(dictationAccess(null, "Couldn't check permissions: service unavailable"), "blocked");
  equal(dictationAccess(granted, null), "ready");
  equal(dictationAccess(denied, null), "blocked");
  equal(dictationAccess({ ...granted, microphone: "not_determined" }, null), "blocked");
  // A later read failure keeps the last known status instead of blocking.
  equal(dictationAccess(granted, "Couldn't check permissions: service unavailable"), "ready");
});

test("microphone prompt state survives rechecks until macOS gives an answer", () => {
  const undetermined: PermissionStatus = { ...granted, microphone: "not_determined" };
  equal(nextMicrophonePromptState(null, undetermined), null);
  equal(nextMicrophonePromptState(null, { ...undetermined, microphone_request: "pending" }), "pending");
  equal(nextMicrophonePromptState(null, { ...undetermined, microphone_request: "timed_out" }), "timed_out");
  equal(nextMicrophonePromptState("pending", undetermined), "pending");
  equal(nextMicrophonePromptState("timed_out", undetermined), "timed_out");
  equal(nextMicrophonePromptState("timed_out", { ...undetermined, microphone_request: "pending" }), "timed_out");
  equal(nextMicrophonePromptState("pending", { ...undetermined, microphone_request: "timed_out" }), "timed_out");
  for (const microphone of ["granted", "denied", "unavailable"] as const) {
    equal(nextMicrophonePromptState("pending", { ...undetermined, microphone }), null);
    equal(nextMicrophonePromptState("timed_out", { ...undetermined, microphone, microphone_request: "denied" }), null);
  }
});

test("allow is offered once for the microphone, then only macOS Settings with a relaunch note", () => {
  equal(canRequestPermission("microphone", "not_determined", null), true);
  equal(canRequestPermission("microphone", "not_determined", "pending"), false);
  equal(canRequestPermission("microphone", "not_determined", "timed_out"), false);
  // macOS never prompts again after a denial.
  equal(canRequestPermission("microphone", "denied", null), false);
  equal(canRequestPermission("microphone", "granted", null), false);
  equal(canRequestPermission("accessibility", "denied", null), true);
  equal(canRequestPermission("screen_recording", "denied", null), true);
  equal(canRequestPermission("accessibility", "granted", null), false);
  equal(microphonePromptNote(null), null);
  for (const prompt of ["pending", "timed_out"] as const) {
    ok(microphonePromptNote(prompt)?.includes("relaunch may be needed") === true);
  }
  ok(microphonePromptNote("timed_out") !== microphonePromptNote("pending"));
});

test("permission states read as plain language", () => {
  equal(permissionStateLabel("granted"), "Granted");
  equal(permissionStateLabel("denied"), "Denied");
  equal(permissionStateLabel("not_determined"), "Not allowed yet");
  equal(permissionStateLabel("unavailable"), "Not available on this system");
  equal(permissionStateLabel(undefined), "Checking…");
  equal(permissionStateLabel(undefined, true), "Couldn't check");
});

test("live grant and revocation replace outdated denial and clear errors", async () => {
  let status = denied;
  let fail = false;
  const h = harness(async () => {
    if (fail) throw new Error("status service unavailable");
    return status;
  });
  await h.controller.refresh();
  equal(isDictationReady(h.statuses[0]), false);
  fail = true;
  await h.controller.refresh();
  equal(h.statuses.length, 1);
  ok(h.errors[h.errors.length - 1]?.includes("status service unavailable") === true);
  status = granted;
  fail = false;
  await h.controller.refresh();
  equal(h.statuses[h.statuses.length - 1], granted);
  equal(h.errors[h.errors.length - 1], null);
  equal(isDictationReady(h.statuses[h.statuses.length - 1]), true);
  status = denied;
  await h.controller.refresh();
  equal(isDictationReady(h.statuses[h.statuses.length - 1]), false);
  h.controller.dispose();
});

test("reads are serialized with one coalesced trailing refresh", async () => {
  const first = deferred<PermissionStatus>();
  let calls = 0;
  let concurrent = 0;
  let maxConcurrent = 0;
  const h = harness(async () => {
    calls += 1;
    concurrent += 1;
    maxConcurrent = Math.max(maxConcurrent, concurrent);
    const status = calls === 1 ? await first.promise : granted;
    concurrent -= 1;
    return status;
  });
  const initial = h.controller.refresh();
  equal(h.controller.refresh(), initial); // Coalesce before the read starts.
  await tick();
  const trailing = h.controller.refresh();
  equal(h.controller.refresh(), trailing);
  equal(calls, 1);
  first.resolve(denied);
  await Promise.all([initial, trailing]);
  equal(calls, 2);
  equal(maxConcurrent, 1);
  equal(h.statuses[0], denied);
  equal(h.statuses[1], granted);
  equal(h.busy[h.busy.length - 1], false);
  h.controller.dispose();
});

test("explicit requests serialize behind reads and are never repeated by refresh", async () => {
  const first = deferred<PermissionStatus>();
  let reads = 0;
  let requests = 0;
  const h = harness(async () => ++reads === 1 ? await first.promise : granted);
  const initial = h.controller.refresh();
  await tick();
  const action = h.controller.runAction("Couldn't request microphone access", async () => {
    requests += 1;
    return granted;
  });
  const trailing = h.controller.refresh();
  equal(requests, 0);
  first.resolve(denied);
  await Promise.all([initial, action, trailing]);
  equal(requests, 1);
  equal(reads, 2);
  equal(h.statuses[0], denied);
  equal(h.statuses[1], granted);
  equal(h.statuses[2], granted);
  h.controller.dispose();
});

test("request and settings failures are errors, not manufactured denial or grant", async () => {
  const h = harness(async () => granted);
  await h.controller.refresh();
  for (const failure of [
    permissionActionFailure("microphone", "request"),
    permissionActionFailure("accessibility", "request"),
    permissionActionFailure("screen_recording", "settings"),
  ]) {
    await h.controller.runAction(failure, async () => { throw "native command unavailable"; });
    equal(h.statuses.length, 1);
    equal(h.errors[h.errors.length - 1], `${failure}: native command unavailable`);
    ok(h.errors[h.errors.length - 1]?.includes("Recheck permissions") === false);
    equal(h.busy[h.busy.length - 1], false);
  }
  await h.controller.refresh();
  equal(h.errors[h.errors.length - 1], null);
  equal(h.statuses.length, 2);
  h.controller.dispose();
  ok(permissionValidationError("Couldn't check permissions", {}).includes("Unknown error"));
});

test("action errors read naturally and never end by repeating the Recheck button", () => {
  equal(permissionActionFailure("microphone", "request"), "Couldn't request microphone access");
  equal(permissionActionFailure("accessibility", "request"), "Couldn't request Accessibility access");
  equal(permissionActionFailure("screen_recording", "settings"), "Couldn't open Screen Recording settings");
  equal(permissionValidationError("Couldn't check permissions", new Error("service down")), "Couldn't check permissions: service down");
  for (const kind of ["microphone", "accessibility", "screen_recording"] as const) {
    for (const action of ["request", "settings"] as const) {
      const message = permissionActionFailure(kind, action);
      ok(!message.includes("optional") && !message.includes("failed") && !message.includes("Recheck"));
    }
  }
});

test("focus/visible event bursts debounce to a read-only check, not polling", async () => {
  let reads = 0;
  const h = harness(async () => { reads += 1; return granted; });
  h.controller.scheduleRefresh();
  h.controller.scheduleRefresh();
  h.controller.scheduleRefresh();
  equal(reads, 0);
  await new Promise<void>((resolve) => setTimeout(resolve, 20));
  equal(reads, 1);
  await new Promise<void>((resolve) => setTimeout(resolve, 20));
  equal(reads, 1);
  h.controller.scheduleRefresh();
  await h.controller.refresh(); // Manual recheck cancels scheduled duplicate.
  await new Promise<void>((resolve) => setTimeout(resolve, 20));
  equal(reads, 2);
  h.controller.dispose();
});

test("unmount suppresses late success/rejection and cancels queued operations", async () => {
  for (const reject of [false, true]) {
    const first = deferred<PermissionStatus>();
    let reads = 0;
    let requests = 0;
    const h = harness(async () => { reads += 1; return first.promise; });
    const initial = h.controller.refresh();
    await tick();
    const trailing = h.controller.refresh();
    const action = h.controller.runAction("Couldn't request microphone access", async () => { requests += 1; return granted; });
    const beforeBusy = h.busy.length;
    h.controller.dispose();
    if (reject) first.reject(new Error("late error")); else first.resolve(granted);
    await Promise.all([initial, trailing, action]);
    equal(h.statuses.length, 0);
    equal(h.errors.length, 0);
    equal(h.busy.length, beforeBusy);
    equal(reads, 1);
    equal(requests, 0);
    await h.controller.refresh();
    equal(reads, 1);
  }
});

test("unmount cancels debounced refresh; old instance cannot update new mount", async () => {
  let reads = 0;
  const scheduled = harness(async () => { reads += 1; return denied; });
  scheduled.controller.scheduleRefresh();
  scheduled.controller.dispose();
  await new Promise<void>((resolve) => setTimeout(resolve, 20));
  equal(reads, 0);
  const late = deferred<PermissionStatus>();
  const old = harness(() => late.promise);
  const pending = old.controller.refresh();
  await tick();
  old.controller.dispose();
  const current = harness(async () => granted);
  await current.controller.refresh();
  late.resolve(denied);
  await pending;
  equal(old.statuses.length, 0);
  equal(current.statuses[0], granted);
  current.controller.dispose();
});

for (const [name, run] of cases) {
  await run();
  console.log(`PASS ${name}`);
}
