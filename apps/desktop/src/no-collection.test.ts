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

test('shipped UI contains no third-party CDN fonts or scripts', () => {
  const index = readFileSync(join(desktop, 'index.html'), 'utf8');
  const css = readFileSync(join(desktop, 'src', 'index.css'), 'utf8');
  const forbiddenOrigins = ['fonts.googleapis.com', 'cdn.jsdelivr.net', 'unpkg.com', 'cdnjs.cloudflare.com'];
  for (const origin of forbiddenOrigins) {
    assert.ok(!index.includes(origin), `${origin} in index.html`);
    assert.ok(!css.includes(origin), `${origin} in index.css`);
  }
});
