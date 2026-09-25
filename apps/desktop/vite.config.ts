import { defineConfig, loadEnv } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";

// The Pro module is closed source: `@pro` points at `src/pro-stub` by default and at
// the open-source stand-in when ULTRAVOX_OPEN_SOURCE=1 (the export script
// hard-wires the stand-in).
const proEntry =
  process.env.ULTRAVOX_OPEN_SOURCE === "1"
    ? path.resolve(__dirname, "./src/pro-stub/index.tsx")
    : path.resolve(__dirname, "./src/pro-stub/index.tsx");

// https://vitejs.dev/config/
export default defineConfig(async ({ mode }) => ({
  plugins: [react(), ...(loadEnv(mode, process.cwd(), 'VITE_').VITE_MIRROR_DEBUG === '1' ? [] : [{
    name: 'consumer-no-mirror', apply: 'build' as const, enforce: 'pre' as const,
    resolveId(id: string) { if (/(?:^|\/)nativeMirror(?:\.ts)?$/.test(id)) return '\0consumer-native-adapter'; },
    load(id: string) { if (id === '\0consumer-native-adapter') return 'export const installNativeMirror = () => {}; export const nativeMirrorRequested = () => false; export const wrapIpcForMirror = (transport) => transport; export const MIRROR_PRO_EVENT = "";'; },
  }])],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: process.env.TAURI_ENV_PLATFORM == "ios" ? "ios15" : "es2021",
  },
  resolve: {
    alias: {
      "@pro": proEntry,
      "@": path.resolve(__dirname, "./src"),
    },
  },
}));
