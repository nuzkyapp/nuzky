import type { ButtonHTMLAttributes, InputHTMLAttributes, ReactNode } from "react";

export function Sheet({ title, children }: { title: string; children: ReactNode }) {
  return (
    <main className="flex min-h-dvh justify-center px-5 pt-[12vh] pb-10">
      <div className="flex w-full max-w-[300px] flex-col gap-3">
        <Logo />
        <h1 className="text-center text-xl font-semibold">{title}</h1>
        {children}
      </div>
    </main>
  );
}

export function Logo() {
  return (
    <div className="mx-auto mb-0.5 flex size-[34px] items-center justify-center rounded-[9px] bg-fg text-bg" aria-hidden>
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2">
        <circle cx="6" cy="6" r="3" />
        <circle cx="6" cy="18" r="3" />
        <path d="M20 4 8.12 15.88M14.47 14.48 20 20M8.12 8.12 12 12" />
      </svg>
    </div>
  );
}

export function Field({ label, hint, ...input }: { label: string; hint?: string } & InputHTMLAttributes<HTMLInputElement>) {
  return (
    <label className="flex flex-col gap-[5px]">
      <span className="text-xs font-medium text-muted">{label}</span>
      <input
        className="h-8 rounded-lg bg-raised px-2.5 text-[13px] text-fg shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)] outline-none placeholder:text-subtle focus:shadow-[0_0_0_2px_var(--color-accent)]"
        {...input}
      />
      {hint ? <span className="text-xs text-muted">{hint}</span> : null}
    </label>
  );
}

const buttonBase =
  "inline-flex h-10 w-full items-center justify-center gap-1.5 rounded-lg px-3.5 text-sm whitespace-nowrap active:brightness-90 disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent";

export function PrimaryButton(props: ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      className={`${buttonBase} bg-accent-strong font-semibold text-white shadow-[inset_0_1px_0_rgb(255_255_255/.2),0_1px_3px_rgb(0_0_0/.45)]`}
      {...props}
    />
  );
}

export function SecondaryButton({ danger, ...props }: { danger?: boolean } & ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      className={`${buttonBase} bg-white/9 font-medium shadow-[inset_0_1px_0_rgb(255_255_255/.07),inset_0_0_0_1px_rgb(255_255_255/.07)] ${danger ? "text-danger" : "text-fg"}`}
      {...props}
    />
  );
}

export function GoogleButton({ href }: { href: string }) {
  return (
    <a
      href={href}
      className="mt-1.5 inline-flex h-10 w-full items-center justify-center gap-2.5 rounded-full bg-[#131314] pr-4 pl-3 text-sm font-medium text-[#e3e3e3] shadow-[inset_0_0_0_1px_#8e918f] [font-family:Roboto,var(--font-sans)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
    >
      <svg width="18" height="18" viewBox="0 0 48 48" aria-hidden>
        <path
          fill="#EA4335"
          d="M24 9.5c3.54 0 6.71 1.22 9.21 3.6l6.85-6.85C35.9 2.38 30.47 0 24 0 14.62 0 6.51 5.38 2.56 13.22l7.98 6.19C12.43 13.72 17.74 9.5 24 9.5z"
        />
        <path
          fill="#4285F4"
          d="M46.98 24.55c0-1.57-.15-3.09-.38-4.55H24v9.02h12.94c-.58 2.96-2.26 5.48-4.78 7.18l7.73 6c4.51-4.18 7.09-10.36 7.09-17.65z"
        />
        <path
          fill="#FBBC05"
          d="M10.53 28.59c-.48-1.45-.76-2.99-.76-4.59s.27-3.14.76-4.59l-7.98-6.19C.92 16.46 0 20.12 0 24c0 3.88.92 7.54 2.56 10.78l7.97-6.19z"
        />
        <path
          fill="#34A853"
          d="M24 48c6.48 0 11.93-2.13 15.89-5.81l-7.73-6c-2.15 1.45-4.92 2.3-8.16 2.3-6.26 0-11.57-4.22-13.47-9.91l-7.98 6.19C6.51 42.62 14.62 48 24 48z"
        />
      </svg>
      Continue with Google
    </a>
  );
}

export function Or() {
  return (
    <div className="flex items-center gap-2.5 text-xs text-muted before:h-px before:flex-1 before:bg-white/8 after:h-px after:flex-1 after:bg-white/8">
      or
    </div>
  );
}

export function Problem({ children }: { children: ReactNode }) {
  return (
    <p role="alert" className="text-xs leading-[18px] text-danger">
      {children}
    </p>
  );
}

export function Note({ children }: { children: ReactNode }) {
  return <p className="text-xs leading-[18px] text-muted">{children}</p>;
}

export function TextLink({ href, children }: { href: string; children: ReactNode }) {
  return (
    <a href={href} className="text-accent hover:underline focus-visible:underline focus-visible:outline-none">
      {children}
    </a>
  );
}
