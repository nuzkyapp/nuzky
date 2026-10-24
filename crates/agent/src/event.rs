use serde::Serialize;
use serde_json::{Value, json};

/// What the panel shows of a turn, in order. A turn ends with exactly one `Done` or `Error`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AgentEvent {
    /// Words of the agent's answer, appended as they stream in.
    Text {
        text: String,
    },
    /// A tool call, first as running (with its arguments once known), then done or failed.
    Tool {
        id: String,
        name: String,
        status: ToolStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        input: Option<Value>,
    },
    /// Comes just before `Done` or `Error` once the agent has kept the conversation: the next
    /// message resumes this id. It is for Nuzky, not for the panel to show.
    Session {
        id: String,
    },
    Done {
        stopped: bool,
    },
    Error {
        code: ErrorCode,
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolStatus {
    Running,
    Done,
    Failed,
}

/// Prefixes the panel reads to say what to do next.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    NotSignedIn,
    UsageLimit,
    /// Signed in to ChatGPT, but it does not let Nuzky use the account's plan.
    PlanUnavailable,
    AgentFailed,
    SessionGone,
}

/// The step input of a web search action as OpenAI reports it to Codex and ChatGPT. A search can name
/// several `queries` in place of one `query`; the step shows them all, so different searches keep their own rows.
pub(crate) fn web_search_input(action: &Value) -> Option<Value> {
    let queries: Vec<&str> = action["queries"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let query =
        if queries.is_empty() { action["query"].as_str().map(str::to_owned) } else { Some(queries.join(" · ")) };
    match (query, action["url"].as_str()) {
        (Some(query), _) => Some(json!({"query": query})),
        (None, Some(url)) => Some(json!({"url": url})),
        (None, None) => None,
    }
}
