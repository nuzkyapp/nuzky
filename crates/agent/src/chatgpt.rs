//! ChatGPT on the user's own plan, without an installed agent: the turn talks to OpenAI's Responses
//! API itself (https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference)
//! and runs Nuzky's tools in the app, as the MCP bridge's clients run them. See
//! docs/AI-ARCHITECTURE.md, "AI panel".

use crate::event::{AgentEvent, ErrorCode, ToolStatus, web_search_input};
use crate::turn::{Inbox, Message, SYSTEM, Turn};
use anyhow::Result;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// Turns do not run on Windows yet.
#[cfg(all(test, unix))]
mod tests;

/// Answers with tool calls one message may take. Counting its steps, the longest flow the guide
/// describes, a rough cut of a few minutes of video with captions and an export, takes 30 to 60 (a
/// waveform look at each cut, job waits of 25 s); a model that needs more is most likely going in
/// circles, which would only use up the user's plan.
const MAX_ROUNDS: usize = 100;
/// How often a waiting turn looks whether it was stopped.
const POLL: Duration = Duration::from_millis(50);
/// A stream that sends nothing for this long has stalled.
const IDLE: Duration = Duration::from_secs(300);
/// Pauses before asking again when ChatGPT is briefly unavailable (503), as OpenAI asks.
const BACKOFF: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(3)];
/// Error answers are short; this keeps a broken one from filling memory.
const MAX_ERROR_BYTES: u64 = 64 * 1024;
const NAMESPACE: &str = "nuzky";
/// What an MCP client has that this turn does not.
const HERE: &str = "Nuzky's tools are the functions in the nuzky namespace. MCP resources and prompts are not available here: the guide and the creator's style are below, and the parameters of apply_edits describe every edit.";
const LIMIT: &str = "You've reached a usage limit of your ChatGPT plan, or one set for Nuzky in ChatGPT's settings.";
const NO_PLAN: &str = "ChatGPT hasn't allowed Nuzky to use your plan yet. Open ChatGPT account… and allow it.";
/// What a call the turn stopped before it finished answers, as ChatGPT needs an answer to every call.
const STOPPED: &str =
    "RUN_STOPPED: the user stopped you before this call finished. Read get_state before changing anything.";

/// Nuzky's tools as the turn runs them, in the app.
pub trait Tools: Send + Sync + 'static {
    /// One call; the answer is an MCP `CallToolResult` as JSON.
    fn call(&self, name: &str, arguments: Value) -> Value;
    /// The turn ended or was stopped: a call still running ends, and a run left open is kept.
    fn end(&self) -> Result<()>;
}

pub struct ChatGptRequest {
    /// `https://api.openai.com/v1`, or a test server.
    pub api: String,
    /// The slug from the account's model list.
    pub model: String,
    pub effort: Option<String>,
    /// Nuzky's editing guide with the creator's style, which an MCP client gets as the server's instructions.
    pub guide: String,
    /// Nuzky's tools as MCP's `tools/list` lists them.
    pub tools: Value,
    pub prompt: Message,
    /// The conversation as the Responses API takes it, as `input`. Nuzky keeps it: nothing is stored
    /// on OpenAI's side. The turn adds what it keeps once it ends.
    pub history: Arc<Mutex<Vec<Value>>>,
    /// A valid access token; `true` renews it although it has not expired, after ChatGPT refused it.
    pub token: Arc<dyn Fn(bool) -> Result<String> + Send + Sync>,
    pub run: Arc<dyn Tools>,
}

impl Turn {
    /// Starts a ChatGPT turn; `emit` gets its events from the turn's thread, ending with `Done` or `Error`.
    pub fn chatgpt(req: ChatGptRequest, mut emit: impl FnMut(AgentEvent) + Send + 'static) -> Result<Turn> {
        if cfg!(windows) {
            anyhow::bail!("The AI panel is not available on Windows yet. Connect your agent in a terminal instead.");
        }
        let (stopping, finished) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let inbox = Arc::new(Mutex::new(Some(Vec::new())));
        let (stop, done, waiting) = (stopping.clone(), finished.clone(), inbox.clone());
        std::thread::spawn(move || {
            let end = run(&req, &stop, &waiting, &mut emit);
            waiting.lock().unwrap().take();
            done.store(true, Ordering::Release);
            emit(end);
        });
        Ok(Turn { pid: None, stopping, finished, inbox: Inbox::ChatGpt(inbox) })
    }
}

fn run(req: &ChatGptRequest, stop: &Arc<AtomicBool>, inbox: &Waiting, emit: &mut dyn FnMut(AgentEvent)) -> AgentEvent {
    let before = req.history.lock().unwrap().clone();
    let mut items = before.clone();
    let (mut end, answered) = match user(&req.prompt) {
        Ok(first) => {
            items.push(first);
            converse(req, &mut items, stop, inbox, emit)
        }
        Err(error) => (failure(ErrorCode::AgentFailed, &format!("{error:#}")), false),
    };
    // A run ChatGPT left open that could not be kept leaves the editor locked, so the panel says so.
    if let Err(error) = req.run.end()
        && !matches!(end, AgentEvent::Error { .. })
    {
        end = failure(
            ErrorCode::AgentFailed,
            &format!("Nuzky couldn't finish ChatGPT's edits. Use Stop and edit in the top bar.\n{error:#}"),
        );
    }
    // A message ChatGPT never answered leaves the conversation when it failed, as the panel gives it back
    // to the field; a stopped one stays, as the panel still shows it.
    let kept = if answered {
        answer_missing(&mut items);
        items
    } else if matches!(end, AgentEvent::Done { .. }) {
        items
    } else {
        before
    };
    *req.history.lock().unwrap() = kept;
    end
}

/// Messages the user sent while ChatGPT works; None once the turn takes no more.
type Waiting = Arc<Mutex<Option<Vec<Message>>>>;

fn user(message: &Message) -> Result<Value> {
    let mut content = vec![json!({"type": "input_text", "text": message.text})];
    for data in message.base64_images()? {
        content.push(json!({"type": "input_image", "image_url": format!("data:image/jpeg;base64,{data}")}));
    }
    Ok(json!({"type": "message", "role": "user", "content": content}))
}

/// Asks ChatGPT and runs the tools it calls until it answers without calling one. Also says whether
/// ChatGPT completed any answer, which decides what the conversation keeps of this turn.
fn converse(
    req: &ChatGptRequest,
    items: &mut Vec<Value>,
    stop: &Arc<AtomicBool>,
    inbox: &Waiting,
    emit: &mut dyn FnMut(AgentEvent),
) -> (AgentEvent, bool) {
    let http = http();
    let instructions = format!("{SYSTEM}\n\n{HERE}\n\n{}", req.guide);
    // OpenAI's web search of its own index, as Codex sends it: no live page fetch.
    let tools = [namespace(&req.tools), json!({"type": "web_search", "external_web_access": false})];
    let mut answered = false;
    for _ in 0..MAX_ROUNDS {
        let mut body = json!({
            "model": req.model,
            "instructions": instructions,
            "input": items,
            "tools": tools,
            "store": false,
            "stream": true,
            // Nothing is stored, so the model's reasoning goes back to it with the history.
            "include": ["reasoning.encrypted_content"],
        });
        if let Some(effort) = &req.effort {
            body["reasoning"] = json!({"effort": effort});
        }
        let output = match respond(&http, req, &body.to_string(), stop, emit) {
            Ok(output) => output,
            Err(end) => return (end, answered),
        };
        answered = true;
        if let Some(other) = output.iter().find(|item| {
            !matches!(item["type"].as_str(), Some("message" | "reasoning" | "function_call" | "web_search_call"))
        }) {
            let what = other["type"].as_str().unwrap_or("unknown");
            return (
                failure(
                    ErrorCode::AgentFailed,
                    &format!("ChatGPT tried to use a tool Nuzky did not give it ({what}), so Nuzky stopped it."),
                ),
                true,
            );
        }
        let calls: Vec<Value> = output.iter().filter(|item| item["type"] == "function_call").cloned().collect();
        items.extend(output.into_iter().filter_map(as_input));
        if calls.is_empty() {
            let mut waiting = inbox.lock().unwrap();
            // A message that came with the answer is asked about at once rather than ending the turn.
            if waiting.as_ref().is_none_or(Vec::is_empty) {
                *waiting = None;
                return (AgentEvent::Done { stopped: false }, true);
            }
        }
        for call in &calls {
            let Some(output) = run_call(req, call, stop, emit) else {
                return (AgentEvent::Done { stopped: true }, true);
            };
            items.push(json!({"type": "function_call_output", "call_id": call["call_id"], "output": output}));
        }
        let waiting = inbox.lock().unwrap().as_mut().map(std::mem::take).unwrap_or_default();
        for message in &waiting {
            match user(message) {
                Ok(message) => items.push(message),
                Err(error) => return (failure(ErrorCode::AgentFailed, &format!("{error:#}")), answered),
            }
        }
    }
    let message = format!(
        "ChatGPT stopped after {MAX_ROUNDS} rounds of tools in one message, so a model going in circles can't use up your plan. Send \"go on\" to let it continue."
    );
    (failure(ErrorCode::AgentFailed, &message), answered)
}

/// What one request's thread reports.
enum Wire {
    /// The answer streams; its events follow.
    Open,
    Refused {
        status: u16,
        body: String,
        request: Option<String>,
    },
    Event(Value),
    End,
    Broken(String),
    /// No valid access token: the sign-in's own error.
    NoToken(anyhow::Error),
}

/// One answer of ChatGPT's, streamed: its words go to the panel as they come, and its output items come
/// back once it completed. Anything else ends the turn with the event that ends it.
fn respond(
    http: &ureq::Agent,
    req: &ChatGptRequest,
    body: &str,
    stop: &Arc<AtomicBool>,
    emit: &mut dyn FnMut(AgentEvent),
) -> Result<Vec<Value>, AgentEvent> {
    let mut renewed = false;
    let mut pauses = BACKOFF.iter().copied();
    let events = loop {
        let url = format!("{}/responses", req.api);
        let wire = post(http.clone(), url, req.token.clone(), renewed, body.to_owned(), stop.clone());
        match next(&wire, stop)? {
            Wire::Open => break wire,
            // An access token can stop working before it expires, e.g. once another window renewed it.
            Wire::Refused { status: 401, .. } if !renewed => renewed = true,
            Wire::Refused { status: 503, body, request } => match pauses.next() {
                Some(time) => pause(time, stop)?,
                None => return Err(refused(503, &body, request.as_deref())),
            },
            Wire::Refused { status, body, request } => return Err(refused(status, &body, request.as_deref())),
            Wire::NoToken(error) => return Err(token_failure(&error)),
            Wire::Broken(error) => {
                return Err(failure(
                    ErrorCode::AgentFailed,
                    &format!("Couldn't reach ChatGPT. Check the internet connection.\n{error}"),
                ));
            }
            _ => return Err(interrupted("ChatGPT answered without a stream")),
        }
    };
    let mut output = Vec::new();
    loop {
        let event = match next(&events, stop)? {
            Wire::Event(event) => event,
            Wire::Broken(error) => return Err(interrupted(&error)),
            _ => return Err(interrupted("The stream ended without response.completed")),
        };
        let id = event["response"]["id"].as_str();
        match event["type"].as_str().unwrap_or_default() {
            "response.output_text.delta" => {
                if let Some(text) = event["delta"].as_str().filter(|text| !text.is_empty()) {
                    emit(AgentEvent::Text { text: text.to_owned() });
                }
            }
            // ChatGPT searches the web on OpenAI's servers within the answer; the panel shows each search.
            "response.output_item.added" if event["item"]["type"] == "web_search_call" => {
                emit(search(&event["item"], ToolStatus::Running));
            }
            "response.output_item.done" => {
                let item = &event["item"];
                if item["type"] == "web_search_call" {
                    let failed = item["status"] == "failed";
                    emit(search(item, if failed { ToolStatus::Failed } else { ToolStatus::Done }));
                }
                output.push(item.clone());
            }
            "response.completed" => return Ok(output),
            // A usage limit can come this way after the answer began.
            "response.failed" => return Err(problem(None, &event["response"]["error"], id)),
            "error" => return Err(problem(None, &event, id)),
            "response.incomplete" => {
                let reason = event["response"]["incomplete_details"]["reason"].as_str().unwrap_or("unknown");
                return Err(failure(
                    ErrorCode::AgentFailed,
                    &format!("ChatGPT stopped before its answer was complete.\nReason: {reason}"),
                ));
            }
            _ => {}
        }
    }
}

fn search(item: &Value, status: ToolStatus) -> AgentEvent {
    AgentEvent::Tool {
        id: item["id"].as_str().unwrap_or_default().to_owned(),
        name: "web_search".into(),
        status,
        input: web_search_input(&item["action"]),
    }
}

/// Gets the token and sends one request on its own thread, so the turn can stop without waiting for
/// the network, a renewal included. A stopped turn drops the receiver, and the thread closes the
/// connection at the next event.
fn post(
    http: ureq::Agent,
    url: String,
    token: Arc<dyn Fn(bool) -> Result<String> + Send + Sync>,
    renew: bool,
    body: String,
    stop: Arc<AtomicBool>,
) -> Receiver<Wire> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let token = match token(renew) {
            Ok(token) => token,
            Err(error) => return drop(tx.send(Wire::NoToken(error))),
        };
        if stop.load(Ordering::Acquire) {
            return;
        }
        let response = http
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .send(body);
        let mut response = match response {
            Ok(response) => response,
            Err(error) => return drop(tx.send(Wire::Broken(error.to_string()))),
        };
        let status = response.status().as_u16();
        if status != 200 {
            let request = response.headers().get("x-request-id").and_then(|v| v.to_str().ok()).map(str::to_owned);
            let body = response.body_mut().with_config().limit(MAX_ERROR_BYTES).read_to_string().unwrap_or_default();
            return drop(tx.send(Wire::Refused { status, body, request }));
        }
        if tx.send(Wire::Open).is_err() {
            return;
        }
        // Server-sent events: `data:` lines, each event ended by an empty line.
        let mut data = String::new();
        for line in BufReader::new(response.into_body().into_reader()).lines() {
            let line = match line {
                Ok(line) => line,
                Err(error) => return drop(tx.send(Wire::Broken(error.to_string()))),
            };
            if let Some(part) = line.strip_prefix("data:") {
                data.push_str(part.strip_prefix(' ').unwrap_or(part));
            } else if line.is_empty() && !data.is_empty() {
                let wire = match serde_json::from_str(&std::mem::take(&mut data)) {
                    Ok(event) => Wire::Event(event),
                    Err(error) => Wire::Broken(format!("Reading ChatGPT's answer: {error}")),
                };
                if tx.send(wire).is_err() {
                    return;
                }
            }
        }
        let _ = tx.send(Wire::End);
    });
    rx
}

fn next(wire: &Receiver<Wire>, stop: &AtomicBool) -> Result<Wire, AgentEvent> {
    let since = Instant::now();
    loop {
        if stop.load(Ordering::Acquire) {
            return Err(AgentEvent::Done { stopped: true });
        }
        match wire.recv_timeout(POLL) {
            Ok(wire) => return Ok(wire),
            Err(RecvTimeoutError::Timeout) if since.elapsed() < IDLE => {}
            Err(RecvTimeoutError::Timeout) => return Err(interrupted("ChatGPT sent nothing for 5 minutes")),
            Err(RecvTimeoutError::Disconnected) => return Ok(Wire::End),
        }
    }
}

fn pause(time: Duration, stop: &AtomicBool) -> Result<(), AgentEvent> {
    let until = Instant::now() + time;
    while Instant::now() < until {
        if stop.load(Ordering::Acquire) {
            return Err(AgentEvent::Done { stopped: true });
        }
        std::thread::sleep(POLL);
    }
    Ok(())
}

/// Runs one call ChatGPT made and gives back its output for ChatGPT; None when the turn was stopped
/// meanwhile. The call runs on its own thread, so Stop ends the turn at once even while it runs; the
/// session refuses its late edits with RUN_STOPPED.
fn run_call(req: &ChatGptRequest, call: &Value, stop: &AtomicBool, emit: &mut dyn FnMut(AgentEvent)) -> Option<Value> {
    let id = call["call_id"].as_str().unwrap_or_default().to_owned();
    let name = call["name"].as_str().unwrap_or_default().to_owned();
    let arguments = serde_json::from_str::<Value>(call["arguments"].as_str().unwrap_or("{}"));
    emit(AgentEvent::Tool {
        id: id.clone(),
        name: name.clone(),
        status: ToolStatus::Running,
        input: arguments.as_ref().ok().cloned(),
    });
    let result = match arguments {
        Err(error) => tool_error(&format!("INVALID_ARGUMENTS: the arguments are not JSON: {error}")),
        Ok(_) if call["namespace"].as_str().is_some_and(|namespace| namespace != NAMESPACE) => {
            tool_error("UNKNOWN_TOOL: only the tools in the nuzky namespace exist")
        }
        Ok(arguments) => {
            let (tx, rx) = mpsc::channel();
            let (run, tool) = (req.run.clone(), name.clone());
            std::thread::spawn(move || drop(tx.send(run.call(&tool, arguments))));
            loop {
                if stop.load(Ordering::Acquire) {
                    return None;
                }
                match rx.recv_timeout(POLL) {
                    Ok(result) => break result,
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break tool_error("TOOL_FAILED: the tool did not answer"),
                }
            }
        }
    };
    let status = if result["isError"] == true { ToolStatus::Failed } else { ToolStatus::Done };
    emit(AgentEvent::Tool { id, name, status, input: None });
    Some(output(&result))
}

fn tool_error(message: &str) -> Value {
    json!({"content": [{"type": "text", "text": message}], "isError": true})
}

/// An MCP tool result as a function call's output: its text, with its images as images.
fn output(result: &Value) -> Value {
    let parts: Vec<Value> = result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| match part["type"].as_str()? {
            "text" => Some(json!({"type": "input_text", "text": part["text"]})),
            "image" => Some(json!({
                "type": "input_image",
                "image_url": format!("data:{};base64,{}", part["mimeType"].as_str()?, part["data"].as_str()?),
            })),
            _ => None,
        })
        .collect();
    if parts.iter().any(|part| part["type"] == "input_image") {
        return Value::from(parts);
    }
    let text: Vec<&str> = parts.iter().filter_map(|part| part["text"].as_str()).collect();
    if text.is_empty() { Value::from(result["structuredContent"].to_string()) } else { Value::from(text.join("\n")) }
}

/// Nuzky's MCP tools as function tools in one namespace, the form ChatGPT plan usage takes them in.
fn namespace(tools: &Value) -> Value {
    let functions: Vec<Value> = tools
        .as_array()
        .into_iter()
        .flatten()
        .map(|tool| {
            let mut parameters = tool["inputSchema"].clone();
            if let Some(schema) = parameters.as_object_mut() {
                schema.remove("$schema");
            }
            // Strict mode would make every optional argument required.
            json!({"type": "function", "name": tool["name"], "description": tool["description"], "parameters": parameters, "strict": false})
        })
        .collect();
    json!({"type": "namespace", "name": NAMESPACE, "description": "Nuzky's tools for editing the project open in the app.", "tools": functions})
}

/// An output item as the next request's input. Without its id, as nothing is stored on OpenAI's side to
/// refer to, except a web search call, whose input schema requires it; reasoning only with its encrypted
/// content, the only form that can go back.
fn as_input(mut item: Value) -> Option<Value> {
    if item["type"] == "reasoning" && !item["encrypted_content"].is_string() {
        return None;
    }
    if item["type"] != "web_search_call" {
        item.as_object_mut()?.remove("id");
    }
    Some(item)
}

/// Every call needs an output before the next request; the calls a stop or a failure left get one.
fn answer_missing(items: &mut Vec<Value>) {
    let answered: HashSet<String> = items
        .iter()
        .filter(|item| item["type"] == "function_call_output")
        .filter_map(|item| item["call_id"].as_str().map(str::to_owned))
        .collect();
    let missing: Vec<Value> = items
        .iter()
        .filter(|item| {
            item["type"] == "function_call" && !item["call_id"].as_str().is_some_and(|id| answered.contains(id))
        })
        .map(|item| json!({"type": "function_call_output", "call_id": item["call_id"], "output": STOPPED}))
        .collect();
    items.extend(missing);
}

fn failure(code: ErrorCode, message: &str) -> AgentEvent {
    AgentEvent::Error { code, message: message.to_owned() }
}

fn interrupted(detail: &str) -> AgentEvent {
    failure(
        ErrorCode::AgentFailed,
        &format!("The connection to ChatGPT broke off before the answer was complete.\n{detail}"),
    )
}

/// The sign-in's own errors carry a code as their prefix.
fn token_failure(error: &anyhow::Error) -> AgentEvent {
    let text = format!("{error:#}");
    let (prefix, message) = text.split_once(": ").unwrap_or(("", &text));
    match prefix {
        "NOT_SIGNED_IN" => failure(ErrorCode::NotSignedIn, message),
        "PLAN_UNAVAILABLE" => failure(ErrorCode::PlanUnavailable, message),
        _ if prefix.bytes().all(|c| c.is_ascii_uppercase() || c == b'_') => failure(ErrorCode::AgentFailed, message),
        _ => failure(ErrorCode::AgentFailed, &text),
    }
}

/// A request ChatGPT refused before answering: OpenAI's error object, or `{"detail": …}` from the
/// check in front of it.
fn refused(status: u16, body: &str, request: Option<&str>) -> AgentEvent {
    let body: Value = serde_json::from_str(body).unwrap_or_default();
    let error = match &body["error"] {
        error @ Value::Object(_) => error.clone(),
        _ => json!({"message": body["detail"].as_str().unwrap_or_default()}),
    };
    problem(Some(status), &error, request)
}

/// What a refusal means for the user, by OpenAI's documented codes
/// (https://developers.openai.com/siwc/token-sharing-open-source/errors-and-recovery).
fn problem(status: Option<u16>, error: &Value, request: Option<&str>) -> AgentEvent {
    let code = error["code"].as_str().unwrap_or_default();
    let message = error["message"].as_str().unwrap_or_default();
    log::warn!(
        "ChatGPT refused a request: status {status:?}, code {code:?}, param {:?}, request {request:?}: {message}",
        error["param"]
    );
    let (kind, text) = match (code, status) {
        ("subscription_sharing_usage_limit_exceeded", _) | (_, Some(429)) => (ErrorCode::UsageLimit, LIMIT),
        ("subscription_sharing_usage_unavailable", _) => {
            (ErrorCode::UsageLimit, "ChatGPT couldn't check your plan's usage just now. Try again in a few minutes.")
        }
        ("subscription_sharing_user_not_eligible", _) => (
            ErrorCode::PlanUnavailable,
            "This ChatGPT account can't use its plan in Nuzky. It takes a plan such as Plus or Pro, in a workspace that allows it.",
        ),
        ("chatpass_v2_scope_not_authorized" | "chatpass_v2_invalid_authorization_context", _) => {
            (ErrorCode::PlanUnavailable, NO_PLAN)
        }
        ("subscription_sharing_invalid_user", _) | (_, Some(401)) => {
            (ErrorCode::NotSignedIn, "ChatGPT didn't accept the sign-in. Sign in again.")
        }
        (_, Some(500..)) => (ErrorCode::AgentFailed, "ChatGPT isn't available just now. Try again in a few minutes."),
        _ => (ErrorCode::AgentFailed, "ChatGPT refused the request."),
    };
    if kind != ErrorCode::AgentFailed {
        return failure(kind, text);
    }
    let details = [
        (!message.is_empty()).then(|| message.to_owned()),
        (!code.is_empty()).then(|| format!("Code: {code}")),
        error["param"].as_str().map(|param| format!("Parameter: {param}")),
        status.map(|status| format!("HTTP {status}")),
        request.map(|request| format!("Request: {request}")),
    ];
    let details: Vec<String> = details.into_iter().flatten().collect();
    failure(kind, &format!("{text}\n{}", details.join("\n")))
}

fn http() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .user_agent(concat!("Nuzky/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}
