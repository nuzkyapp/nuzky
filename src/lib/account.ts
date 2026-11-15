import { create } from "zustand";
import { api, errorText, plainError } from "./api";
import { paneShown } from "./settings";
import { useEditor } from "./store";
import type { AccountProvider, AccountStatus } from "./types";

/**
 * The optional Nuzky account as Settings › Account shows it. The backend keeps the tokens; this knows
 * only who is signed in. A sign-in goes on while Settings is closed and reports in a toast then.
 */
type Account = {
  /** Whether Settings offers the account at all (`account_available`). */
  available: boolean;
  /** Null until the first check answers. */
  status: AccountStatus | null;
  checking: boolean;
  action: "signingIn" | "signingOut" | null;
  /** What went wrong last, in plain words. */
  error: string | null;
};

export const useAccount = create<Account>(() => ({
  available: false,
  status: null,
  checking: false,
  action: null,
  error: null,
}));

void api
  .accountAvailable()
  .then((available) => {
    useAccount.setState({ available });
    // The rail shows who is signed in, so it reads what is stored, without the network.
    if (available) void check(false);
  })
  .catch(() => {});

/** The account's words for what the shared sign-in code reports; the rest reads as anywhere else. */
const WORDS: Record<string, string> = {
  SIGN_IN_DENIED: "Signing in was cancelled in the browser. Choose Continue with Google or Continue with email to try again.",
  SIGN_IN_TIMEOUT: "Nuzky stopped waiting for the browser after 5 minutes. Sign in again when you're ready.",
  KEYCHAIN_UNAVAILABLE:
    "Nuzky keeps your sign-in in the system keychain and couldn't use it. Allow Nuzky when the system asks. On Linux, start GNOME Keyring or KWallet, then restart Nuzky.",
};

const accountError = (text: string) => WORDS[/\b([A-Z][A-Z_]{3,}):/.exec(text)?.[1] ?? ""] ?? plainError(text);

/** Each sign-in's number; a later one, or Cancel, makes an earlier one's answer stale. */
let attempt = 0;

const shown = () => paneShown("account");

let checking: Promise<void> | null = null;

/** One check at a time: a renewal rotates the refresh token, so a second one at once would use a spent token. */
export function check(renew = true) {
  checking ??= checkNow(renew).finally(() => (checking = null));
  return checking;
}

async function checkNow(renew: boolean) {
  useAccount.setState({ checking: true });
  try {
    useAccount.setState({ status: await api.accountStatus(renew) });
  } catch (e) {
    useAccount.setState({ error: accountError(errorText(e)) });
  } finally {
    useAccount.setState({ checking: false });
  }
}

export async function signIn(provider: AccountProvider, anotherAccount = false) {
  const mine = ++attempt;
  useAccount.setState({ action: "signingIn", error: null });
  try {
    const status = await api.accountSignIn(provider, anotherAccount);
    // Cancelled too late to stop it: the sign-in happened, so Settings shows what is stored now.
    if (mine !== attempt) return void check(false);
    useAccount.setState({ status, action: null });
    if (!shown() && status.state === "signedIn")
      useEditor.getState().toast({
        kind: "success",
        text: `Signed in to Nuzky as ${status.email}.`,
      });
  } catch (e) {
    if (mine !== attempt) return;
    const text = errorText(e);
    useAccount.setState({
      action: null,
      error: text.startsWith("CANCELLED") ? null : accountError(text),
    });
    if (!shown() && !text.startsWith("CANCELLED")) useEditor.getState().toast({ kind: "error", text: accountError(text) });
  }
}

export function cancelSignIn() {
  attempt++;
  useAccount.setState({ action: null });
  void api.accountCancelSignIn();
}

export async function signOut() {
  useAccount.setState({ action: "signingOut", error: null });
  try {
    useAccount.setState({ status: await api.accountSignOut() });
  } catch (e) {
    useAccount.setState({ error: accountError(errorText(e)) });
  } finally {
    // A sign-in started meanwhile keeps its own state.
    if (useAccount.getState().action === "signingOut") useAccount.setState({ action: null });
  }
}

export async function manage() {
  useAccount.setState({ error: null });
  try {
    await api.accountManage();
  } catch (e) {
    useAccount.setState({ error: accountError(errorText(e)) });
  }
}

/** The letter in the account's circle: the name's first, else the email's. */
export const initial = (status: Extract<AccountStatus, { state: "signedIn" }>) => [...(status.name || status.email).trim()][0]?.toUpperCase() ?? "";
