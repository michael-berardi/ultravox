import type { ReactNode } from "react";

export function LockIcon() {
  return (
    <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <rect x="4" y="11" width="16" height="10" rx="2" />
      <path d="M8 11V7a4 4 0 0 1 8 0v4" />
    </svg>
  );
}

/** Small padlock shown inside Pro entry points that are still locked. */
export function ProLockBadge() {
  return (
    <span className="pro-lock-badge" aria-hidden="true">
      <LockIcon />
    </span>
  );
}

/**
 * The answer to every locked Pro action: a short prompt that routes to
 * Settings → Pro instead of surfacing a raw `pro-locked:` error.
 */
export function ProLockPrompt({
  copy,
  actionLabel = "Open Pro settings",
  onOpenPro,
}: {
  copy: ReactNode;
  actionLabel?: string;
  onOpenPro: () => void;
}) {
  return (
    <div className="pro-lock-prompt" role="status">
      <ProLockBadge />
      <p className="pro-lock-copy">{copy}</p>
      <button type="button" className="btn btn-primary" onClick={onOpenPro}>
        {actionLabel}
      </button>
    </div>
  );
}
