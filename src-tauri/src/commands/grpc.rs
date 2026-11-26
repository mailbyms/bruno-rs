//! gRPC 命令层（`grpc:*`）。
//!
//! 与 Bruno 前端的契约：
//! - `grpc:start-connection` payload `{ request, collection, environment, runtimeVariables }`
//! - `grpc:send-message` 三个位置参数 `(itemUid, collectionUid, message)`
//! - `grpc:cancel` / `grpc:end` 位置参数 `requestId`；`grpc:cancel-request` / `grpc:end-request`
//!   是 `{ requestId }`（两套入口前端都在用，都实现）
//! - `grpc:load-methods-proto` / `grpc:load-methods-reflection` 返回 `{ methods, error }`，
//!   前端把 `error` 当异常对象用，取不到就 `console.error`，所以给字符串即可
//! - `grpc:generate-sample-message` 返回 `{ success, message }`，`message` 必须是**字符串**
//!   （前端把它塞进 `body.grpc[i].content`，CodeMirror 只接受字符串）
//! - `grpc:generate-grpcurl` 返回命令字符串

use crate::state::AppState;
use bruno_core::grpc::{GrpcSpec, Source};
use bruno_core::grpc_proto::{self, find_method, methods_from_pool, normalize_endpoint};
use bruno_core::grpc_reflection;
use bruno_core::interpolate::{interpolate, Vars};
use bruno_core::net;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::State;

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

/// 前端传入的 item 拆成「连接 id / 请求细节」。
/// `grpc:start-connection` 的 payload.request 是整个 item（可能带 `draft`），
/// 请求细节在 `item.draft.request` / `item.request`，连接 id 用 `item.uid`。
fn item_parts(payload: &Value) -> (String, Value) {
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
    (uid, request)
}

/// 集合目录：树 payload / Redux 里的集合都带 pathname
fn collection_dir(collection: &Value) -> PathBuf {
    let raw = collection
        .get("pathname")
        .or_else(|| collection.get("path"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let raw = raw.strip_prefix(r"\\?\").unwrap_or(raw);
    PathBuf::from(raw)
}

fn resolve_proto(path: &Path, collection_dir: &Path) -> PathBuf {
    let candidate = PathBuf::from(path);
    if candidate.is_absolute() {
        candidate
    } else {
        collection_dir.join(candidate)
    }
}

fn request_url(request: &Value, vars: &Vars) -> String {
    interpolate(request.get("url").and_then(Value::as_str).unwrap_or(""), vars)
        .trim()
        .to_string()
}

/// 取 gRPC 元数据（= 前端 Headers 页签的请求头）+ 认证头
async fn metadata_for(request: &Value, collection: &Value, vars: &Vars) -> Vec<(String, String)> {
    let mut headers = net::merged_headers(request, collection, vars);
    let auth = net::resolve_auth(request.get("auth"), vars).await;
    for (name, value) in &auth.headers {
        upsert_header(&mut headers, name, value);
    }
    for message in &auth.errors {
        eprintln!("[bruno-rs] grpc auth: {message}");
    }
    if !auth.query.is_empty() {
        eprintln!("[bruno-rs] grpc auth: gRPC 不支持 query 认证参数，已忽略");
    }
    headers
}

fn messages_of(request: &Value) -> Vec<String> {
    request
        .get("body")
        .and_then(|b| b.get("grpc"))
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|m| {
                    m.get("content")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string()
                })
                .collect()
        })
        .unwrap_or_default()
}

#[tauri::command]
pub async fn grpc_start_connection(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: Value,
) -> Result<Value, String> {
    let (request_id, request) = item_parts(&payload);
    if request_id.is_empty() {
        return Err("gRPC 请求缺少 uid".into());
    }
    let collection = payload.get("collection").cloned().unwrap_or_default();
    let environment = payload.get("environment").cloned();
    let runtime = payload.get("runtimeVariables").cloned();
    let settings = payload.get("settings").cloned().unwrap_or_else(|| json!({}));
    let collection_uid = collection
        .get("uid")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let vars = net::build_vars(&collection, environment.as_ref(), Some(&request), runtime.as_ref());
    let url = request_url(&request, &vars);
    if url.is_empty() {
        return Err("gRPC 地址为空".into());
    }
    let method_path = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if method_path.is_empty() {
        return Err("还没有选择 gRPC 方法".into());
    }
    let method_type = request
        .get("methodType")
        .or_else(|| request.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("unary")
        .to_string();

    let metadata = metadata_for(&request, &collection, &vars).await;
    let dir = collection_dir(&collection);

    let proto_path = request
        .get("protoPath")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let source = if proto_path.is_empty() {
        Source::Reflection
    } else {
        let file = resolve_proto(Path::new(&proto_path), &dir);
        if !file.is_file() {
            return Err(format!("proto 文件不存在：{}", file.display()));
        }
        // 集合里登记的 protoFiles / importPaths 都要进 include 路径
        let mut includes = grpc_proto::proto_includes(&dir, &collection);
        if let Some(parent) = file.parent() {
            let parent = parent.to_path_buf();
            if !includes.contains(&parent) {
                includes.push(parent);
            }
        }
        Source::Proto {
            files: vec![file],
            includes,
        }
    };

    let snapshot = json!({
        "url": url,
        "method": method_path,
        "methodType": method_type,
        "headers": metadata
            .iter()
            .map(|(name, value)| json!({ "name": name, "value": value }))
            .collect::<Vec<_>>(),
        "body": request.get("body").cloned().unwrap_or(Value::Null),
    });

    let timeout = ms(settings.get("timeout"))
        .or_else(|| ms(request.get("settings").and_then(|s| s.get("timeout"))))
        .filter(|v| *v > 0)
        .map(Duration::from_millis);

    let spec = GrpcSpec {
        request_id: request_id.clone(),
        collection_uid,
        url,
        method_path,
        method_type,
        metadata,
        messages: messages_of(&request),
        source,
        timeout,
        request_snapshot: snapshot,
    };

    state.grpc_hub.start(app, spec)?;
    Ok(json!({ "success": true }))
}

#[tauri::command]
pub fn grpc_send_message(
    state: State<'_, AppState>,
    request_id: String,
    _collection_uid: String,
    message: Value,
) -> Result<Value, String> {
    let text = match message {
        Value::String(text) => text,
        other => other.to_string(),
    };
    let sent = state
        .grpc_hub
        .with(&request_id, |call| call.send_message(text))
        .unwrap_or(false);
    if sent {
        Ok(json!({ "success": true }))
    } else {
        Ok(json!({ "success": false, "error": "gRPC 调用未就绪" }))
    }
}

#[tauri::command]
pub fn grpc_cancel(state: State<'_, AppState>, request_id: String) -> Result<Value, String> {
    let cancelled = state
        .grpc_hub
        .with(&request_id, |call| call.cancel())
        .is_some();
    Ok(json!({ "success": true, "cancelled": cancelled }))
}

#[tauri::command]
pub fn grpc_end(state: State<'_, AppState>, request_id: String) -> Result<Value, String> {
    let ended = state
        .grpc_hub
        .with(&request_id, |call| call.end())
        .unwrap_or(false);
    Ok(json!({ "success": ended }))
}

/// 前端两套入口之一：`grpc:cancel-request` / `grpc:end-request`
#[tauri::command]
pub fn grpc_cancel_request(state: State<'_, AppState>, request_id: String) -> Result<Value, String> {
    grpc_cancel(state, request_id)
}

#[tauri::command]
pub fn grpc_end_request(state: State<'_, AppState>, request_id: String) -> Result<Value, String> {
    grpc_end(state, request_id)
}

#[tauri::command]
pub fn grpc_is_connection_active(
    state: State<'_, AppState>,
    connection_id: String,
) -> Result<Value, String> {
    let active = state
        .grpc_hub
        .with(&connection_id, |call| call.is_active())
        .unwrap_or(false);
    Ok(json!({ "success": true, "isActive": active }))
}

#[tauri::command]
pub fn grpc_load_methods_proto(payload: Value) -> Result<Value, String> {
    let file_path = payload
        .get("filePath")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let collection = payload.get("collection").cloned().unwrap_or_default();
    let dir = collection_dir(&collection);
    let file = resolve_proto(Path::new(&file_path), &dir);
    let includes = grpc_proto::proto_includes(&dir, &collection);

    let result = grpc_proto::compile(&[file], &includes).map(|pool| methods_from_pool(&pool));
    Ok(match result {
        Ok(methods) => json!({ "methods": methods, "error": Value::Null }),
        Err(e) => json!({ "methods": [], "error": e.to_string() }),
    })
}

#[tauri::command]
pub async fn grpc_load_methods_reflection(payload: Value) -> Result<Value, String> {
    let (_, request) = item_parts(&payload);
    let collection = payload.get("collection").cloned().unwrap_or_default();
    let environment = payload.get("environment").cloned();
    let runtime = payload.get("runtimeVariables").cloned();

    let vars = net::build_vars(&collection, environment.as_ref(), Some(&request), runtime.as_ref());
    let url = request_url(&request, &vars);
    if url.is_empty() {
        return Ok(json!({ "methods": [], "error": "gRPC 地址为空" }));
    }
    let metadata = metadata_for(&request, &collection, &vars).await;
    let endpoint = normalize_endpoint(&url);

    match grpc_reflection::load_pool(endpoint, metadata, Duration::from_secs(20)).await {
        Ok(pool) => Ok(json!({ "methods": methods_from_pool(&pool), "error": Value::Null })),
        Err(e) => Ok(json!({ "methods": [], "error": e.to_string() })),
    }
}

#[tauri::command]
pub fn grpc_generate_sample_message(payload: Value) -> Result<Value, String> {
    let existing = payload
        .get("existingMessage")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let options = payload.get("options").cloned().unwrap_or_default();
    let array_size = options
        .get("arraySize")
        .and_then(Value::as_u64)
        .unwrap_or(1) as usize;
    let method_meta = options.get("methodMetadata").cloned();

    let sample = grpc_proto::sample_message(existing.as_deref(), method_meta.as_ref(), array_size);
    let text = serde_json::to_string_pretty(&sample).unwrap_or_else(|_| "{}".to_string());
    Ok(json!({ "success": true, "message": text }))
}

#[tauri::command]
pub fn grpc_generate_grpcurl(payload: Value) -> Result<Value, String> {
    let (_, request) = item_parts(&payload);
    let collection = payload.get("collection").cloned().unwrap_or_default();
    let environment = payload.get("environment").cloned();
    let runtime = payload.get("runtimeVariables").cloned();

    let vars = net::build_vars(&collection, environment.as_ref(), Some(&request), runtime.as_ref());
    let url = request_url(&request, &vars);
    let method_path = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if url.is_empty() || method_path.is_empty() {
        return Err("需要地址与方法才能生成 grpcurl 命令".into());
    }

    let host = normalize_endpoint(&url)
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .to_string();

    let mut parts: Vec<String> = vec!["grpcurl".into()];
    if !normalize_endpoint(&url).starts_with("https://") {
        parts.push("-plaintext".into());
    }
    let proto_path = request
        .get("protoPath")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if !proto_path.is_empty() {
        parts.push(format!("-proto \"{}\"", proto_path));
        let dir = collection_dir(&collection);
        parts.push(format!("-import-path \"{}\"", dir.display()));
    }
    for (name, value) in net::merged_headers(&request, &collection, &vars) {
        parts.push(format!("-H '{}: {}'", name, value.replace('\'', "")));
    }
    let body = messages_of(&request).into_iter().next().unwrap_or_default();
    if !body.trim().is_empty() {
        parts.push(format!("-d '{}'", body.replace('\'', "")));
    }
    parts.push(host);
    parts.push(format!("/{}", method_path.trim_start_matches('/')));

    Ok(json!(parts.join(" ")))
}

/// 给「方法是否在描述符里」这类诊断用：`grpc:load-methods-*` 之外没有前端入口
#[tauri::command]
pub fn grpc_has_method(payload: Value) -> Result<Value, String> {
    let file_path = payload
        .get("filePath")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let method_path = payload
        .get("methodPath")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let collection = payload.get("collection").cloned().unwrap_or_default();
    let dir = collection_dir(&collection);
    let file = resolve_proto(Path::new(&file_path), &dir);
    let includes = grpc_proto::proto_includes(&dir, &collection);
    let pool = grpc_proto::compile(&[file], &includes).map_err(|e| e.to_string())?;
    Ok(json!({ "found": find_method(&pool, &method_path).is_some() }))
}
