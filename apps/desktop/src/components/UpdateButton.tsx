import { updater, useUpdater } from "../lib/useUpdater";

/** Header control: visible only while a newer UltraVox exists. */
export function UpdateButton() {
  const { phase, info, error } = useUpdater();
  if (phase === "hidden" || !info) return null;
  const version = info.latest_version;
  const updating = phase === "updating";
  const failed = phase === "failed";
  return (
    <div className="update-control">
      <button
        className={`update-button ${phase}`}
        type="button"
        disabled={updating}
        aria-busy={updating}
        aria-label={updating ? `Updating to UltraVox ${version}` : failed ? `Retry update to UltraVox ${version}` : `Update to UltraVox ${version}`}
        title={failed ? undefined : `UltraVox ${version}`}
        onClick={() => void updater.install()}
      >
        {updating ? <span className="update-spinner" aria-hidden="true" /> : failed ? <RetryIcon /> : <ArrowIcon />}
        <span>{updating ? "Updating" : failed ? "Retry" : "Update"}</span>
      </button>
      {failed && error && (
        <p className="update-error" role="alert">Update failed: {error}</p>
      )}
    </div>
  );
}

function ArrowIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M12 4v12" />
      <path d="m6 11 6 6 6-6" />
      <path d="M5 20h14" />
    </svg>
  );
}

function RetryIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M20 11a8 8 0 1 0-2.3 5.7" />
      <path d="M20 4v7h-7" />
    </svg>
  );
}
