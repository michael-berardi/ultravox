export function DropOverlay() {
  return (
    <div className="drop-overlay" role="status" aria-live="polite">
      <div className="drop-overlay-card">
        <svg width="34" height="34" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" aria-hidden="true">
          <path d="M3 12c2.1-4.2 4.2 4.2 6.3 0s4.2-4.2 6.3 0 4.2 4.2 5.4 0" />
          <path d="M12 3v6m0 0-2.5-2.5M12 9l2.5-2.5" />
          <path d="M5 18h14" />
        </svg>
        <p>Drop audio to transcribe</p>
        <span>MP3, M4A, WAV, OPUS, OGG, WebM, FLAC…</span>
      </div>
    </div>
  );
}
