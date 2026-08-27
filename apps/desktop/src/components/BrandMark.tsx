import { getCurrentWindow } from "@tauri-apps/api/window";
import type { MouseEvent } from "react";

const MARK_SIZE = 18;
export function startHeaderDrag(event: MouseEvent<HTMLElement>): void {
  if (event.button !== 0 || !(event.target instanceof Element) || event.target.closest(".header-actions")) return;
  void getCurrentWindow().startDragging();
}
export function BrandMark({ suffix = "Light" }: { suffix?: string }) {
  return <span className="brand">
    <svg className="brand-mark" width={MARK_SIZE} height={MARK_SIZE} viewBox="0 0 1024 1024" fill="none" aria-hidden="true">
      <defs><linearGradient id="uv-brand-stroke" x1="292" y1="280" x2="724" y2="742" gradientUnits="userSpaceOnUse"><stop stopColor="var(--accent-cyan, #7ceaff)"/><stop offset="0.55" stopColor="var(--accent, #8eb7ff)"/><stop offset="1" stopColor="var(--accent-violet, #b794ff)"/></linearGradient></defs>
      <g fill="none" stroke="url(#uv-brand-stroke)" strokeWidth="62" strokeLinecap="round"><path d="M312 420V520M412 340V600M512 270V650M612 360V580M712 430V520"/></g>
      <path d="M264 560c0 150 102 236 248 236s248-86 248-236M512 796v70M406 866h212" fill="none" stroke="var(--text, #f8faff)" strokeWidth="54" strokeLinecap="round"/>
    </svg>
    <span className="brand-name">UltraVox {suffix}</span>
  </span>;
}
