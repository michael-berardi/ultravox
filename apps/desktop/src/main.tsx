import React, { useEffect } from "react";
import type { ReactNode } from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ThemeHarness } from "./qa/ThemeHarness";
import { initTheme, THEMES } from "./themes";
import "./index.css";
const params = new URLSearchParams(window.location.search);
const requestedTheme = import.meta.env.DEV ? params.get("qa-theme") : null;
const harnessTheme = THEMES.some(theme => theme.id === requestedTheme) ? requestedTheme : null;
const level = Number(params.get("qa-recording-level") ?? 0);
const recordingLevel = Number.isFinite(level) ? Math.min(1, Math.max(0, level)) : 0;
const duration = Number(params.get("qa-transcription-ms") ?? 0);
const transcriptionDurationMs = Number.isFinite(duration) ? Math.max(0, duration) : 0;
if (/Macintosh|MacIntel/.test(navigator.platform || navigator.userAgent)) document.documentElement.dataset.chrome = "traffic-lights";
if (harnessTheme) document.documentElement.dataset.theme = harnessTheme; else void initTheme();
function BootReady({ children }: { children: ReactNode }) { useEffect(() => { document.getElementById("boot-status")?.remove(); }, []); return children; }
ReactDOM.createRoot(document.getElementById("root")!).render(<React.StrictMode><BootReady>{harnessTheme ? <ThemeHarness theme={harnessTheme} drop={params.get("qa-drop") === "1"} reactive={params.get("qa-reactive") !== "0"} recordingLevel={recordingLevel} transcriptionDurationMs={transcriptionDurationMs}/> : <App/>}</BootReady></React.StrictMode>);
