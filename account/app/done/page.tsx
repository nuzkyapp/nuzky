export default function DonePage() {
  return (
    <main className="flex min-h-dvh justify-center px-5 pt-[18vh] text-center">
      <div>
        <svg
          className="mx-auto text-ok"
          width="26"
          height="26"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2.4"
          aria-hidden
        >
          <path d="M20 6 9 17l-5-5" />
        </svg>
        <h1 className="mt-1.5 text-[15px] font-semibold">You&apos;re signed in</h1>
        <p className="mt-1 text-xs leading-[18px] text-muted">Go back to Nuzky. You can close this tab.</p>
      </div>
    </main>
  );
}
