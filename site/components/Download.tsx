import Image from "next/image";
import { CircleMinus, RefreshCw } from "lucide-react";
import { DownloadRows } from "./GetNuzky";
import { Waves } from "./Waves";
import { type Release, bugReport, buildFromSource, license, releases } from "@/lib/site";

const link = "underline decoration-white/30 underline-offset-4 transition-colors hover:text-fg hover:decoration-fg";

export function Download({ release }: { release: Release }) {
  const links = [
    ["Release notes", release.notes],
    ["Checksums", release.checksums],
    ["All releases", releases],
    ["Build from source", buildFromSource],
    ["GPL-3.0", license],
  ];
  return (
    <section id="download" className="relative mt-32 overflow-hidden pb-28 pt-[120px] sm:mt-0">
      <Waves height={0.5} intensity={1} />
      <div className="relative z-10 mx-auto flex max-w-[960px] flex-col items-center gap-10 px-5">
        <div className="flex flex-col items-center gap-5 text-center">
          <Image src="/media/icon.png" alt="Nuzky app icon" width={120} height={120} />
          <h2 className="text-[44px] font-semibold leading-none tracking-[-0.04em] sm:text-[56px]">Download Nuzky.</h2>
          <p className="max-w-[600px] text-[17px] leading-[1.5] text-muted">
            {release.version && `Version ${release.version}. `}It&apos;s early, so expect rough edges, and{" "}
            <a href={bugReport} className={link}>
              tell me about them
            </a>
            .
          </p>
        </div>

        <div className="w-full">
          <DownloadRows release={release} />
        </div>

        <div className="flex flex-col items-center gap-2 text-center text-[14px] leading-[1.5] text-muted">
          <p className="flex items-start gap-2">
            <RefreshCw aria-hidden className="mt-[3px] size-4 shrink-0" />
            Once it&apos;s installed, new versions install from inside Nuzky in one click.
          </p>
          <p className="flex items-start gap-2">
            <CircleMinus aria-hidden className="mt-[3px] size-4 shrink-0" />
            Intel Macs, and Windows or Linux on ARM, aren&apos;t supported.
          </p>
        </div>

        {/* A phone wraps the row, so it drops the dots instead of starting a line with one. */}
        <p className="flex flex-wrap items-center justify-center gap-x-5 gap-y-1 text-[13px] text-muted sm:gap-x-2.5">
          {links.map(([label, href], i) => (
            <span key={label} className="flex items-center gap-x-2.5">
              {i > 0 && (
                <span aria-hidden className="max-sm:hidden">
                  ·
                </span>
              )}
              <a href={href} className="transition-colors hover:text-fg">
                {label}
              </a>
            </span>
          ))}
        </p>
      </div>
    </section>
  );
}
