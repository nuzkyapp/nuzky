import { Fragment, type ReactNode } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { useEditor } from "../../lib/store";

const ITEM = /^\s*([-*•]|\d+[.)])\s+/;

/**
 * The little Markdown agents write: paragraphs, lists, code blocks, headings, bold, italics, inline
 * code and links. A link to a file on this computer shows the file in its folder; any other shows its
 * address after the text. Everything is drawn as React text, so nothing the agent writes becomes HTML.
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

/** The file a link's destination names on this computer: an absolute path, in angle brackets when it has spaces, or a file:// URL. */
function localFile(dest: string): string | null {
  let path = dest.replace(/^<(.*)>$/, "$1");
  if (path.startsWith("file:")) {
    try {
      const url = new URL(path);
      if (url.hostname && url.hostname !== "localhost") return null;
      path = decodeURIComponent(url.pathname).replace(/^\/([A-Za-z]:)/, "$1");
    } catch {
      return null;
    }
  }
  // Two slashes at the start are a network share on Windows, which revealing would reach.
  return /^(\/(?![\\/])|[A-Za-z]:[\\/])/.test(path) ? path : null;
}

function showFile(path: string) {
  revealItemInDir(path).catch(() =>
    useEditor.getState().toast({ kind: "error", text: `Can't find ${path.split(/[\\/]/).pop()}. Ask the AI where it saved it.` }),
  );
}

function inline(text: string): ReactNode[] {
  const out: ReactNode[] = [];
  const re = /(\[[^\]\n]+\]\((?:<[^>\n]+>|(?:[^()\n]|\([^()\n]*\))+)\)|`[^`]+`|\*\*[^*]+\*\*|__[^_]+__|\*[^*\s][^*]*\*|_[^_\s][^_]*_)/g;
  let last = 0;
  for (const m of text.matchAll(re)) {
    if (m.index! > last) out.push(text.slice(last, m.index));
    const t = m[0];
    const link = t.startsWith("[") ? /^\[([^\]\n]+)\]\((.+)\)$/.exec(t)! : null;
    const file = link && localFile(link[2]);
    if (link && file)
      out.push(
        <button key={out.length} type="button" title={file} onClick={() => showFile(file)} className="text-left text-accent *:text-inherit hover:underline">
          {inline(link[1])}
        </button>,
      );
    else if (link) out.push(<Fragment key={out.length}>{inline(link[1])} ({link[2].replace(/^<(.*)>$/, "$1")})</Fragment>);
    else if (t.startsWith("`")) out.push(<code key={out.length} className="rounded bg-white/[.08] px-1 text-[12px]">{t.slice(1, -1)}</code>);
    else if (t.startsWith("**") || t.startsWith("__")) out.push(<strong key={out.length} className="font-semibold text-fg">{t.slice(2, -2)}</strong>);
    else out.push(<em key={out.length}>{t.slice(1, -1)}</em>);
    last = m.index! + t.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}
