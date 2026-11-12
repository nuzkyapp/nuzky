import type { MetadataRoute } from "next";
import { comparisons } from "@/lib/comparisons";
import { guides } from "@/lib/guides";
import { url } from "@/lib/site";

export default function sitemap(): MetadataRoute.Sitemap {
  return [{ url }, ...comparisons.map((c) => ({ url: url + c.path })), ...guides.map((g) => ({ url: url + g.path }))];
}
