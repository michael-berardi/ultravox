import React, { useEffect } from "react";
import type { ReactNode } from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ThemeHarness, parseHarnessSource, parseHarnessState } from "./qa/ThemeHarness";
import { initTheme, THEMES } from "./themes";
import "./index.css";

const params = new URLSearchParams(window.location.search);
const requestedTheme = import.meta.env.DEV ? params.get("qa-theme") : null;
const harnessTheme = THEMES.some((theme) => theme.id === requestedTheme)
  ? requestedTheme
  : null;

if (harnessTheme) {
  document.documentElement.dataset.theme = harnessTheme;
} else {
  void initTheme();
}

function BootReady({ children }: { children: ReactNode }) {
  useEffect(() => {
    document.getElementById("boot-status")?.remove();
  }, []);
  return children;
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <BootReady>
      {harnessTheme ? (
        <ThemeHarness
          state={parseHarnessState(params.get("qa-state"))}
          source={parseHarnessSource(params.get("qa-source"))}
        />
      ) : (
        <App />
      )}
    </BootReady>
  </React.StrictMode>,
);
