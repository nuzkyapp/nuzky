import "./lib/launch";
import "./lib/exit";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { WindowErrorBoundary } from "./components/WindowError";
import "./index.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <WindowErrorBoundary>
      <App />
    </WindowErrorBoundary>
  </StrictMode>,
);
