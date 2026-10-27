import { AlertCircle, AudioLines, Check, Copy, ExternalLink, Link2, Loader2, Pause, Play, Plus, SlidersHorizontal, WifiOff, X } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { api, errorText, plainError } from "../../lib/api";
import { addSound, answerRights, copyCredits, installYtdlp, loadBuiltIn, loadDownloaded, openLink, searchSounds, stopPreview, togglePreview, useSounds } from "../../lib/sounds";
import { useEditor } from "../../lib/store";
import { formatDuration } from "../../lib/time";
import { SONG, useTour } from "../../lib/tour";
import type { Sound, SoundKind, SoundSettings } from "../../lib/types";
import { Button, Checkbox, IconButton, LockReason, SearchField, lockedProps, trapTab, useLockReason } from "../ui";

/** Round row action, as on the project's own audio. */
const ACTION = "flex h-7 w-7 shrink-0 items-center justify-center rounded-full transition-colors duration-[120ms] ease-out aria-disabled:cursor-not-allowed aria-disabled:opacity-40";
const MUSIC_IDEAS = ["Upbeat", "Chill", "Cinematic", "Happy", "Lofi", "Epic", "Acoustic", "Electronic"];
const FREESOUND_KEYS = "https://freesound.org/apiv2/apply/";

/** One sound: clicking it plays it, + or a double click adds it at the playhead, the licence opens its page. */
function SoundRow({ sound, inProject, showKind = true }: { sound: Sound; inProject: boolean; showKind?: boolean }) {
  const preview = useSounds((s) => (s.preview?.id === sound.id ? s.preview : null));
  const adding = useSounds((s) => s.adding.includes(sound.id));
  const progress = useSounds((s) => s.progress[sound.id] ?? 0);
  const lock = useLockReason();
  // The tour points at this row's +, so it shows without the pointer over it.
  const pointedAt = useTour((s) => s.step === "song") && sound.id === SONG;
  const online = sound.provider !== "Built in";
  const license = sound.license === "by" ? "CC BY" : "CC0";
  const licensed = online && sound.license !== null;
  return (
    <div
      data-sound={sound.id}
      onClick={(e) => e.detail === 1 && !(e.target as Element).closest("button") && togglePreview(sound)}
      onDoubleClick={(e) => !lock && !(e.target as Element).closest("button") && addSound(sound)}
      className={`group relative flex cursor-pointer items-center gap-2.5 rounded-xl p-1.5 focus-within:bg-raised hover:bg-raised ${preview ? "bg-raised" : ""}`}
    >
      <button
        type="button"
        aria-label={preview ? `Stop ${sound.title}` : `Play ${sound.title}`}
        aria-pressed={!!preview}
        title={preview ? "Stop" : "Play"}
        onClick={() => togglePreview(sound)}
        className={`flex h-10 w-10 shrink-0 items-center justify-center rounded-md text-fg ${sound.kind === "music" ? "bg-clip-audio" : "bg-white/[.07]"}`}
      >
        {preview?.loading ? (
          <Loader2 size={16} className="animate-spin" />
        ) : preview ? (
          <Pause size={16} fill="currentColor" />
        ) : (
          <>
            <AudioLines size={18} className="group-focus-within:hidden group-hover:hidden" />
            <Play size={16} fill="currentColor" className="hidden group-focus-within:block group-hover:block" />
          </>
        )}
      </button>
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="truncate text-[13px] text-fg">{sound.title}</span>
        {(online || showKind) && (
          <span className="truncate text-[12px] text-muted">{online || sound.kind === "music" ? `${sound.author} · ${sound.provider}` : sound.category}</span>
        )}
      </div>
      {licensed && (
        <button
          type="button"
          onClick={() => api.openSoundPage(sound.url).catch((e) => useEditor.getState().toast({ kind: "error", text: errorText(e) }))}
          title={`${license} ${sound.licenseVersion}${sound.license === "by" ? ", needs credit" : ", no credit needed"}. Open its page at ${sound.provider} to check the licence.`}
          aria-label={`${license}: open ${sound.title} at ${sound.provider}`}
          className="shrink-0 rounded-[4px] px-[5px] py-px text-[11px] font-medium text-muted shadow-[inset_0_0_0_1px_rgb(255_255_255/.14)] hover:text-fg"
        >
          {license}
        </button>
      )}
      <span className="tabular shrink-0 text-[12px] text-muted">{sound.durationUs > 0 ? formatDuration(sound.durationUs) : ""}</span>
      {adding ? (
        <button
          type="button"
          aria-label={`Stop adding ${sound.title}`}
          title="Downloading. Click to stop."
          // The second click of a double click on + must not stop the add it started.
          onClick={(e) => e.detail === 1 && api.soundCancel(sound.id)}
          className={`text-muted shadow-[inset_0_0_0_1.5px_rgb(255_255_255/.14)] hover:text-fg ${ACTION}`}
        >
          {progress > 0 && progress < 1 ? <span className="tabular text-[11px]">{Math.round(progress * 100)}%</span> : <Loader2 size={15} className="animate-spin" />}
        </button>
      ) : (
        <>
          {inProject && (
            <span aria-hidden className="flex h-7 w-7 shrink-0 items-center justify-center text-ok group-focus-within:hidden group-hover:hidden" title="In this project">
              <Check size={16} />
            </span>
          )}
          <button
            type="button"
            aria-label={inProject ? `Add ${sound.title} at playhead again, it is in this project` : `Add ${sound.title} at playhead`}
            title={inProject ? "In this project. Add it again at the playhead" : "Add at playhead"}
            onClick={() => addSound(sound)}
            {...lockedProps(lock)}
            className={`btn-prominent ${ACTION} ${pointedAt ? "" : inProject ? "hidden group-focus-within:flex group-hover:flex" : "opacity-0 group-focus-within:opacity-100 group-hover:opacity-100"}`}
          >
            <Plus size={16} />
          </button>
        </>
      )}
    </div>
  );
}

/** Where online results come from, and the user's own Freesound key. */
function Sources({ onClose }: { onClose: (keyboard: boolean) => void }) {
  const [settings, setSettings] = useState<SoundSettings | null>(null);
  const [key, setKey] = useState("");
  const [error, setError] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    api.soundSettings().then(setSettings);
    ref.current?.focus();
    const outside = (e: PointerEvent) => !ref.current?.contains(e.target as Node) && !(e.target as Element).closest?.("[data-sources-button]") && onClose(false);
    window.addEventListener("pointerdown", outside, true);
    return () => window.removeEventListener("pointerdown", outside, true);
  }, [onClose]);
  const save = async (freesound: boolean, newKey?: string) => {
    try {
      setSettings(await api.setSoundSettings(freesound, newKey));
      setError(null);
      if (newKey !== undefined) setKey("");
      useSounds.setState((s) => ({ search: { ...s.search, effect: { ...s.search.effect, sounds: [], page: 0, service: null } } }));
      const query = useSounds.getState().search.effect.query;
      if (query.trim()) searchSounds("effect", query);
    } catch (e) {
      setError(plainError(errorText(e)));
    }
  };
  return (
    <div
      ref={ref}
      tabIndex={-1}
      role="dialog"
      aria-label="Where to search"
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Escape") onClose(true);
        trapTab(e);
      }}
      className="overlay absolute top-[44px] right-3.5 z-20 flex w-[300px] flex-col gap-3 rounded-[12px] p-3 text-[12px] outline-none"
    >
      <p className="text-[13px] font-semibold text-fg">Where to search</p>
      <div className="flex flex-col gap-1">
        <p className="text-[13px] text-fg">Openverse</p>
        <p className="text-muted">Music and sounds from Freesound and Wikimedia Commons. Only CC0 and CC BY.</p>
      </div>
      <div className="flex flex-col gap-2 border-t border-white/[.07] pt-3">
        {settings && (
          <Checkbox
            label="Search effects on Freesound instead"
            checked={settings.freesound && settings.hasKey}
            onChange={(on) => save(on)}
            disabled={!settings.hasKey}
            title={settings.hasKey ? undefined : "Save your Freesound API key first"}
          />
        )}
        <p className="text-muted">Needs your own free API key. Freesound offers its API free only for non-commercial use; the sounds stay CC0 or CC BY.</p>
        {settings?.hasKey ? (
          <div className="flex items-center justify-between gap-2">
            <span className="text-fg">Your key is saved</span>
            <Button className="h-7 text-[12px]" onClick={() => save(settings.freesound, "")}>
              Remove key
            </Button>
          </div>
        ) : (
          <form
            className="flex gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              if (key.trim()) save(true, key.trim());
            }}
          >
            <input
              type="password"
              value={key}
              onChange={(e) => setKey(e.target.value)}
              placeholder="Paste your Freesound API key"
              aria-label="Freesound API key"
              autoComplete="off"
              className="h-7 min-w-0 flex-1 rounded-[6px] border border-white/[.08] bg-white/[.055] px-2 text-[12px] text-fg outline-none placeholder:text-muted focus:border-accent"
            />
            <Button type="submit" className="h-7 text-[12px]" disabled={!key.trim()}>
              Save
            </Button>
          </form>
        )}
        {error && <p className="text-danger">{error}</p>}
        <button type="button" onClick={() => api.openSoundPage(FREESOUND_KEYS)} className="flex items-center gap-1 self-start font-medium text-accent hover:underline">
          Get a key <ExternalLink size={12} />
        </button>
      </div>
      <p className="border-t border-white/[.07] pt-3 text-muted">Searching sends only what you type, never your project.</p>
    </div>
  );
}

function SoundSearch({ kind }: { kind: SoundKind }) {
  const query = useSounds((s) => s.search[kind].query);
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(timer.current), []);
  // Each search counts against Openverse's limit, so typing waits for a pause.
  const change = (text: string, now = false) => {
    window.clearTimeout(timer.current);
    useSounds.setState((s) => ({ search: { ...s.search, [kind]: { ...s.search[kind], query: text } } }));
    if (now || !text.trim()) searchSounds(kind, text);
    else if (text.trim().length >= 2) timer.current = window.setTimeout(() => searchSounds(kind, text), 600);
  };
  return <SearchField value={query} onChange={change} onEnter={() => change(query, true)} placeholder={kind === "music" ? "Search free music" : "Search sound effects"} />;
}

/** Music from YouTube has no credit; its file in the library is named after the video. */
const YOUTUBE_FILE = /^youtube-([\w-]{11})\.\w+$/;

/** Ids of the library sounds the project already has. A credited video, such as the tour's sample from YouTube, is no sound. */
function useInProject() {
  const assets = useEditor((s) => s.snap?.project.assets);
  return new Set(
    (assets ?? []).flatMap((a) => {
      if (a.kind !== "audio") return [];
      if (a.credit) return [`${a.credit.source}:${a.credit.id}`];
      const video = YOUTUBE_FILE.exec(a.path.split(/[\\/]/).pop() ?? "")?.[1];
      return video ? [`youtube:${video}`] : [];
    }),
  );
}

function OnlineResults({ kind, titled = kind === "effect" }: { kind: SoundKind; titled?: boolean }) {
  const search = useSounds((s) => s.search[kind]);
  const inProject = useInProject();
  if (!search.query.trim()) return null;
  const offline = search.error?.startsWith("OFFLINE");
  return (
    <section aria-label="Online results" className="flex flex-col gap-0.5">
      {titled && <h3 className="mx-1.5 mt-2 mb-1 text-[13px] font-semibold text-fg">Online</h3>}
      {search.sounds.map((sound) => (
        <SoundRow key={sound.id} sound={sound} inProject={inProject.has(sound.id)} />
      ))}
      {search.loading && (
        <div role="status" aria-label="Searching">
          {[0, 1, 2].map((i) => (
          <div key={i} className="flex items-center gap-2.5 p-1.5">
            <div className="skeleton h-10 w-10 shrink-0 rounded-md" />
            <div className="flex flex-1 flex-col gap-1.5">
              <div className="skeleton h-3 w-2/3 rounded" />
              <div className="skeleton h-2.5 w-1/3 rounded" />
            </div>
          </div>
          ))}
        </div>
      )}
      {search.error && offline && (
        <div className="mx-1.5 flex items-center gap-2 text-[12px] text-muted" role="alert">
          <WifiOff size={14} className="shrink-0" />
          <span className="min-w-0 flex-1">You're offline.</span>
          <Button className="h-7 text-[12px]" onClick={() => searchSounds(kind, search.query)}>
            Try again
          </Button>
        </div>
      )}
      {search.error && !offline && (
        <div className="mx-1.5 my-2 flex flex-col items-center gap-2 rounded-xl p-4 text-center" role="alert">
          <p className="text-[13px] font-semibold text-fg">Search didn't work</p>
          <p className="text-[12px] text-muted">{plainError(search.error)}</p>
          <Button className="mt-1 h-7 text-[12px]" onClick={() => searchSounds(kind, search.query)}>
            Try again
          </Button>
        </div>
      )}
      {!search.loading && !search.error && search.page > 0 && search.sounds.length === 0 && (
        <p className="mx-1.5 my-3 text-[12px] text-muted">Nothing free to use in videos matches “{search.query.trim()}”. Try a simpler word in English.</p>
      )}
      {search.more && !search.loading && (
        <Button className="mx-1.5 mt-1 h-7 text-[12px]" onClick={() => searchSounds(kind, search.query, search.page + 1)}>
          More results
        </Button>
      )}
      {search.service && (
        <p className="mx-1.5 mt-3 text-[11px] leading-[15px] text-muted">
          {search.service === "Openverse" && "Search uses Openverse. Nuzky is not endorsed or certified by Openverse. "}Licence data can be wrong, so check a sound's page before you publish.
        </p>
      )}
    </section>
  );
}

function Effects() {
  const all = useSounds((s) => s.builtIn);
  const builtIn = all.filter((s) => s.kind === "effect");
  const query = useSounds((s) => s.search.effect.query.trim().toLowerCase());
  const [category, setCategory] = useState<string | null>(null);
  const inProject = useInProject();
  const categories = [...new Set(builtIn.map((s) => s.category ?? ""))];
  const shown = builtIn.filter((s) => (query ? [s.title, s.category ?? ""].some((t) => t.toLowerCase().includes(query)) : !category || s.category === category));
  // Kinds and the list scroll together, so the smallest window still shows several sounds.
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-2 pb-3.5">
      {!query && (
        <div role="group" aria-label="Kinds of sound effects" className="flex flex-wrap gap-1.5 px-1.5 pb-2">
          {[null, ...categories].map((c) => (
            <button
              key={c ?? "all"}
              type="button"
              aria-pressed={category === c}
              onClick={() => setCategory(c)}
              className={`h-[26px] rounded-full px-2.5 text-[12px] font-medium transition-colors duration-[120ms] ease-out ${
                category === c ? "bg-accent/[.18] text-fg shadow-[inset_0_0_0_1px_rgb(41_151_255/.5)]" : "bg-white/[.06] text-muted hover:text-fg"
              }`}
            >
              {c ?? "All"}
            </button>
          ))}
        </div>
      )}
      {shown.length > 0 && (
        <section aria-label="Built in" className="flex flex-col gap-0.5">
          {query && (
            <h3 className="mx-1.5 mb-1 flex items-baseline gap-1.5 text-[13px] font-semibold text-fg">
              Built in <span className="tabular text-[12px] font-normal text-muted">{shown.length}</span>
            </h3>
          )}
          {shown.map((sound) => (
            <SoundRow key={sound.id} sound={sound} inProject={inProject.has(sound.id)} showKind={!!query || !category} />
          ))}
        </section>
      )}
      <OnlineResults kind="effect" />
    </div>
  );
}

/** Where a YouTube or YouTube Music link is pasted; pasting looks the video up at once. */
function LinkField() {
  const url = useSounds((s) => s.link.url);
  const [text, setText] = useState(url);
  useEffect(() => setText(url), [url]);
  const clear = () => {
    setText("");
    openLink("");
  };
  return (
    <form
      className="relative flex h-8 min-w-0 flex-1 items-center"
      onSubmit={(e) => {
        e.preventDefault();
        openLink(text);
      }}
    >
      <Link2 size={14} className="pointer-events-none absolute left-2.5 text-muted" />
      <input
        type="text"
        inputMode="url"
        value={text}
        placeholder="Paste a YouTube link"
        aria-label="Paste a YouTube link"
        spellCheck={false}
        autoComplete="off"
        onChange={(e) => setText(e.target.value)}
        onPaste={(e) => {
          const pasted = e.clipboardData.getData("text").trim();
          if (!pasted || (e.target as HTMLInputElement).value) return;
          e.preventDefault();
          setText(pasted);
          openLink(pasted);
        }}
        onKeyDown={(e) => {
          // Typing must not reach the editor's shortcuts; Escape on an empty field still does.
          if (e.key === "Escape" && !text) return;
          e.stopPropagation();
          if (e.key === "Escape") clear();
        }}
        className={`h-8 w-full text-ellipsis rounded-lg border border-white/[.08] bg-white/[.055] pl-8 text-[13px] text-fg outline-none placeholder:text-muted focus:border-accent ${text ? "pr-7" : "pr-2"}`}
      />
      {text && (
        <button type="button" aria-label="Clear link" onClick={clear} className="absolute right-1.5 flex h-5 w-5 items-center justify-center rounded-full text-muted hover:text-fg">
          <X size={13} />
        </button>
      )}
    </form>
  );
}

/** The video of the pasted link as a row to play and add, or why it can't be. */
function LinkResult() {
  const link = useSounds((s) => s.link);
  const inProject = useInProject();
  if (link.loading)
    return (
      <div role="status" aria-label="Looking up the link" className="flex items-center gap-2.5 p-1.5">
        <div className="skeleton h-10 w-10 shrink-0 rounded-md" />
        <div className="flex flex-1 flex-col gap-1.5">
          <div className="skeleton h-3 w-2/3 rounded" />
          <div className="skeleton h-2.5 w-1/3 rounded" />
        </div>
      </div>
    );
  if (link.sound) return <SoundRow sound={link.sound} inProject={inProject.has(link.sound.id)} />;
  if (!link.error) return null;
  if (link.error.startsWith("YTDLP_MISSING")) return <GetYtdlp />;
  if (link.error.startsWith("YTDLP_OUTDATED")) return <GetYtdlp outdated />;
  const offline = link.error.startsWith("OFFLINE");
  return (
    <div role="alert" className="mx-1.5 mb-1 flex items-start gap-2 text-[12px] text-muted">
      {offline ? <WifiOff size={14} className="mt-px shrink-0" /> : <AlertCircle size={14} className="mt-px shrink-0 text-warn" />}
      <span className="min-w-0 flex-1 select-text">
        {plainError(link.error)
          .split(/(yt-dlp)/)
          .map((part, i) => (part === "yt-dlp" ? <Nowrap key={i}>yt-dlp</Nowrap> : part))}
      </span>
      {!link.error.startsWith("INVALID_LINK") && !link.error.startsWith("PLAYLIST") && (
        <Button className="h-7 shrink-0 text-[12px]" onClick={() => openLink(link.url)}>
          Try again
        </Button>
      )}
    </div>
  );
}

/** Keeps "yt-dlp" whole, as a name split at its hyphen reads as two words. */
const Nowrap = ({ children }: { children: string }) => <span className="whitespace-nowrap">{children}</span>;

/** Nuzky downloads yt-dlp itself, so music from YouTube needs no terminal; it also replaces an outdated one the user installed. */
function GetYtdlp({ outdated = false }: { outdated?: boolean }) {
  const installing = useSounds((s) => s.installing);
  const progress = useSounds((s) => s.progress["yt-dlp"] ?? 0);
  return (
    <div role="alert" className="mx-1.5 mb-1 flex items-center gap-2 text-[12px] text-muted">
      <span className="min-w-0 flex-1">
        {installing ? (
          <>
            Downloading <Nowrap>yt-dlp</Nowrap>… <span className="tabular">{Math.round(progress * 100)}%</span>
          </>
        ) : outdated ? (
          <>
            The <Nowrap>yt-dlp</Nowrap> on this computer couldn't get this song, probably because it's out of date. Nuzky can download its own and keep it updated.
          </>
        ) : (
          <>
            Music from YouTube comes through <Nowrap>yt-dlp</Nowrap>, a free tool. Nuzky can download it for you.
          </>
        )}
      </span>
      {installing ? (
        <Button className="h-7 shrink-0 text-[12px]" onClick={() => api.soundCancel("yt-dlp")}>
          Stop
        </Button>
      ) : (
        <Button variant="primary" className="h-7 shrink-0 text-[12px]" onClick={() => installYtdlp(true)}>
          Download yt-dlp
        </Button>
      )}
    </div>
  );
}

/** Asked once, before the first link is looked up: the music's rights are the user's to check. */
function RightsDialog() {
  const open = useSounds((s) => s.rightsFor !== null);
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const d = dialog.current;
    if (!d) return;
    if (!open) return void (d.open && d.close());
    if (!d.open) {
      d.showModal();
      d.focus();
    }
  }, [open]);
  return (
    <dialog
      data-exit
      ref={dialog}
      tabIndex={-1}
      aria-labelledby="youtube-rights-title"
      aria-describedby="youtube-rights-text"
      onCancel={(e) => {
        e.preventDefault();
        answerRights(false);
      }}
      onKeyDown={(e) => {
        // The editor's keys (Space plays, Delete removes) wait until it is answered.
        e.stopPropagation();
        if (e.key === "Enter" && e.target === e.currentTarget) answerRights(true);
        trapTab(e);
      }}
      className="dialog-in overlay fixed inset-0 m-auto w-[440px] rounded-[20px] p-6 text-fg outline-none backdrop:bg-black/45"
    >
      {open && (
        // Answering edits no project, so an agent's run does not lock it.
        <LockReason.Provider value={null}>
          <h2 id="youtube-rights-title" className="text-[17px] font-semibold leading-[22px] tracking-[-0.025em]">
            Music from YouTube is yours to clear
          </h2>
          <div id="youtube-rights-text" className="mt-2 flex flex-col gap-2 text-[13px] text-fg/85">
            <p>
              Nuzky downloads only the sound of the video you link, with <Nowrap>yt-dlp</Nowrap> on this computer.
            </p>
            <p>
              You're responsible for having the rights to use the music in your video and for following YouTube's Terms of Service. Most songs on YouTube are copyrighted, and a video that
              uses one can be muted, blocked or demonetized.
            </p>
          </div>
          <div className="mt-6 flex justify-end gap-2">
            <Button pill onClick={() => answerRights(false)}>
              Cancel
            </Button>
            <Button pill variant="primary" onClick={() => answerRights(true)}>
              I understand
            </Button>
          </div>
        </LockReason.Provider>
      )}
    </dialog>
  );
}

function Music() {
  const query = useSounds((s) => s.search.music.query);
  const linked = useSounds((s) => s.link.sound?.id);
  const downloaded = useSounds((s) => s.downloaded);
  const builtIn = useSounds((s) => s.builtIn);
  const inProject = useInProject();
  // A song the user or an agent adds from a link joins the list with the asset.
  const assets = useEditor((s) => s.snap?.project.assets.length);
  useEffect(() => {
    loadDownloaded().catch((e) => useEditor.getState().toast({ kind: "error", text: `The songs from YouTube could not be read: ${errorText(e)}` }));
  }, [assets]);
  const words = query.trim().toLowerCase();
  const matching = (s: Sound) => !words || [s.title, s.author].some((t) => t.toLowerCase().includes(words));
  const songs = builtIn.filter((s) => s.kind === "music" && matching(s));
  const shown = downloaded.filter((s) => s.id !== linked && matching(s));
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-2 pb-3.5">
      <LinkResult />
      {!words && (
        <div role="group" aria-label="Music ideas" className="flex flex-wrap gap-1.5 px-1.5 pb-2">
          {MUSIC_IDEAS.map((idea) => (
            <button key={idea} type="button" onClick={() => searchSounds("music", idea)} className="h-[26px] rounded-full bg-white/[.06] px-2.5 text-[12px] font-medium text-muted transition-colors duration-[120ms] ease-out hover:text-fg">
              {idea}
            </button>
          ))}
        </div>
      )}
      {songs.length > 0 && (
        <section aria-label="Built in" className="flex flex-col gap-0.5">
          <h3 className="mx-1.5 mb-1 flex items-baseline gap-1.5 text-[13px] font-semibold text-fg">
            Built in <span className="tabular text-[12px] font-normal text-muted">{songs.length}</span>
          </h3>
          {songs.map((sound) => (
            <SoundRow key={sound.id} sound={sound} inProject={inProject.has(sound.id)} />
          ))}
        </section>
      )}
      {shown.length > 0 && (
        <section aria-label="From YouTube" className={`flex flex-col gap-0.5 ${songs.length > 0 ? "mt-2" : ""}`}>
          <h3 className="mx-1.5 mb-1 flex items-baseline gap-1.5 text-[13px] font-semibold text-fg">
            From YouTube <span className="tabular text-[12px] font-normal text-muted">{shown.length}</span>
          </h3>
          {shown.map((sound) => (
            <SoundRow key={sound.id} sound={sound} inProject={inProject.has(sound.id)} />
          ))}
        </section>
      )}
      {words && <OnlineResults kind="music" titled={shown.length + songs.length > 0} />}
    </div>
  );
}

/** Music or sound effects to search, preview and add at the playhead. */
export function SoundLibrary({ kind }: { kind: SoundKind }) {
  const [sources, setSources] = useState(false);
  const sourcesButton = useRef<HTMLDivElement>(null);
  useEffect(() => {
    loadBuiltIn().catch((e) => useEditor.getState().toast({ kind: "error", text: `The built-in sound effects could not be read: ${errorText(e)}` }));
    return () => {
      if (useSounds.getState().preview) stopPreview();
    };
  }, []);
  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 items-center gap-1.5 px-3.5 pt-1 pb-2">
        <SoundSearch kind={kind} />
        <div ref={sourcesButton} className="contents">
          <IconButton label="Where to search" data-sources-button aria-haspopup="dialog" aria-expanded={sources} onClick={() => setSources((open) => !open)}>
            <SlidersHorizontal size={16} />
          </IconButton>
        </div>
      </div>
      {kind === "music" && (
        <div className="flex shrink-0 px-3.5 pb-2">
          <LinkField />
        </div>
      )}
      <RightsDialog />
      {sources && (
        <Sources
          onClose={(keyboard) => {
            setSources(false);
            if (keyboard) sourcesButton.current?.querySelector("button")?.focus();
          }}
        />
      )}
      {kind === "music" ? <Music /> : <Effects />}
    </div>
  );
}

/** How many CC BY sounds and videos play on the timeline; their credits go in the video's description. */
export function CreditsBar() {
  const bar = useRef<HTMLDivElement>(null);
  // What needs credit, by kind, as a string, so the bar redraws only when that changes: "1 video and 2 sounds need credit".
  const needed = useEditor((s) => {
    const project = s.snap?.project;
    if (!project) return null;
    const used = new Set(project.tracks.flatMap((t) => t.clips.flatMap((c) => (c.content.type === "media" ? [c.content.assetId] : []))));
    const credited = project.assets.filter((a) => a.credit?.license === "by" && used.has(a.id));
    if (!credited.length) return null;
    const parts = (["video", "image", "audio"] as const)
      .map((kind) => [credited.filter((a) => a.kind === kind).length, { video: "video", image: "picture", audio: "sound" }[kind]] as const)
      .filter(([n]) => n > 0)
      .map(([n, word]) => `${n} ${word}${n === 1 ? "" : "s"}`);
    const what = parts.length > 1 ? `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}` : parts[0];
    return `${what} ${credited.length === 1 ? "needs" : "need"} credit`;
  });
  // Toasts move above the bar while it shows, so they never cover Copy credits.
  useLayoutEffect(() => {
    if (!needed) return;
    useEditor.setState({ toastLift: bar.current?.offsetHeight ?? 0 });
    return () => useEditor.setState({ toastLift: 0 });
  }, [needed]);
  if (!needed) return null;
  return (
    <div ref={bar} className="flex shrink-0 items-center gap-2 border-t border-white/[.07] px-3.5 py-2.5 text-[12px] text-muted">
      <span className="min-w-0 flex-1">{needed}</span>
      <Button pill className="h-7 gap-1.5 text-[12px]" onClick={copyCredits} title="Copy the credits for the video's description">
        <Copy size={13} /> Copy credits
      </Button>
    </div>
  );
}
