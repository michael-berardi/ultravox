import { useCallback, useEffect, useState } from "react";
import { BrandMark, proBrandSuffix } from "../components/BrandMark";
import { DropOverlay } from "../components/DropOverlay";
import { ProLockBadge } from "../components/ProLock";
import { RecordingSurround } from "../components/ReactiveMeter";
import { voicePhase } from "../components/voiceGlowLifecycle";
import { mapProStatus, type ProStatus } from "../lib/proStatus";

export type HarnessPro = "unlocked" | "locked" | "unavailable";

export function qaProStatus(mode: HarnessPro): ProStatus {
  return mapProStatus({
    available: mode !== "unavailable",
    unlocked: mode === "unlocked",
    state: mode === "unlocked" ? "paid" : mode === "unavailable" ? "unavailable" : "none",
  });
}

export function parseHarnessPro(value: string | null): ProStatus | null {
  return value === "unlocked" || value === "locked" || value === "unavailable"
    ? qaProStatus(value) : null;
}

// Synthetic renderer inputs only; this harness is not native recording proof.
export function ThemeHarness({ drop, pro, recordingActive, recordingLevel, transcriptionDurationMs }: {
  drop: boolean;
  pro: ProStatus | null;
  recordingActive: boolean;
  recordingLevel: number;
  transcriptionDurationMs: number;
}) {
  const [transcriptionActive, setTranscriptionActive] = useState(transcriptionDurationMs > 0);
  useEffect(() => {
    setTranscriptionActive(transcriptionDurationMs > 0);
    if (transcriptionDurationMs <= 0) return;
    const timer = window.setTimeout(() => setTranscriptionActive(false), transcriptionDurationMs);
    return () => window.clearTimeout(timer);
  }, [transcriptionDurationMs]);
  const showTranscribing = !recordingActive && transcriptionActive;
  const phase = voicePhase({ recording: recordingActive, processing: transcriptionActive });
  const readLevel = useCallback(async () => recordingLevel, [recordingLevel]);
  return (
    <div className="app" data-qa-harness="recording-theme" data-qa-synthetic="true" data-qa-transcribing-visible={showTranscribing}>
      {pro && <header className="header"><BrandMark suffix={proBrandSuffix(pro)} /></header>}
      <main className="main">
        <section className="hero" data-voice-phase={phase} aria-labelledby="activity-status">
          <RecordingSurround
            phase={phase}
            readLevel={readLevel}
            theme={document.documentElement.dataset.theme ?? "midnight"}
          />
          <div id="activity-status" aria-live="polite" className={`status-label ${recordingActive ? "recording" : showTranscribing ? "transcribing" : "ready"}`}>
            {recordingActive ? "Recording" : showTranscribing ? "Transcribing" : "Ready"}
          </div>
          <button className="record-button" type="button" aria-label="Start recording">
            <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
              <rect x="9" y="2" width="6" height="12" rx="3" />
              <path d="M5 10a7 7 0 0 0 14 0M12 17v5" />
            </svg>
          </button>
          <div className="secondary-actions">
            {(!pro || pro.available) && <>
              <button type="button" className={`secondary-action${pro && !pro.unlocked ? " pro-locked" : ""}`}>
                {pro && !pro.unlocked && <ProLockBadge />}Meeting mode
              </button>
              <button type="button" className={`secondary-action${pro && !pro.unlocked ? " pro-locked" : ""}`}>
                {pro && !pro.unlocked && <ProLockBadge />}Lecture mode
              </button>
            </>}
            <button type="button" className="secondary-action">Transcribe URL</button>
          </div>
        </section>
        <section className="history-section" aria-labelledby="latest-message-title">
          <div className="history-heading">
            <span id="latest-message-title">Latest message</span>
            <button className="history-link" type="button">View all</button>
          </div>
          <div className="empty-state compact"><p>Your latest transcription will appear here.</p></div>
        </section>
      </main>
      {drop && <DropOverlay />}
    </div>
  );
}
