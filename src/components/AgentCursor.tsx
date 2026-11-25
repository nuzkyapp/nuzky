import { Sparkles } from "lucide-react";
import { type ReactNode, useLayoutEffect, useRef } from "react";
import { CLAY, GEMINI, type Glyph, attachCursor, useAgentCursor } from "../lib/agentCursor";
import { ChatGptLogo } from "./ChatGptLogo";

const OUTLINE = "#0a0a0b";

const SHAPES: Record<Exclude<Glyph, "arrow">, ReactNode> = {
  scissors: (
    <>
      <circle cx="6" cy="6" r="3" />
      <circle cx="6" cy="18" r="3" />
      <path d="M8.12 8.12 12 12M20 4 8.12 15.88M14.8 14.8 20 20" />
    </>
  ),
  resize: <path d="M12 5v14M7.5 8.5 4 12l3.5 3.5M16.5 8.5 20 12l-3.5 3.5" />,
  hand: <path d="M18 11V6a2 2 0 0 0-4 0M14 10V4a2 2 0 0 0-4 0v2M10 10.5V6a2 2 0 0 0-4 0v8M18 8a2 2 0 1 1 4 0v6a8 8 0 0 1-8 8h-2c-2.8 0-4.5-.86-6-2.34l-3.6-3.6a2 2 0 0 1 2.83-2.82L7 15" />,
  grab: <path d="M18 11.5V9a2 2 0 0 0-4 0v1.4M14 10V8a2 2 0 0 0-4 0v2M10 9.9V9a2 2 0 0 0-4 0v5M6 14a2 2 0 0 0-4 0M18 11a2 2 0 1 1 4 0v3a8 8 0 0 1-8 8h-4a8 8 0 0 1-8-8 2 2 0 1 1 4 0" />,
};

/** The pointer in the agent's colour with a dark edge, so it reads over bright footage and dark panes alike. */
function Pointer({ kind, color }: { kind: Glyph; color: string }) {
  if (kind === "arrow") {
    return (
      <svg width="20" height="26" viewBox="0 0 20 26" style={{ left: -2, top: -2 }}>
        <path d="M2 2 L2 20.5 L6.6 16.2 L9.7 23.4 L12.9 22 L9.9 15 L16.2 15 Z" fill={color} stroke={OUTLINE} strokeWidth="1.5" strokeLinejoin="round" />
      </svg>
    );
  }
  return (
    <svg width="24" height="24" viewBox="0 0 24 24" style={{ left: -12, top: -12 }} fill="none" strokeLinecap="round" strokeLinejoin="round">
      {/* A thinner edge on the hands, so the dark does not fill the gaps between the fingers. */}
      <g stroke={OUTLINE} strokeWidth={kind === "hand" || kind === "grab" ? 3.6 : 4.6}>
        {SHAPES[kind]}
      </g>
      <g stroke={color} strokeWidth="2.2">{SHAPES[kind]}</g>
    </svg>
  );
}

function Logo({ name }: { name: string }) {
  if (name === "Claude Code") {
    return (
      <svg width="13" height="13" viewBox="0 0 24 24" className="shrink-0">
        <path d="M12 3v18M3 12h18M5.6 5.6l12.8 12.8M18.4 5.6 5.6 18.4" stroke={CLAY} strokeWidth="2.6" strokeLinecap="round" />
      </svg>
    );
  }
  if (name === "Codex" || name === "ChatGPT") return <ChatGptLogo size={13} className="text-fg" />;
  if (name === "Gemini") {
    return (
      <svg width="13" height="13" viewBox="0 0 24 24" className="shrink-0">
        <path fill={GEMINI} d="M12 2c.6 5.4 4.6 9.4 10 10-5.4.6-9.4 4.6-10 10-.6-5.4-4.6-9.4-10-10 5.4-.6 9.4-4.6 10-10z" />
      </svg>
    );
  }
  return <Sparkles size={13} className="shrink-0 text-muted" />;
}

/** The agent's pointer and the marks it leaves; src/lib/agentCursor.ts moves them, so nothing here renders per frame. */
export function AgentCursor() {
  const { shown, name, color, step, glyph, label: place } = useAgentCursor();
  const fx = useRef<HTMLDivElement>(null);
  const root = useRef<HTMLDivElement>(null);
  const body = useRef<HTMLDivElement>(null);
  const pointer = useRef<HTMLDivElement>(null);
  const label = useRef<HTMLDivElement>(null);
  const text = useRef<HTMLSpanElement>(null);

  useLayoutEffect(() => {
    attachCursor({ root: root.current!, body: body.current!, glyph: pointer.current!, fx: fx.current! });
    return () => attachCursor(null);
  }, []);

  // The label grows and shrinks to its words, so a new step never jumps.
  useLayoutEffect(() => {
    if (label.current && text.current) label.current.style.width = `${text.current.offsetWidth + 16}px`;
  }, [name, step]);

  return (
    <div aria-hidden className="pointer-events-none fixed inset-0 z-[82]" style={{ visibility: shown ? "visible" : "hidden" }} data-agent-cursor={shown ? name : undefined}>
      <div ref={fx} className="absolute inset-0" />
      <div ref={root} className="absolute left-0 top-0 will-change-transform">
        <div ref={body} className="origin-top-left">
          <div ref={pointer} className="agent-pointer">
            <Pointer kind={glyph} color={color} />
          </div>
          <div
            ref={label}
            className="agent-label"
            style={{ left: place.left ? -6 : 15, top: place.dy, transform: place.left ? "translateX(-100%)" : undefined }}
          >
            <span ref={text} className="inline-flex items-center gap-1.5">
              <Logo name={name} />
              <span className="font-medium text-fg">{name}</span>
              {step && (
                <span key={step} className="agent-step text-muted">
                  · {step}
                </span>
              )}
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}
