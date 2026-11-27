import { test } from 'node:test';
import { strict as assert } from 'node:assert';
import { readdirSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const desktop = join(dirname(fileURLToPath(import.meta.url)), '..');
const forbidden = ['/api/app-' + 'telemetry', 'lds.app-' + 'telemetry'];

test('desktop source contains no collection endpoint or schema', () => {
  function scan(dir: string) {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) scan(path);
      else if (entry.isFile() && /\.(rs|ts|tsx|js|jsx)$/.test(entry.name)) {
        const text = readFileSync(path, 'utf8');
        for (const pattern of forbidden) assert.ok(!text.includes(pattern), `${path} contains ${pattern}`);
      }
    }
  }
  scan(join(desktop, 'src'));
  scan(join(desktop, 'src-tauri', 'src'));
});

test('no telemetry packages in the shipped manifests', () => {
  const npm = JSON.parse(readFileSync(join(desktop, 'package.json'), 'utf8')) as {
    dependencies?: Record<string, string>;
    devDependencies?: Record<string, string>;
  };
  const names = new Set([...Object.keys(npm.dependencies ?? {}), ...Object.keys(npm.devDependencies ?? {})]);
  const forbiddenPackages = [
    '@sentry/browser', '@sentry/react', '@sentry/tauri', 'posthog-js',
    'mixpanel-browser', 'analytics.js', '@segment/analytics-next', '@amplitude/analytics-browser',
  ];
  for (const name of forbiddenPackages) assert.ok(!names.has(name), `${name} was added`);

  const cargo = readFileSync(join(desktop, 'src-tauri', 'Cargo.toml'), 'utf8');
  const forbiddenCrates = ['sentry', 'posthog', 'segment-analytics', 'datadog', 'opentelemetry'];
  for (const name of forbiddenCrates) {
    assert.ok(!new RegExp(`^\\s*${name}\\s*=`, 'm').test(cargo), `${name} crate was added`);
  }
});

test('startup never downloads a model on its own', () => {
  const setup = readFileSync(join(desktop, 'src-tauri', 'src', 'lib.rs'), 'utf8');
  const startup = setup.slice(setup.indexOf('.setup(|app|'));
  assert.ok(startup.length > 0, 'Tauri setup block not found');
  assert.ok(!/warm_transcription_model\(\s*\)/.test(startup), 'warm-up must state whether it may download');
  assert.ok(!/warm_transcription_model\(\s*true\s*\)/.test(startup), 'startup must not auto-download models');
  const state = readFileSync(join(desktop, 'src-tauri', 'src', 'state.rs'), 'utf8');
  assert.ok(/if allow_download && english && !ultravox_core::phonon::is_installed/.test(state),
    'Phonon-2 install must be gated on allow_download');
});

test('every update request, including the Pro service channel, goes through the gate', () => {
  const update = readFileSync(join(desktop, 'src-tauri', 'src', 'update.rs'), 'utf8');
  const commands = readFileSync(join(desktop, 'src-tauri', 'src', 'commands.rs'), 'utf8');
  // The command layer only reaches the network through gated_check.
  assert.ok(commands.includes('update::gated_check('), 'check_for_update must use gated_check');
  assert.ok(!/update::check\(/.test(commands), 'commands must not call the ungated check');
  // The ungated public+service check is private to update.rs and only called from gated_check.
  assert.ok(/\nasync fn check\(current_version/.test(update), 'ungated check must stay private');
  const callers = [...update.matchAll(/[^_\w]check\(current_version\)/g)];
  assert.equal(callers.length, 1, 'only gated_check may call the public+service check');
  const serviceCallers = [...update.matchAll(/service_check\(current_version\)/g)];
  assert.equal(serviceCallers.length, 1, 'service_check is reached only through check()');
  // Automatic checks are anonymous: fixed agent, short timeout, public feed only.
  assert.ok(update.includes('const CHECK_USER_AGENT: &str = "ultravox-update-check";'));
  assert.ok(update.includes('const CHECK_TIMEOUT: Duration = Duration::from_secs(4);'));
  assert.ok(update.includes('const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;'));
  assert.ok(/fn may_use_licensed_channel\(trigger: CheckTrigger\) -> bool \{\s*trigger == CheckTrigger::Manual\s*\}/.test(update),
    'only Check Now may use the authenticated channel');
  assert.ok(/if may_use_licensed_channel\(trigger\) \{\s*check\(current_version\)\.await\s*\} else \{\s*public_check\(current_version\)\.await/.test(update),
    'automatic checks must never use the authenticated channel');
  assert.ok(update.includes('pub const NO_UPDATE_CHECK_ENV: &str = "ULTRAVOX_NO_UPDATE_CHECK";'));
});

test('shipped UI contains no third-party CDN fonts or scripts', () => {
  const index = readFileSync(join(desktop, 'index.html'), 'utf8');
  const css = readFileSync(join(desktop, 'src', 'index.css'), 'utf8');
  const forbiddenOrigins = ['fonts.googleapis.com', 'cdn.jsdelivr.net', 'unpkg.com', 'cdnjs.cloudflare.com'];
  for (const origin of forbiddenOrigins) {
    assert.ok(!index.includes(origin), `${origin} in index.html`);
    assert.ok(!css.includes(origin), `${origin} in index.css`);
  }
});
