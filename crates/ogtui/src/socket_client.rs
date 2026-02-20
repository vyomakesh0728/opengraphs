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
