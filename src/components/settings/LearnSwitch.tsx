import { useEffect, useState } from "react";
import { api, errorText, plainError } from "../../lib/api";
import type { StyleSettings } from "../../lib/types";
import { Switch } from "../ui";
import { Item } from "./parts";

/** Whether Your style learns by itself from exports and from changes to a kept AI cut; Learn my style… works either way. */
export function LearnSwitch() {
  const [settings, setSettings] = useState<StyleSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => void api.styleSettings().then(setSettings, (e) => setError(plainError(errorText(e)))), []);
  const change = async (learnFromEdits: boolean) => {
    setError(null);
    try {
      setSettings(await api.setStyleSettings({ learnFromEdits }));
    } catch (e) {
      setError(plainError(errorText(e)));
    }
  };
  return (
    <Item
      id="learn"
      title="Learn from my edits"
      detail={
        error ?? (settings?.learnFromEdits === false ? "Off. Your style learns only when you choose Learn my style…" : "After each export Nuzky reads what you kept and cut. Nothing leaves this computer.")
      }
    >
      {settings && <Switch label="Learn from my edits" checked={settings.learnFromEdits} onChange={(on) => void change(on)} />}
    </Item>
  );
}
