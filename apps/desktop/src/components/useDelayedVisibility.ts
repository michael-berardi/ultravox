import { useEffect, useState } from "react";
export const TRANSCRIBING_INDICATOR_DELAY_MS = 60_000;

export function useDelayedVisibility(key: string | null, delayMs: number): boolean {
  const [visibleKey, setVisibleKey] = useState<string | null>(null);

  useEffect(() => {
    if (!key) {
      setVisibleKey(null);
      return;
    }
    const timer = window.setTimeout(() => setVisibleKey(key), delayMs);
    return () => window.clearTimeout(timer);
  }, [delayMs, key]);

  return key != null && visibleKey === key;
}
