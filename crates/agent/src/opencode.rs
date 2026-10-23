//! OpenCode as `opencode run --format json`: the command, the config it runs on, and a parser of
//! its events. Flags come from OpenCode's CLI and MCP servers' docs; the event shapes were
//! verified empirically against OpenCode 1.19.3, whose docs name the events but not their fields.

use crate::discover::agent_path;
use crate::event::{AgentEvent, ErrorCode, ToolStatus};
use crate::turn::{SYSTEM, Step, TurnRequest};
use anyhow::Context as _;
use serde_json::{Value, json};
use std::process::Command;

/// Everything Nuzky grants for the turn, as the config `opencode run` reads (`OPENCODE_CONFIG`).
/// It stands in for the global config, so the user's own `mcp` servers, plugins and rules do not
/// load; their provider sign-in stays in the data folder, which Nuzky does not touch.
fn config(req: &TurnRequest) -> Value {
    let environment: serde_json::Map<String, Value> = req.mcp.env.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
    json!({
        "$schema": "https://opencode.ai/config.json",
        "autoupdate": false,
        "mcp": {"nuzky": {
            "type": "local",
            "command": [req.mcp.command.to_string_lossy()].into_iter().chain(req.mcp.args.iter().cloned()).collect::<Vec<String>>(),
            "enabled": true,
            "environment": environment,
        }},
        // The panel's system prompt reaches every turn; without it, rules would repeat in each message.
        "instructions": [rules_path(req)],
        // Each built-in is named, since a `*` entry would also swallow Nuzky's tools.
        "tools": {"nuzky_*": true, "bash": false, "read": false, "write": false, "edit": false, "patch": false,
                  "apply_patch": false, "glob": false, "grep": false, "list": false, "task": false,
                  "todowrite": false, "todoread": false, "webfetch": false},
    })
}

/// Where the `instructions` file sits: the panel's own folder, so no rules above it load.
fn rules_path(req: &TurnRequest) -> String {
    req.cwd.join("nuzky-rules.md").to_string_lossy().into_owned()
}

/// Turns `nuzky_get_state` back into `nuzky.get_state` for the panel.
fn tool_name(raw: &str) -> String {
    raw.split_once('_').map_or_else(|| raw.to_owned(), |(n, t)| (n.to_owned() + "." + t))
}

pub(crate) fn command(req: &TurnRequest) -> anyhow::Result<Command> {
    std::fs::write(
        req.cwd.join("opencode-config.json"),
        serde_json::to_vec_pretty(&config(req)).context("Serialising the AI panel's OpenCode config")?,
    )
    .with_context(|| "Writing the AI panel's OpenCode configuration".to_owned())?;
    std::fs::write(req.cwd.join("nuzky-rules.md"), SYSTEM)
        .with_context(|| "Writing the AI panel's OpenCode rules".to_owned())?;
    let mut c = Command::new(&req.exe);
    c.env("OPENCODE_CONFIG", req.cwd.join("opencode-config.json"));
    // The config file stands alone: the global one stays behind this private folder.
    c.env("XDG_CONFIG_HOME", &req.cwd);
    c.env("OPENCODE_DISABLE_AUTOUPDATE", "true");
    c.env("OPENCODE_DISABLE_CLAUDE_CODE", "true");
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
    // The message itself; stdin stays closed, which `run` reads only without a message.
    c.arg(&req.prompt.text);
    Ok(c)
}

pub(crate) struct Parser {
    /// The conversation a later turn resumes, from the lines' `sessionID`.
    session: Option<String>,
    /// Whether OpenCode has kept the conversation: a turn that failed before anything happened cannot be resumed.
    kept: bool,
}

fn code(message: &str) -> ErrorCode {
    // A free-tier request ("can only be used from within OpenCode") is really the login, which the
    // panel can only show as signing in.
    if message.contains("sign in") || message.contains("Sign in") || message.contains("Free tier") {
        ErrorCode::NotSignedIn
    } else if message.contains("usage limit") {
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
                Some(Step::Event(AgentEvent::Text { text: v["part"]["text"].as_str().unwrap_or("").to_owned() }))
            }
            // `run --format json` reports a tool only once finished, so a completed call arrives
            // running and done in one event with its input. Another server's tool would mean its
            // config reached into the turn, so Nuzky stops it.
            Some("tool_use") => self.tool(&v["part"]).into_iter().collect(),
            Some("reasoning") | Some("step_start") | Some("step_finish") => Vec::new(),
            Some("error") => {
                let what = v["error"]["message"].as_str().unwrap_or_default().to_owned();
                vec![Step::Fail(code(&what), what)]
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
    use serde_json::json;
    use std::path::{Path, PathBuf};

    type Outcome = Result<AgentEvent, (ErrorCode, String)>;

    fn parse(lines: &[Value]) -> (Vec<Outcome>, Option<String>) {
        let mut p = Parser::new();
        let steps = lines
            .iter()
            .flat_map(|l| p.line(&l.to_string()))
            .filter_map(|s| match s {
                Step::Event(e) => Some(Ok(e)),
                Step::Fail(c, m) => Some(Err((c, m))),
                Step::ResultError(m) => Some(Err((ErrorCode::AgentFailed, m))),
                Step::Answered => None,
            })
            .collect();
        (steps, p.session())
    }

    // Lines from a real `opencode run --format json` turn of 1.19.3, with Nuzky's settings and a
    // stand-in MCP server named nuzky.
    #[test]
    fn a_turn_names_nuzky_calls_and_keeps_the_session() {
        let lines = [
            r#"{"type":"step_start","timestamp":1792737052373,"sessionID":"ses_a","part":{"id":"prt_1","messageID":"msg_1","sessionID":"ses_a","snapshot":"s","type":"step-start"}}"#,
            r#"{"type":"tool_use","timestamp":1792737052446,"sessionID":"ses_a","part":{"type":"tool","tool":"nuzky_get_state","callID":"call-d69b311b","state":{"status":"completed","input":{},"output":"{\"tracks\":1}"},"id":"prt_2","sessionID":"ses_a","messageID":"msg_1"}}"#,
            r#"{"type":"step_finish","timestamp":1792737053380,"sessionID":"ses_a","part":{"type":"step-finish","reason":"tool-calls","sessionID":"ses_a"}}"#,
            r#"{"type":"text","timestamp":1792737056575,"sessionID":"ses_a","part":{"id":"prt_3","messageID":"msg_2","sessionID":"ses_a","type":"text","text":"It returned one track.","time":{}}}"#,
            r#"{"type":"step_finish","timestamp":1792737057508,"sessionID":"ses_a","part":{"type":"step-finish","reason":"stop","sessionID":"ses_a"}}"#,
        ];
        let (out, session) = parse(&lines.iter().map(|l| serde_json::from_str(l).unwrap()).collect::<Vec<_>>());
        assert_eq!(
            out,
            [
                Ok(AgentEvent::Tool {
                    id: "call-d69b311b".into(),
                    name: "nuzky.get_state".into(),
                    status: ToolStatus::Done,
                    input: Some(json!({})),
                }),
                Ok(AgentEvent::Text { text: "It returned one track.".into() }),
            ]
        );
        assert_eq!(session.as_deref(), Some("ses_a"));
    }

    #[test]
    fn a_failed_tool_says_so_and_a_foreign_tool_fails_the_turn() {
        let failed = json!({"type":"tool_use","sessionID":"ses_a","part":{"type":"tool","tool":"nuzky_fail","callID":"call-2","state":{"status":"error","input":{},"error":"INVALID_RUN: no such run"}}});
        assert_eq!(
            parse(&[failed]).0,
            [Ok(AgentEvent::Tool {
                id: "call-2".into(),
                name: "nuzky.fail".into(),
                status: ToolStatus::Failed,
                input: None,
            })]
        );
        let foreign = json!({"type":"tool_use","sessionID":"ses_a","part":{"type":"tool","tool":"bash","callID":"call-3","state":{"status":"completed","input":{},"output":""}}});
        assert_eq!(parse(&[foreign]).0[0].as_ref().unwrap_err().0, ErrorCode::AgentFailed);
    }

    #[test]
    fn an_error_says_what_the_login_problem_is() {
        let error = json!({"type":"error","timestamp":1,"sessionID":"","error":{"name":"APIError","data":{"message":"OpenCode's free tier can only be used from within OpenCode. Download it at https://opencode.ai and keep building.","statusCode":403}}});
        assert_eq!(parse(&[error]).0[0].as_ref().unwrap_err().0, ErrorCode::NotSignedIn);
        let limit = json!({"type":"error","timestamp":1,"sessionID":"","error":{"message":"usage limit reached"}});
        assert_eq!(parse(&[limit]).0[0].as_ref().unwrap_err().0, ErrorCode::UsageLimit);
        let (out, session) =
            parse(&[json!({"type":"error","sessionID":"ses_a","error":{"message":"connection failed"}})]);
        assert_eq!(out[0].as_ref().unwrap_err().0, ErrorCode::AgentFailed);
        // A turn that failed before OpenCode kept anything cannot be resumed.
        assert_eq!(session, None);
    }

    #[test]
    fn plain_lines_without_json_are_ignored() {
        assert_eq!(parse(&[json!({"type":"unknown"}), Value::Null, json!("oops")]).0, Vec::<Outcome>::new());
    }

    #[test]
    fn the_command_gives_opencode_only_nuzkys_tools_and_resumes_by_its_session() {
        let (cwd, keep) = own_folder();
        let req = TurnRequest {
            agent: crate::AgentId::Opencode,
            exe: "opencode".into(),
            session: "ses_a".into(),
            resume: true,
            model: Some("openai/gpt".into()),
            effort: None,
            prompt: Message { text: String::new(), images: vec!["/tmp/a b.jpg".into()] },
            cwd: cwd.clone(),
            mcp: McpServer {
                command: "/opt/Nuzky \"app\"".into(),
                args: vec!["mcp".into(), "--project".into(), "/home/u/a b.nuzky".into()],
                env: vec![("XDG_RUNTIME_DIR".into(), "/run/user/1000".into())],
            },
        };
        command(&req).unwrap();
        let config: Value =
            serde_json::from_str(&std::fs::read_to_string(cwd.join("opencode-config.json")).unwrap()).unwrap();
        assert_eq!(
            config["mcp"]["nuzky"],
            json!({"type":"local","command":["/opt/Nuzky \"app\"","mcp","--project","/home/u/a b.nuzky"],
                   "enabled":true,"environment":{"XDG_RUNTIME_DIR":"/run/user/1000"}})
        );
        assert_eq!(config["tools"]["nuzky_*"], json!(true));
        assert_eq!(config["tools"]["bash"], json!(false));
        assert_eq!(config["instructions"][0], Value::String(cwd.join("nuzky-rules.md").to_string_lossy().into_owned()));
        assert_eq!(std::fs::read_to_string(cwd.join("nuzky-rules.md")).unwrap(), SYSTEM);
        let c = command(&req).unwrap();
        let args: Vec<_> = c.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args[..4], ["run", "--format", "json", "--pure"]);
        assert!(args.windows(2).any(|a| a == ["--session", "ses_a"]));
        assert!(args.windows(2).any(|a| a == ["--model", "openai/gpt"]));
        assert_eq!(*args.last().unwrap(), "--file=/tmp/a b.jpg");
        assert_eq!(*c.get_env("XDG_CONFIG_HOME").unwrap().unwrap().to_string_lossy(), *cwd.to_string_lossy());
        drop(keep);
        std::fs::remove_dir_all(&cwd).unwrap();
    }

    /// The panel's own folder on a temp path, as the panel makes before it starts.
    fn own_folder() -> (PathBuf, KeepTemp) {
        use std::time::{SystemTime, UNIX_EPOCH};
        let path = std::env::temp_dir().join(format!(
            "nuzky-opencode-test-{}",
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        (path, KeepTemp)
    }

    /// Marks the folder as this test's, which it removes afterwards.
    struct KeepTemp;

    fn _keep_works(path: &Path) -> bool {
        path.is_dir()
    }
}
