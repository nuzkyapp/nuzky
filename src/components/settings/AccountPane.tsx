import { useEffect, useRef } from "react";
import { AlertCircle, AlertTriangle, ExternalLink, Loader2, Mail } from "lucide-react";
import { cancelSignIn, check, initial, manage, signIn, signOut, useAccount } from "../../lib/account";
import type { AccountStatus } from "../../lib/types";
import { GoogleLogo } from "../GoogleLogo";
import { Button } from "../ui";

const CARD = "rounded-xl bg-white/[.055] shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)]";

/** Google's own Sign in with Google button in its dark format, as Google's branding guidelines ask. */
export function GoogleButton({ size = "h-10 w-[236px] text-[14px]" }: { size?: string }) {
  return (
    <button
      type="button"
      data-autofocus
      onClick={() => void signIn("google")}
      className={`inline-flex ${size} max-w-full shrink-0 items-center justify-center gap-2.5 rounded-full bg-[#131314] pr-4 pl-3 font-medium text-[#e3e3e3] shadow-[inset_0_0_0_1px_#8e918f] transition-colors duration-[120ms] ease-out hover:bg-[#1f1f20] active:brightness-90`}
    >
      <GoogleLogo />
      Continue with Google
    </button>
  );
}

function Waiting() {
  return (
    <div role="status" className="flex h-10 w-full items-center gap-2.5 text-[13px] text-fg">
      <Loader2 size={16} className="shrink-0 animate-spin text-accent" />
      <span className="flex-1">Continue in your browser…</span>
      <Button pill data-autofocus onClick={cancelSignIn}>
        Cancel
      </Button>
    </div>
  );
}

function SignedOut({ email }: { email: string | null }) {
  const waiting = useAccount((s) => s.action === "signingIn");
  return (
    <>
      <section aria-label="Sign in to Nuzky" className={`flex flex-col items-start gap-3 p-4 ${CARD}`}>
        <div>
          <h3 className="text-[13px] font-semibold text-fg">Sign in to Nuzky</h3>
          <p className="mt-1 text-[12px] leading-[18px] text-muted">
            Keep Your style and your caption styles the same on every computer. Everything in Nuzky works without an account.
          </p>
        </div>
        {waiting ? (
          <Waiting />
        ) : (
          <>
            <GoogleButton />
            <Button pill onClick={() => void signIn("email")} className="h-10 w-[236px] max-w-full text-[14px]">
              <Mail size={15} />
              Continue with email
            </Button>
            <p className="text-[12px] text-muted">Both open your browser.</p>
          </>
        )}
      </section>
      {email && !waiting && (
        <p className="text-[12px] leading-[18px] text-muted">
          Last signed in as <span className="text-fg/85">{email}</span>.{" "}
          <button type="button" onClick={() => void signIn("email", true)} className="text-accent hover:underline">
            Use another account
          </button>
        </p>
      )}
    </>
  );
}

function SignedIn({ status }: { status: Extract<AccountStatus, { state: "signedIn" }> }) {
  const action = useAccount((s) => s.action);
  return (
    <div role="group" aria-label="Nuzky account" className={`flex flex-col ${CARD}`}>
      <div className="flex min-h-[64px] items-center gap-3 px-3.5 py-3">
        <span aria-hidden="true" className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-white/[.14] text-[16px] font-semibold text-fg">
          {initial(status)}
        </span>
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="truncate text-[13px] font-medium text-fg" title={status.name ?? status.email}>
            {status.name ?? status.email}
          </span>
          {status.name && (
            <span className="truncate text-[12px] text-muted" title={status.email}>
              {status.email}
            </span>
          )}
        </div>
        <Button
          pill
          disabled={action !== null}
          disabledReason={action === "signingOut" ? "Signing out…" : "Finish signing in first"}
          onClick={() => void signOut()}
        >
          {action === "signingOut" ? "Signing out…" : "Sign out"}
        </Button>
      </div>
      <div className="flex min-h-[44px] items-center gap-3 border-t border-white/[.06] px-3.5 py-2.5">
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="text-[13px] font-medium text-fg">Manage account</span>
          <span className="text-[12px] text-muted">Set or change your password, or delete your account</span>
        </div>
        <Button pill data-autofocus onClick={() => void manage()}>
          Open <ExternalLink size={12} />
        </Button>
      </div>
    </div>
  );
}

/** Settings › Account: Continue with Google or email to sign in, or who is signed in, Manage account and Sign out. */
export function AccountPane() {
  const status = useAccount((s) => s.status);
  const checking = useAccount((s) => s.checking);
  const error = useAccount((s) => s.error);
  const action = useAccount((s) => s.action);

  // Showing the account checks the sign-in, renewing it when it is about to expire.
  useEffect(() => {
    if (!useAccount.getState().action) void check();
  }, []);

  // The main action takes the keyboard once it shows, and again when the state changes it, while the keyboard is here.
  const pane = useRef<HTMLDivElement>(null);
  const shown = `${checking && !status ? "checking" : status?.state}:${action}`;
  useEffect(() => {
    const at = document.activeElement;
    if (at && (at === document.body || pane.current?.contains(at) || at.matches("[data-settings-body]")))
      pane.current?.querySelector<HTMLElement>("[data-autofocus]")?.focus();
  }, [shown]);

  const notice = status?.state === "signedOut" ? status.notice : null;
  return (
    <div ref={pane} data-setting="account" className="flex flex-col gap-3 rounded-[14px]">
      {checking && !status ? (
        <p role="status" className="flex h-10 items-center gap-2 text-[12px] text-muted">
          <Loader2 size={14} className="animate-spin" />
          Checking your sign-in…
        </p>
      ) : status?.state === "signedIn" ? (
        <SignedIn status={status} />
      ) : (
        <>
          {notice && (
            <p className="flex items-start gap-1.5 text-[12px] leading-[18px] text-fg" role="status">
              <AlertTriangle size={14} className="mt-0.5 shrink-0 text-warn" />
              {notice === "ended"
                ? "Your sign-in ended. Sign in again to keep using your account."
                : "Signed out on this computer, but Nuzky couldn't reach the account server to end the sign-in there."}
            </p>
          )}
          <SignedOut email={status?.state === "signedOut" ? status.email : null} />
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
