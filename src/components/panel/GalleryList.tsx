import { useEffect } from "react";
import { AlertCircle, Puzzle, WifiOff } from "lucide-react";
import { galleryError, loadGallery, openGalleryPlugin, useGallery, type GalleryPlugin } from "../../lib/gallery";
import { UpdateNuzkyButton } from "../Updates";
import { Button } from "../ui";
import { PluginMark } from "./PluginsTab";

const MARKS: Partial<Record<GalleryPlugin["state"], string>> = { installed: "Installed", update: "Update", changed: "Edited", clash: "Same name as yours" };

function Row({ plugin: p, open }: { plugin: GalleryPlugin; open: boolean }) {
  const off = p.state === "newer" ? "Needs a newer Nuzky" : p.state === "refused" ? "Can’t be installed" : null;
  const mark = MARKS[p.state];
  return (
    <li>
      <button
        type="button"
        data-gallery-plugin={p.id}
        aria-current={open || undefined}
        onClick={() => openGalleryPlugin(p.id)}
        className={`flex min-h-12 w-full items-center gap-2.5 rounded-[10px] py-2 pr-2.5 pl-2 text-left transition-colors duration-[120ms] ease-out ${open ? "bg-white/[.08]" : "hover:bg-white/[.05]"}`}
      >
        <PluginMark plugin={{ name: p.name, error: null }} />
        <span className="flex min-w-0 flex-1 flex-col leading-[17px]">
          <span className="truncate font-medium text-fg">{p.name}</span>
          <span className="truncate text-[12px] text-muted">{off ?? p.description}</span>
        </span>
        {mark && <span className={`shrink-0 text-[12px] ${p.state === "update" ? "text-accent" : "text-muted"}`}>{mark}</span>}
      </button>
    </li>
  );
}

function Problem({ error }: { error: string }) {
  const { title, detail, retry } = galleryError(error);
  return (
    <div className="flex flex-col items-center px-5 pt-8 pb-4 text-center" role="alert">
      {error.startsWith("OFFLINE") ? <WifiOff size={26} strokeWidth={1.6} className="text-muted" aria-hidden /> : <AlertCircle size={26} strokeWidth={1.6} className="text-muted" aria-hidden />}
      <h3 className="mt-2.5 text-[13px] font-semibold text-fg">{title}</h3>
      {detail && <p className="mt-1 max-w-[260px] text-[12px] text-muted">{detail}</p>}
      {retry && (
        <Button className="mt-3 h-7 text-[12px]" onClick={() => void loadGallery()}>
          Try again
        </Button>
      )}
      {error.startsWith("GALLERY_NEWER") && <UpdateNuzkyButton className="mt-3 h-8 px-4" />}
    </div>
  );
}

/** The plugins of the gallery, read each time the Gallery opens and never in the background. */
export function GalleryList() {
  const list = useGallery((s) => s.list);
  const error = useGallery((s) => s.error);
  const open = useGallery((s) => s.open);
  useEffect(() => {
    void loadGallery();
  }, []);
  if (error) return <Problem error={error} />;
  const shared = <p className="shrink-0 px-3.5 pt-1 pb-1.5 text-[12px] text-muted">Shared by other people. Nuzky’s maintainer checks each one before it’s listed.</p>;
  if (!list)
    return (
      <>
        {shared}
        <div role="status" aria-label="Loading the gallery" className="flex flex-col gap-0.5 px-1.5">
          {[0, 1, 2].map((i) => (
            <div key={i} className="flex min-h-12 items-center gap-2.5 py-2 pl-2">
              <div className="skeleton size-[30px] shrink-0 rounded-lg" />
              <div className="flex flex-1 flex-col gap-1.5">
                <div className="skeleton h-3 w-1/2 rounded" />
                <div className="skeleton h-2.5 w-3/4 rounded" />
              </div>
            </div>
          ))}
        </div>
      </>
    );
  if (list.length === 0)
    return (
      <div className="flex flex-col items-center px-5 pt-8 pb-4 text-center">
        <Puzzle size={30} strokeWidth={1.6} className="text-muted" aria-hidden />
        <h3 className="mt-2.5 text-[13px] font-semibold text-fg">The gallery is empty</h3>
        <p className="mt-1 max-w-[260px] text-[12px] text-muted">Plugins other people share will show up here.</p>
      </div>
    );
  return (
    <>
      {shared}
      <ul className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-1.5 pb-2" aria-label="Gallery">
        {list.map((p) => (
          <Row key={p.id} plugin={p} open={p.id === open} />
        ))}
      </ul>
    </>
  );
}
