import type { Metadata } from "next";
import Image from "next/image";
import { Check } from "lucide-react";
import { EditorMock, MOCK_H, MOCK_W } from "./EditorMock";
import { FaqList } from "./Faq";
import { Fit } from "./Fit";
import { Footer } from "./Footer";
import { HeroDownload } from "./GetNuzky";
import { JsonLd } from "./JsonLd";
import { Nav } from "./Nav";
import { SectionHeading } from "./SectionHeading";
import { Waves } from "./Waves";
import { comparison } from "@/lib/comparisons";
import { shareMetadata } from "@/lib/share";
import { getRelease, url } from "@/lib/site";

export function comparisonMetadata(path: string): Metadata {
  const c = comparison(path);
  return {
    title: { absolute: c.metaTitle },
    description: c.description,
    alternates: { canonical: path, types: { "text/markdown": `${path}.md` } },
    ...shareMetadata(path, c.title, c.description),
  };
}

const structuredData = (path: string) => {
  const c = comparison(path);
  return {
    "@context": "https://schema.org",
    "@graph": [
      {
        "@type": "BreadcrumbList",
        itemListElement: [
          { "@type": "ListItem", position: 1, name: "Nuzky", item: url },
          { "@type": "ListItem", position: 2, name: c.name, item: url + path },
        ],
      },
      {
        "@type": "FAQPage",
        mainEntity: c.faq.map(({ q, a }) => ({ "@type": "Question", name: q, acceptedAnswer: { "@type": "Answer", text: a } })),
      },
    ],
  };
};

function Picks({ title, items, ours }: { title: string; items: string[]; ours: boolean }) {
  return (
    <div className="surface flex flex-col gap-5 rounded-3xl p-7 sm:p-9">
      <h3 className="text-[22px] font-semibold tracking-[-0.02em]">{title}</h3>
      <ul className="flex flex-col gap-3.5">
        {items.map((item) => (
          <li key={item} className="flex gap-3 text-[15px] leading-[1.45]">
            <Check className={`mt-[3px] size-4 shrink-0 ${ours ? "text-ok" : "text-muted"}`} />
            {item}
          </li>
        ))}
      </ul>
    </div>
  );
}

const link = "underline decoration-white/20 underline-offset-4 transition-colors hover:text-fg";

export async function ComparisonPage({ path }: { path: string }) {
  const c = comparison(path);
  const release = await getRelease();
  return (
    <>
      <JsonLd data={structuredData(path)} />
      <Nav base="/" />
      <main>
        <div className="mx-auto w-full max-w-[1240px] px-5">
          <header className="flex flex-col items-center pt-16 text-center sm:pt-24">
            <h1 className="heading-xl">
              {c.heading[0]}
              <br />
              <span className="text-subtle">{c.heading[1]}</span>
            </h1>
            <p className="mt-7 max-w-[720px] text-[17px] text-muted sm:text-[20px]">{c.intro}</p>
            <div className="mt-10 w-full">
              <HeroDownload release={release} base="/" />
            </div>

            <Fit width={MOCK_W} height={MOCK_H} className="mt-10 sm:mt-12">
              <div className="relative size-full rounded-[14px] shadow-[0_50px_140px_rgb(0_0_0/0.9),0_0_0_1px_rgb(255_255_255/0.08)]">
                <EditorMock focus="edit" />
              </div>
            </Fit>
          </header>

          <section className="pt-28 sm:pt-[150px]">
            <SectionHeading center first={`Nuzky and ${c.rival}, side by side.`} second="Where each one wins." />
            <div className="surface mx-auto mt-10 max-w-[1000px] overflow-hidden rounded-3xl">
              <table className="w-full border-collapse text-left text-[15px] leading-[1.45]">
                <thead className="max-sm:hidden">
                  <tr className="border-b border-white/[0.07]">
                    <th scope="col" className="w-[22%] px-7 py-5">
                      <span className="sr-only">Compared</span>
                    </th>
                    <th scope="col" className="w-[39%] px-7 py-5 text-[16px] font-semibold">
                      <span className="flex items-center gap-2.5">
                        <Image src="/media/icon.png" alt="" width={24} height={24} />
                        Nuzky
                      </span>
                    </th>
                    <th scope="col" className="w-[39%] px-7 py-5 text-[16px] font-semibold text-muted">
                      {c.rival}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {c.rows.map((row) => (
                    <tr key={row.label} className="border-b border-white/[0.07] last:border-0 max-sm:block max-sm:px-6 max-sm:py-5">
                      <th
                        scope="row"
                        className="px-7 py-5 align-top text-[13px] font-medium text-muted max-sm:block max-sm:p-0 max-sm:pb-3 max-sm:text-[15px] max-sm:font-semibold max-sm:text-fg"
                      >
                        {row.label}
                      </th>
                      <td className="px-7 py-5 align-top max-sm:block max-sm:p-0 max-sm:pb-2.5">
                        <span className="block text-[12px] text-muted sm:hidden">Nuzky</span>
                        {row.nuzky}
                      </td>
                      <td className="px-7 py-5 align-top text-muted max-sm:block max-sm:p-0">
                        <span className="block text-[12px] text-muted sm:hidden">{c.rival}</span>
                        {row.rival}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <div className="mx-auto mt-4 grid max-w-[1000px] gap-4 md:grid-cols-2">
              <Picks title="Pick Nuzky if" items={c.pickNuzky} ours />
              <Picks title={c.stayWith} items={c.pickRival} ours={false} />
            </div>
          </section>
        </div>

        {/* The light runs edge to edge behind the questions, so their glass has something to bend. */}
        <div className="relative overflow-hidden">
          <Waves swellAt="[data-waves=faq]" intensity={1.9} className="[mask-image:linear-gradient(to_bottom,transparent,#000_240px)]" />
          <section className="relative z-10 mx-auto w-full max-w-[1240px] px-5 pt-28 sm:pt-[150px]">
            <SectionHeading center first={c.moving.heading[0]} second={c.moving.heading[1]} />
            <div className="mx-auto mt-10 grid max-w-[1000px] gap-4 md:grid-cols-3">
              {c.moving.items.map(({ title, text }) => (
                <div key={title} className="surface flex flex-col gap-2 rounded-3xl p-7">
                  <h3 className="text-[18px] font-semibold tracking-[-0.01em]">{title}</h3>
                  <p className="text-[14px] leading-[1.5] text-muted">{text}</p>
                </div>
              ))}
            </div>
            <FaqList items={c.faq} className="mt-16" />
          </section>

          <p className="relative z-10 mx-auto max-w-[760px] px-5 pb-16 pt-28 text-center text-[13px] leading-[1.55] text-muted">
            What this page says about {c.rival} comes from{" "}
            {c.sources.map((s, i) => (
              <span key={s.href}>
                {i > 0 && (i === c.sources.length - 1 ? " and " : ", ")}
                <a href={s.href} rel="nofollow noopener" className={link}>
                  {s.label}
                </a>
              </span>
            ))}
            , checked in {c.checked}. {c.fineprint}
          </p>
        </div>
      </main>
      <Footer />
    </>
  );
}
