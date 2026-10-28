import { useEffect, useRef, useState } from "react";
import { AlertCircle, ChevronDown, ChevronRight, FileText } from "lucide-react";
import { api, errorText } from "../lib/api";
import { showBugReport, useBugReport } from "../lib/diagnostics";
import { LogsButton } from "./Feedback";
import { Button, trapTab } from "./ui";

type Step = { kind: "form"; error?: string } | { kind: "sending" } | { kind: "saved"; name: string; openError: string | null };

export function BugReportDialog() {
  const open = useBugReport((s) => s.open);
  return open ? <Dialog /> : null;
}

/** What the folded report holds, counting the project's files once the report is read. */
function includes(report: string | null) {
  const files = report?.split("\n").filter((line) => line.startsWith("File ")).length ?? 0;
  const formats = files === 0 ? "" : files === 1 ? "the format of 1 file, " : `the format of ${files} files, `;
  return `Version, system, graphics, ${formats}your last actions, the log`;
}

function Dialog() {
  const [text, setText] = useState("");
  const [step, setStep] = useState<Step>({ kind: "form" });
  const [report, setReport] = useState<string | null>(null);
  const [unfolded, setUnfolded] = useState(false);
  const dialog = useRef<HTMLDialogElement>(null);
  const field = useRef<HTMLTextAreaElement>(null);
  const opener = useRef<HTMLElement | null>(null);

  useEffect(() => {
    opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    dialog.current?.showModal();
    field.current?.focus();
    let open = true;
    api.diagnostics().then(
      (r) => open && setReport(r),
      (e) => open && setReport(`Couldn't read the report: ${errorText(e)}`),
    );
    return () => {
      open = false;
    };
  }, []);

  // Closed before it leaves the page, which otherwise moves the focus to the page after it was given back.
  const close = () => {
    dialog.current?.close();
    opener.current?.focus();
    useBugReport.setState({ open: false });
  };
  const send = async () => {
    setStep({ kind: "sending" });
    try {
      setStep({ kind: "saved", ...(await api.reportBug(text)) });
    } catch (e) {
      setStep({ kind: "form", error: `Couldn't save the report: ${errorText(e)}` });
    }
  };

  return (
    <dialog
      data-exit
      ref={dialog}
      aria-labelledby="bug-report-title"
      onCancel={(e) => {
        e.preventDefault();
        close();
      }}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Escape") {
          e.preventDefault();
          // Saving already started opens GitHub; Esc waits for it like Cancel.
          return step.kind === "sending" ? undefined : close();
        }
        trapTab(e);
      }}
      className="dialog-in overlay fixed inset-0 m-auto w-[480px] max-w-[calc(100vw-48px)] rounded-[20px] p-6 text-fg outline-none backdrop:bg-black/45"
    >
      {step.kind === "saved" ? (
        <>
          <h2 id="bug-report-title" className="text-[17px] font-semibold leading-[22px] tracking-[-0.025em]">
            Almost done
          </h2>
          <p className="mt-2 text-[13px] leading-[18px] text-muted">
            {step.openError
              ? "Couldn't open the browser. Open github.com/nuzkyapp/nuzky, start a new issue and attach this file."
              : "GitHub opened with the bug form filled in. Click Show in folder, drag the file into the issue, then submit it."}
          </p>
          <div className="mt-4 flex items-center gap-3 rounded-xl bg-white/[.055] p-3 shadow-[inset_0_0_0_1px_rgba(255,255,255,.06)]">
            <FileText size={20} className="shrink-0 text-muted" />
            <div className="min-w-0 flex-1">
              <div className="truncate text-[13px] font-medium" data-report-name>
                {step.name}
              </div>
              <div className="text-[12px] text-muted">In Nuzky's logs folder</div>
            </div>
          </div>
          <div className="mt-5 flex justify-end gap-2">
            <Button pill onClick={close}>
              Done
            </Button>
            <Button pill variant="primary" autoFocus onClick={() => showBugReport()}>
              Show in folder
            </Button>
          </div>
        </>
      ) : (
        <>
          <h2 id="bug-report-title" className="text-[17px] font-semibold leading-[22px] tracking-[-0.025em]">
            Report a bug
          </h2>
          <label htmlFor="bug-report-text" className="mt-4 mb-1.5 block text-[12px] text-muted">
            What happened?
          </label>
          <textarea
            id="bug-report-text"
            ref={field}
            value={text}
            rows={4}
            placeholder="What did you do, and what went wrong?"
            onChange={(e) => setText(e.target.value)}
            className="w-full resize-none rounded-lg border border-white/[.08] bg-white/[.055] px-2.5 py-2 text-[13px] leading-[18px] text-fg outline-offset-0 placeholder:text-subtle focus:border-accent"
          />
          <div className="mt-3.5 rounded-xl bg-white/[.055] px-3 py-1 shadow-[inset_0_0_0_1px_rgba(255,255,255,.06)]">
            <button
              type="button"
              aria-expanded={unfolded}
              onClick={() => setUnfolded(!unfolded)}
              className="-mx-1.5 grid min-h-8 w-[calc(100%+12px)] grid-cols-[14px_1fr] items-center gap-x-1.5 rounded-md px-1.5 py-1.5 text-left transition-colors duration-[120ms] ease-out hover:bg-white/[.05]"
            >
              {unfolded ? <ChevronDown size={14} className="text-muted" /> : <ChevronRight size={14} className="text-muted" />}
              <span className="text-[13px] font-semibold">What the report includes</span>
              {!unfolded && <span className="col-start-2 text-[12px] text-muted">{includes(report)}</span>}
            </button>
            {unfolded && (
              <div className="pb-2.5">
                <pre data-report tabIndex={0} aria-label="Report" className="mt-1 max-h-[min(240px,calc(100vh-420px))] overflow-auto font-mono text-[11px] leading-[17px] whitespace-pre-wrap text-fg/80">
                  {report ?? "Reading the report…"}
                </pre>
                <p className="mt-2 text-[12px] text-muted">It has no folders, captions or other text from your projects; the log names files it had trouble with. Nothing is sent until you submit the issue on GitHub.</p>
              </div>
            )}
          </div>
          {step.kind === "form" && step.error && (
            <div role="alert" className="mt-3 flex gap-1.5 text-[13px] text-danger">
              <AlertCircle size={16} className="mt-px shrink-0" />
              <div className="flex min-w-0 flex-1 flex-col items-start gap-1">
                <p>{step.error}</p>
                <LogsButton />
              </div>
            </div>
          )}
          <div className="mt-5 flex justify-end gap-2">
            <Button pill disabled={step.kind === "sending"} disabledReason="Saving the report…" onClick={close}>
              Cancel
            </Button>
            <Button pill variant="primary" disabled={step.kind === "sending"} disabledReason="Saving the report…" onClick={() => void send()}>
              {step.kind === "sending" ? "Saving…" : "Continue on GitHub"}
            </Button>
          </div>
        </>
      )}
    </dialog>
  );
}
