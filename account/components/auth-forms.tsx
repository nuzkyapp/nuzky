"use client";

import { useState, type FormEvent, type ReactNode } from "react";
import { continueTo, explain, post, type Failure } from "@/lib/client";
import { withQuery } from "@/lib/oauth-query";
import { Field, GoogleButton, Note, Or, PrimaryButton, Problem, TextLink } from "./ui";

type Flow = { oauthQuery: string | null };

function oauth(oauthQuery: string | null) {
  return oauthQuery ? { oauth_query: oauthQuery } : {};
}

function WithGoogle({ oauthQuery, children }: Flow & { children: ReactNode }) {
  return (
    <>
      <GoogleButton href={withQuery("/sign-in/google", oauthQuery)} />
      <Or />
      {children}
    </>
  );
}

function Form({ onSubmit, children }: { onSubmit: () => Promise<void>; children: ReactNode }) {
  const [busy, setBusy] = useState(false);
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (busy) return;
    setBusy(true);
    try {
      await onSubmit();
    } finally {
      setBusy(false);
    }
  }
  return (
    <form onSubmit={submit} className="flex flex-col gap-3" aria-busy={busy}>
      <fieldset disabled={busy} className="contents">
        {children}
      </fieldset>
    </form>
  );
}

export function SignInForm({ oauthQuery, problem: initialProblem }: Flow & { problem?: string }) {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [problem, setProblem] = useState(initialProblem ?? "");
  const [needsCode, setNeedsCode] = useState(false);

  if (needsCode) return <CodeStep email={email} password={password} oauthQuery={oauthQuery} />;

  return (
    <WithGoogle oauthQuery={oauthQuery}>
      <Form
        onSubmit={async () => {
          setProblem("");
          const result = await post("/api/auth/sign-in/email", { email, password, ...oauth(oauthQuery) });
          if (result.ok) return continueTo(result.data, "/account");
          if (result.code === "EMAIL_NOT_VERIFIED") return setNeedsCode(true);
          setProblem(explain(result));
        }}
      >
        <Field
          label="Email"
          type="email"
          name="email"
          autoComplete="username"
          required
          autoFocus
          value={email}
          onChange={(e) => setEmail(e.target.value)}
        />
        <Field
          label="Password"
          type="password"
          name="password"
          autoComplete="current-password"
          required
          value={password}
          onChange={(e) => setPassword(e.target.value)}
        />
        {problem ? <Problem>{problem}</Problem> : null}
        <PrimaryButton type="submit">Sign in</PrimaryButton>
        <div className="flex justify-between text-xs">
          <TextLink href={withQuery("/forgot-password", oauthQuery)}>Forgot password?</TextLink>
          <span className="text-muted">
            New here? <TextLink href={withQuery("/sign-up", oauthQuery)}>Create an account</TextLink>
          </span>
        </div>
      </Form>
    </WithGoogle>
  );
}

export function SignUpForm({ oauthQuery }: Flow) {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [problem, setProblem] = useState("");
  const [needsCode, setNeedsCode] = useState(false);

  if (needsCode) return <CodeStep email={email} password={password} oauthQuery={oauthQuery} />;

  return (
    <WithGoogle oauthQuery={oauthQuery}>
      <Form
        onSubmit={async () => {
          setProblem("");
          const result = await post("/api/auth/sign-up/email", { email, password, name: "" });
          if (result.ok) return setNeedsCode(true);
          setProblem(explain(result));
        }}
      >
        <Field
          label="Email"
          type="email"
          name="email"
          autoComplete="username"
          required
          autoFocus
          value={email}
          onChange={(e) => setEmail(e.target.value)}
        />
        <Field
          label="Password"
          type="password"
          name="password"
          autoComplete="new-password"
          required
          minLength={10}
          hint="At least 10 characters."
          value={password}
          onChange={(e) => setPassword(e.target.value)}
        />
        {problem ? <Problem>{problem}</Problem> : null}
        <PrimaryButton type="submit">Create account</PrimaryButton>
        <p className="text-center text-xs text-muted">
          Have an account? <TextLink href={withQuery("/sign-in", oauthQuery)}>Sign in</TextLink>
        </p>
      </Form>
    </WithGoogle>
  );
}

function CodeStep({ email, password, oauthQuery }: Flow & { email: string; password: string }) {
  const [code, setCode] = useState("");
  const [problem, setProblem] = useState("");
  const [note, setNote] = useState("");

  async function sendAgain() {
    setProblem("");
    setNote("");
    const result = await post("/api/auth/sign-in/email", { email, password, ...oauth(oauthQuery) });
    if (result.ok) return continueTo(result.data, "/account");
    if (result.status === 429) return setProblem(explain(result));
    setNote(`We sent a new code to ${email}.`);
  }

  return (
    <Form
      onSubmit={async () => {
        setProblem("");
        setNote("");
        const result = await post("/api/auth/email-otp/verify-email", { email, otp: code.trim(), ...oauth(oauthQuery) });
        if (result.ok) return continueTo(result.data, "/account");
        setProblem(explain(result));
      }}
    >
      <Note>
        We sent a 6-digit code to <span className="text-fg">{email}</span>. Enter it to finish.
      </Note>
      <Field
        label="Code"
        name="code"
        inputMode="numeric"
        autoComplete="one-time-code"
        pattern="[0-9]{6}"
        required
        autoFocus
        value={code}
        onChange={(e) => setCode(e.target.value.replace(/\s/g, ""))}
      />
      {problem ? <Problem>{problem}</Problem> : null}
      {note ? <Note>{note}</Note> : null}
      <PrimaryButton type="submit">Continue</PrimaryButton>
      <button
        type="button"
        onClick={sendAgain}
        className="self-center text-xs text-accent hover:underline focus-visible:underline focus-visible:outline-none"
      >
        Send a new code
      </button>
    </Form>
  );
}

export function ResetForm({ oauthQuery }: Flow) {
  const [email, setEmail] = useState("");
  const [code, setCode] = useState("");
  const [password, setPassword] = useState("");
  const [problem, setProblem] = useState("");
  const [sent, setSent] = useState(false);

  function fail(result: Failure) {
    setProblem(explain(result));
  }

  if (!sent) {
    return (
      <Form
        onSubmit={async () => {
          setProblem("");
          const result = await post("/api/auth/email-otp/request-password-reset", { email });
          if (result.ok) return setSent(true);
          fail(result);
        }}
      >
        <Note>We'll email you a code to set a new password.</Note>
        <Field
          label="Email"
          type="email"
          name="email"
          autoComplete="username"
          required
          autoFocus
          value={email}
          onChange={(e) => setEmail(e.target.value)}
        />
        {problem ? <Problem>{problem}</Problem> : null}
        <PrimaryButton type="submit">Send code</PrimaryButton>
        <p className="text-center text-xs">
          <TextLink href={withQuery("/sign-in", oauthQuery)}>Back to sign in</TextLink>
        </p>
      </Form>
    );
  }

  return (
    <Form
      onSubmit={async () => {
        setProblem("");
        const reset = await post("/api/auth/email-otp/reset-password", { email, otp: code.trim(), password });
        if (!reset.ok) return fail(reset);
        const signedIn = await post("/api/auth/sign-in/email", { email, password, ...oauth(oauthQuery) });
        continueTo(signedIn.ok ? signedIn.data : null, signedIn.ok ? "/account" : withQuery("/sign-in", oauthQuery));
      }}
    >
      <Note>
        If <span className="text-fg">{email}</span> has a Nuzky account, we sent it a 6-digit code. Enter it and choose a new password.
      </Note>
      <Field
        label="Code"
        name="code"
        inputMode="numeric"
        autoComplete="one-time-code"
        pattern="[0-9]{6}"
        required
        autoFocus
        value={code}
        onChange={(e) => setCode(e.target.value.replace(/\s/g, ""))}
      />
      <Field
        label="New password"
        type="password"
        name="password"
        autoComplete="new-password"
        required
        minLength={10}
        hint="At least 10 characters."
        value={password}
        onChange={(e) => setPassword(e.target.value)}
      />
      {problem ? <Problem>{problem}</Problem> : null}
      <PrimaryButton type="submit">Set password and sign in</PrimaryButton>
      <p className="text-center text-xs">
        <TextLink href={withQuery("/sign-in", oauthQuery)}>Back to sign in</TextLink>
      </p>
    </Form>
  );
}
