import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { Pencil, Trash2 } from "lucide-react";
import { MAX_NAMES, MAX_NAME_CHARS, deleteName, loadNames, renameName, saveName, useNames } from "../../lib/names";
import { Button, IconButton } from "../ui";
import { Group } from "./parts";

const INPUT =
  "h-7 min-w-0 flex-1 rounded-[6px] border border-white/[.08] bg-white/[.055] px-2 text-[12px] text-fg outline-none placeholder:text-muted focus:border-accent";

/** Settings › Models › Names: the names speech recognition reads as a hint, to add, spell anew and delete. */
export function NamesSection() {
  const { names, error } = useNames();
  const [adding, setAdding] = useState("");
  const [editing, setEditing] = useState<{ from: string; to: string } | null>(null);
  // Esc closes the field without saving, and the blur that follows must not save it either.
  const cancelled = useRef(false);
  useEffect(() => void loadNames(), []);
  const full = (names?.length ?? 0) >= MAX_NAMES;
  const add = async () => {
    if (adding.trim() && (await saveName(adding))) setAdding("");
  };
  const rename = async () => {
    if (!editing || cancelled.current) {
      cancelled.current = false;
      return;
    }
    if (editing.to.trim() === editing.from || (editing.to.trim() && (await renameName(editing.from, editing.to)))) setEditing(null);
  };
  const editKeys = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") e.currentTarget.blur();
    if (e.key === "Escape") {
      e.stopPropagation();
      cancelled.current = true;
      e.currentTarget.blur();
      setEditing(null);
    }
  };
  return (
    <Group
      title="Names"
      id="names"
      note="Transcription reads these as a hint, so it is likelier to spell them your way, but it can still mishear one. Transcripts you already have stay as they are."
    >
      {names?.map((name) => (
        <div key={name} className="flex min-h-[40px] items-center gap-2 border-t border-white/[.06] px-3.5 py-1.5 first:border-t-0">
          {editing?.from === name ? (
            <input
              autoFocus
              value={editing.to}
              maxLength={MAX_NAME_CHARS}
              onChange={(e) => setEditing({ from: name, to: e.target.value })}
              onKeyDown={editKeys}
              onBlur={() => void rename()}
              aria-label={`Spell “${name}” as`}
              className={INPUT}
            />
          ) : (
            <span className="min-w-0 flex-1 truncate text-[13px] text-fg">{name}</span>
          )}
          <IconButton label={`Spell “${name}” differently`} onClick={() => setEditing({ from: name, to: name })}>
            <Pencil size={14} />
          </IconButton>
          <IconButton label={`Delete “${name}”`} onClick={() => void deleteName(name)}>
            <Trash2 size={14} />
          </IconButton>
        </div>
      ))}
      {names?.length === 0 && !error && (
        <p className="px-3.5 py-2.5 text-[12px] text-muted">None yet. Save a name from a word you correct in the transcript, or add it here.</p>
      )}
      {error && <p className="px-3.5 py-2.5 text-[12px] text-danger">{error}</p>}
      <form
        className="flex gap-2 border-t border-white/[.06] px-3.5 py-2.5 first:border-t-0"
        onSubmit={(e) => {
          e.preventDefault();
          void add();
        }}
      >
        <input
          value={adding}
          maxLength={MAX_NAME_CHARS}
          onChange={(e) => setAdding(e.target.value)}
          placeholder="Claude Code, Nůžky…"
          aria-label="Name to add"
          className={INPUT}
        />
        <Button
          type="submit"
          className="h-7 text-[12px]"
          disabled={!adding.trim() || full}
          disabledReason={full ? `Nuzky keeps up to ${MAX_NAMES} names. Delete one you no longer say first.` : "Type a name first"}
        >
          Add
        </Button>
      </form>
    </Group>
  );
}
