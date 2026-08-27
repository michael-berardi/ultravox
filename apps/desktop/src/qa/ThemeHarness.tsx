import { useEffect, useMemo, useState } from "react";
import { BrandMark } from "../components/BrandMark";
import { DropOverlay } from "../components/DropOverlay";
import { RecordingMeter } from "../components/ReactiveMeter";
import { useDelayedVisibility, TRANSCRIBING_INDICATOR_DELAY_MS } from "../components/useDelayedVisibility";
import { THEMES } from "../themes";

type Props = { theme: string | null; drop: boolean; reactive: boolean; recordingLevel: number; transcriptionDurationMs: number };
export function ThemeHarness({ theme, drop, reactive, recordingLevel, transcriptionDurationMs }: Props) {
  const [transcribing,setTranscribing]=useState(true);
  const visible=useDelayedVisibility(transcribing?"qa":"",TRANSCRIBING_INDICATOR_DELAY_MS);
  useEffect(()=>{const t=window.setTimeout(()=>setTranscribing(false),transcriptionDurationMs);return()=>window.clearTimeout(t)},[transcriptionDurationMs]);
  const selected=useMemo(()=>THEMES.find(item=>item.id===theme)||THEMES[0],[theme]);
  return <div className="app" data-qa-harness="light-theme"><header className="header"><BrandMark /></header><main className="main"><section className="hero"><div className={`status-label ${visible?"transcribing":"ready"}`}>{visible?"Transcribing":"Ready"}</div>{reactive&&<RecordingMeter active level={recordingLevel}/>}<button type="button" className="record-button"><span className="sr-only">Start recording</span></button><div className="secondary-actions"><button type="button" className="secondary-action">Transcribe URL</button></div></section><section className="history-section"><div className="history-heading">Theme QA</div><div className="recording-card"><strong>{selected.name}</strong><p>{selected.tagline}</p></div></section></main>{drop&&<DropOverlay/>}</div>;
}
