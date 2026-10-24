//! OpenCode as `opencode run --format json`: the command, the config it runs on, and a parser of
//! its events. The event shapes come from OpenCode 1.18.34 run against a scripted provider, as
//! its docs name the events but not their fields.

use crate::discover::agent_path;
use crate::event::{AgentEvent, ErrorCode, ToolStatus};
use crate::turn::{SYSTEM, Step, TurnRequest};
use serde_json::{Value, json};
use std::process::Command;

/// Everything Nuzky grants for the turn. It goes in `OPENCODE_CONFIG_CONTENT`, not in a file, so two windows'
/// turns cannot swap projects, and stands in for the global config, so the user's own `mcp` servers, plugins
/// and rules do not load; their provider sign-in stays in the data folder, which Nuzky does not touch.
fn config(req: &TurnRequest) -> Value {
    let environment: serde_json::Map<String, Value> = req.mcp.env.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
    let command: Vec<String> =
        std::iter::once(req.mcp.command.to_string_lossy().into_owned()).chain(req.mcp.args.iter().cloned()).collect();
    json!({
        "autoupdate": false,
        "share": "disabled",
        "mcp": {"nuzky": {"type": "local", "command": command, "enabled": true, "environment": environment}},
        // In place of OpenCode's own prompt, which is about coding with its shell and files.
        "agent": {"build": {"prompt": SYSTEM}},
        // The last matching rule wins, so every tool but Nuzky's is denied, also built-ins a newer OpenCode
        // adds; a denied tool is not offered to the model at all. OpenCode has no web search of the provider's
        // own: its websearch goes to a search service of its own choosing.
        "permission": {"*": "deny", "nuzky_*": "allow"},
    })
}

/// Turns `nuzky_get_state` back into `nuzky.get_state` for the panel.
fn tool_name(raw: &str) -> String {
    raw.split_once('_').map_or_else(|| raw.to_owned(), |(n, t)| n.to_owned() + "." + t)
}

/// The message goes on stdin, which `run` reads to its end: as an argument it would be read twice, once from
/// each, and one starting with `-` would be taken for a flag.
pub(crate) fn command(req: &TurnRequest) -> Command {
    let mut c = Command::new(&req.exe);
    for own in ["OPENCODE_CONFIG", "OPENCODE_CONFIG_DIR", "OPENCODE_PERMISSION"] {
        c.env_remove(own);
    }
    c.env("OPENCODE_CONFIG_CONTENT", config(req).to_string());
    // The global config, its AGENTS.md and its plugins stay behind this private folder.
    c.env("XDG_CONFIG_HOME", &req.cwd);
    c.env("OPENCODE_DISABLE_AUTOUPDATE", "true");
    c.env("OPENCODE_DISABLE_CLAUDE_CODE", "true");
    // Skills in ~/.agents/skills, which the global config folder does not cover.
    c.env("OPENCODE_DISABLE_EXTERNAL_SKILLS", "true");
    c.env("PATH", agent_path());
    c.args(["run", "--format", "json", "--pure"]);
    if req.resume {
        c.args(["--session", &req.session]);
    }
    if let Some(model) = &req.model {
        c.args(["--model", model]);
    }
    for image in &req.prompt.images {
        // One flag each: `--file a b` would take what follows as more files too.
        let mut flag = std::ffi::OsString::from("--file=");
        flag.push(image);
        c.arg(flag);
    }
    c
}

pub(crate) struct Parser {
    /// The conversation a later turn resumes, from the lines' `sessionID`.
    session: Option<String>,
    /// Whether OpenCode has kept the conversation: a turn that failed before anything happened cannot be resumed.
    kept: bool,
}

/// OpenCode's free models answer only its own app, so in the panel they need a provider sign-in too.
fn code(error: &Value, message: &str) -> ErrorCode {
    let status = &error["data"]["statusCode"];
    if error["name"] == "ProviderAuthError" || *status == 401 || message.contains("free tier") {
        ErrorCode::NotSignedIn
    } else if *status == 429 {
        ErrorCode::UsageLimit
    } else {
        ErrorCode::AgentFailed
    }
}

impl Parser {
    pub(crate) fn new() -> Parser {
        Parser { session: None, kept: false }
    }

    /// The conversation the next message resumes, once OpenCode has kept it.
    pub(crate) fn session(&self) -> Option<String> {
        self.session.clone().filter(|_| self.kept)
    }

    pub(crate) fn line(&mut self, line: &str) -> Vec<Step> {
        let Ok(v) = serde_json::from_str::<Value>(line) else { return Vec::new() };
        if let Some(session) = v["sessionID"].as_str().filter(|s| !s.is_empty()) {
            self.session = Some(session.to_owned());
        }
        match v["type"].as_str() {
            Some("text") => {
                self.kept = true;
                vec![Step::Event(AgentEvent::Text { text: v["part"]["text"].as_str().unwrap_or("").to_owned() })]
            }
            // `run --format json` reports a tool only once finished, so a completed call arrives
            // running and done in one event with its input. Another server's tool would mean its
            // config reached into the turn, so Nuzky stops it.
            Some("tool_use") => self.tool(&v["part"]).into_iter().collect(),
            Some("reasoning") | Some("step_start") | Some("step_finish") => Vec::new(),
            Some("error") => {
                let error = &v["error"];
                let what = error["data"]["message"].as_str().or(error["name"].as_str()).unwrap_or("OpenCode failed.");
                vec![Step::Fail(code(error, what), what.to_owned())]
            }
            _ => Vec::new(),
        }
    }

    fn tool(&mut self, part: &Value) -> Option<Step> {
        if part["type"] != "tool" {
            return None;
        }
        let raw = part["tool"].as_str().unwrap_or("");
        if !raw.starts_with("nuzky_") {
            return Some(Step::Fail(
                ErrorCode::AgentFailed,
                format!(
                    "OpenCode used a tool Nuzky did not give it ({raw}), so Nuzky stopped it. Update OpenCode and Nuzky."
                ),
            ));
        }
        self.kept = true;
        let (status, input) = match part["state"]["status"].as_str() {
            Some("completed") => (ToolStatus::Done, Some(part["state"]["input"].clone())),
            _ => (ToolStatus::Failed, None),
        };
        Some(Step::Event(AgentEvent::Tool {
            id: part["callID"].as_str().unwrap_or_default().to_owned(),
            name: tool_name(raw),
            status,
            input,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turn::{McpServer, Message};

    type Outcome = Result<AgentEvent, (ErrorCode, String)>;

    fn parse(lines: &[&str]) -> (Vec<Outcome>, Option<String>) {
        let mut p = Parser::new();
        let steps = lines
            .iter()
            .flat_map(|l| p.line(l))
            .filter_map(|s| match s {
                Step::Event(e) => Some(Ok(e)),
                Step::Fail(c, m) => Some(Err((c, m))),
                Step::ResultError(_) | Step::Answered => None,
            })
            .collect();
        (steps, p.session())
    }

    // Lines of `opencode run --format json` 1.18.34 with Nuzky's settings, a scripted provider and a stand-in
    // MCP server named nuzky.
    #[test]
    fn a_turn_names_nuzky_calls_and_keeps_the_session() {
        let (out, session) = parse(&[
            r#"{"type":"step_start","timestamp":1792834971749,"sessionID":"ses_e92b","part":{"id":"prt_1","messageID":"msg_1","sessionID":"ses_e92b","type":"step-start"}}"#,
            r#"{"type":"tool_use","timestamp":1792834971768,"sessionID":"ses_e92b","part":{"type":"tool","tool":"nuzky_get_state","callID":"call-1","state":{"status":"completed","input":{},"output":"{\"tracks\":1}","metadata":{"truncated":false},"title":"","time":{"start":1792834971747,"end":1792834971751}},"id":"prt_2","sessionID":"ses_e92b","messageID":"msg_1"}}"#,
            r#"{"type":"tool_use","timestamp":1792834971801,"sessionID":"ses_e92b","part":{"type":"tool","tool":"nuzky_begin_run","callID":"call-2","state":{"status":"error","input":{},"error":"INVALID_RUN: no such run","time":{"start":1792834971784,"end":1792834971787}},"id":"prt_3","sessionID":"ses_e92b","messageID":"msg_2"}}"#,
            "not json",
            r#"{"type":"text","timestamp":1792834975824,"sessionID":"ses_e92b","part":{"id":"prt_4","messageID":"msg_3","sessionID":"ses_e92b","type":"text","text":"It returned one track.","time":{"start":1792834975812,"end":1792834975814}}}"#,
        ]);
        let tool = |id: &str, name: &str, status, input| {
            Ok(AgentEvent::Tool { id: id.into(), name: name.into(), status, input })
        };
        assert_eq!(
            out,
            [
                tool("call-1", "nuzky.get_state", ToolStatus::Done, Some(json!({}))),
                tool("call-2", "nuzky.begin_run", ToolStatus::Failed, None),
                Ok(AgentEvent::Text { text: "It returned one track.".into() }),
            ]
        );
        assert_eq!(session.as_deref(), Some("ses_e92b"));
    }

    #[test]
    fn a_tool_nuzky_did_not_give_fails_the_turn() {
        let (out, _) = parse(&[
            r#"{"type":"tool_use","sessionID":"ses_a","part":{"type":"tool","tool":"websearch","callID":"c","state":{"status":"completed","input":{}}}}"#,
        ]);
        assert_eq!(out[0].as_ref().unwrap_err().0, ErrorCode::AgentFailed);
    }

    #[test]
    fn errors_say_what_to_do_and_a_turn_that_kept_nothing_does_not_resume() {
        // OpenCode 1.18.34 on a free model, which answers only OpenCode's own app.
        let free = r#"{"type":"error","timestamp":1792834931639,"sessionID":"ses_e92b5f","error":{"name":"APIError","data":{"message":"OpenCode's free tier can only be used from within OpenCode. Download it at https://opencode.ai and keep building.","statusCode":403,"isRetryable":false}}}"#;
        let (out, session) = parse(&[free]);
        let (code, message) = out[0].as_ref().unwrap_err();
        assert_eq!(*code, ErrorCode::NotSignedIn);
        assert!(message.starts_with("OpenCode's free tier"), "{message}");
        assert_eq!(session, None);
        let limit = r#"{"type":"error","sessionID":"ses_a","error":{"name":"APIError","data":{"message":"Rate limited","statusCode":429}}}"#;
        assert_eq!(parse(&[limit]).0[0].as_ref().unwrap_err().0, ErrorCode::UsageLimit);
        let other = r#"{"type":"error","sessionID":"ses_a","error":{"name":"UnknownError","data":{"message":"Unexpected server error. Check server logs for details."}}}"#;
        assert_eq!(parse(&[other]).0[0].as_ref().unwrap_err().0, ErrorCode::AgentFailed);
    }

    #[test]
    fn the_command_gives_opencode_only_nuzkys_tools_and_resumes_by_its_session() {
        let req = TurnRequest {
            agent: crate::AgentId::Opencode,
            exe: "opencode".into(),
            session: "ses_a".into(),
            resume: true,
            model: Some("openai/gpt".into()),
            effort: None,
            prompt: Message { text: "- cut the pauses".into(), images: vec!["/tmp/a b.jpg".into()] },
            cwd: "/run/user/1000/nuzky-agent".into(),
            mcp: McpServer {
                command: "/opt/Nuzky app".into(),
                args: vec!["mcp".into(), "--project".into(), "/home/u/a b.nuzky".into()],
                env: vec![("XDG_RUNTIME_DIR".into(), "/run/user/1000".into())],
            },
        };
        let c = command(&req);
        let env = |name: &str| {
            c.get_envs().find(|(k, _)| *k == name).map(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
        };
        let config: Value = serde_json::from_str(&env("OPENCODE_CONFIG_CONTENT").flatten().unwrap()).unwrap();
        assert_eq!(
            config["mcp"]["nuzky"],
            json!({"type":"local","command":["/opt/Nuzky app","mcp","--project","/home/u/a b.nuzky"],
                   "enabled":true,"environment":{"XDG_RUNTIME_DIR":"/run/user/1000"}})
        );
        assert_eq!(config["permission"], json!({"*": "deny", "nuzky_*": "allow"}));
        assert_eq!(config["agent"]["build"]["prompt"], SYSTEM);
        assert_eq!(env("XDG_CONFIG_HOME"), Some(Some("/run/user/1000/nuzky-agent".into())));
        assert_eq!(env("OPENCODE_DISABLE_EXTERNAL_SKILLS"), Some(Some("true".into())));
        assert_eq!(env("OPENCODE_CONFIG"), Some(None));
        let args: Vec<_> = c.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        // The message goes on stdin.
        assert_eq!(
            args,
            ["run", "--format", "json", "--pure", "--session", "ses_a", "--model", "openai/gpt", "--file=/tmp/a b.jpg"]
        );
    }
}
