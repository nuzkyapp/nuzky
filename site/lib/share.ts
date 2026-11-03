import type { Metadata } from "next";
import { url } from "./site";

// The same words as app/opengraph-image.alt.txt, which the home page's share image carries.
const alt =
  "Nuzky: Recording is the fun part. Nuzky does the boring one. Three clips from a hike become a captioned vertical reel, with the ums and retakes cut, captions in your language and music you're allowed to use.";
const image = (src: string) => ({ url: src, width: 2400, height: 1260, alt });

// A page that sets its own Open Graph fields loses the root share image, so it names it again.
export function shareMetadata(path: string, title: string, description: string, type: "article" | "website" = "article"): Metadata {
  return {
    openGraph: { title, description, url: url + path, siteName: "Nuzky", type, locale: "en_US", images: [image("/opengraph-image.jpg")] },
    twitter: { card: "summary_large_image", title, description, images: [image("/twitter-image.jpg")] },
  };
}
