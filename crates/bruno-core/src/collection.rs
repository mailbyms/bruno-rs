//! 集合加载：扫描 OpenCollection YAML 目录树 → 前端可用的集合树 + 内存索引。
//! 仅支持 yml 格式（opencollection.yml + *.yml），不读 bruno.json / .bru。

use crate::model::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 集合根标识：opencollection.yml 存在才算合法集合
pub const ROOT_FILE: &str = "opencollection.yml";
pub const FOLDER_FILE: &str = "folder.yml";
pub const ENV_DIR: &str = "environments";

#[derive(Debug, Clone)]
pub struct Collection {
    pub uid: String,
    pub name: String,
    pub pathname: PathBuf,
    pub root: OpenCollection,
    pub tree: Vec<TreeNode>,
    pub environments: Vec<(String, Environment)>,
    /// pathname -> 请求条目（内存索引，加载时一次扫描）
    pub requests: HashMap<String, RequestFile>,
}

/// 判断目录是否为 OpenCollection 集合
pub fn is_collection_dir(path: &Path) -> bool {
    path.join(ROOT_FILE).is_file()
}

/// 加载集合：读取根文件 + 扫描目录树
pub fn load_collection(path: &Path) -> Result<Collection, String> {
    if !is_collection_dir(path) {
        return Err(format!(
            "不是 OpenCollection 集合（缺少 {}）：{}",
            ROOT_FILE,
            path.display()
        ));
    }
    let raw = std::fs::read_to_string(path.join(ROOT_FILE))
        .map_err(|e| format!("读取 {} 失败：{e}", path.join(ROOT_FILE).display()))?;
    let root: OpenCollection = serde_yml::from_str(&raw)
        .map_err(|e| format!("解析 {} 失败：{e}", ROOT_FILE))?;

    let name = root
        .info
        .name
        .clone()
        .unwrap_or_else(|| "Untitled Collection".into());

    let mut requests = HashMap::new();
    let mut environments = Vec::new();
    let tree = scan_dir(path, path, &mut requests)?;

    // 环境目录
    let env_path = path.join(ENV_DIR);
    if env_path.is_dir() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&env_path)
            .map_err(|e| format!("读取环境目录失败：{e}"))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension()
                    .and_then(|x| x.to_str())
                    .map(|x| x.eq_ignore_ascii_case("yml") || x.eq_ignore_ascii_case("yaml"))
                    .unwrap_or(false)
            })
            .collect();
        entries.sort();
        for entry in entries {
            let raw = match std::fs::read_to_string(&entry) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let env: Environment = match serde_yml::from_str(&raw) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let name = if env.name.is_empty() {
                entry
                    .file_stem()
                    .and_then(|x| x.to_str())
                    .unwrap_or("Untitled")
                    .to_string()
            } else {
                env.name.clone()
            };
            environments.push((name, env));
        }
    }

    Ok(Collection {
        uid: new_uid(),
        name,
        pathname: path.to_path_buf(),
        root,
        tree,
        environments,
        requests,
    })
}

/// 递归扫描目录；folder.yml 表达文件夹自身，其它 *.yml 是请求
fn scan_dir(
    root: &Path,
    dir: &Path,
    requests: &mut HashMap<String, RequestFile>,
) -> Result<Vec<TreeNode>, String> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("读取目录失败 {}：{e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let file_name = p.file_name().and_then(|x| x.to_str()).unwrap_or("");
            // 跳过环境目录、隐藏目录、node_modules、.git
            if p.is_dir() {
                return !file_name.starts_with('.')
                    && file_name != "node_modules"
                    && !file_name.eq_ignore_ascii_case(ENV_DIR);
            }
            if !p.is_file() {
                return false;
            }
            let is_yml = p
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| x.eq_ignore_ascii_case("yml") || x.eq_ignore_ascii_case("yaml"))
                .unwrap_or(false);
            is_yml
                && !file_name.eq_ignore_ascii_case(ROOT_FILE)
                && !file_name.eq_ignore_ascii_case(FOLDER_FILE)
        })
        .collect();
    entries.sort();

    let mut nodes = Vec::new();
    for entry in entries {
        if entry.is_dir() {
            let folder_file = entry.join(FOLDER_FILE);
            let (name, seq, tags, defaults) = if folder_file.is_file() {
                match std::fs::read_to_string(&folder_file)
                    .ok()
                    .and_then(|raw| serde_yml::from_str::<Folder>(&raw).ok())
                {
                    Some(folder) => (
                        folder.info.name.clone().unwrap_or_default(),
                        folder.info.seq.unwrap_or(0),
                        folder.info.tags.clone(),
                        folder.request,
                    ),
                    None => (String::new(), 0, vec![], RequestDefaults::default()),
                }
            } else {
                (String::new(), 0, vec![], RequestDefaults::default())
            };
            let display_name = if name.is_empty() {
                entry
                    .file_stem()
                    .and_then(|x| x.to_str())
                    .unwrap_or("folder")
                    .to_string()
            } else {
                name
            };
            let mut node = TreeNode::folder(
                display_name,
                relative_path(root, &entry),
                FOLDER_FILE.into(),
            );
            node.seq = seq;
            node.tags = tags;
            if !defaults.headers.is_empty() || defaults.auth.is_some() {
                node.root = Some(serde_json::json!({
                    "request": normalize_defaults(&defaults)
                }));
            }
            node.items = scan_dir(root, &entry, requests)?;
            nodes.push(node);
        } else {
            match load_request_file(&entry, root) {
                Ok(file) => {
                    let mut node = TreeNode {
                        uid: file.uid.clone(),
                        name: file.name.clone(),
                        kind: file.kind.clone(),
                        seq: file.seq,
                        tags: file.tags.clone(),
                        filename: file.filename.clone(),
                        pathname: file.pathname.clone(),
                        items: vec![],
                        request: Some(serde_json::to_value(&file.request).unwrap_or_default()),
                        root: None,
                        // WS / gRPC 的设置面板键位不同（timeout / keepAliveInterval），
                        // HTTP 那套 encodeUrl/followRedirects 反而会污染保存
                        settings: if file.kind == "http-request" {
                            Some(settings_to_ui(&file.request.settings))
                        } else {
                            None
                        },
                        collapsed: false,
                    };
                    // 前端 request 字段需要 Bruno 内部形状（method/url 扁平），做一次规范化
                    node.request = Some(normalize_request(&file.request));
                    requests.insert(file.pathname.clone(), file);
                    nodes.push(node);
                }
                Err(_) => continue, // 单个坏文件不阻塞整棵树
            }
        }
    }
    Ok(nodes)
}

/// 集合级/文件夹级默认值 → 前端 root.request 形状
pub fn normalize_defaults(defaults: &RequestDefaults) -> serde_json::Value {
    let headers: Vec<serde_json::Value> = defaults
        .headers
        .iter()
        .map(|h| {
            serde_json::json!({
                "name": h.name,
                "value": h.value,
                "enabled": !h.disabled.unwrap_or(false),
                "description": h.description.as_ref().map(|d| d.text()).unwrap_or_default(),
            })
        })
        .collect();
    let auth = crate::persist::disk_auth_to_ui(defaults.auth.as_ref());
    serde_json::json!({
        "headers": headers,
        "auth": auth,
        "vars": { "req": defaults.variables.iter().map(|v| serde_json::json!({
            "name": v.name,
            "value": v.resolved().unwrap_or_default(),
            "enabled": !v.disabled.unwrap_or(false),
        })).collect::<Vec<_>>() },
        "script": { "req": "", "res": "" },
        "tests": "",
    })
}

/// 相邻排序：按 seq，再按 name
pub fn sort_tree(nodes: &mut [TreeNode]) {
    nodes.sort_by(|a, b| a.seq.cmp(&b.seq).then_with(|| a.name.cmp(&b.name)));
    for node in nodes {
        sort_tree(&mut node.items);
    }
}

fn load_request_file(path: &Path, root: &Path) -> Result<RequestFile, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
    let request: HttpRequest = serde_yml::from_str(&raw)
        .map_err(|e| format!("解析 {} 失败：{e}", path.display()))?;
    // 支持 http / websocket / grpc 三类；其它类型（如 future 的 graphql 单独文件）跳过
    let raw_kind = request.info.r#type.clone().unwrap_or_else(|| "http".into());
    if !matches!(raw_kind.as_str(), "http" | "websocket" | "ws" | "grpc") {
        return Err(format!("跳过未知请求类型：{raw_kind}"));
    }
    let name = request
        .info
        .name
        .clone()
        .unwrap_or_else(|| "Untitled Request".into());
    let pathname = relative_path(root, path);
    let kind = request.kind().to_string();
    Ok(RequestFile {
        uid: crate::ops::stable_uid(&pathname),
        pathname,
        filename: path
            .file_name()
            .and_then(|x| x.to_str())
            .unwrap_or("")
            .to_string(),
        name,
        kind,
        seq: request.info.seq.unwrap_or(1),
        tags: request.info.tags.clone(),
        request,
    })
}

/// 磁盘 settings → 前端 settings（只输出非空项，键统一 camelCase，
/// 避免保存时把 `encode_url: null` 这类别名写回 YAML）
pub fn settings_to_ui(settings: &HttpSettings) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    let bool_value = settings
        .encode_url
        .clone()
        .or_else(|| settings.encode_url_camel.clone());
    if let Some(v) = bool_value {
        map.insert("encodeUrl".into(), serde_json::to_value(v).unwrap_or_default());
    }
    let follow = settings
        .follow_redirects
        .clone()
        .or_else(|| settings.follow_redirects_camel.clone());
    if let Some(v) = follow {
        map.insert("followRedirects".into(), serde_json::to_value(v).unwrap_or_default());
    }
    let max_redirects = settings
        .max_redirects
        .clone()
        .or_else(|| settings.max_redirects_camel.clone());
    if let Some(v) = max_redirects {
        map.insert("maxRedirects".into(), serde_json::to_value(v).unwrap_or_default());
    }
    if let Some(v) = settings.timeout.clone() {
        map.insert("timeout".into(), serde_json::to_value(v).unwrap_or_default());
    }
    let mut omit = settings.omit_headers.clone();
    if omit.is_empty() {
        omit = settings.omit_headers_camel.clone();
    }
    if !omit.is_empty() {
        map.insert("omitHeaders".into(), serde_json::json!(omit));
    }
    serde_json::Value::Object(map)
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// 保存后重新解析单个请求，同步内存索引与树节点（否则重新打开还是旧内容）
pub fn reload_request(collection: &mut Collection, pathname: &str) -> Result<(), String> {
    let normalized = pathname.replace('\\', "/");
    let abs = collection.pathname.join(&normalized);
    let file = load_request_file(&abs, &collection.pathname)?;
    collection.requests.insert(normalized.clone(), file.clone());
    fn walk(nodes: &mut [TreeNode], pathname: &str, file: &RequestFile) -> bool {
        for node in nodes {
            if node.pathname == pathname {
                node.name = file.name.clone();
                node.seq = file.seq;
                node.tags = file.tags.clone();
                node.request = Some(normalize_request(&file.request));
                node.settings = if file.kind == "http-request" {
                    Some(settings_to_ui(&file.request.settings))
                } else {
                    None
                };
                return true;
            }
            if walk(&mut node.items, pathname, file) {
                return true;
            }
        }
        false
    }
    walk(&mut collection.tree, &normalized, &file);
    Ok(())
}

/// 按请求类型分发到对应的前端形状
pub fn normalize_request(req: &HttpRequest) -> serde_json::Value {
    match req.kind() {
        "ws-request" => normalize_ws_for_ui(req),
        "grpc-request" => normalize_grpc_for_ui(req),
        _ => normalize_request_for_ui(req),
    }
}

fn headers_to_ui(headers: &[HttpHeader]) -> Vec<serde_json::Value> {
    headers
        .iter()
        .map(|h| {
            serde_json::json!({
                "name": h.name,
                "value": h.value,
                "enabled": !h.disabled.unwrap_or(false),
                "description": h.description.as_ref().map(|d| d.text()).unwrap_or_default(),
            })
        })
        .collect()
}

/// 运行时公共块（变量 / 断言）
fn runtime_to_ui(req: &HttpRequest) -> (serde_json::Value, serde_json::Value) {
    let vars = serde_json::json!(req
        .runtime
        .variables
        .iter()
        .map(|v| serde_json::json!({
            "name": v.name,
            "value": v.resolved().unwrap_or_default(),
            "enabled": !v.disabled.unwrap_or(false),
        }))
        .collect::<Vec<_>>());
    let assertions = req
        .runtime
        .assertions
        .iter()
        .map(|a| {
            serde_json::json!({
                "name": a.expression,
                "value": format!("{} {}", a.operator, a.value.clone().unwrap_or_default()),
            })
        })
        .collect::<Vec<_>>();
    (vars, serde_json::json!(assertions))
}

fn script_code(req: &HttpRequest, kind: &str) -> String {
    req.runtime
        .scripts
        .iter()
        .filter(|s| s.kind.as_deref() == Some(kind))
        .map(|s| s.code.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

/// WebSocket 请求 → 前端形状（`body.mode = "ws"`，消息数组带 type/content）
pub fn normalize_ws_for_ui(req: &HttpRequest) -> serde_json::Value {
    let ws = req.websocket.clone().unwrap_or_default();
    let mut messages: Vec<serde_json::Value> = Vec::new();
    match ws.message.as_ref() {
        Some(ProtocolMessage::Text(data)) => {
            messages.push(serde_json::json!({
                "name": "message 1",
                "type": "json",
                "content": data,
            }));
        }
        Some(ProtocolMessage::Typed { kind, data }) => {
            messages.push(serde_json::json!({
                "name": "message 1",
                "type": kind.clone().unwrap_or_else(|| "json".into()),
                "content": data,
            }));
        }
        Some(ProtocolMessage::Variants(variants)) => {
            for (index, variant) in variants.iter().enumerate() {
                let message = variant.get("message").cloned().unwrap_or_default();
                messages.push(serde_json::json!({
                    "name": variant
                        .get("title")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| format!("message {}", index + 1)),
                    "type": message.get("type").and_then(|v| v.as_str()).unwrap_or("json"),
                    "content": message.get("data").and_then(|v| v.as_str()).unwrap_or(""),
                    "selected": variant.get("selected").and_then(|v| v.as_bool()).unwrap_or(false),
                }));
            }
        }
        None => {}
    }
    if messages.is_empty() {
        messages.push(serde_json::json!({ "name": "message 1", "type": "json", "content": "" }));
    }

    let (vars, _) = runtime_to_ui(req);
    serde_json::json!({
        "url": ws.url.clone().unwrap_or_default(),
        "headers": headers_to_ui(&ws.headers),
        "body": { "mode": "ws", "ws": messages },
        "auth": crate::persist::disk_auth_to_ui(ws.auth.as_ref()),
        "vars": { "req": vars },
        "script": { "req": script_code(req, "before-request"), "res": script_code(req, "after-response") },
        "tests": script_code(req, "tests"),
    })
}

/// gRPC 请求 → 前端形状（`request.method` / `methodType` / `protoPath` 都挂在 request 上）
pub fn normalize_grpc_for_ui(req: &HttpRequest) -> serde_json::Value {
    let grpc = req.grpc.clone().unwrap_or_default();
    let mut messages: Vec<serde_json::Value> = Vec::new();
    match grpc.message.as_ref() {
        Some(ProtocolMessage::Text(data)) => {
            messages.push(serde_json::json!({ "name": "message 1", "content": data }));
        }
        Some(ProtocolMessage::Typed { data, .. }) => {
            messages.push(serde_json::json!({ "name": "message 1", "content": data }));
        }
        Some(ProtocolMessage::Variants(variants)) => {
            for (index, variant) in variants.iter().enumerate() {
                let content = variant
                    .get("message")
                    .and_then(|m| {
                        if let Some(text) = m.as_str() {
                            Some(text.to_string())
                        } else {
                            m.get("data").and_then(|v| v.as_str()).map(|s| s.to_string())
                        }
                    })
                    .unwrap_or_default();
                messages.push(serde_json::json!({
                    "name": variant
                        .get("title")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| format!("message {}", index + 1)),
                    "content": content,
                }));
            }
        }
        None => {}
    }
    if messages.is_empty() {
        messages.push(serde_json::json!({ "name": "message 1", "content": "{}" }));
    }

    let (vars, assertions) = runtime_to_ui(req);
    serde_json::json!({
        "url": grpc.url.clone().unwrap_or_default(),
        "method": grpc.method.clone().unwrap_or_default(),
        "methodType": grpc.method_type.clone().unwrap_or_else(|| "unary".into()),
        "protoPath": grpc.proto_file_path.clone().unwrap_or_default(),
        "headers": headers_to_ui(&grpc.metadata),
        "body": { "mode": "grpc", "grpc": messages },
        "auth": crate::persist::disk_auth_to_ui(grpc.auth.as_ref()),
        "vars": { "req": vars },
        "script": {
            "beforeCallStart": script_code(req, "grpc:before-call-start"),
            "beforeMessageSend": script_code(req, "grpc:before-message-send"),
            "afterMessageReceive": script_code(req, "grpc:after-message-receive"),
            "afterCallEnd": script_code(req, "grpc:after-call-end"),
        },
        "tests": script_code(req, "tests"),
        "assertions": assertions,
    })
}

/// Bruno 前端期望的 request 形状：{ method, url, headers:[{name,value,enabled}], params, body:{mode,...}, auth:{mode,...} }
pub fn normalize_request_for_ui(req: &HttpRequest) -> serde_json::Value {
    let http = req.http.clone().unwrap_or_default();
    let headers: Vec<serde_json::Value> = http
        .headers
        .iter()
        .map(|h| {
            serde_json::json!({
                "name": h.name,
                "value": h.value,
                "enabled": !h.disabled.unwrap_or(false),
                "description": h.description.as_ref().map(|d| d.text()).unwrap_or_default(),
            })
        })
        .collect();
    let params: Vec<serde_json::Value> = http
        .params
        .iter()
        .map(|p| {
            serde_json::json!({
                "name": p.name,
                "value": p.value,
                "type": p.kind.clone().unwrap_or_else(|| "query".into()),
                "enabled": !p.disabled.unwrap_or(false),
            })
        })
        .collect();
    // 前端把 body.<mode> 直接当 CodeMirror 的 value 用，必须是字符串；
    // 这里统一产出「只填当前 mode 对应字段，其余为空」的形状。
    let body_value = body_to_ui(http.body.as_ref());
    let auth = crate::persist::disk_auth_to_ui(http.auth.as_ref());

    let mut value = serde_json::json!({
        "method": http.method.clone().unwrap_or_else(|| "GET".into()),
        "url": http.url.clone().unwrap_or_default(),
        "headers": headers,
        "params": params,
        "body": body_value,
        "auth": auth,
        "vars": {
            "req": req.runtime.variables.iter().map(|v| serde_json::json!({
                "name": v.name,
                "value": v.resolved().unwrap_or_default(),
                "enabled": !v.disabled.unwrap_or(false),
            })).collect::<Vec<_>>()
        },
        "script": {
            "req": req.runtime.scripts.iter().filter(|s| s.kind.as_deref() == Some("before-request")).map(|s| s.code.clone()).collect::<Vec<_>>().join("\n"),
            "res": req.runtime.scripts.iter().filter(|s| s.kind.as_deref() == Some("after-response")).map(|s| s.code.clone()).collect::<Vec<_>>().join("\n"),
        },
        "tests": req.runtime.scripts.iter().filter(|s| s.kind.as_deref() == Some("tests")).map(|s| s.code.clone()).collect::<Vec<_>>().join("\n"),
        "assertions": req.runtime.assertions.iter().map(|a| serde_json::json!({
            "name": a.expression,
            "value": format!("{} {}", a.operator, a.value.clone().unwrap_or_default()),
        })).collect::<Vec<_>>(),
    });
    // 变体 body 保留原始结构，编辑/保存时直接回填
    if let Some(BodyData::Variants(variants)) = &http.body {
        value["body"] = serde_json::json!({ "mode": "variants", "variants": variants });
    }
    value
}

/// OpenCollection body → Bruno UI body（每种 mode 的字段都是字符串/数组）
pub fn body_to_ui(body: Option<&BodyData>) -> serde_json::Value {
    let mut base = serde_json::json!({
        "mode": "none",
        "json": "",
        "text": "",
        "xml": "",
        "sparql": "",
        "formUrlEncoded": [],
        "multipartForm": [],
        "file": [],
    });
    let Some(body) = body else { return base };
    let raw = serde_json::to_value(body).unwrap_or_default();
    let kind = raw.get("type").and_then(|v| v.as_str()).unwrap_or("none");
    match kind {
        "json" | "text" | "xml" | "sparql" => {
            base["mode"] = serde_json::json!(kind);
            base[kind] = serde_json::json!(raw.get("data").and_then(|v| v.as_str()).unwrap_or(""));
        }
        "form-urlencoded" | "formUrlEncoded" => {
            base["mode"] = serde_json::json!("formUrlEncoded");
            base["formUrlEncoded"] = serde_json::json!(raw
                .get("data")
                .and_then(|v| v.as_array())
                .map(|arr| arr
                    .iter()
                    .map(|e| serde_json::json!({
                        "name": e.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                        "value": e.get("value").and_then(|v| v.as_str()).unwrap_or(""),
                        "enabled": e.get("disabled").and_then(|v| v.as_bool()) != Some(true),
                    }))
                    .collect::<Vec<_>>())
                .unwrap_or_default());
        }
        "multipart-form" | "multipartForm" => {
            base["mode"] = serde_json::json!("multipartForm");
            base["multipartForm"] = serde_json::json!(raw
                .get("data")
                .and_then(|v| v.as_array())
                .map(|arr| arr
                    .iter()
                    .map(|e| {
                        let entry_type = e.get("type").and_then(|v| v.as_str()).unwrap_or("text");
                        let value = e.get("value").cloned().unwrap_or_else(|| {
                            if entry_type == "file" { serde_json::json!([]) } else { serde_json::json!("") }
                        });
                        serde_json::json!({
                            "type": entry_type,
                            "name": e.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                            "value": value,
                            "contentType": e.get("contentType").and_then(|v| v.as_str()).unwrap_or(""),
                            "enabled": e.get("disabled").and_then(|v| v.as_bool()) != Some(true),
                        })
                    })
                    .collect::<Vec<_>>())
                .unwrap_or_default());
        }
        "file" => {
            base["mode"] = serde_json::json!("file");
            base["file"] = serde_json::json!(raw
                .get("data")
                .and_then(|v| v.as_array())
                .map(|arr| arr
                    .iter()
                    .map(|e| serde_json::json!({
                        "filePath": e.get("filePath").and_then(|v| v.as_str()).unwrap_or(""),
                        "contentType": e.get("contentType").and_then(|v| v.as_str()).unwrap_or(""),
                        "selected": e.get("selected").and_then(|v| v.as_bool()).unwrap_or(false),
                        "description": e.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                    }))
                    .collect::<Vec<_>>())
                .unwrap_or_default());
        }
        _ => {}
    }
    base
}
