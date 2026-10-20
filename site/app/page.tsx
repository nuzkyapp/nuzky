import type { Metadata } from "next";
import { Agent } from "@/components/Agent";
import { CapcutDrafts } from "@/components/CapcutDrafts";
import { Download } from "@/components/Download";
import { EditStyle } from "@/components/EditStyle";
import { Faq } from "@/components/Faq";
import { Features } from "@/components/Features";
import { Footer } from "@/components/Footer";
import { Hero } from "@/components/Hero";
import { JsonLd } from "@/components/JsonLd";
import { Maker } from "@/components/Maker";
import { Nav } from "@/components/Nav";
import { Showcase } from "@/components/Showcase";
import { Waves } from "@/components/Waves";
import { faq, features } from "@/lib/content";
import { type Release, description, getRelease, repo, systems, url } from "@/lib/site";

export const metadata: Metadata = {
  alternates: { canonical: "/", types: { "text/markdown": "/index.md" } },
};

// No rating: made-up ratings break Google's structured data rules.
const structuredData = (release: Release) => ({
  "@context": "https://schema.org",
  "@graph": [
    { "@type": "WebSite", "@id": `${url}/#website`, url, name: "Nuzky", description, inLanguage: "en" },
    {
      "@type": "SoftwareApplication",
      "@id": `${url}/#app`,
      name: "Nuzky",
      url,
      description,
      image: `${url}/media/icon.png`,
      applicationCategory: "MultimediaApplication",
      applicationSubCategory: "Video editor",
      operatingSystem: "macOS 14 or newer on Apple silicon, Windows 10 or 11 (64-bit), Linux (x86_64)",
      ...(release.version && { softwareVersion: release.version }),
      releaseNotes: release.notes,
      downloadUrl: [...new Set(systems.flatMap((s) => s.files.map((f) => release.files[f.id].href)))],
      license: "https://www.gnu.org/licenses/gpl-3.0.html",
      isAccessibleForFree: true,
      offers: { "@type": "Offer", price: "0", priceCurrency: "USD" },
      featureList: features,
      sameAs: [repo],
    },
    {
      "@type": "FAQPage",
      mainEntity: faq.map(({ q, a }) => ({ "@type": "Question", name: q, acceptedAnswer: { "@type": "Answer", text: a } })),
    },
  ],
});

export default async function Home() {
  const release = await getRelease();
  return (
    <>
      <JsonLd data={structuredData(release)} />
      <Nav />
      <main>
        {/* The light reaches up behind the nav, so the page starts on one surface. */}
        <div id="top" className="relative -mt-[72px] overflow-hidden pt-[72px]">
          <Waves swellAt="[data-waves=swell]" height={0.75} intensity={1.4} />
          <Hero release={release} />
        </div>
        <CapcutDrafts />
        <Maker />
        <Agent />
        <EditStyle />
        <Showcase />
        <Features />
        <Faq />
        <Download release={release} />
      </main>
      <Footer />
    </>
  );
}
