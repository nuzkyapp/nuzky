"use client";

import Image from "next/image";
import { useEffect, useRef, useState } from "react";
import { CircleCheck, Sparkles } from "lucide-react";
import { AgentPanel, Run, doneStage, exchanges } from "./AgentPanel";
import { ClaudeCodeLogo, OpenAILogo } from "./Logos";
import { SectionHeading } from "./SectionHeading";
import { usePlaying } from "@/lib/playing";

const strip = [1, 2, 3, 4, 5, 6, 7];
const TYPING = -1;

// Plays the exchanges forever while the section is on screen: the user types, the agent replies,
// works through its steps and reports, then the next request after a short look at the result.
// `n` only ever grows, so after the last exchange the first one is simply asked again. It starts
// one lap in, on a finished first run under the rest of the conversation, so the panel is full from
// the first frame; that is also what the server renders and what reduced motion keeps.
const FIRST = exchanges.length;

function useAgentLoop(active: boolean) {
  const [n, setN] = useState(FIRST);
  const [stage, setStage] = useState(doneStage(exchanges[0]));
  const [typed, setTyped] = useState(0);

  useEffect(() => {
    if (!active) return;
    const x = exchanges[n % exchanges.length];
    let id: number;
    if (stage === TYPING) {
      id =
        typed < x.ask.length
          ? window.setTimeout(() => setTyped((t) => t + 1), 24 + ((typed * 37) % 40))
          : window.setTimeout(() => {
              setTyped(0);
              setStage(0);
            }, 450);
    } else if (stage < doneStage(x)) {
      id = window.setTimeout(() => setStage((s) => s + 1), stage === 0 ? 700 : 850);
    } else {
      id = window.setTimeout(
        () => {
          setN((k) => k + 1);
          setStage(TYPING);
        },
        n === FIRST ? 600 : 1600,
      );
    }
    return () => window.clearTimeout(id);
  }, [active, n, stage, typed]);

  return { n, stage, typed };
}

export function Agent() {
  const section = useRef<HTMLElement>(null);
  const { n, stage, typed } = useAgentLoop(usePlaying(section, 0.3));
  const at = (k: number) => exchanges[k % exchanges.length];
  const current = at(n);
  const working = stage >= 0 && stage < doneStage(current);
  // Enough finished runs to overfill the panel even when the short ones come together, so the
  // oldest one leaves the DOM well above the visible part.
  const history = [n - 4, n - 3, n - 2, n - 1];
  const lit = working ? ((n * 3 + stage) % strip.length) + 1 : 4;

  return (
    <section id="agents" ref={section} className="mx-auto w-full max-w-[1240px] scroll-mt-10 px-5 pt-32 sm:pt-[180px]">
      <SectionHeading center first="Or hand the cut to ChatGPT or Claude." second="You watch, and one undo takes it all back." />
      <div className="mt-10 flex flex-col gap-4 lg:h-[580px] lg:flex-row">
        <AgentPanel
          className="h-[580px] lg:h-auto lg:w-[440px] lg:shrink-0"
          draft={stage === TYPING ? current.ask.slice(0, typed) : ""}
          working={working ? `0:${String(stage + 1).padStart(2, "0")}` : undefined}
        >
          {history.map((k) => (
            <Run key={k} x={at(k)} stage={doneStage(at(k))} latest={k === n - 1 && stage === TYPING} />
          ))}
          {stage !== TYPING && <Run key={n} x={current} stage={stage} />}
        </AgentPanel>
        <div className="flex min-w-0 flex-1 flex-col gap-4">
          <div className="surface flex flex-1 flex-col justify-center gap-5 rounded-[20px] p-7">
            <div className="capsule flex h-9 w-fit max-w-full items-center gap-2 rounded-full pl-3.5 pr-1.5 text-[13px] font-medium">
              {working ? (
                <>
                  <Sparkles className="size-3.5 shrink-0 text-accent" />
                  <span className="truncate">
                    AI is editing <span className="font-normal text-muted">· {current.title}</span>
                  </span>
                  <span className="flex h-[26px] shrink-0 items-center rounded-full bg-white/[0.09] px-2.5 text-[12px]">Stop and edit</span>
                </>
              ) : (
                <>
                  <CircleCheck className="size-3.5 text-ok" />
                  <span className="pr-2">Your turn</span>
                </>
              )}
            </div>
            <div className="flex h-14 gap-0.5">
              {strip.map((i) => (
                <div
                  key={i}
                  className={`relative flex-1 overflow-hidden rounded-[7px] transition-shadow duration-200 ${i === lit ? "ring-2 ring-accent" : "ring-1 ring-inset ring-white/10"}`}
                >
                  <Image src={`/media/strip-${i}.jpg`} alt="" fill sizes="(min-width: 1024px) 100px, 14vw" className="object-cover" />
                </div>
              ))}
            </div>
            <div className="flex max-w-[460px] flex-col gap-2 text-[14px] leading-[1.5] text-muted">
              <p>Every change goes through the same undo as yours. Your controls lock while it works and come back when it stops.</p>
              <p>Write to it while it works, attach a screenshot or a video, pick the model, and come back to past chats.</p>
            </div>
          </div>
          <div className="surface flex flex-col gap-4 rounded-[20px] px-7 py-6">
            <span className="text-[15px] font-semibold">Connect your AI</span>
            {/* A picture of the app's sign-in, in its blue, so nobody takes it for a button on this page. */}
            <div aria-hidden className="flex flex-col gap-3 rounded-[14px] bg-panel p-4 ring-1 ring-inset ring-white/[0.07] sm:flex-row sm:items-center">
              <span className="flex flex-1 items-center gap-3">
                <span className="flex size-9 shrink-0 items-center justify-center rounded-full bg-white/[0.07]">
                  <OpenAILogo className="size-[18px]" />
                </span>
                <span className="flex flex-col">
                  <span className="text-[13.5px] font-semibold">ChatGPT</span>
                  <span className="text-[12px] text-muted">Uses your ChatGPT plan</span>
                </span>
              </span>
              <span className="flex h-8 items-center justify-center rounded-[8px] bg-accent-strong px-3.5 text-[12.5px] font-medium text-white">Continue with ChatGPT</span>
            </div>
            <div className="flex flex-wrap items-center gap-x-4 gap-y-2 text-[13px]">
              <span className="text-muted">Already use one of these?</span>
              <span className="flex items-center gap-1.5 font-medium">
                <ClaudeCodeLogo className="size-4 text-claude" />
                Claude Code
              </span>
              <span className="flex items-center gap-1.5 font-medium">
                <OpenAILogo className="size-4" />
                Codex
              </span>
            </div>
            <p className="text-[13px] leading-[1.5] text-muted">
              Sign in with ChatGPT inside Nuzky. No terminal, and no credits to buy. AI editing doesn&apos;t work on Windows yet.
            </p>
          </div>
        </div>
      </div>
    </section>
  );
}
