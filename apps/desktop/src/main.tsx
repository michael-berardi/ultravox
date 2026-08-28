import React, { useEffect, type ReactNode } from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { initTheme } from "./themes";
import "./index.css";

if (/Macintosh|MacIntel/.test(navigator.platform || navigator.userAgent)) {
  document.documentElement.dataset.chrome = "traffic-lights";
}
void initTheme();

function BootReady({ children }: { children: ReactNode }) {
  useEffect(() => {
    document.getElementById("boot-status")?.remove();
  }, []);
  return children;
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <BootReady><App /></BootReady>
  </React.StrictMode>,
);
