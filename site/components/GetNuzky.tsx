"use client";

import { Fragment, useState } from "react";
import { ArrowDown, ArrowDownToLine, CircleMinus, Link2, Monitor, Share, Smartphone, TriangleAlert } from "lucide-react";
import { GitHubLogo, systemLogo } from "./Logos";
import { type FileId, type Release, type SystemId, repo, systems, url } from "@/lib/site";

// The visitor's system goes on <html> as `data-os` while the page is still parsing, so the first paint already shows
// the right download and nothing jumps. Elements with `data-show` show only for the systems they list (globals.css);
// without this script the page shows the "unknown" state, whose links all work. Only Chromium can tell an Intel Mac
// or an ARM PC apart, and only asynchronously, so that correction can come a moment later. An iPad and an Android
// tablet asking for the desktop site say Macintosh and Linux, so touch and Chromium's platform give them away.
const detect = `(function (h) {
  try {
    var ua = navigator.userAgent, d = navigator.userAgentData, os = "unknown";
    if (/iPhone|iPad|iPod|Android/i.test(ua) || (d && d.platform === "Android") || (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1)) os = "phone";
    else if (/Windows/.test(ua)) os = "windows";
    else if (/Mac OS X/.test(ua)) os = "mac";
    else if (/CrOS/.test(ua)) os = "unknown";
    else if (/Linux|X11/.test(ua)) os = /aarch64|arm/i.test(ua) ? "arm" : "linux";
    h.dataset.os = os;
    if (d && d.getHighEntropyValues && os !== "phone" && os !== "unknown")
      d.getHighEntropyValues(["architecture"]).then(function (v) {
        if (os === "mac" && v.architecture === "x86") h.dataset.os = "intel";
        if (os !== "mac" && v.architecture === "arm") h.dataset.os = "arm";
      }, function () {});
  } catch (e) {}
})(document.documentElement);`;

const desktops = ["unknown", "mac", "intel", "windows", "linux", "arm"];
const desktop = desktops.join(" ");
const except = (os: string) => desktops.filter((d) => d !== os).join(" ");

const link = "underline decoration-white/30 underline-offset-4 transition-colors hover:text-fg hover:decoration-fg";
const primary = "btn-light flex h-12 w-full items-center justify-center gap-2.5 rounded-full pl-5 pr-6 text-[15px] font-semibold sm:w-auto";
const secondary = "glass flex h-12 w-full items-center justify-center gap-2 rounded-full px-[22px] text-[15px] font-semibold transition-colors hover:bg-white/10 sm:w-auto";

const code = "font-mono text-[12.5px] text-fg";
const install: Record<FileId, (file: string) => React.ReactNode> = {
  dmg: () => "Open it and drag Nuzky to Applications. The first time, macOS asks whether to open an app from the internet. Click Open.",
  exe: () => "Run it. When Windows says “Windows protected your PC”, click More info, then Run anyway.",
  deb: (file) => (
    <>
      Install it with <code className={code}>sudo apt install ./{file}</code>
    </>
  ),
  appimage: (file) => (
    <>
      Make it executable with <code className={code}>chmod +x {file}</code>, then run it.
    </>
  ),
};

// What to do once the browser has the file. A download that only leads to the release page has no step.
function useStep(release: Release) {
  const [step, setStep] = useState<FileId | null>(null);
  const start = (file: FileId) => release.files[file].name && setStep(file);
  const after = (files: FileId[], center: boolean, idle?: React.ReactNode, className = "") => {
    const file = step && files.includes(step) ? step : null;
    const name = file && release.files[file].name;
    return (
      <div aria-live="polite" className={`flex flex-col gap-1.5 ${center ? "min-h-16 items-center" : ""} ${className}`}>
        {file && name ? (
          <>
            <p className="flex items-center gap-2 text-[14px] font-semibold">
              <ArrowDownToLine aria-hidden className="size-4 shrink-0 text-accent" />
              Your download is starting
            </p>
            <p className="max-w-[540px] text-[14px] leading-[1.5] text-muted">{install[file](name)}</p>
            <p className="text-[13px] text-muted [overflow-wrap:anywhere]">
              Didn&apos;t start?{" "}
              <a href={release.files[file].href} className={link}>
                Download {name}
              </a>
            </p>
          </>
        ) : (
          idle
        )}
      </div>
    );
  };
  return { start, after };
}

// A phone can't install Nuzky, so it sends the page to the visitor's computer: the share sheet, or the link copied.
function useSend() {
  const [copied, setCopied] = useState<"" | "done" | "failed">("");
  const page = `${url}/#download`;
  const copy = () =>
    Promise.resolve()
      .then(() => navigator.clipboard.writeText(page))
      .then(
        () => "done" as const,
        () => "failed" as const,
      )
      .then((result) => {
        setCopied(result);
        window.setTimeout(() => setCopied(""), 2500);
      });
  const send = () => {
    if (!navigator.share) return void copy();
    navigator.share({ title: "Nuzky", text: "Nuzky, a free video editor for Mac, Windows and Linux", url: page }).catch((e) => {
      if (e?.name !== "AbortError") void copy();
    });
  };
  const label = copied === "done" ? "Link copied" : copied === "failed" ? "Couldn't copy" : "Copy link";
  return { send, copy, label };
}

function Status({ children, mark }: { mark?: React.ReactNode; children: React.ReactNode }) {
  return (
    <p className="flex flex-wrap items-center justify-center gap-x-1.5 gap-y-0.5 text-[14px] leading-[1.45]">
      {mark}
      {children}
    </p>
  );
}

const Note = ({ children }: { children: React.ReactNode }) => <p className="text-[13px] leading-[1.5] text-muted">{children}</p>;

/** The hero's download: one state per system, picked before the first paint. `base` leads the links to the home page's Download. */
export function HeroDownload({ release, base = "" }: { release: Release; base?: string }) {
  const { start, after } = useStep(release);
  const { send, copy, label } = useSend();
  const file = (id: FileId) => release.files[id].href;
  const version = (
    <a href={release.notes} className={link}>
      {release.version ? `Version ${release.version}` : "Releases on GitHub"}
    </a>
  );
  const also = (id: SystemId) => {
    const s = systems.find((x) => x.id === id)!;
    return (
      <a href={`${base}#dl-${id}`} className={`${link} whitespace-nowrap`}>
        {s.name}
      </a>
    );
  };
  const star = (
    <a href={repo} className={secondary}>
      <GitHubLogo className="size-[17px]" />
      Star on GitHub
    </a>
  );
  const wrap = "mx-auto flex w-full max-w-[680px] flex-col items-center gap-4";
  const buttons = "flex w-full max-w-[400px] flex-col items-center gap-3 sm:max-w-none sm:flex-row sm:justify-center";

  return (
    <div className="w-full">
      <script dangerouslySetInnerHTML={{ __html: detect }} />

      <div data-show="mac" className={wrap}>
        <div className={buttons}>
          <a href={file("dmg")} onClick={() => start("dmg")} className={primary}>
            <systemLogo.mac className="size-[18px] -translate-y-px" />
            Download for Mac
          </a>
          {star}
        </div>
        {after(
          ["dmg"],
          true,
          <>
            <Status>
              <span className="text-muted">For Macs with Apple silicon and macOS 14 or newer</span>
            </Status>
            <Note>
              Also for {also("linux")} and {also("windows")}. {version}
            </Note>
          </>,
        )}
      </div>

      <div data-show="windows" className={wrap}>
        <div className={buttons}>
          <a href={file("exe")} onClick={() => start("exe")} className={primary}>
            <systemLogo.windows className="size-4" />
            Download for Windows
          </a>
          {star}
        </div>
        {after(
          ["exe"],
          true,
          <>
            <Status>
              <span className="text-muted">For 64-bit Intel or AMD PCs with Windows 10 or 11</span>
            </Status>
            <p className="flex max-w-[540px] items-start gap-2 text-left text-[14px] leading-[1.45] text-fg/85">
              <TriangleAlert aria-hidden className="mt-[2px] size-4 shrink-0 text-warn" />
              <span>
                Windows warns you the first time, because the installer isn&apos;t signed yet.{" "}
                <a href={`${base}#dl-windows`} className={link}>
                  What to expect
                </a>
              </span>
            </p>
            <Note>
              Also for {also("mac")} and {also("linux")}. {version}
            </Note>
          </>,
        )}
      </div>

      <div data-show="linux" className={wrap}>
        <div className={buttons}>
          <a href={file("deb")} onClick={() => start("deb")} className={primary}>
            <systemLogo.linux className="size-[17px]" />
            Download .deb
          </a>
          <a href={file("appimage")} onClick={() => start("appimage")} className={secondary}>
            <ArrowDownToLine className="size-4" />
            Download AppImage
          </a>
        </div>
        {after(
          ["deb", "appimage"],
          true,
          <>
            <Status>
              <span className="text-muted">For 64-bit Intel or AMD PCs</span>
            </Status>
            <Note>
              The .deb is for Ubuntu 24.04 or newer, the AppImage for other distributions with glibc 2.39 or newer.
              <br />
              Also for {also("mac")} and {also("windows")}. {version}
            </Note>
          </>,
        )}
      </div>

      <div data-show="phone" className={wrap}>
        <div className={buttons}>
          <button type="button" onClick={send} className={`${primary} cursor-pointer`}>
            <Share className="size-[17px]" />
            Send to your computer
          </button>
          <button type="button" onClick={copy} className={`${secondary} cursor-pointer`}>
            <Link2 className="size-4" />
            <span aria-live="polite">{label}</span>
          </button>
        </div>
        <div className="flex min-h-16 flex-col items-center gap-1.5">
          <Status mark={<Monitor aria-hidden className="size-4 shrink-0 text-muted" />}>
            <span className="font-semibold">Nuzky is a desktop app</span>
            <span className="text-muted">for Mac, Windows and Linux. Send this page to your computer and download it there.</span>
          </Status>
          <Note>
            {also("mac")} · {also("linux")} · {also("windows")}
          </Note>
        </div>
      </div>

      <div data-show="intel arm" className={wrap}>
        <p className="max-w-[480px] text-[17px] leading-[1.45]">
          <span data-show="intel">This Mac has an Intel chip. Nuzky needs a Mac with Apple silicon, M1 or newer.</span>
          <span data-show="arm">This computer has an ARM chip. On Windows and Linux, Nuzky needs a 64-bit Intel or AMD PC.</span>
        </p>
        <div className={buttons}>
          <a href={`${base}#download`} className={secondary}>
            See all downloads
          </a>
          {star}
        </div>
        <div className="flex min-h-16 flex-col items-center gap-1.5">
          <Status mark={<CircleMinus aria-hidden className="size-4 shrink-0 text-muted" />}>
            <span className="font-semibold">Not supported</span>
            <span className="text-muted">Intel Macs, and Windows or Linux on ARM</span>
          </Status>
          <Note>{version}</Note>
        </div>
      </div>

      <div data-show="unknown" className={wrap}>
        <div className="glass flex w-full max-w-[600px] flex-col gap-1 rounded-[22px] p-2 sm:flex-row">
          {systems.map((s) => {
            const Logo = systemLogo[s.id];
            return (
              <a key={s.id} href={`${base}#dl-${s.id}`} className="flex flex-1 items-center gap-3 rounded-2xl px-3.5 py-2.5 text-left transition-colors hover:bg-white/[0.07]">
                <Logo className="size-5 shrink-0" />
                <span className="text-[15px] font-semibold leading-tight">{s.name}</span>
                <ArrowDown aria-hidden className="ml-auto size-4 shrink-0 text-muted" />
              </a>
            );
          })}
        </div>
        <div className="flex min-h-16 flex-col items-center">
          <Note>
            Choose your system. Nuzky doesn&apos;t run on Chromebooks. {version} ·{" "}
            <a href={repo} className={`${link} whitespace-nowrap`}>
              Star on GitHub
            </a>
          </Note>
        </div>
      </div>
    </div>
  );
}

const rowButton = "flex h-10 items-center justify-center gap-2 rounded-full px-4 text-[14px] font-semibold transition-colors";

/** Every system with its requirements and files. The visitor's row says so and leads with the white button. */
export function DownloadRows({ release }: { release: Release }) {
  const { start, after } = useStep(release);
  const { send, copy, label } = useSend();

  return (
    <div className="glass rounded-[28px] bg-[rgb(22_22_25/0.55)] p-2">
      <div data-show="phone" className="@container mb-2 rounded-[20px] bg-white/[0.05] p-4 ring-1 ring-inset ring-white/[0.08]">
        <div className="flex flex-col gap-3 @xl:flex-row @xl:items-center">
          <p className="flex flex-1 items-start gap-3 text-[14px] leading-[1.5]">
            <Smartphone aria-hidden className="mt-[2px] size-[18px] shrink-0 text-muted" />
            <span>
              <span className="font-semibold">You&apos;re on a phone or tablet.</span>{" "}
              <span className="text-muted">Nuzky runs on computers, so send this page to yours and download it there.</span>
            </span>
          </p>
          <div className="flex gap-2">
            <button type="button" onClick={send} className={`btn-light ${rowButton} flex-1 cursor-pointer`}>
              <Share className="size-[15px]" />
              Send
            </button>
            <button type="button" onClick={copy} className={`glass hover:bg-white/10 ${rowButton} flex-1 cursor-pointer`}>
              <Link2 className="size-[15px]" />
              <span aria-live="polite">{label}</span>
            </button>
          </div>
        </div>
      </div>

      {systems.map((s, i) => {
        const Logo = systemLogo[s.id];
        return (
          <Fragment key={s.id}>
            {i > 0 && <div className="mx-5 h-px bg-white/[0.07]" />}
            <div id={`dl-${s.id}`} className="@container relative scroll-mt-28 rounded-[20px] p-5 @3xl:px-6">
              <span aria-hidden data-show={s.id} className="absolute inset-0 rounded-[20px] bg-white/[0.05] ring-1 ring-inset ring-accent/45" />
              {s.id === "mac" && <span aria-hidden data-show="intel" className="absolute inset-0 rounded-[20px] ring-1 ring-inset ring-white/[0.12]" />}
              <div className="relative grid gap-4 @3xl:grid-cols-[minmax(0,1fr)_minmax(0,1.3fr)_208px] @3xl:items-start @3xl:gap-8">
                <div className="flex items-start gap-3.5">
                  <Logo className="mt-[3px] size-[22px] shrink-0" />
                  <div className="flex flex-col gap-1">
                    <h3 className="flex flex-wrap items-baseline gap-x-2.5 text-[17px] font-semibold tracking-[-0.01em]">
                      {s.name}
                      <span data-show={s.id} className="text-[12px] font-medium tracking-normal text-accent">
                        Your computer
                      </span>
                      {s.id === "mac" && (
                        <span data-show="intel" className="text-[12px] font-medium tracking-normal text-muted">
                          Not for this Mac
                        </span>
                      )}
                    </h3>
                    <span className="text-[14px] leading-[1.45] text-muted">{s.requirement}</span>
                  </div>
                </div>

                <div className="flex flex-col gap-1.5 @3xl:pt-[3px]">
                  {s.note && <p className="text-[14px] leading-[1.5] text-muted">{s.note}</p>}
                  {s.warnings?.map((w) => (
                    <p key={w} className="flex items-start gap-2 text-[14px] leading-[1.5] text-fg/85">
                      <TriangleAlert aria-hidden className="mt-[3px] size-4 shrink-0 text-warn" />
                      <span>{w}</span>
                    </p>
                  ))}
                  {s.id === "mac" && (
                    <p data-show="intel" className="flex items-start gap-2 text-[14px] leading-[1.5] text-fg/85">
                      <CircleMinus aria-hidden className="mt-[3px] size-4 shrink-0 text-muted" />
                      <span>This Mac has an Intel chip, so this download won&apos;t run on it.</span>
                    </p>
                  )}
                </div>

                <div data-show={desktop} className="flex flex-col gap-3">
                  {s.files.map((f, k) => {
                    const href = release.files[f.id].href;
                    const content = (
                      <>
                        <ArrowDownToLine className="size-[15px]" />
                        {f.label}
                      </>
                    );
                    return (
                      <div key={f.id} className="flex flex-col gap-1.5">
                        {k === 0 && (
                          <a href={href} data-show={s.id} onClick={() => start(f.id)} className={`btn-light ${rowButton}`}>
                            {content}
                          </a>
                        )}
                        <a href={href} data-show={k === 0 ? except(s.id) : undefined} onClick={() => start(f.id)} className={`glass hover:bg-white/10 ${rowButton}`}>
                          {content}
                        </a>
                        <span className="text-center text-[12px] leading-[1.4] text-muted">{f.what}</span>
                      </div>
                    );
                  })}
                </div>
                <ul data-show="phone" className="flex flex-col gap-1 text-[13px] leading-[1.45] text-muted">
                  {s.files.map((f) => (
                    <li key={f.id}>{f.phone}</li>
                  ))}
                </ul>
              </div>
              {/* Empty until a download starts, so it takes no room; the step then gets the row's whole width. */}
              {after(
                s.files.map((f) => f.id),
                false,
                undefined,
                "relative pl-9 [&:not(:empty)]:mt-5",
              )}
            </div>
          </Fragment>
        );
      })}
    </div>
  );
}
