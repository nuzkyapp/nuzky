//! Runs the agent for one turn of Nuzky's AI panel. Claude Code runs as `claude -p`, Codex as
//! `codex exec --json`, each with its built-in tools off but its provider's web search, and Nuzky's
//! MCP server for everything else; one process serves one message and the next continues the
//! conversation with the agent's own resume. ChatGPT runs in Nuzky on the user's ChatGPT sign-in,
//! with Nuzky's tools and OpenAI's web search as its only tools. Every search shows as a step.
//! See docs/AI-ARCHITECTURE.md, "AI panel".

mod chatgpt;
mod claude;
mod codex;
mod discover;
mod event;
mod turn;

pub use chatgpt::{ChatGptRequest, Tools};
pub use discover::{AgentId, AgentInfo, SignIn, find, run, run_with_input, signed_in};
pub use event::{AgentEvent, ErrorCode, ToolStatus};
pub use turn::{McpServer, Message, Turn, TurnRequest};
