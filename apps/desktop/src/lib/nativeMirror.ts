type Seed = { storage: Record<string, string> };
/* eslint-disable @typescript-eslint/no-explicit-any -- IPC seam: preserves Tauri's generic invoke signature */
type InvokeFn = <T>(cmd: string, args?: any, options?: any) => Promise<T>;
declare global { interface Window { __ULTRAVOX_MIRROR_SEED__?: Seed; __ULTRAVOX_MIRROR_SNAPSHOT__?: () => Seed; __ULTRAVOX_MIRROR_PRO__?: (state: MirrorProFixture | null) => void; } }

/** Mirror-only Pro status fixtures for visual QA of locked/free states. */
export type MirrorProFixture = 'unlocked' | 'trial' | 'locked' | 'expired' | 'unavailable';
export const MIRROR_PRO_EVENT = 'ultravox:mirror-pro';
const PRO_FIXTURES: Record<MirrorProFixture, unknown> = {
  unlocked: { available: true, unlocked: true, state: 'paid', plan: 'paid', label: 'UltraVox Pro' },
  trial: { available: true, unlocked: true, state: 'trial', plan: 'trial', label: 'UltraVox Pro', trialUntil: new Date(Date.now() + 9 * 864e5).toISOString() },
  locked: { available: true, unlocked: false, state: 'none' },
  expired: { available: true, unlocked: false, state: 'expired', plan: 'trial', error: 'pro-locked: the UltraVox Pro trial has ended.' },
  unavailable: { available: false, unlocked: false, state: 'unavailable' },
};
let proFixture: unknown = null;
export const nativeMirrorRequested = () => import.meta.env.VITE_MIRROR_DEBUG === '1' && new URLSearchParams(location.search).get('native-mirror') === '1';

// Tauri v2 defines __TAURI_INTERNALS__.invoke non-writable and
// non-configurable, so IPC interception happens at the app's own ipc.ts
// choke point via wrapIpcForMirror, not by patching native internals.
const SIMULATED_LICENSE = { active: true, configured: true, plan: 'SIMULATION', label: 'SIMULATION', licenseState: 'active', offline: true };
// Mirror builds never run telemetry (native startup tasks are skipped), so the
// consent flow is simulated as permanently declined instead of blocking the UI.
const SIMULATED_TELEMETRY = { consent: 'declined', enabled: false };
// Mirror sessions must never touch real TCC permission state; the app UI is
// exercised with granted permissions instead.
const SIMULATED_PERMISSIONS = { microphone: 'granted', accessibility: 'granted', screen_recording: 'granted' };
const SIMULATED: Record<string, unknown> = {
  distribution_access_status: SIMULATED_LICENSE,
  bootstrap_distribution_access: SIMULATED_LICENSE,
  register_device_license: SIMULATED_LICENSE,
  activate_distribution_access: SIMULATED_LICENSE,
  get_app_telemetry_status: SIMULATED_TELEMETRY,
  set_app_telemetry_enabled: SIMULATED_TELEMETRY,
  get_permission_status: SIMULATED_PERMISSIONS,
  request_permission: SIMULATED_PERMISSIONS,
};
const SUPPRESSED = new Set(['set_theme_material', 'record_app_telemetry_usage']);
let mirrorRoute: ((cmd: string, args: any, transport: InvokeFn) => Promise<any>) | null = null;

export function wrapIpcForMirror(transport: InvokeFn): InvokeFn {
  return <T,>(cmd: string, args?: any, options?: any): Promise<T> => {
    if (mirrorRoute) return mirrorRoute(cmd, args, transport);
    return transport<T>(cmd, args, options);
  };
}

export function installNativeMirror() {
  if (import.meta.env.VITE_MIRROR_DEBUG !== '1') return;
  window.__ULTRAVOX_MIRROR_SNAPSHOT__ = () => ({ storage: Object.fromEntries(Object.keys(localStorage).map(k => [k, localStorage.getItem(k)!])) });
  if (!nativeMirrorRequested()) return;
  const values = new Map(Object.entries(window.__ULTRAVOX_MIRROR_SEED__?.storage ?? {}));
  const storage: Storage = { get length() { return values.size; }, key: i => [...values.keys()][i] ?? null, getItem: k => values.get(k) ?? null, setItem: (k,v) => { values.set(k,String(v)); }, removeItem: k => { values.delete(k); }, clear: () => values.clear() };
  Object.defineProperty(window, 'localStorage', { configurable: true, value: storage });
  Object.defineProperty(window, 'sessionStorage', { configurable: true, value: storage });
  window.__ULTRAVOX_MIRROR_SNAPSHOT__ = () => ({ storage: Object.fromEntries(values) });
  window.__ULTRAVOX_MIRROR_PRO__ = (state) => {
    proFixture = state ? PRO_FIXTURES[state] ?? null : null;
    window.dispatchEvent(new Event(MIRROR_PRO_EVENT));
  };
  mirrorRoute = (cmd, args, transport) => {
    if (cmd === 'get_pro_status' && proFixture) return Promise.resolve(proFixture);
    if (cmd in SIMULATED) return Promise.resolve(SIMULATED[cmd]);
    if (SUPPRESSED.has(cmd)) return Promise.resolve(undefined);
    return transport(cmd, args);
  };
  // WebKit suspends display-linked frames in invisible windows; keep the
  // mirror UI lifecycle moving without ever showing the window.
  window.requestAnimationFrame = cb => window.setTimeout(() => cb(performance.now()), 16);
  window.cancelAnimationFrame = id => clearTimeout(id);
  window.open = () => null;
}
