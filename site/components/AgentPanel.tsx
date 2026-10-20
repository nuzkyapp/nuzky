import { ArrowUp, Camera, Check, ChevronDown, ChevronRight, CircleCheck, CircleDot, CircleStop, Clapperboard, Film, Loader, MessagesSquare, Plus, SquarePen, Undo2, X } from "lucide-react";
import { OpenAILogo } from "./Logos";

export type Exchange = {
  ask: string;
  /** The chips that went with the message. */
  context?: string;
  reply: string;
  /** Each tool call as the app titles it, with what it shows beside it: how many times, or which file. */
  steps: [string, string?][];
  /** The run the agent opened: its name, which the top bar shows while it edits and the card after, and what changed,
      as the app sums it up from the project before and after. A line with a time leads there. */
  run?: { title: string; changes: [string, string?][]; recap?: boolean };
  /** An export runs after its run has ended, so it gets no card, and the agent says where the file is. */
  outro?: string;
};

export const exchanges: Exchange[] = [
  {
    ask: "Cut the ums and long pauses, then add captions in the Spotlight style.",
    reply: "On it. I'll transcribe first, then cut.",
    steps: [["Transcribing"], ["Finding filler words"], ["Cutting by the transcript"], ["Adding captions"]],
    run: {
      title: "Cut the ums",
      changes: [["Shortened the video from 31.2 s to 24.6 s"], ["Cut out 20 passages, 6.6 s in total"], ["Added 9 captions", "00:00"]],
      recap: true,
    },
  },
  {
    ask: "The intro drags. Start on the first thing I say.",
    context: "1 clip · Playhead 00:00",
    reply: "Trimming the quiet start of the first clip.",
    steps: [["Read the transcript"], ["Editing", "1 change"]],
    run: { title: "Tighten the start", changes: [["Shortened the video from 24.6 s to 22.2 s"], ["Cut out 1 passage, 2.4 s in total"]], recap: true },
  },
  {
    ask: "Put Sunrise under the whole video, quietly.",
    reply: "Adding it under everything at −18 dB, with a fade at the end.",
    steps: [["Importing media", "Sunrise.m4a"], ["Editing", "3 changes"]],
    run: { title: "Add music", changes: [["Added Sunrise.m4a on an audio track", "00:00"]] },
  },
  {
    ask: "The caption at 0:07 should say “ridge”, not “bridge”.",
    context: "Playhead 00:07",
    reply: "Fixing the word in the transcript.",
    steps: [["Read the transcript"], ["Correcting words"]],
    run: { title: "Fix a caption", changes: [["Corrected 1 word"]] },
  },
  {
    ask: "Looks good. Export it for Reels.",
    reply: "Exporting 1080 × 1920 at 30 fps.",
    steps: [["Exporting", "morning-run.mp4"]],
    outro: "Done. It's in Movies as morning-run.mp4, 22 s and 9.8 MB.",
  },
];

/** Stages of one exchange: 0 asked, 1 replied, 2 + k running step k, `doneStage` finished. */
export const doneStage = (x: Exchange) => 2 + x.steps.length;

const appear = "animate-[fade-up_240ms_ease-out]";
const button = "flex h-7 w-fit items-center gap-[5px] rounded-lg bg-white/[0.09] px-2.5 text-[12px] font-medium ring-1 ring-inset ring-white/[0.07]";

/** `past`: an earlier exchange, so its steps are folded and later runs keep its Undo off, as in the app. */
export function Run({ x, stage, past = false }: { x: Exchange; stage: number; past?: boolean }) {
  const running = stage - 2;
  const done = stage >= doneStage(x);
  return (
    <>
      <div className={`flex shrink-0 flex-col items-end gap-1 ${appear}`}>
        <p className="max-w-[290px] rounded-[14px] rounded-br-[4px] bg-raised px-3.5 py-2.5 text-[13px] leading-[1.5]">{x.ask}</p>
        {x.context && <span className="text-[11px] text-muted">{x.context}</span>}
      </div>
      {stage >= 1 && <p className={`shrink-0 text-[13px] leading-[1.5] text-fg/90 ${appear}`}>{x.reply}</p>}
      {stage >= 2 &&
        (past || (done && x.outro) ? (
          <span className="-ml-1 flex shrink-0 items-center gap-1 px-1 text-[12px] text-muted">
            <ChevronRight className="size-3.5" />
            {x.steps.length === 1 ? "1 step" : `${x.steps.length} steps`}
          </span>
        ) : (
          <div className={`shrink-0 border-y border-white/[0.06] py-1.5 ${appear}`}>
            {x.steps.slice(0, running + 1).map(([t, d], j) => (
              <div key={t} className={`flex h-[26px] items-center gap-2 text-[12px] ${appear}`}>
                {j < running ? <Check className="size-3.5 text-ok" /> : <Loader className="size-3.5 animate-[spin_1.6s_linear_infinite] text-accent" />}
                <span className={`flex-1 ${j < running ? "text-fg/80" : ""}`}>{t}</span>
                {d && <span className="tabular-nums text-muted">{d}</span>}
              </div>
            ))}
          </div>
        ))}
      {done && x.outro && <p className={`shrink-0 text-[13px] leading-[1.5] text-fg/90 ${appear}`}>{x.outro}</p>}
      {done && x.run && (
        <div className={`flex shrink-0 flex-col gap-1.5 rounded-[10px] bg-raised p-3 ${appear}`}>
          <div className="flex items-center gap-2">
            <CircleCheck className="size-4 shrink-0 text-ok" />
            <span className="flex-1 truncate text-[13px] font-semibold">{x.run.title}</span>
            <span className={`${button} ${past ? "opacity-45" : ""}`}>
              <Undo2 className="size-[13px]" />
              Undo
            </span>
          </div>
          {x.run.changes.map(([text, at]) => (
            <div key={text} className="flex py-1 pl-[24px] text-[12px] leading-[17px] text-fg/90">
              <span className="flex-1">{text}</span>
              {at && <span className="tabular-nums text-muted">{at}</span>}
            </div>
          ))}
          {x.run.recap && (
            <span className={`ml-6 ${button}`}>
              <Clapperboard className="size-[13px]" />
              Export recap…
            </span>
          )}
        </div>
      )}
    </>
  );
}

// The app's AI panel (DESIGN.md, AI panel), signed in with ChatGPT.
export function AgentPanel({
  className = "",
  draft = "",
  working,
  children,
}: {
  className?: string;
  /** Text being typed into the composer. */
  draft?: string;
  /** While the agent works, how long it has been at it: "0:03". */
  working?: string;
  children: React.ReactNode;
}) {
  return (
    <div className={`flex flex-col overflow-hidden rounded-[20px] bg-panel text-left ring-1 ring-inset ring-white/[0.07] ${className}`}>
      <div className="flex h-[52px] shrink-0 items-center justify-between border-b border-white/[0.07] px-4">
        <span className="flex items-center gap-1.5 text-[13px] font-semibold">
          <OpenAILogo className="size-3.5" />
          ChatGPT <ChevronDown className="size-3.5 text-muted" />
        </span>
        <span className="flex gap-3.5 text-muted">
          <MessagesSquare className="size-3.5" />
          <SquarePen className="size-3.5" />
          <X className="size-3.5" />
        </span>
      </div>
      {/* New messages land at the bottom and push the older ones up and out under a soft fade. */}
      <div className="flex min-h-0 flex-1 flex-col justify-end gap-4 overflow-hidden p-5 [mask-image:linear-gradient(to_bottom,transparent,black_64px)]">{children}</div>
      <div className="p-4 pt-0">
        <div className={`flex flex-col gap-2.5 rounded-xl bg-white/[0.055] p-2.5 ring-1 ring-inset transition-shadow ${draft || working ? "ring-accent/60" : "ring-white/[0.08]"}`}>
          <div className="flex flex-wrap gap-1.5">
            <span className="flex h-6 items-center gap-1.5 rounded-md bg-white/[0.06] px-2 text-[11.5px]">
              <Film className="size-3 text-muted" />1 clip <span className="text-muted">· 00:00–00:08</span>
              <X className="size-3 text-muted" />
            </span>
            <span className="flex h-6 items-center gap-1.5 rounded-md bg-white/[0.06] px-2 text-[11.5px]">
              <CircleDot className="size-3 text-muted" />
              Playhead 00:00
              <X className="size-3 text-muted" />
            </span>
          </div>
          <span className="min-h-5 px-1 text-[13px] leading-[1.45]">
            {draft ? (
              <>
                {draft}
                <span className="ml-px inline-block h-[15px] w-px translate-y-[3px] bg-fg" />
              </>
            ) : (
              <span className="text-muted">{working ? "Tell ChatGPT more while it works…" : "Ask for changes…"}</span>
            )}
          </span>
          <div className="flex h-7 items-center gap-3 px-1 text-muted">
            <Plus className="size-4" />
            <Camera className="size-4" />
            {working ? (
              <>
                <span className="ml-auto flex items-center gap-1.5 text-[12px] tabular-nums">
                  <Loader className="size-3.5 animate-[spin_1.6s_linear_infinite] text-accent" />
                  Working · {working}
                </span>
                <span className={`${button} text-fg`}>
                  <CircleStop className="size-[13px]" />
                  Stop
                </span>
              </>
            ) : (
              <>
                {/* The model the account lists first, at the effort picked in the menu. */}
                <span className="-ml-1.5 flex h-7 items-center gap-1 px-1.5 text-[12px] text-fg/85">
                  GPT 6.1 Sol <span className="text-subtle">·</span> High
                  <ChevronDown className="size-[13px] text-muted" />
                </span>
                <span className={`ml-auto flex size-7 shrink-0 items-center justify-center rounded-full transition-colors ${draft ? "bg-fg text-bg" : "bg-white/[0.08] text-muted"}`}>
                  <ArrowUp className="size-3.5" />
                </span>
              </>
            )}
          </div>
        </div>
        <p className="mt-1.5 flex items-center gap-1.5 px-1 text-[12px] text-muted">
          <OpenAILogo className="size-3" />
          Using ChatGPT plan <span aria-hidden>·</span> <span className="text-accent">Manage usage</span>
        </p>
      </div>
    </div>
  );
}
