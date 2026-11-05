import { useEffect, useRef } from "react";
import { AlertCircle, AlertTriangle, Check, ExternalLink, Loader2 } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { cancelSignIn, check, signIn, signOut, useChatGpt } from "../../lib/chatgpt";
import { ChatGptLogo } from "../ChatGptLogo";
import { Button } from "../ui";

export const USAGE_PAGE = "https://chatgpt.com/settings/usage";
const CARD = "rounded-xl bg-white/[.055] shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)]";

/** OpenAI's approved Continue with ChatGPT button, in its white format. */
export function ContinueButton({ anotherAccount = false, onClick }: { anotherAccount?: boolean; onClick?: () => void }) {
  return (
    <button
      type="button"
      data-autofocus
      onClick={onClick ?? (() => void signIn(anotherAccount))}
      className="inline-flex h-[45px] w-[242px] max-w-full shrink-0 items-center justify-center gap-3 rounded-[12px] bg-white px-5 text-[15px] font-medium leading-[18px] text-black active:brightness-90"
    >
      <ChatGptLogo />
      Continue with ChatGPT
    </button>
  );
}

function Waiting() {
  return (
    <div role="status" className="flex h-[45px] w-full items-center gap-2.5 text-[13px] text-fg">
      <Loader2 size={16} className="shrink-0 animate-spin text-accent" />
      <span className="flex-1">Sign in to ChatGPT in your browser…</span>
      <Button pill data-autofocus onClick={cancelSignIn}>
        Cancel
      </Button>
    </div>
  );
}

function SignedOut() {
  const status = useChatGpt((s) => s.status);
  const waiting = useChatGpt((s) => s.action === "signingIn");
  const email = status?.state === "signedOut" ? status.email : null;
  return (
    <section aria-label="Use your ChatGPT plan" className={`flex flex-col items-start gap-3 p-4 ${CARD}`}>
      <div>
        <h3 className="text-[13px] font-semibold text-fg">Use your ChatGPT plan</h3>
        <p className="mt-1 text-[12px] leading-[18px] text-muted">
          Sign in with ChatGPT, and AI in Nuzky can run on your ChatGPT plan. What it uses counts toward your plan's usage limits.
        </p>
      </div>
      {waiting ? <Waiting /> : <ContinueButton />}
      {email && !waiting && (
        <p className="text-[12px] text-muted">
          Last signed in as <span className="text-fg/85">{email}</span>.{" "}
          <button type="button" onClick={() => void signIn(true)} className="text-accent hover:underline">
            Use another account
          </button>
        </p>
      )}
    </section>
  );
}

function SignedIn({ email, plan }: { email: string | null; plan: boolean }) {
  const action = useChatGpt((s) => s.action);
  const signOutButton = (
    <Button pill variant="ghost" disabled={action !== null} disabledReason={action === "signingOut" ? "Signing out…" : "Finish signing in first"} onClick={() => void signOut()}>
      {action === "signingOut" ? "Signing out…" : "Sign out"}
    </Button>
  );
  return (
    <>
      <div role="group" aria-label="ChatGPT account" className={`flex items-center gap-3 px-3.5 py-3 ${CARD}`}>
        <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-white text-black">
          <ChatGptLogo size={20} />
        </span>
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="truncate text-[13px] font-medium text-fg" title={email ?? undefined}>
            {email ?? "ChatGPT account"}
          </span>
          {plan ? (
            <span className="flex items-center gap-1 text-[12px] text-muted">
              <Check size={13} className="shrink-0 text-ok" />
              Using ChatGPT plan
            </span>
          ) : (
            <span className="flex items-center gap-1 text-[12px] text-fg">
              <AlertTriangle size={13} className="shrink-0 text-warn" />
              Nuzky can't use your ChatGPT plan yet
            </span>
          )}
        </div>
      </div>
      {plan && action === "signingIn" ? (
        <Waiting />
      ) : plan ? (
        <div className="flex gap-2">
          <Button pill onClick={() => void openUrl(USAGE_PAGE)}>
            Manage usage <ExternalLink size={12} />
          </Button>
          {signOutButton}
        </div>
      ) : (
        <div className="flex flex-col items-start gap-3">
          <p className="text-[12px] leading-[18px] text-muted">
            Continue with ChatGPT and allow Nuzky to use your plan when the browser asks. If a work or school workspace blocks it, sign out and use a personal
            account.
          </p>
          {action === "signingIn" ? (
            <Waiting />
          ) : (
            <div className="flex items-center gap-2">
              <ContinueButton />
              {signOutButton}
            </div>
          )}
        </div>
      )}
    </>
  );
}

/** The ChatGPT account in Settings › AI: Continue with ChatGPT to sign in, or who is signed in, Manage usage and Sign out. */
export function ChatGptSection() {
  const status = useChatGpt((s) => s.status);
  const checking = useChatGpt((s) => s.checking);
  const error = useChatGpt((s) => s.error);

  // Showing the account checks the sign-in, renewing it when it is about to expire.
  useEffect(() => {
    if (!useChatGpt.getState().action) void check();
  }, []);

  // The main action takes the keyboard once it shows, and again when the state changes it, while the keyboard is here.
  const action = useChatGpt((s) => s.action);
  const section = useRef<HTMLDivElement>(null);
  const shown = `${checking && !status ? "checking" : status?.state}:${action}`;
  useEffect(() => {
    const at = document.activeElement;
    if (at && (at === document.body || section.current?.contains(at) || at.matches("[data-settings-body]"))) section.current?.querySelector<HTMLElement>("[data-autofocus]")?.focus();
  }, [shown]);

  const notice = status?.state === "signedOut" ? status.notice : null;
  return (
    <div ref={section} className="flex flex-col gap-3">
      {checking && !status ? (
        <p role="status" className="flex h-[45px] items-center gap-2 text-[12px] text-muted">
          <Loader2 size={14} className="animate-spin" />
          Checking your ChatGPT sign-in…
        </p>
      ) : status?.state === "signedIn" ? (
        <SignedIn email={status.email} plan={status.plan} />
      ) : (
        <>
          {notice && (
            <p className="flex items-start gap-1.5 text-[12px] leading-[18px] text-fg" role="status">
              <AlertTriangle size={14} className="mt-0.5 shrink-0 text-warn" />
              {notice === "ended" ? (
                "Your ChatGPT sign-in ended. Continue with ChatGPT to sign in again."
              ) : (
                <span>
                  Signed out here, but Nuzky couldn't reach ChatGPT to end the sign-in there. You can disconnect Nuzky in{" "}
                  <button type="button" onClick={() => void openUrl(USAGE_PAGE)} className="text-accent hover:underline">
                    ChatGPT settings
                  </button>
                  .
                </span>
              )}
            </p>
          )}
          <SignedOut />
        </>
      )}
      {error && (
        <p className="flex items-start gap-1.5 text-[12px] leading-[18px] text-fg" role="alert">
          <AlertCircle size={14} className="mt-0.5 shrink-0 text-danger" />
          {error}
        </p>
      )}
    </div>
  );
}
