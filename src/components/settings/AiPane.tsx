import { closeSettings } from "../../lib/settings";
import { openStyle, useStyle } from "../../lib/style";
import { Button } from "../ui";
import { AgentsSection } from "./AgentsSection";
import { ChatGptSection } from "./ChatGptSection";
import { LearnSwitch } from "./LearnSwitch";
import { Group, Item } from "./parts";

/** Settings › AI: the ChatGPT account, the agents outside Nuzky, and how Your style learns. */
export function AiPane() {
  const rules = useStyle((s) => (s.view ? s.view.own.length + s.view.rules.length : null));
  const suggestions = useStyle((s) => s.view?.suggestions.length ?? 0);
  return (
    <div className="flex flex-col gap-6">
      <section data-setting="chatgpt" aria-label="ChatGPT account" className="flex flex-col gap-2 rounded-[14px]">
        <h3 className="text-[13px] font-semibold text-fg">ChatGPT account</h3>
        <ChatGptSection />
      </section>
      <section data-setting="agents" aria-label="Agents outside Nuzky" className="flex flex-col gap-2 rounded-[14px]">
        <h3 className="text-[13px] font-semibold text-fg">Agents outside Nuzky</h3>
        <AgentsSection />
      </section>
      <Group title="Your style">
        <LearnSwitch />
        <Item
          title="Your style"
          detail={rules === null ? "The rules the AI follows when it edits" : `${rules === 1 ? "1 rule" : `${rules} rules`}${suggestions ? ` · ${suggestions === 1 ? "1 suggestion waits" : `${suggestions} suggestions wait`}` : ""}`}
        >
          <Button
            pill
            onClick={() => {
              closeSettings();
              openStyle();
            }}
          >
            Open Your style
          </Button>
        </Item>
      </Group>
    </div>
  );
}
