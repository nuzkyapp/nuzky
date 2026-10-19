import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import { errorText, plainError } from "./api";
import type { AgentId } from "./agent";
import type { AgentModel } from "./types";

/** The model and effort the AI panel sends with; null sends none, so the agent's own default applies. */
export type Choice = { model: string | null; effort: string | null };

type Models = {
  /** Each agent's models once loaded; ChatGPT's belong to the account in `chatgptFor`. */
  lists: Partial<Record<AgentId, AgentModel[]>>;
  errors: Partial<Record<AgentId, string>>;
  chatgptFor: string | null;
  choices: Record<AgentId, Choice>;
};

const key = (agent: AgentId) => `nuzky.agentModel.${agent}`;

function saved(agent: AgentId): Choice {
  try {
    const c = JSON.parse(localStorage.getItem(key(agent)) ?? "null");
    if (c && typeof c === "object") return { model: typeof c.model === "string" ? c.model : null, effort: typeof c.effort === "string" ? c.effort : null };
  } catch {
    // A broken value is the same as none.
  }
  // Before effort could be chosen, ChatGPT's model was kept on its own.
  return { model: agent === "chatgpt" ? localStorage.getItem("nuzky.chatgptModel") : null, effort: null };
}

export const useModels = create<Models>(() => ({
  lists: {},
  errors: {},
  chatgptFor: null,
  choices: { claude: saved("claude"), codex: saved("codex"), chatgpt: saved("chatgpt") },
}));

export const EFFORTS: Record<string, { label: string; short: string; hint: string }> = {
  low: { label: "Low", short: "Low", hint: "Fastest, for small, clear changes." },
  medium: { label: "Medium", short: "Med", hint: "Quick, for most edits." },
  high: { label: "High", short: "High", hint: "Thinks longer, for a whole video." },
  xhigh: { label: "Extra high", short: "X-High", hint: "Thinks much longer, for tricky edits." },
  max: { label: "Max", short: "Max", hint: "Slowest, uses the most of your plan." },
};

export const effortLabel = (effort: string) => EFFORTS[effort]?.label ?? effort.charAt(0).toUpperCase() + effort.slice(1);

/**
 * What the panel sends with now. Claude Code keeps its own setting until a model is chosen; Codex and ChatGPT
 * start with the first model of their list and its default effort. An effort the model does not take falls
 * back to the model's default, and comes back with a model that takes it.
 */
export function resolved(agent: AgentId, s: Models = useModels.getState()) {
  const list = s.lists[agent] ?? [];
  const choice = s.choices[agent];
  const model = list.find((m) => m.slug === choice.model) ?? (agent === "claude" ? null : (list[0] ?? null));
  // With Claude Code's own model, its effort levels are those all its models take.
  const efforts = model?.efforts ?? (agent === "claude" ? (list[0]?.efforts ?? []) : []);
  const effort = choice.effort && efforts.includes(choice.effort) ? choice.effort : (model?.defaultEffort ?? null);
  return { model, effort, efforts };
}

function choose(agent: AgentId, choice: Choice) {
  localStorage.setItem(key(agent), JSON.stringify(choice));
  useModels.setState((s) => ({ choices: { ...s.choices, [agent]: choice } }));
}

/** `null` is Claude Code's own setting, for the effort too. */
export function chooseModel(agent: AgentId, slug: string | null) {
  choose(agent, slug === null ? { model: null, effort: null } : { ...useModels.getState().choices[agent], model: slug });
}

export function chooseEffort(agent: AgentId, effort: string) {
  choose(agent, { ...useModels.getState().choices[agent], effort });
}

const loading = new Set<AgentId>();

/** `account`: whose ChatGPT models these are, so an answer for an account signed out meanwhile is dropped. */
export async function loadModels(agent: AgentId, account: string | null = null) {
  if (agent === "chatgpt") useModels.setState({ chatgptFor: account });
  else if (loading.has(agent)) return;
  loading.add(agent);
  useModels.setState((s) => ({ lists: { ...s.lists, [agent]: undefined }, errors: { ...s.errors, [agent]: undefined } }));
  try {
    const list = await invoke<AgentModel[]>("agent_models", { agent });
    if (agent === "chatgpt" && useModels.getState().chatgptFor !== account) return;
    useModels.setState((s) => ({ lists: { ...s.lists, [agent]: list } }));
  } catch (e) {
    if (agent === "chatgpt" && useModels.getState().chatgptFor !== account) return;
    useModels.setState((s) => ({ errors: { ...s.errors, [agent]: plainError(errorText(e)) } }));
  } finally {
    loading.delete(agent);
  }
}

/** ChatGPT signed out: its list belonged to the account. */
export function forgetChatGptModels() {
  useModels.setState((s) => ({ chatgptFor: null, lists: { ...s.lists, chatgpt: undefined }, errors: { ...s.errors, chatgpt: undefined } }));
}
