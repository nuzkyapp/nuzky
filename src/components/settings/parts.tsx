import type { ReactNode } from "react";

/** A titled group of settings, as in Connect your agent: rows on white 5.5 % with a white 6 % inner line. */
export function Group({ title, id, children, note }: { title?: string; id?: string; children: ReactNode; note?: ReactNode }) {
  return (
    <section data-setting={id} aria-label={title} className="flex flex-col gap-2 rounded-[14px]">
      {title && <h3 className="text-[13px] font-semibold text-fg">{title}</h3>}
      <div className="flex flex-col rounded-xl bg-white/[.055] shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)]">{children}</div>
      {note && <p className="text-[12px] text-muted">{note}</p>}
    </section>
  );
}

/** One setting: its name over what it does or how it is now, and its control at the right. */
export function Item({ title, detail, id, children }: { title: ReactNode; detail?: ReactNode; id?: string; children?: ReactNode }) {
  return (
    <div data-setting={id} className="flex min-h-[44px] items-center gap-3 border-t border-white/[.06] px-3.5 py-2.5 first:border-t-0">
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="text-[13px] font-medium text-fg">{title}</span>
        {detail && <span className="text-[12px] text-muted">{detail}</span>}
      </div>
      {children}
    </div>
  );
}

export const formatBytes = (bytes: number) =>
  bytes >= 1e9 ? `${(bytes / 1e9).toFixed(1)} GB` : bytes >= 1e6 ? `${Math.round(bytes / 1e6)} MB` : bytes >= 1e3 ? `${Math.round(bytes / 1e3)} KB` : `${bytes} bytes`;
