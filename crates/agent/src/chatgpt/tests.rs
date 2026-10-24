use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread::{self, JoinHandle};

struct Server {
    api: String,
    requests: Arc<Mutex<Vec<Value>>>,
    stream_wait: Arc<Mutex<Option<Receiver<()>>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    fn new(replies: Vec<(u16, String)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stream_wait = Arc::new(Mutex::new(None::<Receiver<()>>));
        let wait = stream_wait.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let (recorded, stopping) = (requests.clone(), stop.clone());
        let thread = thread::spawn(move || {
            let mut replies = replies.into_iter();
            while !stopping.load(Ordering::Acquire) {
                let (mut socket, _) = match listener.accept() {
                    Ok(socket) => socket,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(e) => panic!("accept: {e}"),
                };
                // macOS hands the listener's non-blocking mode to the accepted socket.
                socket.set_nonblocking(false).unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert_eq!(line, "POST /responses HTTP/1.1\r\n");
                let mut length = None;
                loop {
                    line.clear();
                    assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = Some(value.trim().parse::<usize>().unwrap());
                    }
                }
                let mut body = vec![0; length.expect("request content length")];
                reader.read_exact(&mut body).unwrap();
                recorded.lock().unwrap().push(serde_json::from_slice(&body).unwrap());
                let (status, body) = replies.next().expect("unexpected request");
                let headers =
                    format!("HTTP/1.1 {status} Test\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n");
                socket.write_all(headers.as_bytes()).unwrap();
                if let Some(release) = wait.lock().unwrap().take() {
                    let (first, rest) = body.split_once("\n\n").unwrap();
                    socket.write_all(format!("{first}\n\n").as_bytes()).unwrap();
                    release.recv_timeout(Duration::from_secs(3)).unwrap();
                    socket.write_all(rest.as_bytes()).unwrap();
                } else {
                    socket.write_all(body.as_bytes()).unwrap();
                }
            }
        });
        Self { api, requests, stream_wait, stop, thread: Some(thread) }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn sse(events: Vec<Value>) -> (u16, String) {
    (200, events.iter().map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap())).collect())
}

fn delta(text: &str) -> Value {
    json!({"type": "response.output_text.delta", "delta": text})
}
fn item(value: Value) -> Value {
    json!({"type": "response.output_item.done", "item": value})
}
fn completed() -> Value {
    json!({"type": "response.completed", "response": {"id": "response-1"}})
}
fn user() -> Value {
    json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Cut it"}]})
}
fn message() -> Value {
    json!({"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Cut done"}]})
}
fn answer() -> (u16, String) {
    let mut message = message();
    message["id"] = json!("message-1");
    sse(vec![delta("Cut "), delta("done"), item(message), completed()])
}
fn call() -> Value {
    json!({"id": "item-1", "type": "function_call", "namespace": "nuzky", "name": "cut", "call_id": "call-1", "arguments": "{\"at\":2}"})
}
fn tool_reply() -> Value {
    json!({"content": [{"type": "text", "text": "Edited"}], "isError": false})
}

struct FakeTools {
    calls: mpsc::Sender<(String, Value)>,
    result: Value,
    block: Mutex<Option<Receiver<()>>>,
    ended: AtomicBool,
    fail_end: AtomicBool,
}

impl Tools for FakeTools {
    fn call(&self, name: &str, arguments: Value) -> Value {
        self.calls.send((name.to_owned(), arguments)).unwrap();
        if let Some(release) = self.block.lock().unwrap().take() {
            let _ = release.recv_timeout(Duration::from_secs(10));
        }
        self.result.clone()
    }
    fn end(&self) -> Result<()> {
        self.ended.store(true, Ordering::Release);
        anyhow::ensure!(!self.fail_end.load(Ordering::Acquire), "SAVE_FAILED: the disk is full");
        Ok(())
    }
}

struct Fixture {
    turn: Turn,
    events: Receiver<AgentEvent>,
    calls: Receiver<(String, Value)>,
    run: Arc<FakeTools>,
    history: Arc<Mutex<Vec<Value>>>,
    renew: Arc<Mutex<Vec<bool>>>,
}

impl Fixture {
    fn start(s: &Server, res: Value, err: Option<&'static str>, wait: Option<Receiver<()>>, prior: Vec<Value>) -> Self {
        Self::start_with_effort(s, res, err, wait, prior, None)
    }
    fn start_with_effort(
        s: &Server,
        res: Value,
        err: Option<&'static str>,
        wait: Option<Receiver<()>>,
        prior: Vec<Value>,
        effort: Option<String>,
    ) -> Self {
        let (tx, events) = mpsc::channel();
        let (calls_tx, calls) = mpsc::channel();
        let run = Arc::new(FakeTools {
            calls: calls_tx,
            result: res,
            block: Mutex::new(wait),
            ended: AtomicBool::new(false),
            fail_end: AtomicBool::new(false),
        });
        let history = Arc::new(Mutex::new(prior));
        let renew = Arc::new(Mutex::new(Vec::new()));
        let flags = renew.clone();
        let request = ChatGptRequest {
            api: s.api.clone(),
            model: "test-model".into(),
            effort,
            guide: "Our editing guide".into(),
            prompt: "Cut it".into(),
            tools: json!([{ "name": "cut", "description": "Cut a clip", "inputSchema": {
                "$schema": "https://json-schema.org/draft/2020-12/schema", "type": "object",
                "properties": {"at": {"type": "number"}}, "required": ["at"]
            }}]),
            history: history.clone(),
            run: run.clone(),
            token: Arc::new(move |renew| {
                flags.lock().unwrap().push(renew);
                if let Some(error) = err {
                    anyhow::bail!(error);
                }
                Ok(if renew { "renewed-token" } else { "test-token" }.into())
            }),
        };
        let turn = Turn::chatgpt(request, move |event| drop(tx.send(event))).unwrap();
        Self { turn, events, calls, run, history, renew }
    }
    fn finish(&self) -> Vec<AgentEvent> {
        let events: Vec<_> = std::iter::from_fn(|| match self.events.recv_timeout(Duration::from_secs(3)) {
            Ok(event) => Some(event),
            Err(RecvTimeoutError::Disconnected) => None,
            Err(e) => panic!("turn did not finish: {e}"),
        })
        .collect();
        assert!(!self.turn.running());
        assert!(self.run.ended.load(Ordering::Acquire));
        events
    }
}

fn text_events() -> Vec<AgentEvent> {
    ["Cut ", "done"]
        .into_iter()
        .map(|text| AgentEvent::Text { text: text.into() })
        .chain([AgentEvent::Done { stopped: false }])
        .collect()
}
fn error(event: &AgentEvent, expected: ErrorCode) -> &str {
    let AgentEvent::Error { code, message } = event else { panic!("expected error, got {event:?}") };
    assert_eq!(*code, expected);
    message
}

#[test]
fn steering_during_tool_follows_its_output_and_stays_in_history() {
    let server = Server::new(vec![sse(vec![item(call()), completed()]), answer()]);
    let (release, blocked) = mpsc::channel();
    let f = Fixture::start(&server, tool_reply(), None, Some(blocked), vec![]);
    f.calls.recv_timeout(Duration::from_secs(3)).unwrap();
    let accepted = f.turn.steer(&"Keep the ending".into());
    let also_accepted = f.turn.steer(&"Keep the sound too".into());
    release.send(()).unwrap();
    let events = f.finish();
    assert!(accepted && also_accepted);
    assert_eq!(events.last(), Some(&AgentEvent::Done { stopped: false }));
    let mut input_call = call();
    input_call.as_object_mut().unwrap().remove("id");
    let input = vec![
        user(),
        input_call,
        json!({"type": "function_call_output", "call_id": "call-1", "output": "Edited"}),
        json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Keep the ending"}]}),
        json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Keep the sound too"}]}),
    ];
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["input"], json!([user()]));
    assert_eq!(requests[1]["input"], json!(input));
    let mut kept = input;
    kept.push(message());
    assert_eq!(*f.history.lock().unwrap(), kept);
}

#[test]
fn steering_during_final_stream_asks_again_and_stays_in_history() {
    let server = Server::new(vec![answer(), answer()]);
    let (release, blocked) = mpsc::channel();
    *server.stream_wait.lock().unwrap() = Some(blocked);
    let f = Fixture::start(&server, tool_reply(), None, None, vec![]);
    assert_eq!(f.events.recv_timeout(Duration::from_secs(3)).unwrap(), AgentEvent::Text { text: "Cut ".into() });
    let accepted = f.turn.steer(&"Keep the ending".into());
    release.send(()).unwrap();
    let events = f.finish();
    assert!(accepted);
    assert_eq!(events.last(), Some(&AgentEvent::Done { stopped: false }));
    assert_eq!(events.iter().filter(|event| matches!(event, AgentEvent::Done { .. })).count(), 1);
    let input = vec![
        user(),
        message(),
        json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Keep the ending"}]}),
    ];
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1]["input"], json!(input));
    let mut kept = input;
    kept.push(message());
    assert_eq!(*f.history.lock().unwrap(), kept);
}

#[test]
fn steering_inbox_closes_before_done_or_error() {
    for reply in [answer(), (429, json!({"error": {"message": "Usage limit"}}).to_string())] {
        let server = Server::new(vec![reply]);
        let f = Fixture::start(&server, tool_reply(), None, None, vec![]);
        loop {
            let event = f.events.recv_timeout(Duration::from_secs(3)).unwrap();
            if matches!(event, AgentEvent::Done { .. } | AgentEvent::Error { .. }) {
                break;
            }
        }
        assert!(!f.turn.steer(&"Too late".into()));
        let crate::turn::Inbox::ChatGpt(inbox) = &f.turn.inbox else { panic!("missing ChatGPT inbox") };
        assert!(inbox.lock().unwrap().is_none());
        f.finish();
    }
}

#[test]
fn text_stream_and_request_contract() {
    let server = Server::new(vec![answer()]);
    let f = Fixture::start(&server, tool_reply(), None, None, vec![]);
    assert_eq!(f.finish(), text_events());
    assert_eq!(*f.history.lock().unwrap(), vec![user(), message()]);
    assert_eq!(*f.renew.lock().unwrap(), vec![false]);
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let body = &requests[0];
    assert_eq!(body["model"], "test-model");
    assert!(body.get("reasoning").is_none());
    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], true);
    assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
    assert!(body["instructions"].as_str().unwrap().contains("Our editing guide"));
    assert_eq!(body["input"], json!([user()]));
    let tools = json!([{"type": "namespace", "name": "nuzky", "description": "Nuzky's tools for editing the project open in the app.", "tools": [{
        "type": "function", "name": "cut", "description": "Cut a clip", "strict": false,
        "parameters": {"type": "object", "properties": {"at": {"type": "number"}}, "required": ["at"]}
    }]}, {"type": "web_search", "external_web_access": false}]);
    assert_eq!(body["tools"], tools);
    let unsupported = "background conversation max_output_tokens max_tool_calls metadata moderation multi_agent prompt \
        prompt_cache_retention safety_identifier temperature top_logprobs top_p truncation user previous_response_id";
    for field in unsupported.split_whitespace() {
        assert!(body.get(field).is_none(), "unsupported field {field}");
    }
}

#[test]
fn model_effort_reaches_the_responses_request() {
    let server = Server::new(vec![answer()]);
    let f = Fixture::start_with_effort(&server, tool_reply(), None, None, vec![], Some("high".into()));
    assert_eq!(f.finish(), text_events());
    assert_eq!(server.requests.lock().unwrap()[0]["reasoning"], json!({"effort": "high"}));
}

#[test]
fn tools_reasoning_images_and_failures_round_trip() {
    let reasoning = json!({"type": "reasoning", "summary": [], "encrypted_content": "encrypted"});
    let image = json!({"content": [{"type": "text", "text": "Frame"}, {"type": "image", "mimeType": "image/png", "data": "aGVsbG8="}], "isError": false});
    let image_output = json!([{"type": "input_text", "text": "Frame"}, {"type": "input_image", "image_url": "data:image/png;base64,aGVsbG8="}]);
    let failed = json!({"content": [{"type": "text", "text": "Cannot cut"}], "isError": true});
    for (result, output, status) in [
        (tool_reply(), json!("Edited"), ToolStatus::Done),
        (image, image_output, ToolStatus::Done),
        (failed, json!("Cannot cut"), ToolStatus::Failed),
    ] {
        let mut wire_reasoning = reasoning.clone();
        wire_reasoning["id"] = json!("reasoning-1");
        let server = Server::new(vec![sse(vec![item(wire_reasoning), item(call()), completed()]), answer()]);
        let f = Fixture::start(&server, result, None, None, vec![]);
        let running = AgentEvent::Tool {
            id: "call-1".into(),
            name: "cut".into(),
            status: ToolStatus::Running,
            input: Some(json!({"at": 2})),
        };
        let done = AgentEvent::Tool { id: "call-1".into(), name: "cut".into(), status, input: None };
        let mut expected = vec![running, done];
        expected.extend(text_events());
        assert_eq!(f.finish(), expected);
        assert_eq!(f.calls.try_iter().collect::<Vec<_>>(), vec![("cut".into(), json!({"at": 2}))]);
        let mut input_call = call();
        input_call.as_object_mut().unwrap().remove("id");
        let result = json!({"type": "function_call_output", "call_id": "call-1", "output": output});
        let input = vec![user(), reasoning.clone(), input_call, result];
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1]["input"], json!(input));
        let mut kept = input;
        kept.push(message());
        assert_eq!(*f.history.lock().unwrap(), kept);
    }
}

#[test]
fn a_web_search_shows_as_a_step_and_goes_back_with_its_id() {
    let started = json!({"type": "web_search_call", "id": "ws-1", "status": "in_progress"});
    let searched = json!({"type": "web_search_call", "id": "ws-1", "status": "completed",
        "action": {"type": "search", "query": "Spotify logo svg"}});
    let several = json!({"type": "web_search_call", "id": "ws-2", "status": "completed",
        "action": {"type": "search", "queries": ["Spotify brand colors", "Spotify green hex"]}});
    let added = json!({"type": "response.output_item.added", "item": started});
    let first = sse(vec![added, item(searched.clone()), item(several.clone()), item(call()), completed()]);
    let server = Server::new(vec![first, answer()]);
    let f = Fixture::start(&server, tool_reply(), None, None, vec![]);
    let search = |id: &str, status, input| AgentEvent::Tool { id: id.into(), name: "web_search".into(), status, input };
    let mut expected = vec![
        search("ws-1", ToolStatus::Running, None),
        search("ws-1", ToolStatus::Done, Some(json!({"query": "Spotify logo svg"}))),
        search("ws-2", ToolStatus::Done, Some(json!({"query": "Spotify brand colors · Spotify green hex"}))),
        AgentEvent::Tool {
            id: "call-1".into(),
            name: "cut".into(),
            status: ToolStatus::Running,
            input: Some(json!({"at": 2})),
        },
        AgentEvent::Tool { id: "call-1".into(), name: "cut".into(), status: ToolStatus::Done, input: None },
    ];
    expected.extend(text_events());
    assert_eq!(f.finish(), expected);
    let mut input_call = call();
    input_call.as_object_mut().unwrap().remove("id");
    let result = json!({"type": "function_call_output", "call_id": "call-1", "output": "Edited"});
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests[1]["input"], json!([user(), searched, several, input_call, result]));
}

#[test]
fn another_hosted_tool_ends_the_turn() {
    for kind in ["image_generation_call", "code_interpreter_call"] {
        let server = Server::new(vec![sse(vec![
            item(json!({"type": kind, "id": "hosted-1", "status": "completed"})),
            completed(),
        ])]);
        let f = Fixture::start(&server, tool_reply(), None, None, vec![]);
        let events = f.finish();
        assert_eq!(events.len(), 1, "{events:?}");
        assert!(error(&events[0], ErrorCode::AgentFailed).contains(kind));
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }
}

#[test]
fn usage_limits_preserve_prior_history() {
    for code in ["subscription_sharing_usage_limit_exceeded", "subscription_sharing_usage_unavailable"] {
        for streaming in [true, false] {
            let problem = json!({"code": code, "message": format!("Private detail: {code}")});
            let reply = if streaming {
                sse(vec![delta("Partial"), json!({"type": "response.failed", "response": {"error": problem}})])
            } else {
                (429, json!({"error": problem}).to_string())
            };
            let server = Server::new(vec![reply]);
            let f = Fixture::start(&server, tool_reply(), None, None, vec![message()]);
            let events = f.finish();
            if streaming {
                assert_eq!(events[0], AgentEvent::Text { text: "Partial".into() });
            }
            assert_eq!(events.len(), if streaming { 2 } else { 1 });
            let detail = error(events.last().unwrap(), ErrorCode::UsageLimit);
            assert!(!detail.contains(code));
            assert!(!detail.contains("Private detail"));
            assert_eq!(*f.history.lock().unwrap(), vec![message()]);
        }
    }
}

#[test]
fn incomplete_and_unterminated_streams_fail() {
    let incomplete =
        json!({"type": "response.incomplete", "response": {"incomplete_details": {"reason": "max_output_tokens"}}});
    for terminal in [Some(incomplete), None] {
        let mut events = vec![delta("Partial")];
        events.extend(terminal);
        let server = Server::new(vec![sse(events)]);
        let f = Fixture::start(&server, tool_reply(), None, None, vec![]);
        let events = f.finish();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], AgentEvent::Text { text: "Partial".into() });
        error(&events[1], ErrorCode::AgentFailed);
        assert!(f.history.lock().unwrap().is_empty());
    }
}

#[test]
fn unauthorized_renews_once_and_token_errors_make_no_request() {
    for succeeds in [true, false] {
        let refusal = (401, json!({"error": {"message": "Expired"}}).to_string());
        let server = Server::new(vec![refusal.clone(), if succeeds { answer() } else { refusal }]);
        let f = Fixture::start(&server, tool_reply(), None, None, vec![]);
        let events = f.finish();
        if succeeds {
            assert_eq!(events, text_events());
        } else {
            assert_eq!(events.len(), 1);
            error(&events[0], ErrorCode::NotSignedIn);
            assert!(f.history.lock().unwrap().is_empty());
        }
        assert_eq!(*f.renew.lock().unwrap(), vec![false, true]);
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
    }
    for (problem, code) in [
        ("NOT_SIGNED_IN: Sign in again", ErrorCode::NotSignedIn),
        ("PLAN_UNAVAILABLE: Enable your plan", ErrorCode::PlanUnavailable),
    ] {
        let server = Server::new(vec![]);
        let f = Fixture::start(&server, tool_reply(), Some(problem), None, vec![]);
        let events = f.finish();
        assert_eq!(events.len(), 1);
        assert_eq!(error(&events[0], code), problem.split_once(": ").unwrap().1);
        assert_eq!(*f.renew.lock().unwrap(), vec![false]);
        assert!(server.requests.lock().unwrap().is_empty());
    }
}

#[test]
fn stop_during_tool_ends_promptly_and_answers_pending_call() {
    let server = Server::new(vec![sse(vec![item(call()), completed()])]);
    let (release, blocked) = mpsc::channel();
    let f = Fixture::start(&server, tool_reply(), None, Some(blocked), vec![]);
    assert_eq!(f.calls.recv_timeout(Duration::from_secs(3)).unwrap(), ("cut".into(), json!({"at": 2})));
    let running = f.events.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(matches!(running, AgentEvent::Tool { status: ToolStatus::Running, input: Some(_), .. }));
    f.turn.stop();
    assert_eq!(f.events.recv_timeout(Duration::from_secs(1)).unwrap(), AgentEvent::Done { stopped: true });
    assert!(f.run.ended.load(Ordering::Acquire));
    assert!(!f.turn.running());
    assert!(!f.turn.steer(&"Too late".into()));
    let crate::turn::Inbox::ChatGpt(inbox) = &f.turn.inbox else { panic!("missing ChatGPT inbox") };
    assert!(inbox.lock().unwrap().is_none());
    let history = f.history.lock().unwrap();
    assert_eq!(history.len(), 3);
    assert_eq!(history[2]["type"], "function_call_output");
    assert_eq!(history[2]["call_id"], "call-1");
    assert!(history[2]["output"].as_str().unwrap().starts_with("RUN_STOPPED"));
    release.send(()).unwrap();
    assert!(matches!(f.events.recv_timeout(Duration::from_secs(1)), Err(RecvTimeoutError::Disconnected)));
}

#[test]
fn a_run_that_cannot_be_kept_ends_the_turn_with_an_error() {
    let server = Server::new(vec![sse(vec![item(call()), completed()]), answer()]);
    let (release, blocked) = mpsc::channel();
    let f = Fixture::start(&server, tool_reply(), None, Some(blocked), vec![]);
    f.calls.recv_timeout(Duration::from_secs(3)).unwrap();
    f.run.fail_end.store(true, Ordering::Release);
    release.send(()).unwrap();
    let events = f.finish();
    let message = error(events.last().unwrap(), ErrorCode::AgentFailed);
    assert!(
        message.starts_with("Nuzky couldn't finish ChatGPT's edits") && message.contains("SAVE_FAILED"),
        "{message}"
    );
}

#[test]
fn stop_while_the_token_renews_sends_nothing() {
    let server = Server::new(vec![]);
    let (tx, events) = mpsc::channel();
    let run = Arc::new(FakeTools {
        calls: mpsc::channel().0,
        result: tool_reply(),
        block: Mutex::default(),
        ended: AtomicBool::new(false),
        fail_end: AtomicBool::new(false),
    });
    let request = ChatGptRequest {
        api: server.api.clone(),
        model: "test-model".into(),
        effort: None,
        guide: String::new(),
        prompt: "Cut it".into(),
        tools: json!([]),
        history: Arc::default(),
        run: run.clone(),
        // A renewal stuck on a slow network.
        token: Arc::new(|_| {
            thread::sleep(Duration::from_secs(2));
            Ok("late-token".into())
        }),
    };
    let turn = Turn::chatgpt(request, move |event| drop(tx.send(event))).unwrap();
    thread::sleep(Duration::from_millis(100));
    turn.stop();
    assert_eq!(events.recv_timeout(Duration::from_millis(500)).unwrap(), AgentEvent::Done { stopped: true });
    assert!(run.ended.load(Ordering::Acquire));
    thread::sleep(Duration::from_secs(2));
    assert!(server.requests.lock().unwrap().is_empty());
}
