import { headers } from "next/headers";
import { redirect } from "next/navigation";
import type { ReactNode } from "react";
import { DeleteAccount, PasswordForm, SignOutButton } from "@/components/account-actions";
import { Logo } from "@/components/ui";
import { getAuth } from "@/lib/server";

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-3 rounded-xl bg-white/5.5 p-4 shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)]">
      <h2 className="text-[13px] font-semibold">{title}</h2>
      {children}
    </section>
  );
}

export default async function AccountPage() {
  const requestHeaders = await headers();
  const auth = await getAuth();
  const session = await auth.api.getSession({ headers: requestHeaders });
  if (!session) redirect("/sign-in");
  const accounts = await auth.api.listUserAccounts({ headers: requestHeaders });
  const hasPassword = accounts.some((account) => account.providerId === "credential");
  const hasGoogle = accounts.some((account) => account.providerId === "google");
  const { user } = session;
  const methods = [hasGoogle ? "Google" : null, hasPassword ? "email and password" : null].filter(Boolean).join(" or ");

  return (
    <main className="flex min-h-dvh justify-center px-5 pt-[10vh] pb-10">
      <div className="flex w-full max-w-[360px] flex-col gap-3">
        <Logo />
        <h1 className="mb-2 text-center text-xl font-semibold">Nuzky account</h1>
        <div className="flex items-center gap-3 rounded-xl bg-white/5.5 px-4 py-3 shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)]">
          <div className="flex size-10 shrink-0 items-center justify-center rounded-full bg-[#3a3a40] text-base font-semibold" aria-hidden>
            {(user.name || user.email).slice(0, 1).toUpperCase()}
          </div>
          <div className="flex min-w-0 flex-col gap-0.5">
            <span className="truncate text-[13px] font-medium">{user.name || user.email}</span>
            <span className="truncate text-xs text-muted">
              {user.name ? `${user.email} · ` : ""}Signs in with {methods}
            </span>
          </div>
        </div>
        <Section title={hasPassword ? "Change password" : "Password"}>
          <PasswordForm email={user.email} hasPassword={hasPassword} />
        </Section>
        <Section title="This browser">
          <SignOutButton />
        </Section>
        <Section title="Delete account">
          <DeleteAccount />
        </Section>
      </div>
    </main>
  );
}
