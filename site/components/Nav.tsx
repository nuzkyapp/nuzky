import Image from "next/image";
import { ArrowDownToLine } from "lucide-react";
import { GitHubLogo } from "./Logos";
import { repo } from "@/lib/site";

const links = [
  ["CapCut drafts", "#capcut"],
  ["AI agents", "#agents"],
  ["Editor", "#editor"],
  ["Features", "#features"],
  ["Download", "#download"],
] as const;

// `base` is "/" on other pages, so the section links lead back to the home page. On the home page they stay
// bare fragments, which scroll without reloading even when the address carries a query string.
export function Nav({ base = "" }: { base?: string }) {
  return (
    <header className="relative z-20 mx-auto flex h-[72px] w-full max-w-[1440px] items-center justify-between px-5 sm:px-10">
      <a href={`${base}#top`} className="flex items-center gap-2.5 rounded-lg lg:w-60" aria-label="Nuzky home">
        <Image src="/media/icon.png" alt="" width={34} height={34} loading="eager" />
        <span className="text-[16px] font-semibold tracking-[-0.01em]">Nuzky</span>
      </a>
      <nav aria-label="Sections" className="hidden items-center gap-8 lg:flex">
        {links.map(([label, href]) => (
          <a key={href} href={base + href} className="text-[14px] text-muted transition-colors hover:text-fg">
            {label}
          </a>
        ))}
      </nav>
      <div className="flex items-center justify-end gap-5 lg:w-60">
        <a href={repo} className="hidden items-center gap-1.5 text-[14px] text-muted transition-colors hover:text-fg sm:flex">
          <GitHubLogo className="size-4" />
          GitHub
        </a>
        <a href={`${base}#download`} className="btn-light flex h-[34px] items-center gap-1.5 rounded-full pl-3.5 pr-4 text-[13px] font-semibold">
          <ArrowDownToLine className="size-[14px]" />
          Download
        </a>
      </div>
    </header>
  );
}
