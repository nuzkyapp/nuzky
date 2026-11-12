import "./lib/launch";
import "./lib/exit";
import { StrictMode, Suspense, lazy } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { WindowErrorBoundary } from "./components/WindowError";
import "./index.css";

// The preview window (src-tauri/src/preview_window.rs) loads only its own page, never the editor and its stores.
const Root = lazy(() => (getCurrentWindow().label === "preview" ? import("./components/preview/PreviewWindow") : import("./App")));

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <WindowErrorBoundary>
      <Suspense>
        <Root />
      </Suspense>
    </WindowErrorBoundary>
  </StrictMode>,
);
