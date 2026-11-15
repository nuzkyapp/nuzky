import { useEffect, useRef, useState } from "react";
import { AlertCircle, Check, ChevronDown, ChevronRight, Loader2, Mail } from "lucide-react";
import { cancelSignIn, signIn, useAccount } from "../lib/account";
import { answerTelemetry, useTelemetry } from "../lib/telemetry";
import { GoogleButton } from "./settings/AccountPane";
import { Button, Checkbox, trapTab } from "./ui";

const EXAMPLE = `Export failed: MEDIA_UNREADABLE
File 1 (video, 4 clips): mov · hevc 1080×1920 29.97 fps VFR · rotated 90° · aac 48 kHz · iPhone 15 Pro
Nuzky 0.2.0 · macOS 26.1 · Apple M3 · 16 GB
Before it: import 2 files · user splitClip · job captions done · job export started`;

const CHOICES = [
  {
    label: "Problems and usage",
    detail: "Crashes, errors, which features you use and how fast Nuzky runs. Never your videos, sound, captions or file names.",
  },
  {
    label: "AI conversations",
    detail:
      "What you write in the AI panel, what the AI answers and the tools it calls, so I can see where it misunderstands you. Its answers can quote what is said in your videos.",
  },
] as const;

/** The account's row: sign in to keep Your style the same everywhere, or who is signed in. */
function AccountRow() {
  const { status, action, error } = useAccount();
  return (
    <div className="flex flex-col gap-2.5 px-3.5 py-3">
      <div className="flex flex-col gap-0.5">
        <span className="text-[13px] font-medium">Your style on every computer</span>
        <span className="text-[12px] text-muted">Sign in to keep Your style and your caption styles the same everywhere.</span>
      </div>
      {status?.state === "signedIn" ? (
        <p role="status" className="flex items-center gap-1.5 text-[12px] text-fg">
          <Check size={14} strokeWidth={2.5} className="shrink-0 text-ok" />
          Signed in as {status.email}
        </p>
      ) : action === "signingIn" ? (
        <div role="status" className="flex h-8 items-center gap-2.5 text-[13px]">
          <Loader2 size={16} className="shrink-0 animate-spin text-accent" />
          <span className="flex-1">Continue in your browser…</span>
          <Button pill onClick={cancelSignIn}>
            Cancel
          </Button>
        </div>
      ) : (
        <div className="flex flex-wrap gap-2">
          <GoogleButton size="h-8 text-[13px]" />
          <Button pill onClick={() => void signIn("email")}>
            <Mail size={14} />
            Continue with email
          </Button>
        </div>
      )}
      {error && (
        <p role="alert" className="flex items-start gap-1.5 text-[12px] leading-[18px]">
          <AlertCircle size={14} className="mt-0.5 shrink-0 text-danger" />
          {error}
        </p>
      )}
    </div>
  );
}

/**
 * The question after the update: what Nuzky may send its author, both unticked, and the optional account. Share
 * keeps what is ticked; Don't share, Esc and a click beside it keep both off, and it does not ask again by itself.
 */
export function ImproveDialog() {
  const open = useTelemetry((s) => s.open);
  const account = useAccount((s) => s.available);
  const [ticked, setTicked] = useState<Record<string, boolean>>({});
  const [example, setExample] = useState(false);
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const d = dialog.current;
    if (!d) return;
    if (!open) return void (d.open && d.close());
    if (d.open) return;
    d.showModal();
    // It opens unasked, so the dialog takes focus rather than a choice: a key typed meanwhile ticks nothing.
    d.focus();
  }, [open]);
  const dontShare = () => void answerTelemetry(null);
  const problems = !!ticked[CHOICES[0].label];
  const conversations = !!ticked[CHOICES[1].label];

  return (
    <dialog
      data-exit
      data-improve
      ref={dialog}
      tabIndex={-1}
      aria-labelledby="improve-title"
      aria-describedby="improve-text"
      onCancel={(e) => {
        e.preventDefault();
        dontShare();
      }}
      onPointerDown={(e) => {
        // A press on the backdrop lands on the dialog itself, outside its box.
        const b = e.currentTarget.getBoundingClientRect();
        if (e.target === e.currentTarget && (e.clientX < b.left || e.clientX > b.right || e.clientY < b.top || e.clientY > b.bottom)) dontShare();
      }}
      onKeyDown={(e) => {
        // The editor's keys wait until it is answered; F8 still reaches the toasts.
        if (e.key !== "F8") e.stopPropagation();
        trapTab(e);
      }}
      className="dialog-in overlay overlay-solid fixed inset-0 m-auto max-h-[calc(100vh-48px)] w-[480px] overflow-hidden rounded-[20px] p-0 text-fg outline-none backdrop:bg-black/45 open:flex open:flex-col"
    >
      {open && (
        <>
          <div className="min-h-0 overflow-y-auto px-6 pt-6 pb-2">
            <h2 id="improve-title" className="text-[17px] font-semibold leading-[22px] tracking-[-0.025em]">
              Can you help me make Nuzky better?
            </h2>
            <div id="improve-text" className="mt-2 flex flex-col gap-2 text-[13px]">
              <p className="text-fg/85">
                I build Nuzky alone in my free time and can only test it on my own videos and my own computer. To make it work well for you too, I need to see where
                it breaks on other setups and how people really use it.
              </p>
              <p className="text-[12px] text-muted">Tomáš Mach, author of Nuzky</p>
              <p className="text-muted">What you share is never tied to your name, email or account. You can change it any time in Settings › Privacy.</p>
            </div>
            <div role="group" aria-label="What Nuzky may send" className="mt-4 flex flex-col rounded-xl bg-white/[.055] shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)]">
              {CHOICES.map((c, i) => (
                <div key={c.label} className="flex border-t border-white/[.06] px-3.5 py-3 first:border-t-0">
                  <Checkbox label={c.label} checked={!!ticked[c.label]} onChange={(on) => setTicked({ ...ticked, [c.label]: on })} describedBy={`improve-choice-${i}`}>
                    <span className="flex flex-col gap-0.5">
                      <span className="text-[13px] font-medium">{c.label}</span>
                      <span id={`improve-choice-${i}`} className="text-[12px] text-muted">
                        {c.detail}
                      </span>
                    </span>
                  </Checkbox>
                </div>
              ))}
            </div>
            <button
              type="button"
              aria-expanded={example}
              onClick={() => setExample(!example)}
              className="-mx-2 mt-2 flex h-8 items-center gap-1.5 rounded-md px-2 text-[13px] font-semibold transition-colors duration-[120ms] ease-out hover:bg-white/[.05]"
            >
              {example ? <ChevronDown size={14} className="text-muted" /> : <ChevronRight size={14} className="text-muted" />}
              Example of a problem report
            </button>
            {example && (
              <pre className="mt-1 whitespace-pre-wrap break-words rounded-[10px] bg-black/25 px-3 py-2.5 font-mono text-[11px] leading-[17px] text-fg/80 shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)]">
                {EXAMPLE}
              </pre>
            )}
            {account && (
              <div className="mt-3 rounded-xl bg-white/[.055] shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)]">
                <AccountRow />
              </div>
            )}
          </div>
          <div className="flex shrink-0 justify-end gap-2 px-6 pt-4 pb-6">
            <Button pill onClick={dontShare}>
              Don't share
            </Button>
            <Button
              pill
              variant="primary"
              disabled={!problems && !conversations}
              disabledReason="Tick what Nuzky may send first"
              onClick={() => void answerTelemetry({ problems, conversations })}
            >
              Share
            </Button>
          </div>
        </>
      )}
    </dialog>
  );
}
