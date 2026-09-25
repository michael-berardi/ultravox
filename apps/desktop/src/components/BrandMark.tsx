import { getCurrentWindow } from "@tauri-apps/api/window";
import type { MouseEvent } from "react";

import type { ProStatus } from "../ipc";



/**
 * UltraVox brand cluster for window chrome: a compact voice-ring mark, the
 * wordmark, and the Implose Cybernetics attribution. Every color derives from
 * the active theme's custom properties, so the mark integrates with all
 * fourteen themes without per-theme overrides.
 */
export function proBrandSuffix(pro: ProStatus | null): string | undefined {
  return pro?.unlocked ? "Pro" : undefined;
}
export function startHeaderDrag(event: MouseEvent<HTMLElement>): void {
  if (
    event.button !== 0 ||
    !(event.target instanceof Element) ||
    event.target.closest(".header-actions")
  ) {
    return;
  }
  void getCurrentWindow().startDragging();
}


export function BrandMark({ suffix }: { suffix?: string }) {
  return (
    <span className="brand">
      <span className="brand-name">{suffix ? `UltraVox ${suffix}` : "UltraVox"}</span>
      <span className="brand-by">by Implose Cybernetics</span>
    </span>
  );
}
