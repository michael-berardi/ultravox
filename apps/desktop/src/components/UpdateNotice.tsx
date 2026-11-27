import type { UpdateInfo } from "../ipc";
import { updater, useUpdater } from "../lib/useUpdater";
import type { UpdatePhase } from "../lib/updater";

/** Headline shared by the notice and Settings → Updates. */
export function updateHeadline(info: Pick<UpdateInfo, "latest_version" | "current_version">): string {
  return `UltraVox ${info.latest_version} is available — you have ${info.current_version}.`;
}

export type UpdateNoticeViewProps = {
  info: UpdateInfo;
  phase: UpdatePhase;
  error: string | null;
  automatic: boolean;
  onInstall: () => void;
  onLater: () => void;
  onSkip: () => void;
  onAutomaticChange: (enabled: boolean) => void;
};

/**
 * Non-modal update notice under the header: work (recording, history) stays
 * usable underneath. Text leads; Skip leads the action row and Remind Me
 * Later + Install Update share the content's trailing edge.
 */
export function UpdateNoticeView({ info, phase, error, automatic, onInstall, onLater, onSkip, onAutomaticChange }: UpdateNoticeViewProps) {
  const busy = phase === "updating";
  return (
    <aside className="update-notice" role="region" aria-labelledby="update-notice-title">
      <h2 id="update-notice-title">{updateHeadline(info)}</h2>
      {info.release_notes && (
        <p className="update-notes" tabIndex={0} aria-label="Release notes">{info.release_notes}</p>
      )}
      <label className="update-automatic-choice">
        <input
          type="checkbox"
          checked={automatic}
          disabled={busy}
          onChange={(event) => onAutomaticChange(event.target.checked)}
        />
        <span>Automatically download and install updates in the future</span>
      </label>
      {error && <p className="update-notice-error" role="alert">{phase === "failed" ? `Update failed: ${error}` : error}</p>}
      <div className="update-actions">
        <button className="btn" type="button" disabled={busy} onClick={onSkip}>Skip This Version</button>
        <span className="update-actions-trailing">
          <button className="btn" type="button" disabled={busy} onClick={onLater}>Remind Me Later</button>
          <button className="btn btn-primary" type="button" disabled={busy} aria-busy={busy} onClick={onInstall}>
            {busy ? "Installing…" : "Install Update"}
          </button>
        </span>
      </div>
    </aside>
  );
}

/** Shown by the main window when an automatic check finds a newer release. */
export function UpdateNotice() {
  const { notice, info, phase, error, automatic } = useUpdater();
  if (!notice || !info || phase === "hidden") return null;
  return (
    <UpdateNoticeView
      info={info}
      phase={phase}
      error={error}
      automatic={automatic}
      onInstall={() => void updater.install()}
      onLater={() => updater.later()}
      onSkip={() => void updater.skip()}
      onAutomaticChange={(enabled) => void updater.setAutomatic(enabled, false)}
    />
  );
}
