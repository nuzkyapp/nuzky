import type { ReactNode } from "react";

const ITEM = /^\s*([-*•]|\d+[.)])\s+/;

/**
 * The little Markdown agents write: paragraphs, lists, code blocks, headings, bold, italics and
 * inline code. Everything is drawn as React text, so nothing the agent writes becomes HTML.
 */
export function Markdown({ text }: { text: string }) {
  const blocks: ReactNode[] = [];
  const lines = text.replace(/\r\n/g, "\n").split("\n");
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.trim() === "") {
      i++;
    } else if (line.startsWith("```")) {
      const code: string[] = [];
      for (i++; i < lines.length && !lines[i].startsWith("```"); i++) code.push(lines[i]);
      i++;
      blocks.push(
        <pre key={blocks.length} className="overflow-x-auto rounded-md bg-white/[.06] px-2 py-1.5 text-[12px] leading-[18px]">
          {code.join("\n")}
        </pre>,
      );
    } else if (ITEM.test(line)) {
      const ordered = /^\s*\d/.test(line);
      const items: string[] = [];
      let indent = 0;
      while (i < lines.length) {
        const marker = ITEM.exec(lines[i]);
        if (marker) {
          indent = marker[0].length;
          items.push(lines[i++].slice(indent));
        } else if (/^\s+\S/.test(lines[i])) {
          // A wrapped line joins the item's text; a line indented past it is code and keeps its line and indent.
          const deeper = lines[i].search(/\S/) > indent;
          items[items.length - 1] += deeper ? "\n" + lines[i++].slice(indent) : " " + lines[i++].trim();
        } else {
          // Agents write loose lists, with a blank line between items.
          let next = i;
          while (next < lines.length && lines[next].trim() === "") next++;
          if (next === i || next === lines.length || !ITEM.test(lines[next]) || /^\s*\d/.test(lines[next]) !== ordered) break;
          i = next;
        }
      }
      const List = ordered ? "ol" : "ul";
      blocks.push(
        <List
          key={blocks.length}
          start={ordered ? parseInt(line, 10) : undefined}
          className={`flex flex-col gap-1 pl-5 ${ordered ? "list-decimal" : "list-disc"} marker:text-muted`}
        >
          {items.map((t, k) => (
            <li key={k} className="whitespace-pre-wrap">
              {inline(t)}
            </li>
          ))}
        </List>,
      );
    } else {
      const para: string[] = [];
      for (; i < lines.length && lines[i].trim() !== "" && !lines[i].startsWith("```") && !ITEM.test(lines[i]); i++) para.push(lines[i]);
      const heading = /^#{1,6}\s+/.test(para[0]);
      const body = para.map((p) => p.replace(/^#{1,6}\s+/, "")).join("\n");
      blocks.push(
        <p key={blocks.length} className={`whitespace-pre-wrap ${heading ? "font-semibold text-fg" : ""}`}>
          {inline(body)}
        </p>,
      );
    }
  }
  return <div className="flex flex-col gap-2">{blocks}</div>;
}

function inline(text: string): ReactNode[] {
  const out: ReactNode[] = [];
  const re = /(`[^`]+`|\*\*[^*]+\*\*|__[^_]+__|\*[^*\s][^*]*\*|_[^_\s][^_]*_)/g;
  let last = 0;
  for (const m of text.matchAll(re)) {
    if (m.index! > last) out.push(text.slice(last, m.index));
    const t = m[0];
    if (t.startsWith("`")) out.push(<code key={out.length} className="rounded bg-white/[.08] px-1 text-[12px]">{t.slice(1, -1)}</code>);
    else if (t.startsWith("**") || t.startsWith("__")) out.push(<strong key={out.length} className="font-semibold text-fg">{t.slice(2, -2)}</strong>);
    else out.push(<em key={out.length}>{t.slice(1, -1)}</em>);
    last = m.index! + t.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}
