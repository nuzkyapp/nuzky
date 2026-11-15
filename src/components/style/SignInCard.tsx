import { useState } from "react";
import { AlertCircle, Loader2, Mail, X } from "lucide-react";
import { cancelSignIn, signIn, useAccount } from "../../lib/account";
import { GoogleButton } from "../settings/AccountPane";
import { Button, IconButton } from "../ui";

const KEY = "nuzky.styleSignIn";

/**
 * Your style's offer of the Nuzky account, where keeping the style on every computer means something: while the
 * build offers the account and nobody is signed in, until the creator closes it for good.
 */
export function SignInCard() {
  const { available, status, action, error } = useAccount();
  const [closed, setClosed] = useState(() => localStorage.getItem(KEY) === "closed");
  if (!available || closed || status?.state !== "signedOut") return null;
  const close = () => {
    localStorage.setItem(KEY, "closed");
    setClosed(true);
  };
  return (
    <section
      data-style-sign-in
      aria-labelledby="style-sign-in-title"
      className="flex flex-col gap-3 rounded-xl bg-white/[.055] px-3.5 py-3 shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)]"
    >
      <div className="flex items-start gap-3">
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <h2 id="style-sign-in-title" className="text-[13px] font-medium text-fg">
            Keep your style on every computer
          </h2>
          <p className="text-[12px] text-muted">Sign in and Your style and your caption styles come along to every computer you edit on.</p>
        </div>
        <IconButton label="Don't show again" onClick={close}>
          <X size={14} />
        </IconButton>
      </div>
      {action === "signingIn" ? (
        <div role="status" className="flex h-8 items-center gap-2.5 text-[13px] text-fg">
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
        <p role="alert" className="flex items-start gap-1.5 text-[12px] leading-[18px] text-fg">
          <AlertCircle size={14} className="mt-0.5 shrink-0 text-danger" />
          {error}
        </p>
      )}
    </section>
  );
}
