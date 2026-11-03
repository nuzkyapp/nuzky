import type { Metadata } from "next";
import Link from "next/link";
import { Check } from "lucide-react";
import { EditorMock } from "./EditorMock";
import { FaqList } from "./Faq";
import { Fit } from "./Fit";
import { Footer } from "./Footer";
import { HeroDownload } from "./GetNuzky";
import { JsonLd } from "./JsonLd";
import { Nav } from "./Nav";
import { SectionHeading } from "./SectionHeading";
import { guide, guides } from "@/lib/guides";
import { shareMetadata } from "@/lib/share";
import { getRelease, url } from "@/lib/site";

export function guideMetadata(path: string): Metadata {
  const g = guide(path);
  return {
    title: { absolute: `${g.title} | Nuzky` },
    description: g.description,
    alternates: { canonical: path, types: { "text/markdown": `${path}.md` } },
    ...shareMetadata(path, g.title, g.description),
  };
}

const structuredData = (path: string) => {
  const g = guide(path);
  return {
    "@context": "https://schema.org",
    "@graph": [
      {
        "@type": "WebPage",
        "@id": url + path,
        url: url + path,
        name: g.title,
        description: g.description,
        inLanguage: "en",
        isPartOf: { "@id": `${url}/#website` },
        about: { "@id": `${url}/#app` },
      },
      {
        "@type": "BreadcrumbList",
        itemListElement: [
          { "@type": "ListItem", position: 1, name: "Nuzky", item: url },
          { "@type": "ListItem", position: 2, name: g.name, item: url + path },
        ],
      },
      {
        "@type": "FAQPage",
        mainEntity: g.faq.map(({ q, a }) => ({ "@type": "Question", name: q, acceptedAnswer: { "@type": "Answer", text: a } })),
      },
    ],
  };
};

export async function GuidePage({ path }: { path: string }) {
  const g = guide(path);
  const release = await getRelease();
  const others = [...guides.filter((o) => o.path !== path), { path: "/capcut-alternative", name: "Nuzky vs CapCut" }];
  return (
    <>
      <JsonLd data={structuredData(path)} />
      <Nav base="/" />
      <main>
        <div className="mx-auto w-full max-w-[1240px] px-5">
          <header className="flex flex-col items-center pt-16 text-center sm:pt-24">
            <h1 className="heading-xl">
              {g.heading[0]}
              <br />
              <span className="text-subtle">{g.heading[1]}</span>
            </h1>
            <p className="mt-7 max-w-[720px] text-[17px] text-muted sm:text-[20px]">{g.intro}</p>
            <div className="mt-10 w-full">
              <HeroDownload release={release} base="/" />
            </div>
            <Fit width={1200} height={740} className="mt-10 sm:mt-12">
              <div className="relative size-full rounded-[14px] shadow-[0_50px_140px_rgb(0_0_0/0.9),0_0_0_1px_rgb(255_255_255/0.08)]">
                <EditorMock focus={g.focus} />
              </div>
            </Fit>
          </header>

          <section className="pt-28 sm:pt-[150px]">
            <SectionHeading center first="How it works." second={`${g.steps.length} steps in Nuzky.`} />
            <ol className="mx-auto mt-10 grid max-w-[1000px] gap-4 md:grid-cols-2">
              {g.steps.map(({ title, text }, i) => (
                <li key={title} className="surface flex gap-5 rounded-3xl p-7 md:odd:last:col-span-2">
                  <span className="flex size-8 shrink-0 items-center justify-center rounded-full bg-white/[0.08] text-[14px] font-semibold tabular-nums">
                    {i + 1}
                  </span>
                  <div className="flex flex-col gap-2">
                    <h3 className="text-[18px] font-semibold tracking-[-0.01em]">{title}</h3>
                    <p className="text-[15px] leading-[1.5] text-muted">{text}</p>
                  </div>
                </li>
              ))}
            </ol>
            <div className="surface mx-auto mt-4 max-w-[1000px] rounded-3xl p-7 sm:p-9">
              <h3 className="text-[22px] font-semibold tracking-[-0.02em]">Good to know</h3>
              <ul className="mt-5 grid gap-x-8 gap-y-3.5 md:grid-cols-2">
                {g.details.map((item) => (
                  <li key={item} className="flex gap-3 text-[15px] leading-[1.45]">
                    <Check className="mt-[3px] size-4 shrink-0 text-ok" />
                    {item}
                  </li>
                ))}
              </ul>
            </div>
          </section>

          <section className="pb-16 pt-28 sm:pt-[150px]">
            <SectionHeading center first="Questions." second="Short answers." />
            <FaqList items={g.faq} className="mt-12" />
            <nav aria-label="More about Nuzky" className="mx-auto mt-16 flex max-w-[820px] flex-wrap justify-center gap-x-7 gap-y-2">
              {others.map((o) => (
                <Link key={o.path} href={o.path} className="text-[14px] text-muted underline decoration-white/20 underline-offset-4 transition-colors hover:text-fg">
                  {o.name}
                </Link>
              ))}
            </nav>
          </section>
        </div>
      </main>
      <Footer />
    </>
  );
}
