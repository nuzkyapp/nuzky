//! Runs the agent for one turn of Nuzky's AI panel. Claude Code runs as `claude -p`, Codex as
//! `codex exec --json`, each with its built-in tools off and Nuzky's MCP server as its only tools;
//! one process serves one message and the next continues the conversation with the agent's own
//! resume. ChatGPT runs in Nuzky on the user's ChatGPT sign-in, with Nuzky's tools as its only
//! tools. See docs/AI-ARCHITECTURE.md, "AI panel".

mod chatgpt;
mod claude;
mod codex;
mod discover;
mod event;
mod turn;

pub use chatgpt::{ChatGptRequest, Tools};
pub use discover::{AgentId, AgentInfo, SignIn, find, run, signed_in};
pub use event::{AgentEvent, ErrorCode, ToolStatus};
pub use turn::{McpServer, Turn, TurnRequest};
