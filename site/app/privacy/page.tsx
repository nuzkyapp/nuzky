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
      "Nuzky edits your videos on your computer, and your videos and projects stay there. We never receive them, and Nuzky sends no usage statistics.",
      "The AI panel is the exception you choose. The agent you pick there, such as ChatGPT, Codex, Claude Code, OpenCode or Antigravity CLI, sends your messages to its provider together with what Nuzky's tools give it: the transcript, details of your project and the frames of your video it asks to look at.",
      "Other features that need the internet talk straight to the service they use: GitHub for updates and plugins, Hugging Face when you download a speech or vision model, Openverse or Freesound when you search for sounds, YouTube when you paste a link, and the address of a picture an AI agent asks Nuzky to download. Edit from your phone sends exports only to the phone you connect on your own network, and a plugin you allow full access can read your files and use the internet like any program you run. These services follow their own privacy policies. We don't see that traffic.",
    ],
  },
  {
    heading: "The optional Nuzky account",
    body: [
      "You don't need an account to use Nuzky. If you create one, the account server at account.nuzky.app keeps your email address, your name, your profile picture address when you sign in with Google, and a one-way hash of your password, never the password itself. When you sign in with Google, it receives your name, email address and profile picture from Google and keeps Google's sign-in tokens encrypted. For each signed-in browser or computer it keeps the IP address and the browser's user agent until you sign out there or delete the account.",
      "While Nuzky is signed in, the account also keeps your style (your EDIT.md and which of its rules Nuzky may change) and your own caption styles, so they are the same on every computer you sign in on. For each synced version it keeps the name of the computer it came from and what changed, for 90 days, so you can see and restore earlier versions. It never receives your videos, projects, transcripts or what Nuzky learned from your videos.",
      "When you choose a password, the server sends the first five characters of its SHA-1 hash to Have I Been Pwned to check whether it appeared in a data leak. The password itself never leaves the server.",
      "The server sends you sign-in and password codes by email through Resend. Its logs record the IP address, time and page of each request, without codes, tokens or cookies. To stop password guessing it also counts recent requests per IP address and page, even before you sign in. We use these only to find faults and stop abuse.",
      "The server and its backups run on Oracle Cloud in Frankfurt, Germany. We use your data only to sign you in and to keep your account working. We don't sell it, share it for advertising or use it to train AI.",
    ],
  },
  {
    heading: "This website",
    body: [
      "nuzky.app runs on Vercel. Vercel Web Analytics records the page, the page you came from, your approximate location (country, region and city) and your browser, operating system and device type, without cookies. It tells visitors apart by a hash that it discards after a day, so a visit can't be tied to you.",
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
