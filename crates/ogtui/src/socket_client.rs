use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Default socket path matching the Python daemon.
fn default_socket_path() -> PathBuf {
    let tmpdir = std::env::var("TMPDIR")
        .or_else(|_| std::env::var("TEMP"))
        .or_else(|_| std::env::var("TMP"))
        .unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(tmpdir).join("opengraphs-ogd.sock")
}

/// Resolve the socket path from env or default.
pub fn socket_path() -> PathBuf {
    std::env::var("OGD_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_socket_path())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub sender: String,
    pub content: String,
    pub timestamp: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionPlanResponse {
    pub diagnosis: String,
    pub action: String,
    pub code_changes: String,
    pub raw_output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertInfo {
    pub metric: String,
    pub threshold: f64,
    pub current: f64,
    pub message: String,
    pub timestamp: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunStateResponse {
    pub metrics: serde_json::Map<String, Value>,
    pub logs: Vec<String>,
    pub alerts: Vec<AlertInfo>,
    pub current_step: i64,
    pub auto_mode: bool,
    pub runtime: Option<String>,
    pub runtime_status: Option<String>,
    pub runtime_id: Option<String>,
    pub runtime_failure_reason: Option<String>,
    pub runtime_error_type: Option<String>,
    pub runtime_restarts: Option<i64>,
    pub runtime_last_heartbeat: Option<f64>,
    pub runtime_last_exit_code: Option<i64>,
}

#[derive(Debug)]
pub enum ClientError {
    SocketNotFound(PathBuf),
    ConnectionFailed(std::io::Error),
    SendFailed(std::io::Error),
    RecvFailed(std::io::Error),
    InvalidResponse(String),
    DaemonError(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::SocketNotFound(p) => write!(f, "Socket not found: {}", p.display()),
            ClientError::ConnectionFailed(e) => write!(f, "Connection failed: {e}"),
            ClientError::SendFailed(e) => write!(f, "Send failed: {e}"),
            ClientError::RecvFailed(e) => write!(f, "Recv failed: {e}"),
            ClientError::InvalidResponse(s) => write!(f, "Invalid response: {s}"),
            ClientError::DaemonError(s) => write!(f, "Daemon error: {s}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// Send a JSON request to the daemon and return the parsed response.
fn send_request(payload: &Value, sock_path: &Path) -> Result<Value, ClientError> {
    if !sock_path.exists() {
        return Err(ClientError::SocketNotFound(sock_path.to_path_buf()));
    }

    let mut stream = UnixStream::connect(sock_path).map_err(ClientError::ConnectionFailed)?;
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(5))).ok();

    let mut msg = serde_json::to_string(payload).unwrap();
    msg.push('\n');
    stream
        .write_all(msg.as_bytes())
        .map_err(ClientError::SendFailed)?;
    stream.flush().map_err(ClientError::SendFailed)?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(ClientError::RecvFailed)?;

    let resp: Value = serde_json::from_str(line.trim())
        .map_err(|e| ClientError::InvalidResponse(e.to_string()))?;

    if resp.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let err = resp
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error")
            .to_string();
        return Err(ClientError::DaemonError(err));
    }

    Ok(resp)
}

/// Ping the daemon.
pub fn ping(sock_path: &Path) -> Result<bool, ClientError> {
    let resp = send_request(&serde_json::json!({"type": "ping"}), sock_path)?;
    Ok(resp.get("type").and_then(|v| v.as_str()) == Some("pong"))
}

/// Get chat history from the daemon.
pub fn get_chat_history(sock_path: &Path) -> Result<Vec<ChatMessage>, ClientError> {
    let resp = send_request(&serde_json::json!({"type": "get_chat_history"}), sock_path)?;
    let history = resp
        .get("chat_history")
        .cloned()
        .unwrap_or(Value::Array(vec![]));
    let messages: Vec<ChatMessage> =
        serde_json::from_value(history).map_err(|e| ClientError::InvalidResponse(e.to_string()))?;
    Ok(messages)
}

/// Send a chat message to the daemon and get the response.
pub fn send_chat_message(
    content: &str,
    sock_path: &Path,
) -> Result<(ActionPlanResponse, Vec<ChatMessage>), ClientError> {
    let resp = send_request(
        &serde_json::json!({"type": "chat_message", "content": content}),
        sock_path,
    )?;

    let plan: ActionPlanResponse = resp
        .get("response")
        .cloned()
        .ok_or_else(|| ClientError::InvalidResponse("missing response".into()))
        .and_then(|v| {
            serde_json::from_value(v).map_err(|e| ClientError::InvalidResponse(e.to_string()))
        })?;

    let history_val = resp
        .get("chat_history")
        .cloned()
        .unwrap_or(Value::Array(vec![]));
    let history: Vec<ChatMessage> = serde_json::from_value(history_val)
        .map_err(|e| ClientError::InvalidResponse(e.to_string()))?;

    Ok((plan, history))
}

/// Apply a refactor plan (approve from TUI).
pub fn apply_refactor(
    plan: &ActionPlanResponse,
    sock_path: &Path,
) -> Result<(bool, Vec<ChatMessage>), ClientError> {
    let resp = send_request(
        &serde_json::json!({
            "type": "apply_refactor",
            "diagnosis": plan.diagnosis,
            "action": plan.action,
            "code_changes": plan.code_changes,
            "raw_output": plan.raw_output,
        }),
        sock_path,
    )?;

    let success = resp
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let history_val = resp
        .get("chat_history")
        .cloned()
        .unwrap_or(Value::Array(vec![]));
    let history: Vec<ChatMessage> = serde_json::from_value(history_val)
        .map_err(|e| ClientError::InvalidResponse(e.to_string()))?;

    Ok((success, history))
}

/// Update daemon training file path.
pub fn set_training_file(path: &Path, sock_path: &Path) -> Result<(), ClientError> {
    let _resp = send_request(
        &serde_json::json!({
            "type": "set_training_file",
            "path": path.to_string_lossy().to_string(),
        }),
        sock_path,
    )?;
    Ok(())
}

/// Toggle daemon auto mode.
pub fn set_auto_mode(enabled: bool, sock_path: &Path) -> Result<bool, ClientError> {
    let resp = send_request(
        &serde_json::json!({"type": "set_auto_mode", "enabled": enabled}),
        sock_path,
    )?;
    Ok(resp
        .get("auto_mode")
        .and_then(|v| v.as_bool())
        .unwrap_or(enabled))
}

/// Switch daemon runtime backend.
pub fn set_runtime(runtime: &str, sock_path: &Path) -> Result<String, ClientError> {
    let resp = send_request(
        &serde_json::json!({"type": "set_runtime", "runtime": runtime}),
        sock_path,
    )?;
    Ok(resp
        .get("runtime")
        .and_then(|v| v.as_str())
        .unwrap_or(runtime)
        .to_string())
}

/// Restart/start daemon training process with current run_state config.
pub fn start_training(sock_path: &Path) -> Result<(), ClientError> {
    let _resp = send_request(&serde_json::json!({"type": "start_training"}), sock_path)?;
    Ok(())
}

/// Get run state from the daemon.
pub fn get_run_state(sock_path: &Path) -> Result<RunStateResponse, ClientError> {
    let resp = send_request(
        &serde_json::json!({"type": "get_run_state", "log_tail": 200, "metric_tail": 64}),
        sock_path,
    )?;

    let run_state = resp
        .get("run_state")
        .cloned()
        .ok_or_else(|| ClientError::InvalidResponse("missing run_state".into()))?;

    serde_json::from_value(run_state).map_err(|e| ClientError::InvalidResponse(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::os::unix::net::UnixListener;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_SOCKET_TEST_ID: AtomicU64 = AtomicU64::new(0);

    struct TempSocketPath {
        dir: PathBuf,
        socket: PathBuf,
    }

    impl TempSocketPath {
        fn new(prefix: &str) -> Self {
            let unique_id = NEXT_SOCKET_TEST_ID.fetch_add(1, Ordering::Relaxed);
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "og-{prefix}-{}-{unique_id}-{:x}",
                std::process::id(),
                unique & 0xffff_ffff
            ));
            fs::create_dir_all(&dir).unwrap();
            let socket = dir.join("ogd.sock");
            Self { dir, socket }
        }
    }

    impl Drop for TempSocketPath {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.socket);
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn json_line(value: Value) -> String {
        format!("{}\n", serde_json::to_string(&value).unwrap())
    }

    fn with_server<F, R>(response_fn: F, action: impl FnOnce(&Path) -> R) -> (Value, R)
    where
        F: FnOnce(Value) -> String + Send + 'static,
    {
        let temp = TempSocketPath::new("socket-client-test");
        let listener = UnixListener::bind(&temp.socket).unwrap();
        let (request_tx, request_rx) = mpsc::channel();

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            reader.read_line(&mut line).unwrap();

            let request: Value = serde_json::from_str(line.trim()).unwrap();
            request_tx.send(request.clone()).unwrap();

            let response = response_fn(request);
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
        });

        let result = action(&temp.socket);
        let request = request_rx.recv().unwrap();
        server.join().unwrap();
        (request, result)
    }

    #[test]
    fn send_request_returns_socket_not_found_for_missing_socket() {
        let temp = TempSocketPath::new("missing-socket");
        let err = ping(&temp.socket).unwrap_err();

        match err {
            ClientError::SocketNotFound(path) => assert_eq!(path, temp.socket),
            other => panic!("expected SocketNotFound, got {other}"),
        }
    }

    #[test]
    fn send_request_returns_connection_failed_for_non_socket_path() {
        let temp = TempSocketPath::new("non-socket");
        fs::write(&temp.socket, b"not a socket").unwrap();

        let err = ping(&temp.socket).unwrap_err();
        assert!(matches!(err, ClientError::ConnectionFailed(_)));
    }

    #[test]
    fn ping_round_trips_over_the_socket_protocol() {
        let (request, result) =
            with_server(|_| json_line(json!({"ok": true, "type": "pong"})), ping);

        assert_eq!(request, json!({"type": "ping"}));
        assert_eq!(result.unwrap(), true);
    }

    #[test]
    fn get_chat_history_parses_messages_from_the_daemon() {
        let expected_history = vec![ChatMessage {
            sender: "agent".into(),
            content: "hello".into(),
            timestamp: 12.5,
        }];

        let (request, result) = with_server(
            move |_| json_line(json!({"ok": true, "chat_history": expected_history})),
            get_chat_history,
        );

        assert_eq!(request, json!({"type": "get_chat_history"}));

        let history = result.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].sender, "agent");
        assert_eq!(history[0].content, "hello");
        assert_eq!(history[0].timestamp, 12.5);
    }

    #[test]
    fn send_chat_message_round_trips_request_and_structured_response() {
        let (request, result) = with_server(
            |_| {
                json_line(json!({
                    "ok": true,
                    "response": {
                        "diagnosis": "Issue found",
                        "action": "Patch it",
                        "code_changes": "diff --git",
                        "raw_output": "done"
                    },
                    "chat_history": [
                        {
                            "sender": "user",
                            "content": "fix this",
                            "timestamp": 1.0
                        },
                        {
                            "sender": "agent",
                            "content": "working on it",
                            "timestamp": 2.0
                        }
                    ]
                }))
            },
            |sock_path| send_chat_message("fix this", sock_path),
        );

        assert_eq!(
            request,
            json!({"type": "chat_message", "content": "fix this"})
        );

        let (plan, history) = result.unwrap();
        assert_eq!(plan.diagnosis, "Issue found");
        assert_eq!(plan.action, "Patch it");
        assert_eq!(plan.code_changes, "diff --git");
        assert_eq!(plan.raw_output, "done");
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].sender, "agent");
    }

    #[test]
    fn apply_refactor_sends_plan_fields_and_returns_history() {
        let plan = ActionPlanResponse {
            diagnosis: "bad state".into(),
            action: "apply fix".into(),
            code_changes: "patch body".into(),
            raw_output: "stderr".into(),
        };

        let (request, result) = with_server(
            |_| {
                json_line(json!({
                    "ok": true,
                    "success": true,
                    "chat_history": [
                        {
                            "sender": "system",
                            "content": "applied",
                            "timestamp": 3.0
                        }
                    ]
                }))
            },
            |sock_path| apply_refactor(&plan, sock_path),
        );

        assert_eq!(
            request,
            json!({
                "type": "apply_refactor",
                "diagnosis": "bad state",
                "action": "apply fix",
                "code_changes": "patch body",
                "raw_output": "stderr"
            })
        );

        let (success, history) = result.unwrap();
        assert!(success);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content, "applied");
    }

    #[test]
    fn get_run_state_parses_run_state_payloads() {
        let (request, result) = with_server(
            |_| {
                json_line(json!({
                    "ok": true,
                    "run_state": {
                        "metrics": {
                            "loss": 0.42
                        },
                        "logs": ["line one", "line two"],
                        "alerts": [
                            {
                                "metric": "loss",
                                "threshold": 1.0,
                                "current": 1.5,
                                "message": "too high",
                                "timestamp": 123.0
                            }
                        ],
                        "current_step": 7,
                        "auto_mode": true,
                        "runtime": "local",
                        "runtime_status": "running",
                        "runtime_id": "run-1",
                        "runtime_failure_reason": null,
                        "runtime_error_type": null,
                        "runtime_restarts": 2,
                        "runtime_last_heartbeat": 321.0,
                        "runtime_last_exit_code": 0
                    }
                }))
            },
            get_run_state,
        );

        assert_eq!(
            request,
            json!({"type": "get_run_state", "log_tail": 200, "metric_tail": 64})
        );

        let run_state = result.unwrap();
        assert_eq!(run_state.current_step, 7);
        assert_eq!(run_state.logs.len(), 2);
        assert_eq!(run_state.alerts.len(), 1);
        assert_eq!(run_state.runtime.as_deref(), Some("local"));
        assert_eq!(run_state.runtime_restarts, Some(2));
    }

    #[test]
    fn send_request_surfaces_daemon_errors() {
        let (_request, result) = with_server(
            |_| json_line(json!({"ok": false, "error": "daemon exploded"})),
            ping,
        );

        match result.unwrap_err() {
            ClientError::DaemonError(message) => assert_eq!(message, "daemon exploded"),
            other => panic!("expected DaemonError, got {other}"),
        }
    }

    #[test]
    fn send_request_rejects_invalid_json_responses() {
        let (_request, result) = with_server(|_| "not-json\n".to_string(), ping);

        assert!(matches!(
            result.unwrap_err(),
            ClientError::InvalidResponse(_)
        ));
    }
}
