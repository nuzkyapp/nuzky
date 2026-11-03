import type { MetadataRoute } from "next";
import { guides } from "@/lib/guides";
import { url } from "@/lib/site";

export default function sitemap(): MetadataRoute.Sitemap {
  return [{ url }, { url: `${url}/capcut-alternative` }, ...guides.map((g) => ({ url: url + g.path }))];
}
