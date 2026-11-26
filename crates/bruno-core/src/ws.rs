//! WebSocket 客户端：连接管理、消息收发、事件推送。
//!
//! 与 Bruno 前端的事件契约（`utils/network/ws-event-listeners.js`）：
//! 每个事件 payload 都是 `[requestId, collectionUid, eventData]` 三元组，
//! 前端按 `(requestId, collectionUid, eventData)` 解构。`eventData` 的形状由
//! `wsResponseReceived` reducer 决定，这里逐字对齐，不要「优化」字段名。

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

/// 事件出口：`(事件名, 完整 payload)`。payload 一般已是数组（三元组）。
pub type Emit = Arc<dyn Fn(&'static str, Value) + Send + Sync>;

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 连接状态：前端 `getWsConnectionStatus` 只认这四个值
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsStatus {
    Connecting,
    Connected,
    Disconnecting,
    Disconnected,
}

impl WsStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            WsStatus::Connecting => "connecting",
            WsStatus::Connected => "connected",
            WsStatus::Disconnecting => "disconnecting",
            WsStatus::Disconnected => "disconnected",
        }
    }
}

pub struct WsSpec {
    pub request_id: String,
    pub collection_uid: String,
    pub url: String,
    /// 已插值的请求头（含认证头）
    pub headers: Vec<(String, String)>,
    /// 连接超时；None 用默认 30s
    pub timeout: Option<Duration>,
    /// > 0 时定时发 Ping 保活
    pub keep_alive: Option<Duration>,
    /// `main:ws:request` 的 eventData（前端写进 item.requestSent）
    pub request_snapshot: Value,
}

enum Cmd {
    Text(String),
    Binary(Vec<u8>),
    Close,
}

#[derive(Clone)]
pub struct WsConnection {
    pub status: Arc<Mutex<WsStatus>>,
    cmd: mpsc::UnboundedSender<Cmd>,
}

impl WsConnection {
    pub fn send_text(&self, text: String) -> bool {
        self.cmd.send(Cmd::Text(text)).is_ok()
    }

    pub fn send_binary(&self, bytes: Vec<u8>) -> bool {
        self.cmd.send(Cmd::Binary(bytes)).is_ok()
    }

    pub fn close(&self) {
        let _ = self.cmd.send(Cmd::Close);
    }

    pub fn status_str(&self) -> &'static str {
        self.status.lock().map(|s| s.as_str()).unwrap_or("disconnected")
    }
}

fn hex_dump(bytes: &[u8]) -> String {
    bytes
        .iter()
        .enumerate()
        .map(|(i, b)| {
            if i > 0 && i % 16 == 0 {
                format!("\n{b:02x}")
            } else {
                format!("{b:02x}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// 建连 + 起读写任务。`on_exit` 在连接彻底结束后调用（用于清理注册表）。
pub fn spawn(
    spec: WsSpec,
    emit: Emit,
    on_exit: Arc<dyn Fn() + Send + Sync>,
) -> WsConnection {
    let status = Arc::new(Mutex::new(WsStatus::Connecting));
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();

    let conn = WsConnection {
        status: status.clone(),
        cmd: cmd_tx,
    };

    let rid = spec.request_id.clone();
    let cuid = spec.collection_uid.clone();

    tokio::spawn(async move {
        let payload3 = |data: Value| json!([rid.clone(), cuid.clone(), data]);
        let seq = Arc::new(Mutex::new(0u64));
        let next_seq = {
            let seq = seq.clone();
            move || {
                let mut s = seq.lock().unwrap();
                *s += 1;
                *s
            }
        };

        emit("main:ws:connecting", payload3(json!({ "url": spec.url, "timestamp": now_ms() })));

        // ---- 建连 ----
        let mut request = match spec.url.as_str().into_client_request() {
            Ok(r) => r,
            Err(e) => {
                emit(
                    "main:ws:error",
                    payload3(json!({ "error": format!("无效的 WebSocket URL：{e}"), "timestamp": now_ms(), "seq": next_seq() })),
                );
                *status.lock().unwrap() = WsStatus::Disconnected;
                on_exit();
                return;
            }
        };
        for (name, value) in &spec.headers {
            if name.is_empty() {
                continue;
            }
            // 握手必需头交给 tungstenite 自己生成，用户自定义会破坏握手
            let lower = name.to_ascii_lowercase();
            if matches!(
                lower.as_str(),
                "connection" | "upgrade" | "sec-websocket-key" | "sec-websocket-version" | "sec-websocket-extensions"
            ) {
                continue;
            }
            if let (Ok(hname), Ok(hval)) = (
                http::header::HeaderName::from_bytes(name.as_bytes()),
                http::header::HeaderValue::from_str(value),
            ) {
                request.headers_mut().insert(hname, hval);
            }
        }

        let timeout = spec.timeout.unwrap_or(Duration::from_secs(30));
        let connected = tokio::time::timeout(timeout, tokio_tungstenite::connect_async(request)).await;

        let (stream, response) = match connected {
            Err(_) => {
                emit(
                    "main:ws:error",
                    payload3(json!({
                        "error": format!("连接超时（{}s）", timeout.as_secs()),
                        "timestamp": now_ms(),
                        "seq": next_seq()
                    })),
                );
                *status.lock().unwrap() = WsStatus::Disconnected;
                on_exit();
                return;
            }
            Ok(Err(e)) => {
                emit(
                    "main:ws:error",
                    payload3(json!({ "error": format!("连接失败：{e}"), "timestamp": now_ms(), "seq": next_seq() })),
                );
                *status.lock().unwrap() = WsStatus::Disconnected;
                on_exit();
                return;
            }
            Ok(Ok(v)) => v,
        };

        // 握手响应头（upgrade）
        let mut header_map = serde_json::Map::new();
        for (k, v) in response.headers() {
            if let Ok(text) = v.to_str() {
                header_map.insert(k.as_str().to_string(), json!(text));
            }
        }
        emit(
            "main:ws:upgrade",
            payload3(json!({ "headers": header_map, "timestamp": now_ms(), "seq": next_seq() })),
        );

        *status.lock().unwrap() = WsStatus::Connected;
        emit(
            "main:ws:request",
            payload3(spec.request_snapshot.clone()),
        );
        emit(
            "main:ws:open",
            payload3(json!({ "url": spec.url, "timestamp": now_ms(), "seq": next_seq() })),
        );

        // ---- 收发循环 ----
        let (mut write, mut read) = stream.split();
        let mut cmd_rx = cmd_rx;
        let mut close_emitted = false;
        let mut ticker = spec
            .keep_alive
            .filter(|d| !d.is_zero())
            .map(|d| tokio::time::interval(d));

        loop {
            tokio::select! {
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(Cmd::Text(text)) => {
                            if let Err(e) = write.send(Message::text(text.clone())).await {
                                emit("main:ws:error", payload3(json!({ "error": format!("发送失败：{e}"), "timestamp": now_ms(), "seq": next_seq() })));
                                break;
                            }
                            emit("main:ws:message", payload3(json!({
                                "type": "outgoing", "message": text, "timestamp": now_ms(), "seq": next_seq()
                            })));
                        }
                        Some(Cmd::Binary(bytes)) => {
                            if let Err(e) = write.send(Message::binary(bytes.clone())).await {
                                emit("main:ws:error", payload3(json!({ "error": format!("发送失败：{e}"), "timestamp": now_ms(), "seq": next_seq() })));
                                break;
                            }
                            emit("main:ws:message", payload3(json!({
                                "type": "outgoing",
                                "message": String::from_utf8_lossy(&bytes).to_string(),
                                "messageHexdump": hex_dump(&bytes),
                                "timestamp": now_ms(),
                                "seq": next_seq()
                            })));
                        }
                        Some(Cmd::Close) | None => {
                            *status.lock().unwrap() = WsStatus::Disconnecting;
                            emit("main:ws:disconnecting", payload3(json!({ "timestamp": now_ms() })));
                            let _ = write.close().await;
                            // 主动断开也要让前端收到 close：否则状态条停在 DISCONNECTING
                            emit("main:ws:close", payload3(json!({
                                "code": 1000, "reason": "", "timestamp": now_ms(), "seq": next_seq()
                            })));
                            close_emitted = true;
                            break;
                        }
                    }
                }
                msg = read.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            emit("main:ws:message", payload3(json!({
                                "type": "incoming", "message": text.to_string(), "timestamp": now_ms(), "seq": next_seq()
                            })));
                        }
                        Some(Ok(Message::Binary(bytes))) => {
                            emit("main:ws:message", payload3(json!({
                                "type": "incoming",
                                "message": String::from_utf8_lossy(&bytes).to_string(),
                                "messageHexdump": hex_dump(&bytes),
                                "timestamp": now_ms(),
                                "seq": next_seq()
                            })));
                        }
                        Some(Ok(Message::Close(frame))) => {
                            let (code, reason) = match frame {
                                Some(f) => (u16::from(f.code), f.reason.to_string()),
                                None => (1005u16, String::new()),
                            };
                            emit("main:ws:close", payload3(json!({
                                "code": code, "reason": reason, "timestamp": now_ms(), "seq": next_seq()
                            })));
                            close_emitted = true;
                            break;
                        }
                        Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) | Some(Ok(Message::Frame(_))) => {
                            // tungstenite 自动回 Pong，这里不打扰 UI
                        }
                        Some(Err(e)) => {
                            emit("main:ws:error", payload3(json!({
                                "error": format!("{e}"), "timestamp": now_ms(), "seq": next_seq()
                            })));
                            break;
                        }
                        None => break,
                    }
                }
                _ = async {
                    match ticker.as_mut() {
                        Some(t) => { t.tick().await; }
                        None => std::future::pending::<()>().await,
                    }
                } => {
                    let _ = write.send(Message::Ping(Vec::new().into())).await;
                }
            }
        }

        *status.lock().unwrap() = WsStatus::Disconnected;
        // 服务端先关时前端也要收到一次 close，保证状态条能收尾；
        // 连接异常断开（读流结束/出错）同样补一条
        if !close_emitted {
            emit("main:ws:close", payload3(json!({
                "code": 1006, "reason": "", "timestamp": now_ms(), "seq": next_seq()
            })));
        }
        let _ = cmd_rx.close();
        on_exit();
    });

    conn
}
