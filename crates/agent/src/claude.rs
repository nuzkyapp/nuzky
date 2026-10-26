//! Claude Code as `claude -p` with stream-json output: the command and a parser of its lines.

use crate::event::{AgentEvent, ErrorCode, ToolStatus};
use crate::turn::{Message, SYSTEM, Step, TurnRequest};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::process::Command;

/// Settings Nuzky needs; `outputStyle` doubles as a check that Claude applied them at all,
/// since it silently ignores a settings value it does not accept.
fn settings() -> Value {
    json!({"disableAllHooks": true, "outputStyle": "default", "autoMemoryEnabled": false, "claudeMdExcludes": ["**"]})
}

pub(crate) fn command(req: &TurnRequest) -> Command {
    let env: Map<String, Value> = req.mcp.env.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect();
    let mcp = json!({"mcpServers": {"nuzky": {"type": "stdio", "command": req.mcp.command, "args": req.mcp.args, "env": env}}});
    let mut c = Command::new(&req.exe);
    c.env("PATH", crate::discover::agent_path());
    c.args([
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
    ])
    // User settings keep the login, model and gateway; claudeMdExcludes skips user/project instructions and imports.
    // The folder's settings, hooks and foreign MCP servers stay out.
    .args(["--setting-sources", "user", "--settings", &settings().to_string()])
    .args(["--strict-mcp-config", "--mcp-config", &mcp.to_string()])
    // No shell, files or page fetching: only Nuzky's tools and Claude's own web search, allowed without asking.
    .args(["--tools", "WebSearch", "--allowedTools", "mcp__nuzky,WebSearch", "--permission-mode", "dontAsk"])
    .args(["--disable-slash-commands", "--append-system-prompt", SYSTEM])
    .args([if req.resume { "--resume" } else { "--session-id" }, &req.session]);
    if let Some(model) = &req.model {
        c.args(["--model", model]);
    }
    if let Some(effort) = &req.effort {
        c.args(["--effort", effort]);
    }
    c
}

/// A message for Claude's stream-json input, one line.
pub(crate) fn message(message: &Message) -> anyhow::Result<String> {
    // Without pictures, the plain text every Claude Code version takes.
    let content = if message.images.is_empty() {
        json!(message.text)
    } else {
        let mut blocks = vec![json!({"type": "text", "text": message.text})];
        for data in message.base64_images()? {
            blocks
                .push(json!({"type": "image", "source": {"type": "base64", "media_type": "image/jpeg", "data": data}}));
        }
        Value::Array(blocks)
    };
    Ok(format!("{}\n", json!({"type": "user", "message": {"role": "user", "content": content}})))
}

#[derive(Default)]
pub(crate) struct Parser {
    /// Text of the current message already streamed in pieces, so its full copy is skipped.
    streamed: bool,
    /// Tool names by call id; results carry only the id.
    tools: HashMap<String, String>,
    /// The tools Claude Code offered, each one `allowed`.
    offered: Vec<String>,
}

fn text_of(content: &Value) -> String {
    content.as_array().into_iter().flatten().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n")
}

fn tool(id: &str, name: &str, status: ToolStatus, input: Option<Value>) -> Step {
    Step::Event(AgentEvent::Tool { id: id.to_owned(), name: name.to_owned(), status, input })
}

/// The tools Nuzky gives Claude: its own, and Claude's web search, which runs on Anthropic's servers.
fn allowed(name: &str) -> bool {
    name.starts_with("mcp__nuzky__") || name == "WebSearch"
}

impl Parser {
    pub(crate) fn line(&mut self, line: &str) -> Vec<Step> {
        let Ok(v) = serde_json::from_str::<Value>(line) else { return Vec::new() };
        // Work of a subagent is not the panel's to show; none are available anyway.
        if !v["parent_tool_use_id"].is_null() {
            return Vec::new();
        }
        match v["type"].as_str() {
            Some("system") if v["subtype"] == "init" => {
                self.offered =
                    v["tools"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect();
                init(&v).into_iter().collect()
            }
            Some("stream_event") => {
                let e = &v["event"];
                match e["type"].as_str() {
                    Some("message_start") => {
                        self.streamed = false;
                        Vec::new()
                    }
                    Some("content_block_start") if e["content_block"]["type"] == "tool_use" => {
                        self.tool_use(&e["content_block"], None).into_iter().collect()
                    }
                    Some("content_block_delta") if e["delta"]["type"] == "text_delta" => {
                        self.streamed = true;
                        vec![Step::Event(AgentEvent::Text {
                            text: e["delta"]["text"].as_str().unwrap_or("").to_owned(),
                        })]
                    }
                    _ => Vec::new(),
                }
            }
            Some("assistant") => {
                let content = &v["message"]["content"];
                if let Some(error) = v["error"].as_str() {
                    let text = text_of(content);
                    return vec![match error {
                        "authentication_failed" | "oauth_org_not_allowed" => Step::Fail(ErrorCode::NotSignedIn, text),
                        "rate_limit" | "billing_error" => Step::Fail(ErrorCode::UsageLimit, text),
                        _ => Step::Fail(ErrorCode::AgentFailed, if text.is_empty() { error.to_owned() } else { text }),
                    }];
                }
                let mut out = Vec::new();
                for c in content.as_array().into_iter().flatten() {
                    match c["type"].as_str() {
                        Some("tool_use") => out.extend(self.tool_use(c, Some(c["input"].clone()))),
                        Some("text") if !self.streamed => out
                            .push(Step::Event(AgentEvent::Text { text: c["text"].as_str().unwrap_or("").to_owned() })),
                        _ => {}
                    }
                }
                out
            }
            Some("user") => v["message"]["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|c| c["type"] == "tool_result")
                .filter_map(|c| {
                    let id = c["tool_use_id"].as_str().unwrap_or("");
                    let status = if c["is_error"] == true { ToolStatus::Failed } else { ToolStatus::Done };
                    Some(tool(id, self.tools.get(id)?, status, None))
                })
                .collect(),
            Some("result") if v["is_error"] == true => {
                let text = v["result"].as_str().unwrap_or("").to_owned();
                vec![
                    Step::ResultError(if text.is_empty() { "Claude Code stopped with an error.".into() } else { text }),
                    Step::Answered,
                ]
            }
            Some("result") => vec![Step::Answered],
            _ => Vec::new(),
        }
    }

    fn tool_use(&mut self, block: &Value, input: Option<Value>) -> Option<Step> {
        let (id, name) = (block["id"].as_str().unwrap_or(""), block["name"].as_str().unwrap_or(""));
        // The guide names Nuzky's tools without Claude's prefix, and the model now and then calls one so.
        // Claude Code has no such tool and answers with an error, so the model calls it again by its full name.
        if self.offered.iter().any(|t| t.strip_prefix("mcp__nuzky__") == Some(name)) {
            return None;
        }
        if !allowed(name) {
            return Some(Step::Fail(
                ErrorCode::AgentFailed,
                format!(
                    "Claude Code used a tool Nuzky did not give it ({name}), so Nuzky stopped it. Update Claude Code and Nuzky."
                ),
            ));
        }
        let (name, input) = if name == "WebSearch" {
            ("web_search", input.map(|i| json!({"query": i["query"]})))
        } else {
            (name, input)
        };
        self.tools.insert(id.to_owned(), name.to_owned());
        Some(tool(id, name, ToolStatus::Running, input))
    }
}

/// Fails closed unless Claude runs with Nuzky's settings and nothing but Nuzky's tools and its web search.
fn init(v: &Value) -> Option<Step> {
    let foreign = v["tools"].as_array().into_iter().flatten().filter_map(Value::as_str).find(|t| !allowed(t));
    if let Some(tool) = foreign {
        return Some(Step::Fail(
            ErrorCode::AgentFailed,
            format!(
                "Claude Code offered a tool Nuzky did not give it ({tool}), so Nuzky stopped it. Update Claude Code and Nuzky."
            ),
        ));
    }
    let nuzky = v["mcp_servers"].as_array().into_iter().flatten().find(|s| s["name"] == "nuzky");
    if nuzky.is_none_or(|s| s["status"] != "connected") {
        return Some(Step::Fail(
            ErrorCode::AgentFailed,
            "Claude Code could not reach Nuzky's tools. Try again; if it keeps failing, restart Nuzky.".into(),
        ));
    }
    if v.get("output_style").is_some_and(|s| s != "default") {
        return Some(Step::Fail(
            ErrorCode::AgentFailed,
            "Claude Code ignored Nuzky's settings, so Nuzky stopped it. Update Claude Code and try again.".into(),
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_keeps_user_settings_without_instruction_files() {
        let req = TurnRequest {
            agent: crate::AgentId::Claude,
            exe: "claude".into(),
            session: "session".into(),
            resume: false,
            model: None,
            effort: None,
            prompt: Message::default(),
            cwd: Default::default(),
            mcp: crate::turn::McpServer { command: "nuzky".into(), args: vec![], env: vec![] },
        };
        let args: Vec<_> = command(&req).get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(args.windows(2).any(|a| a == ["--setting-sources", "user"]));
        assert!(args.windows(2).any(|a| a == ["--append-system-prompt", SYSTEM]));
        let settings = args.windows(2).find(|a| a[0] == "--settings").unwrap();
        let settings: Value = serde_json::from_str(&settings[1]).unwrap();
        assert_eq!(settings["claudeMdExcludes"], json!(["**"]));
        assert_eq!(settings["autoMemoryEnabled"], false);
    }

    fn events(lines: &[Value]) -> Vec<Result<AgentEvent, (ErrorCode, String)>> {
        let mut p = Parser::default();
        lines
            .iter()
            .flat_map(|l| p.line(&l.to_string()))
            .filter_map(|s| match s {
                Step::Event(e) => Some(Ok(e)),
                Step::Fail(c, m) => Some(Err((c, m))),
                Step::ResultError(m) => Some(Err((ErrorCode::AgentFailed, m))),
                Step::Answered => None,
            })
            .collect()
    }

    fn init_line(tools: &[&str], style: &str) -> Value {
        json!({"type":"system","subtype":"init","tools":tools,"mcp_servers":[{"name":"nuzky","status":"connected"}],"output_style":style})
    }

    // Shapes from a real claude 2.1.292 turn (docs/research/2026-10-08-acp-spike.md and the panel transport proposals).
    #[test]
    fn a_turn_streams_text_once_and_names_tool_results() {
        let lines = [
            init_line(&["mcp__nuzky__get_state"], "default"),
            json!({"type":"stream_event","parent_tool_use_id":null,"event":{"type":"message_start"}}),
            json!({"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_start","content_block":{"type":"tool_use","id":"t1","name":"mcp__nuzky__get_state","input":{}}}}),
            json!({"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"tool_use","id":"t1","name":"mcp__nuzky__get_state","input":{"range":null}}]}}),
            json!({"type":"user","parent_tool_use_id":null,"message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"{}"}]}]}}),
            json!({"type":"stream_event","parent_tool_use_id":null,"event":{"type":"message_start"}}),
            json!({"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hoto"}}}),
            json!({"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"vo."}}}),
            json!({"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"text","text":"Hotovo."}]}}),
            json!({"type":"result","subtype":"success","is_error":false,"result":"Hotovo."}),
        ];
        let get_state = |status, input| {
            Ok(AgentEvent::Tool { id: "t1".into(), name: "mcp__nuzky__get_state".into(), status, input })
        };
        assert_eq!(
            events(&lines),
            [
                get_state(ToolStatus::Running, None),
                get_state(ToolStatus::Running, Some(json!({"range":null}))),
                get_state(ToolStatus::Done, None),
                Ok(AgentEvent::Text { text: "Hoto".into() }),
                Ok(AgentEvent::Text { text: "vo.".into() }),
            ]
        );
    }

    #[test]
    fn foreign_tools_ignored_settings_and_login_problems_fail_the_turn() {
        assert_eq!(
            events(&[init_line(&["mcp__nuzky__get_state", "Bash"], "default")])[0].as_ref().unwrap_err().0,
            ErrorCode::AgentFailed
        );
        assert_eq!(
            events(&[init_line(&["mcp__nuzky__get_state"], "Proactive")])[0].as_ref().unwrap_err().0,
            ErrorCode::AgentFailed
        );
        let logged_out = json!({"type":"assistant","error":"authentication_failed","parent_tool_use_id":null,"message":{"content":[{"type":"text","text":"Not logged in · Please run /login"}]}});
        assert_eq!(events(&[logged_out]), [Err((ErrorCode::NotSignedIn, "Not logged in · Please run /login".into()))]);
        let limit = json!({"type":"assistant","error":"rate_limit","parent_tool_use_id":null,"message":{"content":[{"type":"text","text":"You've hit your limit · resets 9pm"}]}});
        assert_eq!(events(&[limit]), [Err((ErrorCode::UsageLimit, "You've hit your limit · resets 9pm".into()))]);
    }

    // Shapes from a real claude 2.1.309 turn with Nuzky's flags.
    #[test]
    fn a_web_search_is_a_step_with_its_query() {
        let lines = [
            init_line(&["WebSearch", "mcp__nuzky__get_state"], "default"),
            json!({"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_start","content_block":{"type":"tool_use","id":"toolu_01VH","name":"WebSearch","input":{},"caller":{"type":"direct"}}}}),
            json!({"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"tool_use","id":"toolu_01VH","name":"WebSearch","input":{"query":"Spotify logo svg wikimedia commons"},"caller":{"type":"direct"}}]}}),
            json!({"type":"user","parent_tool_use_id":null,"message":{"content":[{"tool_use_id":"toolu_01VH","type":"tool_result","content":"Web search results for query: \"Spotify logo svg wikimedia commons\"\n\nLinks: [{\"title\":\"File:Spotify logo.svg\",\"url\":\"https://commons.wikimedia.org/wiki/File:Spotify_logo.svg\"}]"}]}}),
        ];
        let search =
            |status, input| Ok(AgentEvent::Tool { id: "toolu_01VH".into(), name: "web_search".into(), status, input });
        assert_eq!(
            events(&lines),
            [
                search(ToolStatus::Running, None),
                search(ToolStatus::Running, Some(json!({"query": "Spotify logo svg wikimedia commons"}))),
                search(ToolStatus::Done, None),
            ]
        );
    }

    // Shapes from claude 2.1.310 answering a call without the prefix.
    #[test]
    fn a_nuzky_tool_called_without_the_prefix_is_retried_not_stopped() {
        let lines = [
            init_line(&["mcp__nuzky__apply_edits"], "default"),
            json!({"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_start","content_block":{"type":"tool_use","id":"t1","name":"apply_edits","input":{}}}}),
            json!({"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"tool_use","id":"t1","name":"apply_edits","input":{"run_id":"x"}}]}}),
            json!({"type":"user","parent_tool_use_id":null,"message":{"content":[{"type":"tool_result","content":"<tool_use_error>Error: No such tool available: apply_edits</tool_use_error>","is_error":true,"tool_use_id":"t1"}]}}),
            json!({"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"tool_use","id":"t2","name":"mcp__nuzky__apply_edits","input":{"run_id":"x"}}]}}),
        ];
        assert_eq!(
            events(&lines),
            [Ok(AgentEvent::Tool {
                id: "t2".into(),
                name: "mcp__nuzky__apply_edits".into(),
                status: ToolStatus::Running,
                input: Some(json!({"run_id":"x"})),
            })]
        );
    }

    #[test]
    fn a_tool_nuzky_did_not_give_fails_the_turn_when_offered_or_used() {
        let offered = events(&[init_line(&["WebSearch", "WebFetch", "mcp__nuzky__get_state"], "default")]);
        assert!(offered[0].as_ref().unwrap_err().1.contains("(WebFetch)"), "{offered:?}");
        for name in ["Bash", "WebFetch"] {
            let started = json!({"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_start","content_block":{"type":"tool_use","id":"t1","name":name,"input":{}}}});
            let used = json!({"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"tool_use","id":"t1","name":name,"input":{"url":"http://192.168.1.1/"}}]}});
            for line in [started, used] {
                let message = format!(
                    "Claude Code used a tool Nuzky did not give it ({name}), so Nuzky stopped it. Update Claude Code and Nuzky."
                );
                assert_eq!(events(&[line]), [Err((ErrorCode::AgentFailed, message))]);
            }
        }
    }
}
