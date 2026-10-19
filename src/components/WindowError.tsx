import { Component, type ErrorInfo, type ReactNode } from "react";
import { AlertCircle, CheckCircle2 } from "lucide-react";
import { api } from "../lib/api";
import { reportBugNow, showBugReport, type Say } from "../lib/diagnostics";
import { LogsButton } from "./Feedback";
import { Button } from "./ui";

const log = (message: string) => void api.logUiError(message).catch(() => {});

window.addEventListener("error", (e) => log(`${e.message} at ${e.filename}:${e.lineno}:${e.colno}`));
window.addEventListener("unhandledrejection", (e) => log(`Unhandled rejection: ${e.reason instanceof Error ? (e.reason.stack ?? e.reason.message) : String(e.reason)}`));

/** A bug while drawing the interface shows this instead of an empty window; the project lives in Rust and stays saved. */
export class WindowErrorBoundary extends Component<{ children: ReactNode }, { failed: boolean; said: Parameters<Say> | null; saving: boolean }> {
  state = { failed: false, said: null, saving: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    log(`The window stopped drawing: ${error.stack ?? error.message}${info.componentStack ?? ""}`);
  }

  render() {
    if (!this.state.failed) return this.props.children;
    return (
      <div className="flex h-screen items-center justify-center p-6">
        <div role="alertdialog" aria-labelledby="window-error-title" aria-describedby="window-error-text" className="overlay w-[440px] rounded-[20px] p-6 text-fg">
          <h2 id="window-error-title" className="text-[17px] font-semibold">
            Something went wrong in this window
          </h2>
          <p id="window-error-text" className="mt-3 text-[13px] text-muted">
            Your project is saved. Reload the window to keep editing.
          </p>
          {/* Toasts went with the rest of the window, so how reporting went shows here. */}
          {this.state.said?.[0] === "success" && (
            <div role="status" className="mt-3 flex gap-1.5 text-[13px] text-muted">
              <CheckCircle2 size={16} className="mt-px shrink-0 text-ok" />
              <div className="flex min-w-0 flex-1 flex-col items-start gap-1">
                <p>{this.state.said[1]}</p>
                <button type="button" onClick={() => showBugReport((...said) => this.setState({ said }))} className="-mx-1.5 rounded-md px-1.5 py-0.5 text-[12px] font-semibold text-accent transition-colors duration-[120ms] ease-out hover:bg-white/[.08]">
                  Show in folder
                </button>
              </div>
            </div>
          )}
          {this.state.said?.[0] === "error" && (
            <div role="alert" className="mt-3 flex gap-1.5 text-[13px] text-danger">
              <AlertCircle size={16} className="mt-px shrink-0" />
              <div className="flex min-w-0 flex-1 flex-col items-start gap-1">
                <p>{this.state.said[1]}</p>
                <LogsButton />
              </div>
            </div>
          )}
          <div className="mt-6 flex justify-end gap-2.5">
            <Button
              pill
              disabled={this.state.saving || this.state.said?.[0] === "success"}
              disabledReason={this.state.saving ? "Saving the report…" : "The report is saved"}
              onClick={() => {
                this.setState({ saving: true });
                void reportBugNow((...said) => this.setState({ said, saving: false }));
              }}
            >
              Report a bug…
            </Button>
            <Button pill variant="primary" autoFocus onClick={() => window.location.reload()}>
              Reload
            </Button>
          </div>
        </div>
      </div>
    );
  }
}
