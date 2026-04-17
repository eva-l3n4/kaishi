use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::event::{AppEvent, ApprovalOption, SessionInfo, Usage};

// -------------------------------------------------------------------
// JSON-RPC wire types
// -------------------------------------------------------------------

#[derive(Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(serde::Deserialize, Debug)]
struct JsonRpcMessage {
    id: Option<Value>,
    method: Option<String>,
    params: Option<Value>,
    result: Option<Value>,
    error: Option<Value>,
}

// -------------------------------------------------------------------
// ACP Client
// -------------------------------------------------------------------

type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>>;

pub struct AcpClient {
    stdin: Arc<Mutex<tokio::process::ChildStdin>>,
    child: Arc<Mutex<Child>>,
    next_id: AtomicU64,
    pending: PendingMap,
    #[allow(dead_code)]
    event_tx: mpsc::UnboundedSender<AppEvent>,
}

impl AcpClient {
    /// Spawn `hermes acp` and start the reader task.
    pub async fn spawn(
        event_tx: mpsc::UnboundedSender<AppEvent>,
        profile: Option<&str>,
    ) -> Result<Self> {
        let mut cmd = Command::new("hermes");
        cmd.arg("acp");
        if let Some(p) = profile {
            cmd.arg("--profile").arg(p);
        }
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit());

        let mut child = cmd.spawn().context("Failed to spawn `hermes acp`")?;

        let stdin = child.stdin.take().context("No stdin on child")?;
        let stdout = child.stdout.take().context("No stdout on child")?;

        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let event_tx_clone = event_tx.clone();
        let pending_clone = pending.clone();

        // Reader task: parse JSON-RPC messages from stdout
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.replace('\0', "").trim().to_string();
                if line.is_empty() {
                    continue;
                }
                let msg: JsonRpcMessage = match serde_json::from_str(&line) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                Self::dispatch_message(msg, &pending_clone, &event_tx_clone).await;
            }
            // Subprocess stdout closed — signal error
            let _ = event_tx_clone.send(AppEvent::AcpError("ACP subprocess exited".into()));
        });

        Ok(Self {
            stdin: Arc::new(Mutex::new(stdin)),
            child: Arc::new(Mutex::new(child)),
            next_id: AtomicU64::new(1),
            pending,
            event_tx,
        })
    }

    /// Send a JSON-RPC request and wait for the response.
    pub async fn request(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        };
        let mut payload = serde_json::to_string(&req)?;
        payload.push('\n');

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        {
            let mut stdin = self.stdin.lock().await;
            stdin.write_all(payload.as_bytes()).await?;
            stdin.flush().await?;
        }

        rx.await?.context("ACP request failed")
    }

    /// Send a JSON-RPC notification (no response expected).
    #[allow(dead_code)]
    pub async fn notify(&self, method: &str, params: Option<Value>) -> Result<()> {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params.unwrap_or(Value::Null),
        });
        let mut payload = serde_json::to_string(&msg)?;
        payload.push('\n');

        let mut stdin = self.stdin.lock().await;
        stdin.write_all(payload.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    /// Send a JSON-RPC response (for server-to-client requests like
    /// request_permission).
    pub async fn respond(&self, id: Value, result: Value) -> Result<()> {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        });
        let mut payload = serde_json::to_string(&msg)?;
        payload.push('\n');

        let mut stdin = self.stdin.lock().await;
        stdin.write_all(payload.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    // ---- Dispatch incoming messages ----------------------------------------

    async fn dispatch_message(
        msg: JsonRpcMessage,
        pending: &PendingMap,
        event_tx: &mpsc::UnboundedSender<AppEvent>,
    ) {
        // Response to one of our requests
        if let Some(id_val) = &msg.id {
            if msg.method.is_none() {
                // This is a response, not a request
                if let Some(id_num) = id_val.as_u64() {
                    let mut pending = pending.lock().await;
                    if let Some(tx) = pending.remove(&id_num) {
                        if let Some(err) = msg.error {
                            let _ = tx.send(Err(anyhow::anyhow!("RPC error: {}", serde_json::to_string_pretty(&err).unwrap_or_else(|_| err.to_string()))));
                        } else {
                            let _ = tx.send(Ok(msg.result.unwrap_or(Value::Null)));
                        }
                    }
                }
                return;
            }
        }

        // Server-to-client request (has method + id)
        if let (Some(method), Some(id)) = (&msg.method, &msg.id) {
            if method == "request_permission" {
                Self::handle_permission_request(id.clone(), &msg.params, event_tx);
            }
            return;
        }

        // Notification (has method, no id) — session_update events
        if let Some(method) = &msg.method {
            if method == "session_update" || method == "notifications/session_update" {
                if let Some(params) = &msg.params {
                    Self::handle_session_update(params, event_tx);
                }
            }
        }
    }

    fn handle_permission_request(
        id: Value,
        params: &Option<Value>,
        event_tx: &mpsc::UnboundedSender<AppEvent>,
    ) {
        let params = match params {
            Some(p) => p,
            None => return,
        };
        let command = params
            .pointer("/tool_call/title")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown command")
            .to_string();
        let options: Vec<ApprovalOption> = params
            .get("options")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|o| {
                        Some(ApprovalOption {
                            id: o.get("option_id")?.as_str()?.to_string(),
                            name: o.get("name")?.as_str()?.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let _ = event_tx.send(AppEvent::ApprovalRequest {
            request_id: id,
            command,
            options,
        });
    }

    fn handle_session_update(
        params: &Value,
        event_tx: &mpsc::UnboundedSender<AppEvent>,
    ) {
        let update_type = params
            .get("sessionUpdate")
            .or_else(|| params.get("session_update"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        match update_type {
            "agent_message_chunk" => {
                if let Some(text) = params
                    .pointer("/content/text")
                    .and_then(|v| v.as_str())
                {
                    let _ = event_tx.send(AppEvent::AgentMessage(text.to_string()));
                }
            }
            "agent_thought_chunk" => {
                if let Some(text) = params
                    .pointer("/content/text")
                    .and_then(|v| v.as_str())
                {
                    let _ = event_tx.send(AppEvent::AgentThought(text.to_string()));
                }
            }
            "tool_call" => {
                let id = params
                    .get("tool_call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = params
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let kind = params
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let _ = event_tx.send(AppEvent::ToolCallStart { id, name, kind });
            }
            "tool_call_update" => {
                let id = params
                    .get("tool_call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let status = params
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let content = params
                    .pointer("/content/0/text")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let _ = event_tx.send(AppEvent::ToolCallUpdate {
                    id,
                    status,
                    content,
                });
            }
            "prompt_done" | "done" => {
                let stop_reason = params
                    .get("stop_reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("end_turn")
                    .to_string();
                let usage = params.get("usage").and_then(|u| {
                    Some(Usage {
                        input_tokens: u.get("input_tokens")?.as_u64()?,
                        output_tokens: u.get("output_tokens")?.as_u64()?,
                    })
                });
                let _ = event_tx.send(AppEvent::PromptDone { stop_reason, usage });
            }
            _ => {}
        }
    }

    // ---- High-level ACP operations -----------------------------------------

    pub async fn initialize(&self) -> Result<Value> {
        self.request(
            "initialize",
            Some(serde_json::json!({
                "client_info": {
                    "name": "hermes-tui",
                    "version": env!("CARGO_PKG_VERSION"),
                },
            })),
        )
        .await
    }

    pub async fn list_sessions(&self) -> Result<Vec<SessionInfo>> {
        let result = self
            .request("list_sessions", Some(serde_json::json!({})))
            .await?;
        let sessions = result
            .get("sessions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| {
                        Some(SessionInfo {
                            session_id: s.get("session_id")?.as_str()?.to_string(),
                            cwd: s
                                .get("cwd")
                                .and_then(|v| v.as_str())
                                .unwrap_or(".")
                                .to_string(),
                            model: s
                                .get("model")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            history_len: s
                                .get("history_len")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0)
                                as usize,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(sessions)
    }

    pub async fn new_session(&self, cwd: &str) -> Result<String> {
        let result = self
            .request(
                "new_session",
                Some(serde_json::json!({ "cwd": cwd })),
            )
            .await?;
        let session_id = result
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(session_id)
    }

    pub async fn resume_session(&self, cwd: &str, session_id: &str) -> Result<()> {
        self.request(
            "resume_session",
            Some(serde_json::json!({
                "cwd": cwd,
                "session_id": session_id,
            })),
        )
        .await?;
        Ok(())
    }

    pub async fn prompt(&self, text: &str, session_id: &str) -> Result<Value> {
        self.request(
            "prompt",
            Some(serde_json::json!({
                "session_id": session_id,
                "prompt": [{ "type": "text", "text": text }],
            })),
        )
        .await
    }

    pub async fn cancel(&self, session_id: &str) -> Result<()> {
        self.request(
            "cancel",
            Some(serde_json::json!({ "session_id": session_id })),
        )
        .await?;
        Ok(())
    }

    /// Kill the subprocess on shutdown.
    pub async fn shutdown(&self) {
        let mut child = self.child.lock().await;
        let _ = child.kill().await;
    }
}
