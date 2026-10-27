import type { UpdateInfo, UpdatePreferences } from "../ipc";
import { createUpdater, UPDATE_BUSY_PREFIX, type UpdaterDeps } from "./updater.ts";

function equal(actual: unknown, expected: unknown, label = ""): void {
  if (!Object.is(actual, expected)) throw new Error(`${label} expected ${String(expected)}, received ${String(actual)}`);
}

const next: UpdateInfo = { current_version: "0.12.6", latest_version: "0.12.7", release_url: "https://example.invalid" };

function harness(options: {
  automatic?: boolean;
  candidate?: UpdateInfo | null;
  install?: (info: UpdateInfo) => Promise<void>;
  checkFails?: boolean;
} = {}) {
  const timers: { run: () => void; ms: number }[] = [];
  const installs: UpdateInfo[] = [];
  let preferences: UpdatePreferences = { automatic: options.automatic ?? false };
  const deps: UpdaterDeps = {
    check: async () => {
      if (options.checkFails) throw new Error("offline");
      return options.candidate === undefined ? next : options.candidate;
    },
    install: async (info) => {
      installs.push(info);
      if (options.install) await options.install(info);
    },
    getPreferences: async () => preferences,
    setPreferences: async (value) => { preferences = value; },
    setTimer: (run, ms) => { timers.push({ run, ms }); return timers.length; },
    clearTimer: () => {},
  };
  const updater = createUpdater(deps, { checkEveryMs: 1000, busyRetryMs: 10 });
  return { updater, timers, installs, prefs: () => preferences };
}

const tests: [string, () => Promise<void>][] = [
  ["no newer release keeps the button hidden", async () => {
    const { updater } = harness({ candidate: null });
    await updater.start();
    equal(updater.getState().phase, "hidden");
  }],
  ["a newer release shows the button and waits for a click when automatic is off", async () => {
    const { updater, installs } = harness();
    await updater.start();
    equal(updater.getState().phase, "available");
    equal(installs.length, 0);
  }],
  ["a click shows the spinner state while installing", async () => {
    let release!: () => void;
    const { updater } = harness({ install: () => new Promise<void>((resolve) => { release = resolve; }) });
    await updater.start();
    const pending = updater.install();
    equal(updater.getState().phase, "updating");
    release();
    await pending;
  }],
  ["a failed install names the cause and offers retry", async () => {
    let attempts = 0;
    const { updater } = harness({ install: async () => { attempts += 1; if (attempts === 1) throw "Checksum mismatch for UltraVox-macos-arm64.zip"; } });
    await updater.start();
    await updater.install();
    equal(updater.getState().phase, "failed");
    equal(updater.getState().error, "Checksum mismatch for UltraVox-macos-arm64.zip");
    await updater.install();
    equal(attempts, 2);
    equal(updater.getState().error, null);
  }],
  ["automatic installs right away when idle", async () => {
    const { updater, installs } = harness({ automatic: true });
    await updater.start();
    equal(installs.length, 1);
    equal(updater.getState().phase, "updating");
  }],
  ["automatic never interrupts a recording: busy refusals wait and retry", async () => {
    let busy = true;
    const { updater, timers, installs } = harness({
      automatic: true,
      install: async () => { if (busy) throw `${UPDATE_BUSY_PREFIX} UltraVox is recording.`; },
    });
    await updater.start();
    equal(updater.getState().phase, "available", "after busy refusal");
    equal(updater.getState().error, null);
    const retry = timers.find((timer) => timer.ms === 10);
    if (!retry) throw new Error("no busy retry scheduled");
    busy = false;
    retry.run();
    await new Promise((resolve) => setTimeout(resolve, 0));
    equal(installs.length, 2);
    equal(updater.getState().phase, "updating");
  }],
  ["a manual click while busy reports the reason instead of hiding it", async () => {
    const { updater } = harness({ install: async () => { throw `${UPDATE_BUSY_PREFIX} UltraVox is recording.`; } });
    await updater.start();
    await updater.install();
    equal(updater.getState().phase, "failed");
  }],
  ["turning automatic on persists and installs a waiting update", async () => {
    const { updater, installs, prefs } = harness();
    await updater.start();
    await updater.setAutomatic(true);
    equal(prefs().automatic, true);
    equal(installs.length, 1);
  }],
  ["a failed check is recorded, not silent", async () => {
    const { updater } = harness({ checkFails: true });
    await updater.start();
    equal(updater.getState().phase, "hidden");
    equal(updater.getState().checkError, "Error: offline");
  }],
];

let failed = 0;
for (const [name, run] of tests) {
  try {
    await run();
    console.log(`ok - ${name}`);
  } catch (error) {
    failed += 1;
    console.error(`not ok - ${name}: ${String(error)}`);
  }
}
if (failed) process.exit(1);
