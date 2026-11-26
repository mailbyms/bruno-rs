//! 请求落盘：把前端编辑后的 item（Bruno UI 形状）合并回 OpenCollection YAML。
//!
//! 设计约束：
//! - **不整份重写**：先读磁盘上的 YAML，只覆盖 `info` / `http` / `runtime` / `settings`，
//!   其它顶层键（docs、extensions、未知键）原样保留。
//! - `http` / `runtime` 内部同样做映射级合并，未知键保留。
//! - 注释不保留（serde_yml 无注释模型），这是已知取舍。
//! - 写入走「临时文件 + rename」，失败时回退直接写，避免半截文件。

use serde_json::Value as J;
use serde_yml::Value as Y;
use std::path::Path;

type Map = serde_yml::Mapping;

/// 把前端 item 保存到磁盘上的请求 yml
pub fn save_request_file(path: &Path, item: &J) -> Result<(), String> {
    let existing = std::fs::read_to_string(path).ok();
    let mut doc: Y = existing
        .as_deref()
        .and_then(|raw| serde_yml::from_str(raw).ok())
        .unwrap_or_else(empty_map);

    if !doc.is_mapping() {
        doc = empty_map();
    }
    let root = doc.as_mapping_mut().ok_or("请求文件不是 YAML 映射")?;

    let request = item
        .get("request")
        .ok_or_else(|| "item 缺少 request".to_string())?;

    // 请求种类：http / ws / grpc。磁盘上分别落在 `http:` / `websocket:` / `grpc:` 段，
    // `info.type` 也跟着变（http / websocket / grpc），漏掉会让前端把请求当 HTTP 渲染。
    let body_mode = request
        .get("body")
        .and_then(|b| b.get("mode"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let kind = if item_type == "ws-request" || body_mode == "ws" {
        "ws"
    } else if item_type == "grpc-request" || body_mode == "grpc" {
        "grpc"
    } else {
        "http"
    };

    // info -----------------------------------------------------------------
    let mut info = take_mapping(root, "info");
    set_str(&mut info, "name", item.get("name").and_then(|v| v.as_str()).unwrap_or("Untitled Request"));
    let info_type = match kind {
        "ws" => "websocket",
        "grpc" => "grpc",
        _ => "http",
    };
    if info.get("type").and_then(|v| v.as_str()) != Some(info_type) {
        set_str(&mut info, "type", info_type);
    }
    if let Some(seq) = item.get("seq").and_then(|v| v.as_i64()) {
        info.insert(key("seq"), Y::Number(seq.into()));
    }
    match item.get("tags") {
        Some(J::Array(tags)) if !tags.is_empty() => {
            let values = tags
                .iter()
                .filter_map(|t| t.as_str())
                .map(|t| Y::String(t.to_string()))
                .collect::<Vec<_>>();
            info.insert(key("tags"), Y::Sequence(values));
        }
        Some(J::Array(_)) => {
            info.remove(key("tags"));
        }
        _ => {}
    }
    match item.get("description") {
        Some(J::String(s)) if !s.trim().is_empty() => set_str(&mut info, "description", s),
        Some(J::String(_)) => {
            info.remove(key("description"));
        }
        _ => {}
    }
    root.insert(key("info"), Y::Mapping(info));

    // 请求体 ---------------------------------------------------------------
    // `websocket:` / `grpc:` 是独立段，不能写进 `http:`；反之亦然
    match kind {
        "ws" => {
            let existing = take_mapping(root, "websocket");
            root.insert(key("websocket"), Y::Mapping(ws_section(existing, request)));
        }
        "grpc" => {
            let existing = take_mapping(root, "grpc");
            root.insert(key("grpc"), Y::Mapping(grpc_section(existing, request)));
        }
        _ => {
            let existing = take_mapping(root, "http");
            root.insert(key("http"), Y::Mapping(http_section(existing, request)));
        }
    }

    // runtime --------------------------------------------------------------
    let mut runtime = take_mapping(root, "runtime");
    if let Some(vars) = request
        .get("vars")
        .and_then(|v| v.get("req"))
        .and_then(|v| v.as_array())
    {
        let values = vars
            .iter()
            .filter_map(|v| {
                let name = v.get("name").and_then(|v| v.as_str())?;
                if name.is_empty() {
                    return None;
                }
                let mut entry = Map::new();
                set_str(&mut entry, "name", name);
                set_str(&mut entry, "value", v.get("value").and_then(|s| s.as_str()).unwrap_or(""));
                if v.get("enabled").and_then(|s| s.as_bool()) == Some(false) {
                    entry.insert(key("disabled"), Y::Bool(true));
                }
                Some(Y::Mapping(entry))
            })
            .collect::<Vec<_>>();
        if values.is_empty() {
            runtime.remove(key("variables"));
        } else {
            runtime.insert(key("variables"), Y::Sequence(values));
        }
    }

    // scripts：先剔除托管的类型（http 三个 + grpc 四个），保留其它类型（hooks 等）
    let managed = [
        "before-request",
        "after-response",
        "tests",
        "grpc:before-call-start",
        "grpc:before-message-send",
        "grpc:after-message-receive",
        "grpc:after-call-end",
    ];
    let mut scripts: Vec<Y> = runtime
        .get("scripts")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter(|s| {
                    !matches!(
                        s.get("type").and_then(|t| t.as_str()),
                        Some(t) if managed.contains(&t)
                    )
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let script_text = |path: &[&str]| -> Option<String> {
        let mut cur = request.get("script");
        for part in path {
            cur = cur?.get(*part);
        }
        cur.and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.trim().is_empty())
    };
    let tests = || {
        request
            .get("tests")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.trim().is_empty())
    };
    let script_entries: Vec<(&str, Option<String>)> = if kind == "grpc" {
        vec![
            ("grpc:before-call-start", script_text(&["beforeCallStart"])),
            ("grpc:before-message-send", script_text(&["beforeMessageSend"])),
            ("grpc:after-message-receive", script_text(&["afterMessageReceive"])),
            ("grpc:after-call-end", script_text(&["afterCallEnd"])),
            ("tests", tests()),
        ]
    } else {
        vec![
            ("before-request", script_text(&["req"])),
            ("after-response", script_text(&["res"])),
            ("tests", tests()),
        ]
    };
    for (script_kind, text) in script_entries {
        if let Some(code) = text {
            let mut entry = Map::new();
            set_str(&mut entry, "type", script_kind);
            set_str(&mut entry, "code", &code);
            scripts.push(Y::Mapping(entry));
        }
    }
    if scripts.is_empty() {
        runtime.remove(key("scripts"));
    } else {
        runtime.insert(key("scripts"), Y::Sequence(scripts));
    }

    if let Some(assertions) = request.get("assertions").and_then(|v| v.as_array()) {
        let values = assertions
            .iter()
            .filter_map(|a| {
                let expression = a.get("name").and_then(|v| v.as_str())?;
                if expression.is_empty() {
                    return None;
                }
                let raw = a.get("value").and_then(|v| v.as_str()).unwrap_or("");
                let mut parts = raw.splitn(2, ' ');
                let operator = parts.next().unwrap_or("eq");
                let expected = parts.next().unwrap_or("");
                let mut entry = Map::new();
                set_str(&mut entry, "expression", expression);
                set_str(&mut entry, "operator", if operator.is_empty() { "eq" } else { operator });
                set_str(&mut entry, "value", expected);
                if a.get("disabled").and_then(|v| v.as_bool()) == Some(true)
                    || a.get("enabled").and_then(|v| v.as_bool()) == Some(false)
                {
                    entry.insert(key("disabled"), Y::Bool(true));
                }
                Some(Y::Mapping(entry))
            })
            .collect::<Vec<_>>();
        if values.is_empty() {
            runtime.remove(key("assertions"));
        } else {
            runtime.insert(key("assertions"), Y::Sequence(values));
        }
    }

    if runtime.is_empty() {
        root.remove(key("runtime"));
    } else {
        root.insert(key("runtime"), Y::Mapping(runtime));
    }

    // settings -------------------------------------------------------------
    match item.get("settings") {
        Some(J::Object(settings)) if !settings.is_empty() => {
            let mut merged = take_mapping(root, "settings");
            for (k, v) in settings {
                if v.is_null() {
                    continue;
                }
                if let Ok(y) = serde_yml::to_value(v) {
                    merged.insert(key(k), y);
                }
            }
            root.insert(key("settings"), Y::Mapping(merged));
        }
        _ => {}
    }

    write_atomic(path, &serde_yml::to_string(&doc).map_err(|e| format!("序列化 YAML 失败：{e}"))?)
}

/// 请求头 / metadata 数组 → YAML 序列（沿用原键位：enabled=false → disabled=true）
fn write_headers(map: &mut Map, request: &J, field: &str) {
    let Some(headers) = request.get(field).and_then(|v| v.as_array()) else {
        return;
    };
    let values = headers
        .iter()
        .filter_map(|h| {
            let name = h.get("name").and_then(|v| v.as_str())?;
            if name.is_empty() {
                return None;
            }
            let mut entry = Map::new();
            set_str(&mut entry, "name", name);
            set_str(&mut entry, "value", h.get("value").and_then(|v| v.as_str()).unwrap_or(""));
            if let Some(desc) = h.get("description").and_then(|v| v.as_str()) {
                if !desc.trim().is_empty() {
                    set_str(&mut entry, "description", desc);
                }
            }
            if h.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
                entry.insert(key("disabled"), Y::Bool(true));
            }
            Some(Y::Mapping(entry))
        })
        .collect::<Vec<_>>();
    if values.is_empty() {
        map.remove(key(field));
    } else {
        map.insert(key(field), Y::Sequence(values));
    }
}

/// `http:` 段（原有逻辑，抽出来供三类请求共用）
fn http_section(mut http: Map, request: &J) -> Map {
    let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("GET");
    let url = request.get("url").and_then(|v| v.as_str()).unwrap_or("");
    set_str(&mut http, "method", method);
    set_str(&mut http, "url", url);

    if let Some(params) = request.get("params").and_then(|v| v.as_array()) {
        let values = params
            .iter()
            .filter_map(|p| {
                let name = p.get("name").and_then(|v| v.as_str())?;
                if name.is_empty() {
                    return None;
                }
                let mut entry = Map::new();
                set_str(&mut entry, "name", name);
                set_str(&mut entry, "value", p.get("value").and_then(|v| v.as_str()).unwrap_or(""));
                if let Some(kind) = p.get("type").and_then(|v| v.as_str()) {
                    if !kind.is_empty() && kind != "query" {
                        set_str(&mut entry, "type", kind);
                    }
                }
                if p.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
                    entry.insert(key("disabled"), Y::Bool(true));
                }
                Some(Y::Mapping(entry))
            })
            .collect::<Vec<_>>();
        if values.is_empty() {
            http.remove(key("params"));
        } else {
            http.insert(key("params"), Y::Sequence(values));
        }
    }

    write_headers(&mut http, request, "headers");

    match ui_body_to_disk(request.get("body")) {
        Some(body) => {
            http.insert(key("body"), body);
        }
        None => {
            http.remove(key("body"));
        }
    }

    match request.get("auth").and_then(ui_auth_to_disk) {
        Some(auth) => {
            http.insert(key("auth"), auth);
        }
        None => {
            http.remove(key("auth"));
        }
    }
    http
}

/// `websocket:` 段：单条消息写 `{type,data}`，多条写变体数组 `[{title,selected,message}]`
fn ws_section(mut ws: Map, request: &J) -> Map {
    set_str(&mut ws, "url", request.get("url").and_then(|v| v.as_str()).unwrap_or(""));
    write_headers(&mut ws, request, "headers");

    let messages = request
        .get("body")
        .and_then(|b| b.get("ws"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    match messages.len() {
        0 => {
            ws.remove(key("message"));
        }
        1 => {
            let message = &messages[0];
            let mut entry = Map::new();
            set_str(&mut entry, "type", message.get("type").and_then(|v| v.as_str()).unwrap_or("json"));
            set_str(&mut entry, "data", message.get("content").and_then(|v| v.as_str()).unwrap_or(""));
            ws.insert(key("message"), Y::Mapping(entry));
        }
        _ => {
            let variants = messages
                .iter()
                .enumerate()
                .map(|(index, message)| {
                    let mut variant = Map::new();
                    let name = message
                        .get("name")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| format!("message {}", index + 1));
                    set_str(&mut variant, "title", &name);
                    if message.get("selected").and_then(|v| v.as_bool()) == Some(true) {
                        variant.insert(key("selected"), Y::Bool(true));
                    }
                    let mut inner = Map::new();
                    set_str(&mut inner, "type", message.get("type").and_then(|v| v.as_str()).unwrap_or("json"));
                    set_str(&mut inner, "data", message.get("content").and_then(|v| v.as_str()).unwrap_or(""));
                    variant.insert(key("message"), Y::Mapping(inner));
                    Y::Mapping(variant)
                })
                .collect::<Vec<_>>();
            ws.insert(key("message"), Y::Sequence(variants));
        }
    }

    match request.get("auth").and_then(ui_auth_to_disk) {
        Some(auth) => {
            ws.insert(key("auth"), auth);
        }
        None => {
            ws.remove(key("auth"));
        }
    }
    ws
}

/// `grpc:` 段：`method` / `methodType` / `protoFilePath` / `metadata` / `message`
fn grpc_section(mut grpc: Map, request: &J) -> Map {
    set_str(&mut grpc, "url", request.get("url").and_then(|v| v.as_str()).unwrap_or(""));
    set_str(&mut grpc, "method", request.get("method").and_then(|v| v.as_str()).unwrap_or(""));
    let method_type = request
        .get("methodType")
        .or_else(|| request.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("unary");
    set_str(&mut grpc, "methodType", method_type);
    let proto_path = request
        .get("protoPath")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if proto_path.is_empty() {
        grpc.remove(key("protoFilePath"));
    } else {
        set_str(&mut grpc, "protoFilePath", proto_path);
    }
    write_headers(&mut grpc, request, "headers");
    // gRPC 的元数据在磁盘上是 `metadata`，前端仍叫 headers
    if let Some(metadata) = grpc.remove(key("headers")) {
        grpc.insert(key("metadata"), metadata);
    }

    let messages = request
        .get("body")
        .and_then(|b| b.get("grpc"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    match messages.len() {
        0 => {
            grpc.remove(key("message"));
        }
        1 => {
            // 单条：直接写 JSON 字符串
            set_str(
                &mut grpc,
                "message",
                messages[0].get("content").and_then(|v| v.as_str()).unwrap_or(""),
            );
        }
        _ => {
            let variants = messages
                .iter()
                .enumerate()
                .map(|(index, message)| {
                    let mut variant = Map::new();
                    let name = message
                        .get("name")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| format!("message {}", index + 1));
                    set_str(&mut variant, "title", &name);
                    // 变体里 message 是字符串（不是 {type,data}）
                    set_str(&mut variant, "message", message.get("content").and_then(|v| v.as_str()).unwrap_or(""));
                    Y::Mapping(variant)
                })
                .collect::<Vec<_>>();
            grpc.insert(key("message"), Y::Sequence(variants));
        }
    }

    match request.get("auth").and_then(ui_auth_to_disk) {
        Some(auth) => {
            grpc.insert(key("auth"), auth);
        }
        None => {
            grpc.remove(key("auth"));
        }
    }
    grpc
}

/// 前端 body（Bruno UI 形状）→ OpenCollection body
pub fn ui_body_to_disk(body: Option<&J>) -> Option<Y> {
    let body = body?;
    let mode = body.get("mode").and_then(|v| v.as_str()).unwrap_or("none");
    let mut map = Map::new();
    match mode {
        "none" | "" => return None,
        "json" | "text" | "xml" | "sparql" => {
            set_str(&mut map, "type", mode);
            set_str(
                &mut map,
                "data",
                body.get(mode).and_then(|v| v.as_str()).unwrap_or(""),
            );
        }
        "formUrlEncoded" => {
            set_str(&mut map, "type", "form-urlencoded");
            let entries = body
                .get("formUrlEncoded")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|e| {
                            let name = e.get("name").and_then(|v| v.as_str())?;
                            if name.is_empty() {
                                return None;
                            }
                            let mut entry = Map::new();
                            set_str(&mut entry, "name", name);
                            set_str(&mut entry, "value", e.get("value").and_then(|v| v.as_str()).unwrap_or(""));
                            if e.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
                                entry.insert(key("disabled"), Y::Bool(true));
                            }
                            Some(Y::Mapping(entry))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if !entries.is_empty() {
                map.insert(key("data"), Y::Sequence(entries));
            }
        }
        "multipartForm" => {
            set_str(&mut map, "type", "multipart-form");
            let entries = body
                .get("multipartForm")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|e| {
                            let name = e.get("name").and_then(|v| v.as_str())?;
                            if name.is_empty() {
                                return None;
                            }
                            let kind = e.get("type").and_then(|v| v.as_str()).unwrap_or("text");
                            let mut entry = Map::new();
                            set_str(&mut entry, "name", name);
                            set_str(&mut entry, "type", kind);
                            if let Some(value) = e.get("value") {
                                let y = match value {
                                    J::Array(items) => Y::Sequence(
                                        items
                                            .iter()
                                            .filter_map(|x| x.as_str())
                                            .map(|s| Y::String(s.to_string()))
                                            .collect(),
                                    ),
                                    J::String(s) => Y::String(s.clone()),
                                    other => serde_yml::to_value(other).unwrap_or(Y::String(String::new())),
                                };
                                entry.insert(key("value"), y);
                            }
                            if let Some(ct) = e.get("contentType").and_then(|v| v.as_str()) {
                                if !ct.trim().is_empty() {
                                    set_str(&mut entry, "contentType", ct);
                                }
                            }
                            if e.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
                                entry.insert(key("disabled"), Y::Bool(true));
                            }
                            Some(Y::Mapping(entry))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if !entries.is_empty() {
                map.insert(key("data"), Y::Sequence(entries));
            }
        }
        "file" => {
            set_str(&mut map, "type", "file");
            let entries = body
                .get("file")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|e| {
                            let file_path = e.get("filePath").and_then(|v| v.as_str()).unwrap_or("");
                            if file_path.is_empty() {
                                return None;
                            }
                            let mut entry = Map::new();
                            set_str(&mut entry, "filePath", file_path);
                            set_str(&mut entry, "contentType", e.get("contentType").and_then(|v| v.as_str()).unwrap_or(""));
                            entry.insert(
                                key("selected"),
                                Y::Bool(e.get("selected").and_then(|v| v.as_bool()).unwrap_or(false)),
                            );
                            Some(Y::Mapping(entry))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if !entries.is_empty() {
                map.insert(key("data"), Y::Sequence(entries));
            }
        }
        other => {
            // 未支持的 body 模式：保持磁盘上的原值不变
            let _ = other;
            return None;
        }
    }
    Some(Y::Mapping(map))
}

/// 前端 auth（`{mode, [mode]: {...}}`）→ OpenCollection auth
pub fn ui_auth_to_disk(auth: &J) -> Option<Y> {
    let mode = auth.get("mode").and_then(|v| v.as_str()).unwrap_or("none");
    if mode == "none" || mode.is_empty() {
        return None;
    }
    if mode == "inherit" {
        return Some(Y::String("inherit".into()));
    }
    let mut map = Map::new();
    set_str(&mut map, "type", mode);
    // 兼容扁平旧形状与嵌套新形状
    let config = auth.get(mode).and_then(|v| v.as_object()).cloned().or_else(|| {
        let mut flat = auth.as_object().cloned()?;
        flat.remove("mode");
        Some(flat)
    });
    if let Some(config) = config {
        for (k, v) in &config {
            if v.is_null() {
                continue;
            }
            if let Some(s) = v.as_str() {
                if s.is_empty() {
                    continue;
                }
                let value = if mode == "apikey" && k == "placement" && s == "queryparams" {
                    "query".to_string()
                } else {
                    s.to_string()
                };
                set_str(&mut map, k, &value);
            } else if let Ok(y) = serde_yml::to_value(v) {
                map.insert(key(k), y);
            }
        }
    }
    Some(Y::Mapping(map))
}

/// 磁盘 auth → 前端 `{mode, [mode]: {...}}`（供集合树/请求加载使用）
pub fn disk_auth_to_ui(auth: Option<&crate::model::Auth>) -> J {
    use crate::model::Auth;
    match auth {
        None => serde_json::json!({ "mode": "none" }),
        Some(Auth::Inherit(s)) => serde_json::json!({ "mode": s }),
        Some(Auth::Typed(typed)) => {
            let mode = typed.type_name();
            let mut value = serde_json::to_value(typed).unwrap_or_else(|_| serde_json::json!({}));
            if let Some(obj) = value.as_object_mut() {
                obj.remove("type");
                if mode == "apikey" {
                    if let Some(placement) = obj.get_mut("placement") {
                        if placement.as_str() == Some("query") {
                            *placement = serde_json::json!("queryparams");
                        }
                    }
                }
            }
            serde_json::json!({ "mode": mode, mode: value })
        }
    }
}

fn empty_map() -> Y {
    Y::Mapping(Map::new())
}

fn key(s: &str) -> Y {
    Y::String(s.to_string())
}

fn set_str(map: &mut Map, k: &str, v: &str) {
    map.insert(key(k), Y::String(v.to_string()));
}

fn take_mapping(root: &mut Map, k: &str) -> Map {
    match root.remove(key(k)) {
        Some(Y::Mapping(m)) => m,
        _ => Map::new(),
    }
}

fn write_atomic(path: &Path, content: &str) -> Result<(), String> {
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("yml")
    ));
    std::fs::write(&tmp, content).map_err(|e| format!("写入临时文件失败：{e}"))?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(_) => {
            // Windows 上 rename 不能覆盖已存在文件
            let result = std::fs::write(path, content).map_err(|e| format!("写入失败：{e}"));
            let _ = std::fs::remove_file(&tmp);
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str, body: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("bruno-rs-persist-{name}"));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("req.yml");
        std::fs::write(&path, body).unwrap();
        path
    }

    const ORIGINAL: &str = "info:\n  name: Old\n  type: http\n  seq: 1\n\nhttp:\n  method: GET\n  url: http://old\n\nsettings:\n  encodeUrl: true\n  timeout: 0\n";

    fn ui_item(url: &str, body: J) -> J {
        serde_json::json!({
            "uid": "abc",
            "type": "http-request",
            "name": "New Name",
            "seq": 3,
            "request": {
                "method": "POST",
                "url": url,
                "params": [{"name": "q", "value": "1", "type": "query", "enabled": true}],
                "headers": [{"name": "X-Test", "value": "yes", "enabled": true}],
                "body": body,
                "auth": {"mode": "basic", "basic": {"username": "u", "password": "p"}},
                "vars": {"req": [{"name": "token", "value": "t", "enabled": true}]},
                "script": {"req": "req.setHeader('x-script','1')", "res": "console.log(res.getStatus())"},
                "tests": "test('ok', function() { expect(res.getStatus()).to.equal(200); });",
                "assertions": [{"name": "res.status", "value": "eq 200"}]
            },
            "settings": {"encodeUrl": false}
        })
    }

    #[test]
    fn merges_without_dropping_unknown_keys() {
        let path = temp_file(
            "merge",
            "info:\n  name: Old\n  type: http\n  seq: 1\n  tags: [a]\ncustomTop: keep-me\nhttp:\n  method: GET\n  url: http://old\n  customHttp: stay\nsettings:\n  encodeUrl: true\n",
        );
        let item = ui_item("http://new", serde_json::json!({"mode": "none"}));
        save_request_file(&path, &item).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("keep-me"), "顶层未知键必须保留：{raw}");
        assert!(raw.contains("stay"), "http 未知键必须保留：{raw}");
        assert!(raw.contains("New Name"));
        assert!(raw.contains("http://new"));
        assert!(!raw.contains("body:"), "mode=none 不应写 body");
    }

    #[test]
    fn round_trips_ui_item() {
        let path = temp_file("roundtrip", ORIGINAL);
        let item = ui_item(
            "http://new",
            serde_json::json!({"mode": "json", "json": "{\"a\":1}"}),
        );
        save_request_file(&path, &item).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        let parsed: crate::model::HttpRequest = serde_yml::from_str(&raw).unwrap();
        let http = parsed.http.as_ref().unwrap();
        assert_eq!(http.method.as_deref(), Some("POST"));
        assert_eq!(http.url.as_deref(), Some("http://new"));
        assert_eq!(http.headers.len(), 1);
        assert!(matches!(http.body, Some(crate::model::BodyData::Raw { .. })));
        assert_eq!(parsed.runtime.scripts.len(), 3);
        assert_eq!(parsed.runtime.assertions.len(), 1);
        assert_eq!(parsed.runtime.variables.len(), 1);
        // settings 合并：新值覆盖、旧键保留
        assert!(!raw.contains("body:\n    type: json\n    data: ''"));
        assert!(raw.contains("encodeUrl: false"), "{raw}");
        assert!(raw.contains("timeout: 0"), "settings 未编辑键应保留：{raw}");
    }

    #[test]
    fn writes_multipart_and_file_bodies() {
        let path = temp_file("bodies", ORIGINAL);
        let item = ui_item(
            "http://new",
            serde_json::json!({
                "mode": "multipartForm",
                "multipartForm": [
                    {"name": "field", "type": "text", "value": "v", "enabled": true},
                    {"name": "upload", "type": "file", "value": ["a.txt"], "enabled": true}
                ]
            }),
        );
        save_request_file(&path, &item).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("type: multipart-form"), "{raw}");
        assert!(raw.contains("a.txt"));

        let item = ui_item(
            "http://new",
            serde_json::json!({"mode": "file", "file": [{"filePath": "x.bin", "contentType": "", "selected": true}]}),
        );
        save_request_file(&path, &item).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("type: file"), "{raw}");
        assert!(raw.contains("x.bin"));
        assert!(raw.contains("selected: true"));
    }

    #[test]
    fn auth_round_trip_is_nested_ui_shape() {
        let path = temp_file("auth", ORIGINAL);
        let item = ui_item("http://new", serde_json::json!({"mode": "none"}));
        save_request_file(&path, &item).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("type: basic"), "{raw}");
        let parsed: crate::model::HttpRequest = serde_yml::from_str(&raw).unwrap();
        let ui = disk_auth_to_ui(parsed.http.unwrap().auth.as_ref());
        assert_eq!(ui["mode"], "basic");
        assert_eq!(ui["basic"]["username"], "u");
    }
}
