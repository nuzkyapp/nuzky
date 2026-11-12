import path from "node:path";
import type { NextConfig } from "next";

// The site is an npm workspace of the Nuzky repo, so its packages are hoisted to the repo root.
const repoRoot = path.join(__dirname, "..");

// An agent that asks for Markdown first gets the page as Markdown at the page's own URL (see lib/markdown.ts).
// Browsers never put it first, and a client that refuses it with q=0 keeps the HTML. Plain patterns without
// lookahead, since Vercel evaluates these conditions in its own router.
const accept = (value: string) => [{ type: "header" as const, key: "accept", value }];
const markdown = [
  ["/", "/index.md"],
  ...["/capcut-alternative", "/edits-alternative", "/auto-captions", "/remove-filler-words", "/ai-video-editor"].map((page) => [page, `${page}.md`]),
].map(([source, destination]) => ({
  source,
  destination,
  has: accept("\\s*text/markdown\\b.*"),
  missing: accept("\\s*text/markdown\\s*;\\s*q=0(\\.0*)?\\s*(,.*)?"),
}));

const config: NextConfig = {
  turbopack: { root: repoRoot },
  outputFileTracingRoot: repoRoot,
  // 512 sits between the default 384 and the smallest device size, 640, where the editor tour's tiles on a
  // phone and the hero reels on a 2× screen would otherwise load 640 px. The images stay WebP: Vercel's AVIF
  // smeared grass, rocks and faces in the small clip tiles.
  images: { imageSizes: [32, 48, 64, 96, 128, 256, 384, 512] },
  rewrites: async () => ({ beforeFiles: markdown, afterFiles: [], fallback: [] }),
};

export default config;
