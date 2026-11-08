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
