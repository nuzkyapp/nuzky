import { useEffect, useState, type ReactNode } from "react";
import { useShallow } from "zustand/react/shallow";
import { AlertCircle, AlertTriangle, Ellipsis, Palette, Plus } from "lucide-react";
import { COMMON_PROFILES, LOG_GROUPS, LOG_PROFILES, applyLut, loadLuts, lutOf, pickAndImportLut, removeLut, useColorPreviews, useLuts, type ColorLook, type LutInfo } from "../../lib/color";
import { FILTERS, adjustCss, sameAdjust } from "../../lib/presets";
import { findClip, useEditor } from "../../lib/store";
import type { Adjust, LogProfile, Lut } from "../../lib/types";
import { BeforeAfter, Button, LockReason, Menu, PresetTile, lockedProps, useLockReason } from "../ui";

// Shown when the clip has no thumbnail yet: a sky, a horizon and grass reveal colour shifts.
const SCENE = "linear-gradient(180deg, #7fb2e5 0%, #d9c7a3 52%, #4f7d3f 53%, #2c4a25 100%)";
const GRID = "grid grid-cols-[repeat(auto-fill,minmax(75px,1fr))] gap-2";
const NO_CLIP = "Select a video or image clip first";

function Picture({ thumb }: { thumb: string | null | undefined }) {
  // A little above the middle, so a portrait phone clip shows the face.
  return <span className="absolute inset-0" style={thumb ? { backgroundImage: `url(${thumb})`, backgroundSize: "cover", backgroundPosition: "center 30%" } : { background: SCENE }} />;
}

/** A tile's picture: split as recorded and with its look once the look's picture is made, else as recorded alone. */
function Look({ before, after }: { before: string | null | undefined; after: string | null | undefined }) {
  return after ? <BeforeAfter before={<Picture thumb={before} />} after={<Picture thumb={after} />} /> : <Picture thumb={before} />;
}

function FilterPreview({ adjust, thumb }: { adjust: Adjust; thumb: string | null }) {
  const css = adjustCss(adjust);
  return (
    <span className="absolute inset-0" style={{ filter: css.filter }}>
      <Picture thumb={thumb} />
      {css.tint && <span className="absolute inset-0 mix-blend-multiply" style={{ background: css.tint }} />}
      {css.vignette > 0 && <span className="absolute inset-0" style={{ background: `radial-gradient(ellipse at center, transparent 45%, rgba(0,0,0,${css.vignette}) 100%)` }} />}
    </span>
  );
}

/** The single selected video or image clip, if any. */
function useFilterTarget() {
  return useEditor(
    useShallow((s) => {
      const none = { clipId: null, assetId: null, adjust: null, log: "none" as LogProfile, lut: null as Lut | null, thumb: null };
      if (!s.snap || s.selection.length !== 1) return none;
      const found = findClip(s.snap.project, s.selection[0]);
      const c = found?.clip.content;
      const asset = c?.type === "media" ? s.snap.project.assets.find((a) => a.id === c.assetId) : undefined;
      if (!found || c?.type !== "media" || !asset || asset.kind === "audio") return none;
      return { clipId: found.clip.id, assetId: asset.id, adjust: c.adjust, log: c.log ?? "none", lut: c.lut ?? null, thumb: s.thumbs[asset.id] ?? null };
    }),
  );
}

function Heading({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="flex min-h-6 items-center justify-between gap-2">
      <h3 className="text-[13px] font-semibold text-fg">{title}</h3>
      {children}
    </div>
  );
}

export function FiltersTab() {
  const target = useFilterTarget();
  const { clipId, assetId, adjust, log } = target;
  const edit = useEditor((s) => s.edit);
  // Filters go over the colour the clip's log profile and LUT give, so their tiles start from it, without a LUT
  // this computer lacks, as the clip draws; the strength in tenths, so dragging it makes a handful of pictures.
  const luts = useLuts((s) => s.list);
  const named = lutOf(target.lut ?? undefined);
  const lut = named && luts?.some((l) => l.id === named.id) ? named : null;
  const graded = log !== "none" || !!lut;
  const [look] = useColorPreviews(graded ? assetId : null, [{ log, lut: lut?.id ?? null, strength: Math.round((lut?.strength ?? 1) * 10) / 10 }]);
  const thumb = graded ? (look ?? target.thumb) : target.thumb;
  return (
    <div className="scroll-shown flex flex-col gap-4 overflow-y-auto p-3.5">
      {!clipId && (
        <p className="flex items-center gap-1.5 text-[12px] text-muted">
          <Palette size={14} className="shrink-0" /> Select one video or image clip to convert log footage or add a LUT or filter.
        </p>
      )}
      <LogFootage target={target} />
      <Luts target={target} />
      <section className="flex flex-col gap-2" aria-label="Filters">
        <Heading title="Filters" />
        <div className={GRID}>
          {FILTERS.map((f) => (
            <PresetTile
              key={f.id}
              label={f.label}
              selected={!!adjust && sameAdjust(adjust, f.adjust)}
              disabled={!clipId}
              title={clipId ? `Apply ${f.label}` : NO_CLIP}
              onClick={() => clipId && edit({ type: "updateClip", clipId, adjust: f.adjust })}
            >
              {f.id === "none" ? <Picture thumb={thumb} /> : <BeforeAfter before={<Picture thumb={thumb} />} after={<FilterPreview adjust={f.adjust} thumb={thumb} />} />}
            </PresetTile>
          ))}
        </div>
        {clipId && adjust && !FILTERS.some((f) => sameAdjust(adjust, f.adjust)) && <p className="text-[12px] text-muted">This clip uses custom adjustments. Picking a filter replaces them.</p>}
      </section>
    </div>
  );
}

type Target = ReturnType<typeof useFilterTarget>;

/** Camera log profiles: each tile splits the clip's picture as recorded and converted, and one click converts it. */
function LogFootage({ target }: { target: Target }) {
  const { clipId, assetId, log, thumb } = target;
  const edit = useEditor((s) => s.edit);
  const [all, setAll] = useState(false);
  const shown = all ? LOG_PROFILES : LOG_PROFILES.filter((p) => COMMON_PROFILES.includes(p.id) || p.id === log);
  const previews = useColorPreviews(
    assetId,
    shown.map((p) => ({ log: p.id, lut: null, strength: 1 })),
  );
  const tile = (p: (typeof LOG_PROFILES)[number]) => {
    const i = shown.indexOf(p);
    const selected = log === p.id;
    return (
      <PresetTile
        key={p.id}
        label={p.label}
        selected={selected}
        disabled={!clipId}
        title={!clipId ? NO_CLIP : selected ? `Show ${p.name} as recorded` : `Convert from ${p.name}${p.recordedOn ? `, ${p.recordedOn}` : ""}`}
        onClick={() => clipId && edit({ type: "updateClip", clipId, log: selected ? "none" : p.id })}
      >
        <Look before={thumb} after={previews[i]} />
      </PresetTile>
    );
  };
  return (
    <section className="flex flex-col gap-2" aria-label="Log footage">
      <Heading title="Log footage">
        {/* It only changes what the list shows, so it stays open while an agent edits. */}
        <LockReason.Provider value={null}>
          <Button variant="ghost" className="-mr-1 h-6 px-1 text-[12px] text-accent! hover:text-accent!" aria-expanded={all} onClick={() => setAll(!all)}>
            {all ? "Show fewer" : `Show all ${LOG_PROFILES.length}`}
          </Button>
        </LockReason.Provider>
      </Heading>
      {all ? (
        <div className="flex flex-col gap-4">
          {LOG_GROUPS.map((g) => (
            <div key={g.name} className="flex flex-col gap-1.5">
              <span className="text-[12px] text-muted">{g.name}</span>
              <div className={GRID}>{g.profiles.map(tile)}</div>
            </div>
          ))}
        </div>
      ) : (
        <div className={GRID}>{shown.map(tile)}</div>
      )}
    </section>
  );
}

/** The user's LUTs, over the colour the clip's log profile gives; Import copies .cube files into the library. */
function Luts({ target }: { target: Target }) {
  const { clipId, assetId, log, lut, thumb } = target;
  const { list, error } = useLuts();
  const lock = useLockReason();
  const edit = useEditor((s) => s.edit);
  const [menu, setMenu] = useState<{ lut: LutInfo; at: { x: number; y: number }; keyboard: boolean } | null>(null);
  useEffect(() => {
    if (list === null) void loadLuts();
  }, [list]);
  const luts = list ?? [];
  const looks: ColorLook[] = [{ log, lut: null, strength: 1 }, ...luts.map((l) => ({ log, lut: l.id, strength: 1 }))];
  const [base, ...previews] = useColorPreviews(assetId, looks);
  const before = log === "none" ? thumb : (base ?? thumb);
  const current = lutOf(lut ?? undefined);
  const elsewhere = current && list !== null && !luts.some((l) => l.id === current.id);
  const openMenu = (l: LutInfo, at: { x: number; y: number }, keyboard: boolean) => !lock && setMenu({ lut: l, at, keyboard });
  return (
    <section className="flex flex-col gap-2" aria-label="LUTs">
      <Heading title="LUTs">
        <Button variant="ghost" className="-mr-1 h-6 gap-1 px-1 text-[12px] text-accent! hover:text-accent!" data-import-lut onClick={() => void pickAndImportLut()}>
          <Plus size={13} /> Import
        </Button>
      </Heading>
      {error && (
        <p role="alert" className="flex items-start gap-1.5 text-[12px] text-fg">
          <AlertCircle size={14} className="mt-px shrink-0 text-danger" />
          <span>{error}</span>
        </p>
      )}
      {elsewhere && (
        <p role="status" className="flex items-start gap-1.5 text-[12px] text-muted">
          <AlertTriangle size={14} className="mt-px shrink-0 text-warn" />
          <span>{`This clip's ${current.name} LUT isn't on this computer.`}</span>
        </p>
      )}
      <div className={GRID}>
        {luts.map((l, i) => {
          const selected = current?.id === l.id;
          return (
            <div key={l.id} className="group relative min-w-0" data-lut={l.name}>
              <div
                className="grid"
                onContextMenu={(e) => {
                  e.preventDefault();
                  openMenu(l, { x: e.clientX, y: e.clientY }, false);
                }}
                onKeyDown={(e) => {
                  if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) {
                    e.preventDefault();
                    const r = (e.target as HTMLElement).getBoundingClientRect();
                    openMenu(l, { x: r.left, y: r.bottom + 4 }, true);
                  }
                }}
              >
                <PresetTile
                  label={l.name}
                  selected={selected}
                  disabled={!clipId}
                  title={!clipId ? NO_CLIP : selected ? `Remove ${l.name} from the clip` : `Apply ${l.name}`}
                  onClick={() => clipId && (selected ? edit({ type: "updateClip", clipId, lut: { type: "none" } }) : applyLut(clipId, l))}
                >
                  <Look before={before} after={previews[i]} />
                </PresetTile>
              </div>
              {!lock && (
                // Over the bottom right corner of the tile's picture, which is as wide as the tile and 4:3.
                <span className="pointer-events-none absolute inset-x-0 top-0 aspect-[4/3]">
                  <button
                    type="button"
                    tabIndex={-1}
                    aria-hidden
                    title="More"
                    onClick={(e) => {
                      const r = e.currentTarget.getBoundingClientRect();
                      openMenu(l, { x: r.left, y: r.bottom + 4 }, false);
                    }}
                    className="pointer-events-auto absolute right-1 bottom-1 flex h-5 w-5 items-center justify-center rounded-full bg-black/55 text-fg opacity-0 transition-opacity duration-[120ms] group-hover:opacity-100 hover:bg-black/75"
                  >
                    <Ellipsis size={12} />
                  </button>
                </span>
              )}
            </div>
          );
        })}
        {list !== null && luts.length === 0 && (
          <button
            type="button"
            {...lockedProps(lock)}
            onClick={lock ? undefined : () => void pickAndImportLut()}
            className="flex aspect-[4/3] min-w-0 flex-col items-center justify-center gap-1 rounded-[10px] border border-dashed border-white/[.15] px-1 text-center text-[11px] text-muted transition-colors duration-[120ms] hover:border-white/30 hover:text-fg aria-disabled:cursor-not-allowed aria-disabled:opacity-40"
          >
            <Plus size={12} /> Import .cube
          </button>
        )}
      </div>
      {menu && (
        <Menu
          label={`${menu.lut.name} LUT`}
          at={menu.at}
          keyboard={menu.keyboard}
          onClose={() => setMenu(null)}
          items={[{ label: "Move to Trash", danger: true, run: () => void removeLut(menu.lut) }]}
        />
      )}
    </section>
  );
}
