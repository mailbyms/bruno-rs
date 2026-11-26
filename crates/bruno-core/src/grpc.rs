//! gRPC 调用驱动：unary / server-streaming / client-streaming / bidi-streaming。
//!
//! 事件契约（`utils/network/grpc-event-listeners.js`）：
//! `grpc:request` / `grpc:response` / `grpc:metadata` / `grpc:status` /
//! `grpc:error` / `grpc:server-end-stream` / `grpc:server-cancel-stream`，
//! payload 一律 `[requestId, collectionUid, eventData]`，`requestId` 就是 item.uid。

use crate::grpc_codec::DynCodec;
use crate::grpc_proto::{self, find_method, normalize_endpoint};
use crate::ws::{now_ms, Emit};
use base64::Engine as _;
use prost_reflect::{DescriptorPool, DynamicMessage, MessageDescriptor};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Endpoint;
use tonic::Request;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrpcStatus {
    Running,
    Ended,
}

impl GrpcStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            GrpcStatus::Running => "running",
            GrpcStatus::Ended => "ended",
        }
    }
}

pub enum Source {
    Proto { files: Vec<PathBuf>, includes: Vec<PathBuf> },
    Reflection,
}

pub struct GrpcSpec {
    pub request_id: String,
    pub collection_uid: String,
    /// 已插值的 `host:port` 或 `http(s)://host:port`
    pub url: String,
    pub method_path: String,
    pub method_type: String,
    pub metadata: Vec<(String, String)>,
    /// body.grpc[i].content（JSON 字符串）
    pub messages: Vec<String>,
    pub source: Source,
    pub timeout: Option<Duration>,
    /// `grpc:request` 的 eventData
    pub request_snapshot: Value,
}

enum Cmd {
    Message(String),
    End,
}

pub struct GrpcCall {
    pub status: Arc<Mutex<GrpcStatus>>,
    cmd: mpsc::UnboundedSender<Cmd>,
    handle: tokio::task::JoinHandle<()>,
}

impl GrpcCall {
    pub fn send_message(&self, text: String) -> bool {
        self.cmd.send(Cmd::Message(text)).is_ok()
    }

    pub fn end(&self) -> bool {
        self.cmd.send(Cmd::End).is_ok()
    }

    /// 取消：直接 abort 任务，tonic 的流被 drop 时会发 RST_STREAM
    pub fn cancel(&self) {
        self.handle.abort();
    }

    pub fn is_active(&self) -> bool {
        !self.handle.is_finished()
    }
}

fn valid_metadata_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    !lower.is_empty()
        && !lower.starts_with("grpc-")
        && !matches!(lower.as_str(), "te" | "content-type" | "host" | "connection" | "user-agent")
}

fn apply_metadata<T>(request: &mut Request<T>, metadata: &[(String, String)]) {
    for (k, v) in metadata {
        if !valid_metadata_key(k) {
            continue;
        }
        if let (Ok(key), Ok(value)) = (
            tonic::metadata::MetadataKey::from_bytes(k.to_ascii_lowercase().as_bytes()),
            tonic::metadata::MetadataValue::try_from(v.as_str()),
        ) {
            request.metadata_mut().insert(key, value);
        }
    }
}

fn metadata_to_json(map: &tonic::metadata::MetadataMap) -> Value {
    let mut out = serde_json::Map::new();
    for entry in map.iter() {
        match entry {
            tonic::metadata::KeyAndValueRef::Ascii(key, value) => {
                if let Ok(text) = value.to_str() {
                    out.insert(key.as_str().to_string(), json!(text));
                }
            }
            tonic::metadata::KeyAndValueRef::Binary(key, value) => {
                out.insert(key.as_str().to_string(), json!(base64::engine::general_purpose::STANDARD.encode(value.as_encoded_bytes())));
            }
        }
    }
    Value::Object(out)
}

fn status_to_json(status: &tonic::Status) -> Value {
    json!({
        "code": status.code() as i32,
        "details": status.message(),
        "metadata": metadata_to_json(status.metadata()),
    })
}

pub fn spawn(spec: GrpcSpec, emit: Emit, on_exit: Arc<dyn Fn() + Send + Sync>) -> GrpcCall {
    let status = Arc::new(Mutex::new(GrpcStatus::Running));
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let status_clone = status.clone();
    let handle = tokio::spawn(async move {
        run(spec, emit, on_exit, cmd_rx).await;
        *status_clone.lock().unwrap() = GrpcStatus::Ended;
    });
    GrpcCall {
        status,
        cmd: cmd_tx,
        handle,
    }
}

async fn run(spec: GrpcSpec, emit: Emit, on_exit: Arc<dyn Fn() + Send + Sync>, cmd_rx: mpsc::UnboundedReceiver<Cmd>) {
    let rid = spec.request_id.clone();
    let cuid = spec.collection_uid.clone();
    let p3 = {
        let rid = rid.clone();
        let cuid = cuid.clone();
        move |data: Value| json!([rid.clone(), cuid.clone(), data])
    };
    let finish = |emit: &Emit, p3: P3<'_>| {
        emit("grpc:server-end-stream", p3(json!({ "timestamp": now_ms() })));
    };

    // 解析描述符（失败也走 grpc:error，让前端有反馈）
    let pool = match resolve_pool(&spec).await {
        Ok(p) => p,
        Err(e) => {
            emit("grpc:error", p3(json!({ "error": { "code": 2, "details": e.to_string() } })));
            finish(&emit, &p3);
            on_exit();
            return;
        }
    };

    let Some(method) = find_method(&pool, &spec.method_path) else {
        emit(
            "grpc:error",
            p3(json!({ "error": { "code": 12, "details": format!("方法 {} 不在描述符里", spec.method_path) } })),
        );
        finish(&emit, &p3);
        on_exit();
        return;
    };

    let (client_streaming, server_streaming) = (method.is_client_streaming(), method.is_server_streaming());
    let input_desc = method.input();
    let output_desc = method.output();

    emit("grpc:request", p3(spec.request_snapshot.clone()));

    let endpoint = normalize_endpoint(&spec.url);
    let channel = match Endpoint::from_shared(endpoint.clone()) {
        Ok(ep) => match ep.connect_timeout(Duration::from_secs(10)).connect().await {
            Ok(c) => c,
            Err(e) => {
                emit("grpc:error", p3(json!({ "error": { "code": 14, "details": format!("连接 {endpoint} 失败：{e}") } })));
                finish(&emit, &p3);
                on_exit();
                return;
            }
        },
        Err(e) => {
            emit(
                "grpc:error",
                p3(json!({ "error": { "code": 3, "details": format!("无效的 gRPC 地址 {endpoint}：{e}") } })),
            );
            finish(&emit, &p3);
            on_exit();
            return;
        }
    };

    let mut grpc = tonic::client::Grpc::new(channel);
    if let Err(e) = grpc.ready().await {
        emit("grpc:error", p3(json!({ "error": { "code": 14, "details": format!("通道未就绪：{e}") } })));
        finish(&emit, &p3);
        on_exit();
        return;
    }

    let path = match format!("/{}", spec.method_path.trim_start_matches('/')).parse::<http::uri::PathAndQuery>() {
        Ok(p) => p,
        Err(e) => {
            emit("grpc:error", p3(json!({ "error": { "code": 3, "details": format!("方法路径非法：{e}") } })));
            finish(&emit, &p3);
            on_exit();
            return;
        }
    };

    let codec = || DynCodec {
        output: output_desc.clone(),
    };

    match (client_streaming, server_streaming) {
        (false, false) => unary(&mut grpc, path, codec(), &spec, &input_desc, &emit, &p3).await,
        (false, true) => server_streaming_call(&mut grpc, path, codec(), &spec, &input_desc, &emit, &p3).await,
        _ => streaming_call(&mut grpc, path, codec(), &spec, &input_desc, &emit, &p3, cmd_rx, server_streaming).await,
    }

    finish(&emit, &p3);
    on_exit();
}

async fn resolve_pool(spec: &GrpcSpec) -> anyhow::Result<Arc<DescriptorPool>> {
    match &spec.source {
        Source::Proto { files, includes } => Ok(Arc::new(grpc_proto::compile(files, includes)?)),
        Source::Reflection => {
            let endpoint = normalize_endpoint(&spec.url);
            let pool = crate::grpc_reflection::load_pool(endpoint, spec.metadata.clone(), Duration::from_secs(20)).await?;
            Ok(Arc::new(pool))
        }
    }
}

fn build_message(desc: &MessageDescriptor, json_text: &str) -> anyhow::Result<DynamicMessage> {
    let text = if json_text.trim().is_empty() { "{}" } else { json_text };
    let mut de = serde_json::Deserializer::from_str(text);
    DynamicMessage::deserialize(desc.clone(), &mut de).map_err(|e| anyhow::anyhow!("请求消息不是合法 JSON：{e}"))
}

fn message_to_json(msg: &DynamicMessage) -> Value {
    serde_json::to_value(msg).unwrap_or_else(|e| json!({ "error": format!("序列化响应失败：{e}") }))
}

type P3<'a> = &'a (dyn Fn(Value) -> Value + Sync);

async fn unary(
    grpc: &mut tonic::client::Grpc<tonic::transport::Channel>,
    path: http::uri::PathAndQuery,
    codec: DynCodec,
    spec: &GrpcSpec,
    input: &MessageDescriptor,
    emit: &Emit,
    p3: P3<'_>,
) {
    let text = spec.messages.first().cloned().unwrap_or_default();
    let msg = match build_message(input, &text) {
        Ok(m) => m,
        Err(e) => {
            emit("grpc:error", p3(json!({ "error": { "code": 3, "details": e.to_string() } })));
            return;
        }
    };
    let mut request = Request::new(msg);
    apply_metadata(&mut request, &spec.metadata);

    let fut = grpc.unary(request, path, codec);
    let result = match spec.timeout {
        Some(t) => match tokio::time::timeout(t, fut).await {
            Ok(r) => r,
            Err(_) => {
                emit(
                    "grpc:error",
                    p3(json!({ "error": { "code": 4, "details": format!("请求超时（{}s）", t.as_secs()) } })),
                );
                return;
            }
        },
        None => fut.await,
    };

    match result {
        Ok(response) => {
            let metadata = metadata_to_json(response.metadata());
            emit("grpc:metadata", p3(json!({ "metadata": metadata })));
            let body: DynamicMessage = response.into_inner();
            let value = message_to_json(&body);
            emit("grpc:response", p3(json!({ "res": value, "error": null })));
            emit("grpc:status", p3(json!({ "status": { "code": 0, "details": "", "metadata": {} } })));
        }
        Err(status) => {
            emit("grpc:error", p3(json!({ "error": status_to_json(&status) })));
            emit("grpc:status", p3(json!({ "status": status_to_json(&status) })));
        }
    }
}

async fn server_streaming_call(
    grpc: &mut tonic::client::Grpc<tonic::transport::Channel>,
    path: http::uri::PathAndQuery,
    codec: DynCodec,
    spec: &GrpcSpec,
    input: &MessageDescriptor,
    emit: &Emit,
    p3: P3<'_>,
) {
    let text = spec.messages.first().cloned().unwrap_or_default();
    let msg = match build_message(input, &text) {
        Ok(m) => m,
        Err(e) => {
            emit("grpc:error", p3(json!({ "error": { "code": 3, "details": e.to_string() } })));
            return;
        }
    };
    let mut request = Request::new(msg);
    apply_metadata(&mut request, &spec.metadata);

    let response = match grpc.server_streaming(request, path, codec).await {
        Ok(r) => r,
        Err(status) => {
            emit("grpc:error", p3(json!({ "error": status_to_json(&status) })));
            emit("grpc:status", p3(json!({ "status": status_to_json(&status) })));
            return;
        }
    };
    let metadata = metadata_to_json(response.metadata());
    emit("grpc:metadata", p3(json!({ "metadata": metadata })));
    let mut stream = response.into_inner();
    loop {
        match stream.message().await {
            Ok(Some(msg)) => {
                let value = message_to_json(&msg);
                emit("grpc:response", p3(json!({ "res": value, "error": null })));
            }
            Ok(None) => {
                emit("grpc:status", p3(json!({ "status": { "code": 0, "details": "", "metadata": {} } })));
                break;
            }
            Err(status) => {
                emit("grpc:error", p3(json!({ "error": status_to_json(&status) })));
                emit("grpc:status", p3(json!({ "status": status_to_json(&status) })));
                break;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn streaming_call(
    grpc: &mut tonic::client::Grpc<tonic::transport::Channel>,
    path: http::uri::PathAndQuery,
    codec: DynCodec,
    spec: &GrpcSpec,
    input: &MessageDescriptor,
    emit: &Emit,
    p3: P3<'_>,
    cmd_rx: mpsc::UnboundedReceiver<Cmd>,
    server_streaming: bool,
) {
    let mut initial: Vec<DynamicMessage> = Vec::new();
    for text in &spec.messages {
        match build_message(input, text) {
            Ok(m) => initial.push(m),
            Err(e) => {
                emit("grpc:error", p3(json!({ "error": { "code": 3, "details": e.to_string() } })));
                return;
            }
        }
    }

    let (tx, rx) = mpsc::channel::<DynamicMessage>(16);
    let mut tx = Some(tx);
    for msg in initial {
        if let Some(sender) = tx.as_ref() {
            if sender.send(msg).await.is_err() {
                break;
            }
        }
    }
    let stream = ReceiverStream::new(rx);
    let mut request = Request::new(stream);
    apply_metadata(&mut request, &spec.metadata);

    let response = match grpc.streaming(request, path, codec).await {
        Ok(r) => r,
        Err(status) => {
            emit("grpc:error", p3(json!({ "error": status_to_json(&status) })));
            emit("grpc:status", p3(json!({ "status": status_to_json(&status) })));
            return;
        }
    };
    let metadata = metadata_to_json(response.metadata());
    emit("grpc:metadata", p3(json!({ "metadata": metadata })));
    let mut inbound = response.into_inner();
    let mut cmd_rx = cmd_rx;
    let mut closed = false;

    loop {
        tokio::select! {
            cmd = cmd_rx.recv(), if !closed => {
                match cmd {
                    Some(Cmd::Message(text)) => {
                        match build_message(input, &text) {
                            Ok(msg) => {
                                let sent = match tx.as_ref() {
                                    Some(sender) => sender.send(msg).await.is_ok(),
                                    None => false,
                                };
                                if sent {
                                    emit("grpc:message", p3(json!({ "message": text, "timestamp": now_ms() })));
                                } else {
                                    closed = true;
                                    tx = None;
                                }
                            }
                            Err(e) => {
                                emit("grpc:error", p3(json!({ "error": { "code": 3, "details": e.to_string() } })));
                            }
                        }
                    }
                    Some(Cmd::End) | None => {
                        closed = true;
                        tx = None;
                    }
                }
            }
            incoming = inbound.message() => {
                match incoming {
                    Ok(Some(msg)) => {
                        let value = message_to_json(&msg);
                        emit("grpc:response", p3(json!({ "res": value, "error": null })));
                        if !server_streaming {
                            emit("grpc:status", p3(json!({ "status": { "code": 0, "details": "", "metadata": {} } })));
                            return;
                        }
                    }
                    Ok(None) => {
                        emit("grpc:status", p3(json!({ "status": { "code": 0, "details": "", "metadata": {} } })));
                        return;
                    }
                    Err(status) => {
                        emit("grpc:error", p3(json!({ "error": status_to_json(&status) })));
                        emit("grpc:status", p3(json!({ "status": status_to_json(&status) })));
                        return;
                    }
                }
            }
            else => {
                emit("grpc:status", p3(json!({ "status": { "code": 0, "details": "", "metadata": {} } })));
                return;
            }
        }
    }
}
