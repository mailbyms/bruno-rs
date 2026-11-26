//! WebSocket 命令层（`renderer:ws:*`）。
//!
//! 与 Bruno 前端的契约（对齐 `bruno-electron/src/ipc/network/ws-event-handlers.js`）：
//! - `renderer:ws:start-connection` 的 payload 是
//!   `{ request, collection, environment, runtimeVariables, settings, options }`，
//!   其中 **`request` 是整个 item**（可能带 `draft`），真正的请求细节在
//!   `request.draft.request` / `request.request`；连接 id 用 `item.uid`。
//! - `options.connectOnly` 为假时，原版会把**所有非空消息**排队（不只是选中的那条）；
//!   这里同样处理：连接建立后由 `ws.rs` 的通道顺序发出。
//! - `renderer:ws:queue-message` 只排 `body.ws[selectedMessageIndex]` 这一条。
//! - `renderer:ws:is-connection-active` 返回 `{ success, isActive }`（原版形状）。

use crate::state::AppState;
use bruno_core::interpolate::{interpolate, Vars};
use bruno_core::net;
use bruno_core::ws::{self, WsSpec};
use serde_json::{json, Value};
use std::time::Duration;
use tauri::State;

/// 前端传入的 item 拆成「连接 id / 请求细节 / 设置」
struct ItemParts {
    uid: String,
    request: Value,
    settings: Value,
}

fn item_parts(payload: &Value) -> ItemParts {
    let item = payload.get("request").cloned().unwrap_or_default();
    let draft = item.get("draft");
    let request = draft
        .and_then(|d| d.get("request"))
        .or_else(|| item.get("request"))
        .cloned()
        .unwrap_or_else(|| item.clone());
    let uid = item
        .get("uid")
        .and_then(Value::as_str)
        .or_else(|| request.get("uid").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string();
    let settings = payload
        .get("settings")
        .cloned()
        .filter(|v| v.is_object())
        .or_else(|| draft.and_then(|d| d.get("settings")).cloned())
        .or_else(|| item.get("settings").cloned())
        .unwrap_or_else(|| json!({}));
    ItemParts {
        uid,
        request,
        settings,
    }
}

/// 毫秒数值：前端设置面板里是字符串，也可能是数字
fn ms(raw: Option<&Value>) -> Option<u64> {
    match raw {
        Some(Value::Number(n)) => n
            .as_u64()
            .or_else(|| n.as_f64().map(|f| f.max(0.0) as u64)),
        Some(Value::String(s)) => s
            .trim()
            .parse::<f64>()
            .ok()
            .map(|f| f.max(0.0) as u64),
        _ => None,
    }
}

fn upsert_header(headers: &mut Vec<(String, String)>, name: &str, value: &str) {
    if let Some(slot) = headers.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
        slot.1 = value.to_string();
    } else {
        headers.push((name.to_string(), value.to_string()));
    }
}

/// 把 query 参数并进 URL（WS 面板没有 params 页签，但集合里可能带着）
fn with_query(url: &str, request: &Value, vars: &Vars) -> String {
    let Some(params) = request.get("params").and_then(Value::as_array) else {
        return url.to_string();
    };
    let mut extra: Vec<(String, String)> = Vec::new();
    for param in params {
        if param.get("enabled").and_then(Value::as_bool) == Some(false) {
            continue;
        }
        if param.get("type").and_then(Value::as_str) == Some("path") {
            continue;
        }
        let Some(name) = param.get("name").and_then(Value::as_str) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let value = interpolate(param.get("value").and_then(Value::as_str).unwrap_or(""), vars);
        extra.push((name.to_string(), value));
    }
    if extra.is_empty() {
        return url.to_string();
    }
    let mut out = url.to_string();
    let mut separator = if out.contains('?') { '&' } else { '?' };
    for (name, value) in extra {
        out.push(separator);
        out.push_str(&urlencoding(&name));
        out.push('=');
        out.push_str(&urlencoding(&value));
        separator = '&';
    }
    out
}

fn urlencoding(raw: &str) -> String {
    use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
    utf8_percent_encode(raw, NON_ALPHANUMERIC).to_string()
}

fn send_payload(conn: &ws::WsConnection, content: &str, kind: &str) -> bool {
    if kind == "binary" {
        use base64::Engine as _;
        match base64::engine::general_purpose::STANDARD.decode(content.as_bytes()) {
            Ok(bytes) => conn.send_binary(bytes),
            Err(_) => conn.send_text(content.to_string()),
        }
    } else {
        conn.send_text(content.to_string())
    }
}

#[tauri::command]
pub async fn ws_start_connection(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: Value,
) -> Result<Value, String> {
    let parts = item_parts(&payload);
    if parts.uid.is_empty() {
        return Err("WebSocket 请求缺少 uid".into());
    }
    let collection = payload.get("collection").cloned().unwrap_or_default();
    let environment = payload.get("environment").cloned();
    let runtime = payload.get("runtimeVariables").cloned();
    let collection_uid = collection
        .get("uid")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let vars = net::build_vars(&collection, environment.as_ref(), Some(&parts.request), runtime.as_ref());
    let raw_url = parts.request.get("url").and_then(Value::as_str).unwrap_or("");
    let url = with_query(&interpolate(raw_url, &vars), &parts.request, &vars);
    if url.trim().is_empty() {
        return Err("WebSocket URL 为空".into());
    }

    let mut headers = net::merged_headers(&parts.request, &collection, &vars);
    let auth = net::resolve_auth(parts.request.get("auth"), &vars).await;
    for (name, value) in &auth.headers {
        upsert_header(&mut headers, name, value);
    }
    for message in &auth.errors {
        // 认证不支持时不要冒充 WS 错误事件（会把状态条打成 ERROR），只记录
        eprintln!("[bruno-rs] ws auth: {message}");
    }

    let snapshot = json!({
        "type": "request",
        "url": url,
        "headers": headers
            .iter()
            .map(|(name, value)| json!({ "name": name, "value": value }))
            .collect::<Vec<_>>(),
        "body": parts.request.get("body").cloned().unwrap_or(Value::Null),
    });

    let spec = WsSpec {
        request_id: parts.uid.clone(),
        collection_uid,
        url,
        headers,
        // 前端设置面板写的是毫秒
        timeout: ms(parts.settings.get("timeout")).map(Duration::from_millis),
        keep_alive: ms(parts.settings.get("keepAliveInterval"))
            .filter(|v| *v > 0)
            .map(Duration::from_millis),
        request_snapshot: snapshot,
    };

    state.ws_hub.start(app, spec)?;

    // 原版语义：非 connectOnly 时把「所有内容非空」的消息一次性排队。
    // 通道是无界的，连接建立后按顺序发出，所以现在塞进去即可。
    let connect_only = payload
        .get("options")
        .and_then(|o| o.get("connectOnly"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !connect_only {
        if let Some(messages) = parts
            .request
            .get("body")
            .and_then(|b| b.get("ws"))
            .and_then(Value::as_array)
        {
            for message in messages {
                let content = interpolate(
                    message.get("content").and_then(Value::as_str).unwrap_or(""),
                    &vars,
                );
                if content.is_empty() {
                    continue;
                }
                let kind = message.get("type").and_then(Value::as_str).unwrap_or("json");
                state
                    .ws_hub
                    .with(&parts.uid, |conn| send_payload(conn, &content, kind));
            }
        }
    }

    Ok(json!({ "success": true }))
}

#[tauri::command]
pub fn ws_send_message(
    state: State<'_, AppState>,
    request_id: String,
    _collection_uid: String,
    message: Value,
) -> Result<Value, String> {
    // 前端传的是整条消息（`{name, content, type, selected}`），不是裸字符串；
    // 直接 `to_string()` 会把 JSON 外壳发出去。
    let (content, kind) = match &message {
        Value::String(text) => (text.clone(), "text"),
        Value::Object(_) => (
            message
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            message.get("type").and_then(Value::as_str).unwrap_or("text"),
        ),
        other => (other.to_string(), "text"),
    };
    let sent = state
        .ws_hub
        .with(&request_id, |conn| send_payload(conn, &content, kind))
        .unwrap_or(false);
    Ok(sent_json(sent))
}

/// 单条消息发送：按 `selectedMessageIndex` 取 `body.ws[i].content`，插值后入队
#[tauri::command]
pub fn ws_queue_message(state: State<'_, AppState>, payload: Value) -> Result<Value, String> {
    let item = payload.get("item").cloned().unwrap_or_default();
    let collection = payload.get("collection").cloned().unwrap_or_default();
    let environment = payload.get("environment").cloned();
    let runtime = payload.get("runtimeVariables").cloned();
    let index = payload
        .get("selectedMessageIndex")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;

    let uid = item
        .get("uid")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if uid.is_empty() {
        return Ok(sent_json(false));
    }

    // draft 里改了但没保存时，消息体在 draft.request 下
    let request = item
        .get("draft")
        .and_then(|d| d.get("request"))
        .or_else(|| item.get("request"))
        .cloned()
        .unwrap_or_default();

    let messages = request
        .get("body")
        .and_then(|b| b.get("ws"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let Some(selected) = messages.get(index) else {
        return Ok(json!({ "success": false, "error": "找不到要发送的消息" }));
    };

    let vars = net::build_vars(&collection, environment.as_ref(), Some(&request), runtime.as_ref());
    let content = interpolate(
        selected.get("content").and_then(Value::as_str).unwrap_or(""),
        &vars,
    );
    let kind = selected.get("type").and_then(Value::as_str).unwrap_or("json");

    let sent = state
        .ws_hub
        .with(&uid, |conn| send_payload(conn, &content, kind))
        .unwrap_or(false);
    Ok(sent_json(sent))
}

fn sent_json(sent: bool) -> Value {
    if sent {
        json!({ "success": true })
    } else {
        json!({ "success": false, "error": "WebSocket 连接未就绪" })
    }
}

#[tauri::command]
pub fn ws_close_connection(state: State<'_, AppState>, request_id: String) -> Result<Value, String> {
    // 没有连接时也返回成功：前端「断开」按钮不该为此报错
    let closed = state
        .ws_hub
        .with(&request_id, |conn| {
            conn.close();
        })
        .is_some();
    Ok(json!({ "success": true, "closed": closed }))
}

#[tauri::command]
pub fn ws_connection_status(state: State<'_, AppState>, request_id: String) -> Result<Value, String> {
    let status = state
        .ws_hub
        .with(&request_id, |conn| conn.status_str().to_string())
        .unwrap_or_else(|| ws::WsStatus::Disconnected.as_str().to_string());
    Ok(json!({ "success": true, "status": status }))
}

#[tauri::command]
pub fn ws_is_connection_active(state: State<'_, AppState>, request_id: String) -> Result<Value, String> {
    let is_active = state
        .ws_hub
        .with(&request_id, |conn| {
            matches!(conn.status_str(), "connected" | "connecting")
        })
        .unwrap_or(false);
    Ok(json!({ "success": true, "isActive": is_active }))
}
