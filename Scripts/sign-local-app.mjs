import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { resolve } from "node:path";

if (process.platform !== "darwin") process.exit(0);

const appPath = resolve("target/release/bundle/macos/UltraVox.app");
if (!existsSync(appPath)) {
  throw new Error(`UltraVox bundle not found: ${appPath}`);
}

const requirement = '=designated => identifier "com.imploselabs.ultravox"';
const result = spawnSync(
  "codesign",
  ["--force", "--deep", "--sign", "-", "--requirements", requirement, appPath],
  { stdio: "inherit" },
);

if (result.error) throw result.error;
if (result.status !== 0) process.exit(result.status ?? 1);
