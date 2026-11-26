//! gRPC 描述符：纯 Rust 编译 `.proto`、枚举方法、生成示例消息。
//!
//! 用 `protox`（= 纯 Rust protoc）把 .proto 编译成 `FileDescriptorSet`，
//! 再用 `prost-reflect` 拿 `DescriptorPool`。这样发布包不需要自带 protoc 二进制。
//!
//! 方法列表的每个条目会附带 `requestFields`（请求消息的字段树）和
//! `sampleRequest`（示例 JSON）。前端「Regenerate sample message」会把
//! `requestFields` 原样回传，后端据此生成示例，无需缓存描述符。

use prost_reflect::{DescriptorPool, EnumDescriptor, Kind, MessageDescriptor};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

/// 递归深度上限，防止自引用消息无限展开
const MAX_DEPTH: usize = 6;

/// 编译一组 .proto 文件。`includes` 是 import 解析路径（集合目录、proto 所在目录、importPaths）。
pub fn compile(files: &[PathBuf], includes: &[PathBuf]) -> anyhow::Result<DescriptorPool> {
    if files.is_empty() {
        anyhow::bail!("没有提供 .proto 文件");
    }
    for f in files {
        if !f.exists() {
            anyhow::bail!("proto 文件不存在：{}", f.display());
        }
    }
    // protox 对「包含 include 之外的绝对路径」比较宽容，但仍把每个 proto 的父目录加进去，
    // 这样同目录下的 import 才解析得到。
    let mut all_includes: Vec<PathBuf> = Vec::new();
    for inc in includes {
        if inc.exists() && !all_includes.contains(inc) {
            all_includes.push(inc.clone());
        }
    }
    for f in files {
        if let Some(parent) = f.parent() {
            let parent = parent.to_path_buf();
            if !all_includes.contains(&parent) {
                all_includes.push(parent);
            }
        }
    }
    if all_includes.is_empty() {
        all_includes.push(PathBuf::from("."));
    }

    let fds = protox::compile(files, &all_includes).map_err(|e| anyhow::anyhow!("解析 proto 失败：{e}"))?;
    DescriptorPool::from_file_descriptor_set(fds).map_err(|e| anyhow::anyhow!("构建描述符池失败：{e}"))
}

/// 从描述符池列出所有方法（前端 MethodDropdown 用）
pub fn methods_from_pool(pool: &DescriptorPool) -> Vec<Value> {
    let mut out = Vec::new();
    for service in pool.services() {
        let service_name = service.full_name().to_string();
        for method in service.methods() {
            let (type_, client_streaming, server_streaming) = match (method.is_client_streaming(), method.is_server_streaming()) {
                (false, false) => ("unary", false, false),
                (true, false) => ("client-streaming", true, false),
                (false, true) => ("server-streaming", false, true),
                (true, true) => ("bidi-streaming", true, true),
            };
            let input = method.input();
            let output = method.output();
            let fields = field_tree(&input, 0, &mut vec![input.full_name().to_string()]);
            out.push(json!({
                "name": method.name(),
                "path": format!("/{service_name}/{}", method.name()),
                "type": type_,
                "serviceName": service_name,
                "methodName": method.name(),
                "requestType": input.full_name(),
                "responseType": output.full_name(),
                "clientStreaming": client_streaming,
                "serverStreaming": server_streaming,
                "requestFields": fields,
                "sampleRequest": sample_from_fields(&fields, 1),
            }));
        }
    }
    out
}

/// 消息字段树：[{name, jsonName, kind, repeated, fields?, enumValues?}]
fn field_tree(desc: &MessageDescriptor, depth: usize, seen: &mut Vec<String>) -> Vec<Value> {
    if depth > MAX_DEPTH {
        return Vec::new();
    }
    let mut out = Vec::new();
    for field in desc.fields() {
        let mut entry = Map::new();
        entry.insert("name".into(), json!(field.name()));
        entry.insert("jsonName".into(), json!(field.json_name()));
        entry.insert("repeated".into(), json!(field.is_list()));
        entry.insert("map".into(), json!(field.is_map()));
        match field.kind() {
            Kind::Bool => {
                entry.insert("kind".into(), json!("boolean"));
            }
            Kind::String => {
                entry.insert("kind".into(), json!("string"));
            }
            Kind::Bytes => {
                entry.insert("kind".into(), json!("bytes"));
            }
            Kind::Message(msg) => {
                entry.insert("kind".into(), json!("message"));
                entry.insert("messageType".into(), json!(msg.full_name()));
                if !seen.contains(&msg.full_name().to_string()) {
                    seen.push(msg.full_name().to_string());
                    entry.insert("fields".into(), json!(field_tree(&msg, depth + 1, seen)));
                    seen.pop();
                }
            }
            Kind::Enum(en) => {
                entry.insert("kind".into(), json!("enum"));
                entry.insert("enumValues".into(), json!(enum_values(&en)));
            }
            _ => {
                entry.insert("kind".into(), json!("number"));
            }
        }
        out.push(Value::Object(entry));
    }
    out
}

fn enum_values(en: &EnumDescriptor) -> Vec<String> {
    en.values().map(|v| v.name().to_string()).collect()
}

/// 由字段树生成示例 JSON。`array_size` 控制 repeated 字段的元素个数。
pub fn sample_from_fields(fields: &[Value], array_size: usize) -> Value {
    let mut map = Map::new();
    for f in fields {
        let Some(name) = f.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let kind = f.get("kind").and_then(|v| v.as_str()).unwrap_or("string");
        let repeated = f.get("repeated").and_then(|v| v.as_bool()).unwrap_or(false);
        let is_map = f.get("map").and_then(|v| v.as_bool()).unwrap_or(false);
        let single = match kind {
            "boolean" => json!(false),
            "number" => json!(0),
            "bytes" => json!(""),
            "enum" => f
                .get("enumValues")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .cloned()
                .unwrap_or_else(|| json!("")),
            "message" => {
                let sub = f.get("fields").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                sample_from_fields(&sub, array_size)
            }
            _ => json!(""),
        };
        let value = if is_map {
            json!({})
        } else if repeated {
            let n = array_size.max(1);
            json!(vec![single; n])
        } else {
            single
        };
        map.insert(name.to_string(), value);
    }
    Value::Object(map)
}

/// 按方法路径在描述符池里找方法（`/pkg.Service/Method`，允许省略前导斜杠）
pub fn find_method(pool: &DescriptorPool, path: &str) -> Option<prost_reflect::MethodDescriptor> {
    let trimmed = path.trim_start_matches('/');
    let (service_name, method_name) = trimmed.rsplit_once('/')?;
    let service = pool.get_service_by_name(service_name)?;
    let mut found: Option<prost_reflect::MethodDescriptor> = None;
    for method in service.methods() {
        if method.name() == method_name {
            found = Some(method);
            break;
        }
    }
    found
}

/// 生成示例消息：优先用前端回传的 `methodMetadata.requestFields`，
/// 退化到 `sampleRequest`，再退化到 `{}`。
pub fn sample_message(existing: Option<&str>, method_meta: Option<&Value>, array_size: usize) -> Value {
    let fields = method_meta
        .and_then(|m| m.get("requestFields"))
        .and_then(|v| v.as_array())
        .cloned();
    let generated = match fields {
        Some(fields) => sample_from_fields(&fields, array_size),
        None => method_meta
            .and_then(|m| m.get("sampleRequest"))
            .cloned()
            .unwrap_or_else(|| json!({})),
    };

    // 已有内容能解析成 JSON 时保留其取值，只补缺失的键
    let Some(existing) = existing.filter(|s| !s.trim().is_empty()) else {
        return generated;
    };
    let Ok(mut current) = serde_json::from_str::<Value>(existing) else {
        return generated;
    };
    if current.is_object() {
        merge_missing(&mut current, &generated);
        current
    } else {
        generated
    }
}

fn merge_missing(target: &mut Value, template: &Value) {
    if let (Some(t), Some(tpl)) = (target.as_object_mut(), template.as_object()) {
        for (k, v) in tpl {
            match t.get_mut(k) {
                Some(existing) => {
                    if existing.is_object() && v.is_object() {
                        merge_missing(existing, v);
                    }
                }
                None => {
                    t.insert(k.clone(), v.clone());
                }
            }
        }
    }
}

/// 把 gRPC 地址规范成 tonic 需要的 URI
pub fn normalize_endpoint(url: &str) -> String {
    let url = url.trim();
    if let Some(rest) = url.strip_prefix("grpc://") {
        return format!("http://{rest}");
    }
    if let Some(rest) = url.strip_prefix("grpcs://") {
        return format!("https://{rest}");
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        return url.to_string();
    }
    format!("http://{url}")
}

/// 收集 proto 相关的 import 路径（集合目录 + brunoConfig.protobuf.importPaths）
pub fn proto_includes(collection_dir: &Path, collection: &Value) -> Vec<PathBuf> {
    let mut out = vec![collection_dir.to_path_buf()];
    let protobuf = collection
        .get("brunoConfig")
        .and_then(|c| c.get("protobuf"))
        .or_else(|| collection.get("root").and_then(|r| r.get("brunoConfig")).and_then(|c| c.get("protobuf")));
    if let Some(paths) = protobuf.and_then(|p| p.get("importPaths")).and_then(|v| v.as_array()) {
        for entry in paths {
            if entry.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
                continue;
            }
            if let Some(p) = entry.get("path").and_then(|v| v.as_str()) {
                let full = if Path::new(p).is_absolute() {
                    PathBuf::from(p)
                } else {
                    collection_dir.join(p)
                };
                if full.exists() && !out.contains(&full) {
                    out.push(full);
                }
            }
        }
    }
    // 集合设置里登记的 proto 文件所在目录也算 import 路径
    if let Some(files) = protobuf.and_then(|p| p.get("protoFiles")).and_then(|v| v.as_array()) {
        for entry in files {
            if let Some(p) = entry.get("path").and_then(|v| v.as_str()) {
                let full = if Path::new(p).is_absolute() {
                    PathBuf::from(p)
                } else {
                    collection_dir.join(p)
                };
                if let Some(parent) = full.parent() {
                    let parent = parent.to_path_buf();
                    if parent.exists() && !out.contains(&parent) {
                        out.push(parent);
                    }
                }
            }
        }
    }
    out
}
