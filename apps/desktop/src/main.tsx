import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ThemeHarness, parseHarnessState } from "./qa/ThemeHarness";
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

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {harnessTheme ? (
      <ThemeHarness state={parseHarnessState(params.get("qa-state"))} />
    ) : (
      <App />
    )}
  </React.StrictMode>,
);
