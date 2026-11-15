import type { Metadata } from "next";
import { Footer } from "@/components/Footer";
import { Nav } from "@/components/Nav";

const contact = "tomades1@gmail.com";
const description = "What Nuzky, the optional Nuzky account and this website keep about you, where it is stored and how to delete it.";

export const metadata: Metadata = {
  title: "Privacy policy",
  description,
  alternates: { canonical: "/privacy" },
};

const sections: { heading: string; body: React.ReactNode[] }[] = [
  {
    heading: "Nuzky on your computer",
    body: [
      "Nuzky edits your videos on your computer. Your videos, projects, transcripts and exports stay there. We never receive them, and Nuzky sends no usage statistics.",
      "Features that need the internet talk straight to the service they use: GitHub for updates and plugins, Hugging Face when you download a speech or vision model, Openverse or Freesound when you search for sounds, YouTube when you paste a link, OpenAI when you use ChatGPT in the AI panel, and the address of a picture an AI agent asks Nuzky to download. Those services follow their own privacy policies. We don't see that traffic.",
    ],
  },
  {
    heading: "The optional Nuzky account",
    body: [
      "You don't need an account to use Nuzky. If you create one, the account server at account.nuzky.app keeps your email address, your name, your profile picture address when you sign in with Google, and a one-way hash of your password, never the password itself. When you sign in with Google, it receives your name, email address and profile picture from Google and keeps Google's sign-in tokens encrypted. For each signed-in browser or computer it keeps the IP address and browser name, until you sign out or the sign-in expires.",
      "When you choose a password, the server sends the first five characters of its SHA-1 hash to Have I Been Pwned to check whether it appeared in a data leak. The password itself never leaves the server.",
      "The server sends you sign-in and password codes by email through Resend. Its logs record the IP address, time and page of each request, without codes, tokens or cookies, and we use them only to find faults and stop abuse.",
      "The server and its backups run on Oracle Cloud in Frankfurt, Germany. We use your data only to sign you in and to keep your account working. We don't sell it, share it for advertising or use it to train AI.",
    ],
  },
  {
    heading: "This website",
    body: [
      "nuzky.app runs on Vercel. Vercel Web Analytics counts page visits without cookies and without storing anything that identifies you.",
    ],
  },
  {
    heading: "Deleting your data",
    body: [
      <>
        Sign in at{" "}
        <a href="https://account.nuzky.app/account" className="text-fg underline decoration-white/30 underline-offset-4">
          account.nuzky.app/account
        </a>{" "}
        and choose Delete account. Your account is deleted at once and every computer is signed out. Copies in the backups expire on their own. You can also
        ask us by email to delete your account or to send you what it holds.
      </>,
    ],
  },
  {
    heading: "Your rights and contact",
    body: [
      <>
        Nuzky is run by Tomáš Mach. Write to{" "}
        <a href={`mailto:${contact}`} className="text-fg underline decoration-white/30 underline-offset-4">
          {contact}
        </a>{" "}
        to see, correct or delete what we keep about you, or with any question about this policy. If you think we handle your data wrongly, you can also
        complain to the Czech Office for Personal Data Protection (ÚOOÚ).
      </>,
      "When this policy changes, we update it here and change the date below.",
    ],
  },
];

export default function Page() {
  return (
    <>
      <Nav base="/" />
      <main className="mx-auto w-full max-w-[760px] px-5 pb-24 pt-16 sm:pt-24">
        <h1 className="heading-lg">Privacy policy</h1>
        <p className="mt-5 text-[17px] leading-[1.55] text-muted">{description}</p>
        {sections.map(({ heading, body }) => (
          <section key={heading} className="mt-12">
            <h2 className="text-[22px] font-semibold tracking-[-0.02em]">{heading}</h2>
            {body.map((paragraph, i) => (
              <p key={i} className="mt-4 text-[16px] leading-[1.6] text-muted">
                {paragraph}
              </p>
            ))}
          </section>
        ))}
        <p className="mt-12 text-[14px] text-subtle">Last updated 15 November 2026.</p>
      </main>
      <Footer />
    </>
  );
}
