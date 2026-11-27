import type { UpdateCheckTrigger, UpdateInfo, UpdateNotice, UpdatePreferences } from "../ipc";
import { CHECK_EVERY_MS, createUpdater, UPDATE_BUSY_PREFIX, type UpdaterDeps } from "./updater.ts";

function equal(actual: unknown, expected: unknown, label = ""): void {
  if (!Object.is(actual, expected)) throw new Error(`${label} expected ${String(expected)}, received ${String(actual)}`);
}

const next: UpdateInfo = { current_version: "0.12.6", latest_version: "0.12.7", release_url: "https://example.invalid" };

function harness(options: {
  automatic?: boolean;
  checkAutomatically?: boolean;
  candidate?: UpdateInfo | null;
  install?: (info: UpdateInfo) => Promise<void>;
  checkFails?: boolean;
  blockedReason?: string;
  /** Native notice gate; default: show once per version per day like update.rs. */
  claimNotice?: (version: string) => Promise<UpdateNotice>;
} = {}) {
  const timers: { run: () => void; ms: number }[] = [];
  const installs: UpdateInfo[] = [];
  const checks: UpdateCheckTrigger[] = [];
  const claims: string[] = [];
  const skips: string[] = [];
  const shown = new Set<string>();
  let candidate = options.candidate === undefined ? next : options.candidate;
  let preferences: UpdatePreferences = {
    automatic: options.automatic ?? false,
    check_automatically: options.checkAutomatically ?? true,
  };
  const deps: UpdaterDeps = {
    check: async (trigger) => {
      checks.push(trigger);
      if (options.checkFails) throw new Error("offline");
      return candidate;
    },
    install: async (info) => {
      installs.push(info);
      if (options.install) await options.install(info);
    },
    getPreferences: async () => preferences,
    setPreferences: async (value) => { preferences = value; },
    getStatus: options.blockedReason === undefined ? undefined : async () => ({
      blocked_reason: options.blockedReason ?? null, last_checked_at: null, last_error: null,
    }),
    claimNotice: async (version) => {
      claims.push(version);
      if (options.claimNotice) return await options.claimNotice(version);
      if (skips.includes(version)) return "skipped";
      if (shown.has(version)) return "recent";
      shown.add(version);
      return "show";
    },
    skipVersion: async (version) => { skips.push(version); },
    setTimer: (run, ms) => { timers.push({ run, ms }); return timers.length; },
    clearTimer: () => {},
  };
  const updater = createUpdater(deps, { checkEveryMs: 1000, busyRetryMs: 10 });
  return {
    updater, timers, installs, checks, claims, skips, prefs: () => preferences,
    offer: (info: UpdateInfo | null) => { candidate = info; },
    forgetShown: () => shown.clear(),
  };
}

const tests: [string, () => Promise<void>][] = [
  ["defaults: checks on, automatic install off", async () => {
    const fresh = createUpdater({
      check: async () => null, install: async () => {},
      getPreferences: async () => ({ automatic: false, check_automatically: true }),
      setPreferences: async () => {}, setTimer: () => 1, clearTimer: () => {},
    });
    equal(fresh.getState().automatic, false);
    equal(fresh.getState().checkAutomatically, true);
  }],
  ["an unreadable preference file makes no update request", async () => {
    const checks: unknown[] = [];
    const broken = createUpdater({
      check: async () => { checks.push(null as never); return null; },
      install: async () => {},
      getPreferences: async () => { throw new Error("parse error"); },
      setPreferences: async () => {},
      setTimer: () => 1,
      clearTimer: () => {},
    }, { checkEveryMs: 1000, busyRetryMs: 10 });
    await broken.start();
    equal(checks.length, 0);
    equal(broken.getState().checkAutomatically, false);
    equal(broken.getState().automatic, false);
  }],
  ["launch and periodic checks are automatic; the native gate limits them to daily", async () => {
    const { updater, timers, checks } = harness({ candidate: null });
    await updater.start();
    equal(checks.join(), "automatic");
    equal(timers[0]?.ms, 1000);
    equal(CHECK_EVERY_MS >= 60 * 60 * 1000, true, "tick is at most hourly");
    timers[0].run();
    await new Promise((resolve) => setTimeout(resolve, 0));
    equal(checks.join(), "automatic,automatic");
  }],
  ["opting out of checks stops launch and periodic network calls", async () => {
    const { updater, timers, checks } = harness({ checkAutomatically: false, automatic: true });
    await updater.start();
    equal(checks.length, 0, "no launch check");
    for (const timer of timers) timer.run();
    await Promise.resolve();
    equal(checks.length, 0, "no periodic check");
  }],
  ["an environment block skips the native checker entirely, including Check Now", async () => {
    const { updater, timers, checks } = harness({
      blockedReason: "Update checks are off in development mirror builds.",
    });
    await updater.start();
    equal(checks.length, 0, "no launch check");
    for (const timer of timers) timer.run();
    await Promise.resolve();
    equal(checks.length, 0, "no periodic check");
    await updater.checkNow();
    equal(checks.length, 0, "no manual check");
    equal(updater.getState().checkError, null, "no spurious IPC failure");
    equal(updater.getState().blockedReason, "Update checks are off in development mirror builds.");
  }],
  ["Check Now works while automatic checks are off and reports up to date", async () => {
    const { updater, checks } = harness({ checkAutomatically: false, candidate: null });
    await updater.start();
    const pending = updater.checkNow();
    equal(updater.getState().checking, true);
    await pending;
    equal(checks.join(), "manual");
    equal(updater.getState().checking, false);
    equal(updater.getState().upToDate, true);
  }],
  ["a failed Check Now names the cause", async () => {
    const { updater } = harness({ checkAutomatically: false, checkFails: true });
    await updater.start();
    await updater.checkNow();
    equal(updater.getState().checkError, "Error: offline");
    equal(updater.getState().upToDate, false);
  }],
  ["turning checks back on performs a check and preserves the install preference", async () => {
    const { updater, checks, prefs } = harness({ checkAutomatically: false, automatic: false });
    await updater.start();
    equal(checks.length, 0);
    await updater.setCheckAutomatically(true);
    equal(checks.length, 1);
    equal(prefs().automatic, false);
    equal(prefs().check_automatically, true);
  }],
  ["turning automatic install on keeps the check preference", async () => {
    const { updater, prefs } = harness({ checkAutomatically: false, candidate: null });
    await updater.start();
    await updater.setAutomatic(true);
    equal(prefs().automatic, true);
    equal(prefs().check_automatically, false);
  }],
  ["the native status supplies the last check time, blocked reason and failure cause", async () => {
    const updater = createUpdater({
      check: async () => null, install: async () => {},
      getPreferences: async () => ({ automatic: false, check_automatically: true }),
      setPreferences: async () => {},
      getStatus: async () => ({ blocked_reason: "Update checks are off in CI.", last_checked_at: 1764100000, last_error: "timed out" }),
      setTimer: () => 1, clearTimer: () => {},
    });
    await updater.start();
    equal(updater.getState().blockedReason, "Update checks are off in CI.");
    equal(updater.getState().lastCheckedAt, 1764100000);
    equal(updater.getState().checkError, "timed out");
  }],
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
  ["an automatic check that finds a release shows the notice once and records it natively", async () => {
    const { updater, claims } = harness();
    await updater.start();
    equal(updater.getState().notice, true);
    equal(claims.join(), "0.12.7");
    await updater.check("automatic");
    equal(updater.getState().notice, true, "an hourly tick keeps the notice that is showing");
  }],
  ["Remind Me Later hides the notice until the next day; the Update button stays", async () => {
    const { updater, forgetShown } = harness();
    await updater.start();
    updater.later();
    equal(updater.getState().notice, false);
    equal(updater.getState().phase, "available", "header Update button still offered");
    await updater.check("automatic");
    equal(updater.getState().notice, false, "shown within 24 h: quiet");
    forgetShown(); // the native gate's day has passed
    await updater.check("automatic");
    equal(updater.getState().notice, true);
  }],
  ["Skip This Version hides that version everywhere until a newer release", async () => {
    const { updater, skips, offer } = harness();
    await updater.start();
    await updater.skip();
    equal(skips.join(), "0.12.7");
    equal(updater.getState().notice, false);
    equal(updater.getState().phase, "hidden", "no Update button for a skipped version");
    equal(updater.getState().skipped, true);
    await updater.check("automatic");
    equal(updater.getState().phase, "hidden", "an automatic check honours the skip");
    offer({ ...next, latest_version: "0.12.8" });
    await updater.check("automatic");
    equal(updater.getState().phase, "available", "a newer release is offered again");
    equal(updater.getState().notice, true);
    equal(updater.getState().skipped, false);
  }],
  ["Check Now offers a skipped version, without the notice", async () => {
    const { updater } = harness();
    await updater.start();
    await updater.skip();
    await updater.checkNow();
    equal(updater.getState().phase, "available");
    equal(updater.getState().notice, false);
  }],
  ["automatic install never installs a skipped version", async () => {
    const { updater, installs } = harness({ automatic: true, claimNotice: async () => "skipped" });
    await updater.start();
    equal(installs.length, 0);
    equal(updater.getState().phase, "hidden");
  }],
  ["with automatic install on, a found release installs instead of showing the notice", async () => {
    const { updater, installs } = harness({ automatic: true });
    await updater.start();
    equal(updater.getState().notice, false);
    equal(installs.length, 1);
  }],
  ["the notice's checkbox turns automatic install on for the future without installing now", async () => {
    const { updater, installs, prefs } = harness();
    await updater.start();
    await updater.setAutomatic(true, false);
    equal(prefs().automatic, true);
    equal(installs.length, 0);
    equal(updater.getState().phase, "available");
  }],
  ["a notice that cannot be recorded still shows, and Later keeps it hidden this session", async () => {
    const { updater } = harness({ claimNotice: async () => { throw new Error("read-only"); } });
    await updater.start();
    equal(updater.getState().notice, true);
    updater.later();
    await updater.check("automatic");
    equal(updater.getState().notice, false);
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
