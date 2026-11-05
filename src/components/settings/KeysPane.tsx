import { Fragment, useRef, useState, type KeyboardEvent } from "react";
import { AlertCircle, Check, ChevronDown, RotateCcw } from "lucide-react";
import { open as pickFile } from "@tauri-apps/plugin-dialog";
import { api, errorText } from "../../lib/api";
import { ACTIONS, PRESETS, choosePreset, chordsOf, keysOf, reserved, resetKey, setKey, takeImported, useKeys, type ActionId, type PresetId, type Saved } from "../../lib/keys";
import { Button, IconButton, Menu, SearchField } from "../ui";


const GROUPS = [...new Set(ACTIONS.map((a) => a.group))];
const labelOf = (id: ActionId) => ACTIONS.find((a) => a.id === id)!.label;

/** A key another action holds, waiting for Use it here; `reset` when ↺ asked for it, so the set's other keys come back too. */
type Pending = { id: ActionId; chord: string; from: ActionId; reset?: boolean };
/** The action other than `id` that holds `chord` now. */
const holder = (chord: string, id?: ActionId) => ACTIONS.find((a) => a.id !== id && keysOf(a.id).includes(chord))?.id;

/** Settings › Keyboard: the editor's keys, a set like another editor's, a file of keys from one, and each key by hand. */
export function KeysPane() {
  const s = useKeys();
  const [query, setQuery] = useState("");
  const [recording, setRecording] = useState<ActionId | null>(null);
  const [pending, setPending] = useState<Pending | null>(null);
  const [note, setNote] = useState<{ id: ActionId; text: string } | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; keyboard: boolean } | null>(null);
  // What the last Reset all, set or import did, with the keys before it for Undo.
  const [status, setStatus] = useState<{ ok: boolean; text: string; undo?: Saved } | null>(null);
  const dialog = useRef<HTMLDivElement>(null);
  // Space released after it was recorded would click the keys again and start waiting anew.
  const spaceTaken = useRef(false);
  const keyButton = (id: ActionId) => dialog.current?.querySelector<HTMLButtonElement>(`[data-keys="${id}"]`);

  const record = (id: ActionId, e: KeyboardEvent<HTMLButtonElement>) => {
    if (e.key === "Tab") return setRecording(null);
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") return setRecording(null);
    const chords = chordsOf(e.nativeEvent);
    if (!chords.length) return;
    spaceTaken.current = e.key === " ";
    setRecording(null);
    if (chords.some(reserved)) return setNote({ id, text: `${chords[0]} keeps its own job in Nuzky. Try another key.` });
    // Another editor's set names some keys by their place (Ctrl+Shift+[), so the key is taken in the form an action holds.
    const chord = chords.find((c) => holder(c)) ?? chords[0];
    const from = holder(chord, id);
    if (from) return setPending({ id, chord, from });
    setKey(id, chord);
  };

  const importFile = async () => {
    const path = await pickFile({ multiple: false, filters: [{ name: "Premiere Pro, Final Cut Pro or DaVinci Resolve keys", extensions: ["kys", "commandset", "txt"] }] });
    if (typeof path !== "string") return;
    setPending(null);
    setNote(null);
    const name = path.split(/[\\/]/).pop();
    const undo = useKeys.getState();
    try {
      const file = await api.importShortcuts(path);
      const { count, lost } = takeImported(file.program, file.keys);
      const none = lost.length ? ` ${lost.join(", ")} ${lost.length === 1 ? "has" : "have"} no key now.` : "";
      setStatus({ ok: true, text: `Imported ${count === 1 ? "1 key" : `${count} keys`} from “${name}” over ${PRESETS[file.program].name}'s keys.${none}`, undo });
    } catch (e) {
      setStatus({ ok: false, text: `Couldn't import “${name}”. ${errorText(e)} Export the keys from Premiere Pro (.kys), Final Cut Pro (.commandset) or DaVinci Resolve (.txt) and pick that file.` });
    }
  };

  /** Puts a whole set in place; changes made by hand go, with Undo. */
  const takeSet = (id: PresetId, text: string) => {
    const undo = useKeys.getState();
    setPending(null);
    setNote(null);
    choosePreset(id);
    const dropped = Object.keys(undo.changed).length;
    setStatus(dropped ? { ok: true, text: `${text} ${dropped === 1 ? "Your 1 changed key is" : `Your ${dropped} changed keys are`} gone.`, undo } : null);
  };

  const q = query.trim().toLowerCase();
  const shown = ACTIONS.filter((a) => !q || a.label.toLowerCase().includes(q) || keysOf(a.id, s).some((k) => k.toLowerCase().includes(q)));
  const changed = Object.keys(s.changed).length > 0;

  return (
    <div ref={dialog} className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-2">
        <Button
          data-preset
          pill
          aria-haspopup="menu"
          aria-expanded={!!menu}
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            setMenu({ x: r.left, y: r.bottom + 4, keyboard: e.detail === 0 });
          }}
          className="shrink-0"
        >
          Keys like {PRESETS[s.preset].name} <ChevronDown size={13} className="text-muted" />
        </Button>
        <Button pill className="shrink-0" onClick={() => void importFile()}>
          Import file…
        </Button>
        <SearchField value={query} onChange={setQuery} placeholder="Search actions or keys" />
      </div>
      {status && (
        <p role="status" className={`mt-3 flex items-start gap-1.5 text-[12px] ${status.ok ? "text-muted" : "text-danger"}`}>
          {status.ok ? <Check size={14} className="mt-px shrink-0 text-ok" /> : <AlertCircle size={14} className="mt-px shrink-0" />}
          <span className="min-w-0 flex-1">{status.text}</span>
          {status.undo && (
            <button
              type="button"
              onClick={() => {
                useKeys.setState(status.undo!);
                setStatus(null);
              }}
              className="-my-0.5 shrink-0 rounded-md px-1.5 py-0.5 font-semibold text-accent hover:bg-white/[.08]"
            >
              Undo
            </button>
          )}
        </p>
      )}
      <div className="mt-3" data-keys-list>
        {shown.length === 0 && <p className="py-6 text-center text-[13px] text-muted">No action or key matches “{query.trim()}”.</p>}
        {GROUPS.map((group) => {
          const rows = shown.filter((a) => a.group === group);
          if (rows.length === 0) return null;
          return (
            <section key={group} className="mt-3 first:mt-1">
              <h3 className="mb-1.5 text-[13px] font-semibold">{group}</h3>
              <div className="rounded-[10px] bg-white/[.035] px-2.5 shadow-[inset_0_0_0_1px_rgb(255_255_255/.05)]">
                {rows.map(({ id, label }) => {
                  const keys = keysOf(id, s);
                  const own = !!s.changed[id];
                  return (
                    <Fragment key={id}>
                      <div className="flex min-h-[38px] items-center gap-2 border-t border-white/[.05] first:border-t-0" data-action={id}>
                        <span className="min-w-0 flex-1 truncate text-[13px]">{label}</span>
                        <button
                          type="button"
                          data-keys={id}
                          aria-label={recording === id ? `${label}: press the new keys, Esc cancels` : `${label}: ${keys.join(" or ") || "no key"}. Change`}
                          onClick={() => {
                            setNote(null);
                            setPending(null);
                            setRecording(recording === id ? null : id);
                          }}
                          onKeyDown={(e) => recording === id && record(id, e)}
                          onKeyUp={(e) => {
                            if (e.key === " " && spaceTaken.current) {
                              e.preventDefault();
                              spaceTaken.current = false;
                            }
                          }}
                          onBlur={() => recording === id && setRecording(null)}
                          className={`flex h-7 shrink-0 items-center gap-1 rounded-md px-1 transition-colors duration-[120ms] ease-out ${recording === id ? "px-2.5 text-[12px] text-accent shadow-[inset_0_0_0_1.5px_var(--color-accent)]" : "hover:bg-white/[.06]"}`}
                        >
                          {recording === id ? "Press the new keys… Esc cancels" : keys.length ? keys.map((k, i) => <Keycap key={k} chord={k} changed={own} or={i > 0} />) : <span className="px-1 text-[12px] text-muted">No key</span>}
                        </button>
                        <span className="flex w-7 shrink-0 justify-center">
                          {own && (
                            <IconButton
                              label={`Back to ${PRESETS[s.preset].name}'s key${PRESETS[s.preset].keys[id].length ? ` (${PRESETS[s.preset].keys[id][0]})` : ""}`}
                              onClick={() => {
                                const held = PRESETS[s.preset].keys[id].find((k) => holder(k, id));
                                if (held) return setPending({ id, chord: held, from: holder(held, id)!, reset: true });
                                resetKey(id);
                                keyButton(id)?.focus();
                              }}
                              className="h-6 w-6"
                            >
                              <RotateCcw size={13} />
                            </IconButton>
                          )}
                        </span>
                      </div>
                      {note?.id === id && (
                        <p role="alert" className="mb-2 text-[12px] text-warn">
                          {note.text}
                        </p>
                      )}
                      {pending?.id === id && (
                        <div role="alert" className="mb-2 flex items-center gap-2 rounded-lg bg-warn/10 px-2.5 py-2 text-[12px] shadow-[inset_0_0_0_1px_rgb(255_159_10/.3)]">
                          <span className="min-w-0 flex-1">
                            <b className="font-semibold">{pending.chord}</b> is the key of {labelOf(pending.from)}. Use it here instead?
                            {keysOf(pending.from).length === 1 && ` ${labelOf(pending.from)} will have no key.`}
                          </span>
                          <Button
                            pill
                            className="h-7 shrink-0 px-3"
                            onClick={() => {
                              setPending(null);
                              keyButton(id)?.focus();
                            }}
                          >
                            Cancel
                          </Button>
                          <Button
                            pill
                            variant="primary"
                            autoFocus
                            className="h-7 shrink-0 px-3"
                            onClick={() => {
                              setKey(pending.id, pending.chord);
                              if (pending.reset) resetKey(pending.id);
                              setPending(null);
                              keyButton(id)?.focus();
                            }}
                          >
                            Use it here
                          </Button>
                        </div>
                      )}
                    </Fragment>
                  );
                })}
              </div>
            </section>
          );
        })}
      </div>
      <div className="mt-4 flex items-center justify-end">
        <Button pill disabled={!changed} disabledReason="Every key is as in the set you chose" onClick={() => takeSet(s.preset, `Every key is back as in ${PRESETS[s.preset].name}.`)}>
          Reset all
        </Button>
      </div>
      {menu && (
        <Menu
          label="Keys like"
          at={menu}
          keyboard={menu.keyboard}
          onClose={() => {
            setMenu(null);
            dialog.current?.querySelector<HTMLElement>("[data-preset]")?.focus();
          }}
          items={(Object.keys(PRESETS) as PresetId[]).map((id) => ({
            label: PRESETS[id].name,
            checked: id === s.preset,
            run: () => takeSet(id, `Keys like ${PRESETS[id].name}.`),
          }))}
        />
      )}
    </div>
  );
}

function Keycap({ chord, changed, or }: { chord: string; changed: boolean; or: boolean }) {
  return (
    <>
      {or && <span className="px-0.5 text-[11px] text-muted">or</span>}
      {chord.split(/\+(?!$)/).map((part, i) => (
        <kbd key={i} className={`min-w-[22px] rounded-[5px] px-1.5 text-center font-sans leading-[22px] ${/^[`',.;]$/.test(part) ? "text-[15px]" : "text-[12px]"} font-medium shadow-[inset_0_-1px_0_rgb(0_0_0/.35)] ${changed ? "bg-accent/[.18] text-fg" : "bg-white/[.08] text-fg/90"}`}>
          {part}
        </kbd>
      ))}
    </>
  );
}

