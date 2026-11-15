"use client";

import { useState, type FormEvent } from "react";
import { explain, post } from "@/lib/client";
import { Field, Note, PrimaryButton, Problem, SecondaryButton } from "./ui";

export function PasswordForm({ email, hasPassword }: { email: string; hasPassword: boolean }) {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [problem, setProblem] = useState("");
  const [done, setDone] = useState("");
  const [busy, setBusy] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setProblem("");
    setDone("");
    setBusy(true);
    const result = hasPassword
      ? await post("/api/auth/change-password", { currentPassword: current, newPassword: next, revokeOtherSessions: true })
      : await post("/api/account/set-password", { newPassword: next });
    setBusy(false);
    if (!result.ok) return setProblem(explain(result));
    setCurrent("");
    setNext("");
    if (!hasPassword) return window.location.reload();
    setDone("Password changed. Other browsers are signed out.");
  }

  return (
    <form onSubmit={submit} className="flex flex-col gap-3">
      <fieldset disabled={busy} className="contents">
        <input type="email" name="email" autoComplete="username" value={email} readOnly hidden />
        {hasPassword ? (
          <Field
            label="Current password"
            type="password"
            autoComplete="current-password"
            required
            value={current}
            onChange={(e) => setCurrent(e.target.value)}
          />
        ) : (
          <Note>Set a password to also sign in with your email address.</Note>
        )}
        <Field
          label="New password"
          type="password"
          autoComplete="new-password"
          required
          minLength={10}
          hint="At least 10 characters."
          value={next}
          onChange={(e) => setNext(e.target.value)}
        />
        {problem ? <Problem>{problem}</Problem> : null}
        {done ? <Note>{done}</Note> : null}
        <SecondaryButton type="submit">{hasPassword ? "Change password" : "Set password"}</SecondaryButton>
      </fieldset>
    </form>
  );
}

export function SignOutButton() {
  const [busy, setBusy] = useState(false);
  return (
    <SecondaryButton
      disabled={busy}
      onClick={async () => {
        setBusy(true);
        await post("/api/auth/sign-out");
        window.location.assign("/sign-in");
      }}
    >
      Sign out of this browser
    </SecondaryButton>
  );
}

export function DeleteAccount() {
  const [confirming, setConfirming] = useState(false);
  const [problem, setProblem] = useState("");
  const [needsSignIn, setNeedsSignIn] = useState(false);
  const [busy, setBusy] = useState(false);

  if (!confirming) {
    return (
      <SecondaryButton danger onClick={() => setConfirming(true)}>
        Delete account…
      </SecondaryButton>
    );
  }

  return (
    <div className="flex flex-col gap-3">
      <Note>
        Nuzky signs out on every computer, and the account can&apos;t be brought back. Your videos and projects stay on your computers.
      </Note>
      {problem ? <Problem>{problem}</Problem> : null}
      {needsSignIn ? (
        <PrimaryButton onClick={() => window.location.assign("/sign-in")}>Sign in again</PrimaryButton>
      ) : (
        <SecondaryButton
          danger
          disabled={busy}
          onClick={async () => {
            setBusy(true);
            setProblem("");
            const result = await post("/api/auth/delete-user");
            setBusy(false);
            if (result.ok) return window.location.assign("/sign-in?deleted=1");
            if (result.code === "SESSION_EXPIRED") {
              setNeedsSignIn(true);
              return setProblem("For your safety, sign in again, then delete the account.");
            }
            setProblem(explain(result));
          }}
        >
          Delete account
        </SecondaryButton>
      )}
      <SecondaryButton onClick={() => setConfirming(false)}>Keep account</SecondaryButton>
    </div>
  );
}
