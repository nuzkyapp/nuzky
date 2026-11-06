import { useEffect, useState } from "react";
import { ExternalLink } from "lucide-react";
import { api, errorText, plainError } from "../../lib/api";
import { searchSounds, useSounds } from "../../lib/sounds";
import type { SoundSettings } from "../../lib/types";
import { Button, Checkbox } from "../ui";

const FREESOUND_KEYS = "https://freesound.org/apiv2/apply/";
export const OPENVERSE = "Music and sounds from Freesound and Wikimedia Commons. Only CC0 and CC BY.";
export const SEARCH_SENDS = "Searching sends only what you type, never your project.";

/** Searching effects on Freesound with the user's own key: the switch, the key and where to get one. In Where to search and in Settings. */
export function FreesoundKey() {
  const [settings, setSettings] = useState<SoundSettings | null>(null);
  const [key, setKey] = useState("");
  const [error, setError] = useState<string | null>(null);
  useEffect(() => void api.soundSettings().then(setSettings), []);
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
    <>
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
    </>
  );
}
