//! WS / gRPC 子系统测试：
//! - WebSocket 真连本地 echo 服务，校验 `main:ws:*` 事件序列与消息回显；
//! - gRPC 覆盖 proto 编译 / 方法枚举 / 示例消息 / 不可达地址的错误事件；
//! - WS / gRPC 请求的保存 → 重新加载闭环（`websocket:` / `grpc:` 段）。

use bruno_core::collection::load_collection;
use bruno_core::grpc::{self, GrpcSpec, Source};
use bruno_core::grpc_proto::{self, find_method, methods_from_pool, sample_message};
use bruno_core::persist::save_request_file;
use bruno_core::ws::{self, WsSpec};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

/// 事件收集器：`(事件名, payload)`，测试里按顺序断言
type Events = Arc<Mutex<Vec<(String, Value)>>>;

fn recorder() -> (Events, ws::Emit) {
    let events: Events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let emit: ws::Emit = Arc::new(move |name: &'static str, payload: Value| {
        sink.lock().unwrap().push((name.to_string(), payload));
    });
    (events, emit)
}

fn wait_for(events: &Events, name: &str) -> Option<Value> {
    events
        .lock()
        .unwrap()
        .iter()
        .find(|(event, _)| event == name)
        .map(|(_, payload)| payload.clone())
}

fn payload_data(payload: &Value) -> Value {
    // payload 是 [requestId, collectionUid, eventData] 三元组
    payload
        .as_array()
        .and_then(|a| a.get(2))
        .cloned()
        .unwrap_or(Value::Null)
}

async fn wait_for_event(events: &Events, name: &str, timeout: Duration) -> Option<Value> {
    let started = std::time::Instant::now();
    while started.elapsed() < timeout {
        if let Some(value) = wait_for(events, name) {
            return Some(value);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    None
}

/// 最小 WS echo 服务：收到文本原样发回，收到 Close 就关
async fn spawn_echo_server() -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else {
                    return;
                };
                while let Some(Ok(message)) = socket.next().await {
                    match message {
                        Message::Text(text) => {
                            let _ = socket.send(Message::Text(format!("echo:{text}").into())).await;
                        }
                        Message::Close(_) => break,
                        _ => {}
                    }
                }
            });
        }
    });
    addr
}

fn ws_spec(url: String, request_id: &str) -> WsSpec {
    WsSpec {
        request_id: request_id.into(),
        collection_uid: "coll".into(),
        headers: vec![("x-client".into(), "bruno-rs".into())],
        timeout: Some(Duration::from_secs(5)),
        keep_alive: None,
        request_snapshot: json!({ "url": url.clone(), "headers": [] }),
        url,
    }
}

#[tokio::test]
async fn websocket_streams_open_message_and_close() {
    let addr = spawn_echo_server().await;
    let (events, emit) = recorder();
    let exited = Arc::new(Mutex::new(false));
    let on_exit = {
        let flag = exited.clone();
        Arc::new(move || {
            *flag.lock().unwrap() = true;
        })
    };

    let conn = ws::spawn(ws_spec(format!("ws://{addr}"), "req-1"), emit, on_exit);
    let open = wait_for_event(&events, "main:ws:open", Duration::from_secs(5))
        .await
        .expect("没有收到 main:ws:open");
    assert_eq!(payload_data(&open)["url"], json!(format!("ws://{addr}")));
    assert_eq!(conn.status_str(), "connected");

    // 连接前的 upgrade 事件要带握手响应头
    let upgrade = wait_for(&events, "main:ws:upgrade").expect("缺少 main:ws:upgrade");
    assert!(payload_data(&upgrade)["headers"].is_object());

    assert!(conn.send_text("hi".into()));
    let mut echoed = None;
    for _ in 0..250 {
        if let Some(value) = events
            .lock()
            .unwrap()
            .iter()
            .find(|(name, payload)| {
                name == "main:ws:message"
                    && payload_data(payload)["type"] == json!("incoming")
            })
            .map(|(_, payload)| payload_data(payload))
        {
            echoed = Some(value);
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let echoed = echoed.expect("没有收到服务端消息");
    assert_eq!(echoed["message"], json!("echo:hi"));

    // 发出的消息也要在事件流里留一条 outgoing
    let outgoing = events
        .lock()
        .unwrap()
        .iter()
        .any(|(name, payload)| name == "main:ws:message" && payload_data(payload)["type"] == json!("outgoing"));
    assert!(outgoing, "缺少 outgoing 消息事件");

    conn.close();
    wait_for_event(&events, "main:ws:close", Duration::from_secs(5))
        .await
        .expect("没有收到 main:ws:close");
    assert_eq!(conn.status_str(), "disconnected");
    for _ in 0..100 {
        if *exited.lock().unwrap() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(*exited.lock().unwrap(), "连接结束后没有调用 on_exit");
}

#[tokio::test]
async fn websocket_reports_connection_failure() {
    // 127.0.0.1:1 一般无人监听：应当拿到 error 事件而不是挂住
    let (events, emit) = recorder();
    let on_exit = Arc::new(|| {});
    let conn = ws::spawn(ws_spec("ws://127.0.0.1:1".into(), "req-2"), emit, on_exit);
    let error = wait_for_event(&events, "main:ws:error", Duration::from_secs(10))
        .await
        .expect("连接失败没有 main:ws:error");
    let data = payload_data(&error);
    assert!(data["error"].as_str().unwrap_or("").contains("连接"));
    assert_eq!(conn.status_str(), "disconnected");
}

fn protocols_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/protocols")
}

#[test]
fn grpc_proto_lists_methods_and_samples() {
    let dir = protocols_dir();
    let pool = grpc_proto::compile(&[dir.join("echo.proto")], &[dir.clone()]).expect("proto 编译失败");
    let methods = methods_from_pool(&pool);
    let paths: Vec<String> = methods
        .iter()
        .filter_map(|m| m.get("path").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    assert!(paths.contains(&"/echo.Echo/Unary".to_string()), "缺少 Unary：{paths:?}");
    assert!(paths.contains(&"/echo.Echo/Stream".to_string()));

    let unary = methods
        .iter()
        .find(|m| m.get("path").and_then(|v| v.as_str()) == Some("/echo.Echo/Unary"))
        .cloned()
        .unwrap();
    assert_eq!(unary["type"], json!("unary"));
    assert_eq!(unary["serverStreaming"], json!(false));

    // 示例消息：按 requestFields 生成，已有内容只补缺失键
    let sample = sample_message(Some(r#"{"message":"keep"}"#), Some(&unary), 2);
    assert_eq!(sample["message"], json!("keep"));
    assert_eq!(sample["count"], json!(0));

    assert!(find_method(&pool, "echo.Echo/Unary").is_some());
    assert!(find_method(&pool, "/echo.Echo/Missing").is_none());
}

#[test]
fn grpc_sample_variants_for_repeated_fields() {
    let dir = protocols_dir();
    let pool = grpc_proto::compile(&[dir.join("echo.proto")], &[dir]).unwrap();
    let methods = methods_from_pool(&pool);
    let unary = methods
        .iter()
        .find(|m| m.get("path").and_then(|v| v.as_str()) == Some("/echo.Echo/Unary"))
        .cloned()
        .unwrap();
    let sample = sample_message(None, Some(&unary), 1);
    assert_eq!(sample["message"], json!(""));
}

#[tokio::test]
async fn grpc_unreachable_endpoint_emits_error() {
    let (events, emit) = recorder();
    let on_exit = Arc::new(|| {});
    let dir = protocols_dir();
    let spec = GrpcSpec {
        request_id: "grpc-1".into(),
        collection_uid: "coll".into(),
        url: "127.0.0.1:1".into(),
        method_path: "/echo.Echo/Unary".into(),
        method_type: "unary".into(),
        metadata: vec![],
        messages: vec![r#"{"message":"hi"}"#.into()],
        source: Source::Proto {
            files: vec![dir.join("echo.proto")],
            includes: vec![dir],
        },
        timeout: Some(Duration::from_secs(3)),
        request_snapshot: json!({ "url": "127.0.0.1:1", "method": "/echo.Echo/Unary" }),
    };
    let _call = grpc::spawn(spec, emit, on_exit);

    let error = wait_for_event(&events, "grpc:error", Duration::from_secs(20))
        .await
        .expect("gRPC 不可达没有 grpc:error");
    let data = payload_data(&error);
    // code 14 = UNAVAILABLE
    assert_eq!(data["error"]["code"], json!(14), "非预期错误：{data}");
    wait_for_event(&events, "grpc:server-end-stream", Duration::from_secs(5))
        .await
        .expect("结束时应发 grpc:server-end-stream");
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bruno-rs-protocols-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn saves_and_reloads_websocket_and_grpc_requests() {
    let dir = temp_dir("roundtrip");
    std::fs::write(
        dir.join("opencollection.yml"),
        "opencollection: \"1.0.0\"\ninfo:\n  name: Protocols\n",
    )
    .unwrap();

    let ws_item = json!({
        "name": "WS Echo",
        "type": "ws-request",
        "seq": 1,
        "request": {
            "url": "ws://127.0.0.1:18082",
            "headers": [{ "name": "x-client", "value": "bruno-rs", "enabled": true }],
            "body": {
                "mode": "ws",
                "ws": [
                    { "name": "ping", "type": "json", "content": "{\"a\":1}", "selected": true },
                    { "name": "second", "type": "text", "content": "plain" }
                ]
            },
            "auth": { "mode": "none" },
            "vars": { "req": [] },
            "script": { "req": "console.log('ws')", "res": "" },
            "tests": "test('ok', () => expect(1).to.equal(1))"
        }
    });
    save_request_file(&dir.join("ws-echo.yml"), &ws_item).unwrap();

    let grpc_item = json!({
        "name": "gRPC Echo",
        "type": "grpc-request",
        "seq": 2,
        "request": {
            "url": "127.0.0.1:18083",
            "method": "/echo.Echo/Unary",
            "methodType": "unary",
            "protoPath": "echo.proto",
            "headers": [{ "name": "x-trace", "value": "bruno-rs", "enabled": true }],
            "body": { "mode": "grpc", "grpc": [{ "name": "message 1", "content": "{\"message\":\"hi\"}" }] },
            "auth": { "mode": "none" },
            "vars": { "req": [] },
            "script": {
                "beforeCallStart": "console.log('before')",
                "beforeMessageSend": "",
                "afterMessageReceive": "",
                "afterCallEnd": ""
            },
            "tests": ""
        }
    });
    save_request_file(&dir.join("grpc-echo.yml"), &grpc_item).unwrap();

    // 磁盘形状必须是 websocket: / grpc: 段，且 info.type 正确
    let ws_raw = std::fs::read_to_string(dir.join("ws-echo.yml")).unwrap();
    assert!(ws_raw.contains("type: websocket"), "WS 段类型不对：\n{ws_raw}");
    assert!(ws_raw.contains("websocket:"), "缺少 websocket 段：\n{ws_raw}");
    assert!(ws_raw.contains("title: ping"), "多条消息应写变体数组：\n{ws_raw}");
    assert!(ws_raw.contains("before-request"), "WS 脚本没有落盘：\n{ws_raw}");

    let grpc_raw = std::fs::read_to_string(dir.join("grpc-echo.yml")).unwrap();
    assert!(grpc_raw.contains("type: grpc"), "gRPC 段类型不对：\n{grpc_raw}");
    assert!(grpc_raw.contains("method: /echo.Echo/Unary"), "缺少 method：\n{grpc_raw}");
    assert!(grpc_raw.contains("methodType: unary"));
    assert!(grpc_raw.contains("protoFilePath: echo.proto"));
    assert!(grpc_raw.contains("metadata:"), "gRPC 元数据应写成 metadata：\n{grpc_raw}");
    assert!(grpc_raw.contains("grpc:before-call-start"), "gRPC 脚本类型不对：\n{grpc_raw}");

    // 重新加载：树节点类型 + 前端形状
    let collection = load_collection(&dir).unwrap();
    let kinds: Vec<(String, String)> = collection
        .tree
        .iter()
        .map(|n| (n.name.clone(), n.kind.clone()))
        .collect();
    assert!(kinds.contains(&("WS Echo".to_string(), "ws-request".to_string())), "{kinds:?}");
    assert!(
        kinds.contains(&("gRPC Echo".to_string(), "grpc-request".to_string())),
        "{kinds:?}"
    );

    let ws_node = collection.tree.iter().find(|n| n.kind == "ws-request").unwrap();
    let ws_request = ws_node.request.clone().unwrap();
    assert_eq!(ws_request["body"]["mode"], json!("ws"));
    let messages = ws_request["body"]["ws"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["type"], json!("json"));
    assert_eq!(messages[0]["content"], json!("{\"a\":1}"));
    assert_eq!(messages[1]["content"], json!("plain"));

    let grpc_node = collection.tree.iter().find(|n| n.kind == "grpc-request").unwrap();
    let grpc_request = grpc_node.request.clone().unwrap();
    assert_eq!(grpc_request["method"], json!("/echo.Echo/Unary"));
    assert_eq!(grpc_request["methodType"], json!("unary"));
    assert_eq!(grpc_request["protoPath"], json!("echo.proto"));
    assert_eq!(grpc_request["headers"][0]["name"], json!("x-trace"));
    assert_eq!(grpc_request["body"]["grpc"][0]["content"], json!("{\"message\":\"hi\"}"));
    assert_eq!(grpc_request["script"]["beforeCallStart"], json!("console.log('before')"));

    let _ = std::fs::remove_dir_all(&dir);
}
