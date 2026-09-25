import { useCallback, useEffect, useState } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";

import { getProStatus, onProStatusChanged } from "../ipc";
import { MIRROR_PRO_EVENT, nativeMirrorRequested } from "./nativeMirror";
import { mapProStatus, type ProStatus } from "./proStatus";

const UNAVAILABLE: ProStatus = mapProStatus(null);

/**
 * Live Pro status: the verified entitlement reported by the backend, kept
 * current through `pro://status-changed`. Failures resolve to "unavailable" so
 * Pro entry points stay hidden instead of unlocking anything.
 */
export function useProStatus(): [ProStatus, () => Promise<void>] {
  const [status, setStatus] = useState<ProStatus>(UNAVAILABLE);

  const refresh = useCallback(async () => {
    try {
      setStatus(await getProStatus());
    } catch (error) {
      console.error("Pro status unavailable:", error);
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    let unlisten: UnlistenFn | undefined;
    void refresh();
    // Private mirror builds re-read status when a QA fixture is swapped in.
    const mirrorPro = nativeMirrorRequested() && MIRROR_PRO_EVENT ? MIRROR_PRO_EVENT : null;
    if (mirrorPro) window.addEventListener(mirrorPro, refresh);
    void onProStatusChanged((next) => {
      if (!cancelled) setStatus(mapProStatus(next));
    })
      .then((stop) => {
        if (cancelled) stop();
        else unlisten = stop;
      })
      .catch(() => {
        // Without the event channel the status is still read once on demand.
      });
    return () => {
      cancelled = true;
      if (mirrorPro) window.removeEventListener(mirrorPro, refresh);
      unlisten?.();
    };
  }, [refresh]);

  return [status, refresh];
}
