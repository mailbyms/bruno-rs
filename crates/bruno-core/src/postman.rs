//! Postman Collection v2.0 / v2.1 → Bruno 内存集合。
//!
//! 原版把这一步放在 Electron 主进程（`@usebruno/converters`，依赖 jscodeshift 等
//! Node 专用包）。这里用 Rust 重写常用子集：
//! - item 树（文件夹 / 请求）、header、query / path 参数
//! - body：raw(json/xml/text)、urlencoded、formdata、file、graphql
//! - auth：basic / bearer / apikey / digest / awsv4 / ntlm / oauth2（部分）/ none
//! - 事件脚本：prerequest → script.req，test → tests（`pm.*` 做常见映射）
//! - 集合变量、集合级 auth 与脚本
//!
//! 不追求 100% 复刻原版：无法映射的项统一记入 `issues`，由前端提示用户。

use crate::model::new_uid;
use serde_json::{json, Value as J};

/// Postman 集合 → `{ collection, issues }`（与前端 `postmanToBruno` 返回形状一致）
pub fn postman_to_bruno(input: &J) -> Result<J, String> {
    let root = input
        .get("collection")
        .filter(|v| v.is_object())
        .unwrap_or(input);
    let info = root.get("info").cloned().unwrap_or_else(|| json!({}));
    let name = info
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "不是 Postman 集合：缺少 info.name".to_string())?;

    let mut issues: Vec<J> = Vec::new();

    let items = convert_items(root.get("item"), &mut issues);
    let vars = convert_variables(root.get("variable"));
    let (pre, tests) = convert_events(root.get("event"), &mut issues);
    let auth = convert_auth(root.get("auth"), &mut issues);

    let collection = json!({
        "uid": new_uid(),
        "name": name,
        "version": "1",
        "type": "collection",
        "items": items,
        "root": {
            "request": {
                "headers": [],
                "auth": auth,
                "vars": { "req": vars, "res": [] },
                "script": { "req": pre, "res": "" },
                "tests": tests
            }
        },
        "brunoConfig": {
            "version": "1",
            "name": name,
            "type": "collection",
            "ignore": ["node_modules", ".git"]
        }
    });

    Ok(json!({ "collection": collection, "issues": issues }))
}

// ---------------------------------------------------------------------------
// item 树
// ---------------------------------------------------------------------------

fn convert_items(value: Option<&J>, issues: &mut Vec<J>) -> Vec<J> {
    let Some(list) = value.and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (idx, node) in list.iter().enumerate() {
        let seq = idx as i64 + 1;
        if node.get("item").and_then(|v| v.as_array()).is_some() {
            out.push(convert_folder(node, seq, issues));
        } else if node.get("request").is_some() {
            out.push(convert_request(node, seq, issues));
        } else {
            issues.push(issue(
                &format!("item[{}]", idx),
                "既不是文件夹也没有 request，已跳过",
                node.get("name").and_then(|v| v.as_str()),
            ));
        }
    }
    out
}

fn convert_folder(node: &J, seq: i64, issues: &mut Vec<J>) -> J {
    let name = node_name(node, "Folder");
    let items = convert_items(node.get("item"), issues);
    let (pre, tests) = convert_events(node.get("event"), issues);
    let auth = node
        .get("auth")
        .map(|a| convert_auth(Some(a), issues))
        .unwrap_or_else(|| json!({ "mode": "inherit" }));
    let vars = convert_variables(node.get("variable"));

    // 注意：文件夹对象不能带 `request` 键，否则前端 isItemAFolder 判定失败
    json!({
        "uid": new_uid(),
        "name": name,
        "type": "folder",
        "seq": seq,
        "items": items,
        "root": {
            "request": {
                "headers": [],
                "auth": auth,
                "vars": { "req": vars, "res": [] },
                "script": { "req": pre, "res": "" },
                "tests": tests
            }
        }
    })
}

fn convert_request(node: &J, seq: i64, issues: &mut Vec<J>) -> J {
    let name = node_name(node, "Untitled Request");
    let req = node.get("request").cloned().unwrap_or_else(|| json!({}));

    // request 允许是纯字符串（Postman 简写：只给 URL）
    let (method, url_value) = match &req {
        J::String(url) => ("GET".to_string(), json!(url)),
        _ => (
            req.get("method")
                .and_then(|v| v.as_str())
                .unwrap_or("GET")
                .to_uppercase(),
            req.get("url").cloned().unwrap_or_else(|| json!("")),
        ),
    };

    let (url, mut params) = convert_url(&url_value, issues);
    let headers = convert_headers(req.get("header"));
    let body = convert_body(req.get("body"), issues);
    let auth = convert_auth(req.get("auth"), issues);
    let vars = convert_variables(req.get("variable"));
    let (pre, tests) = convert_events(node.get("event"), issues);

    // path 变量单独补进 params
    if let Some(vars) = url_value.get("variable").and_then(|v| v.as_array()) {
        for v in vars {
            let Some(key) = v.get("key").and_then(|x| x.as_str()) else {
                continue;
            };
            if key.is_empty() {
                continue;
            }
            params.push(json!({
                "name": key,
                "value": v.get("value").and_then(|x| x.as_str()).unwrap_or(""),
                "type": "path",
                "enabled": v.get("disabled").and_then(|x| x.as_bool()) != Some(true)
            }));
        }
    }

    let mut request = json!({
        "method": method,
        "url": url,
        "headers": headers,
        "params": params,
        "body": body,
        "auth": auth,
        "vars": { "req": vars, "res": [] },
        "script": { "req": pre, "res": "" },
        "tests": tests,
        "assertions": [],
        "docs": text_of(node.get("description")).unwrap_or_default()
    });

    if let Some(desc) = text_of(req.get("description")) {
        if !desc.trim().is_empty() {
            request["docs"] = json!(desc);
        }
    }

    json!({
        "uid": new_uid(),
        "name": name,
        "type": "http-request",
        "seq": seq,
        "request": request,
        "settings": {
            "encodeUrl": true,
            "timeout": 0,
            "followRedirects": true,
            "maxRedirects": 10
        }
    })
}

fn node_name(node: &J, fallback: &str) -> String {
    node.get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

// ---------------------------------------------------------------------------
// URL / header
// ---------------------------------------------------------------------------

/// 返回 (不含 query 的 url, params)。query 统一放进 params，避免发送时重复。
fn convert_url(url: &J, issues: &mut Vec<J>) -> (String, Vec<J>) {
    let mut params = Vec::new();

    let raw = match url {
        J::String(s) => s.clone(),
        J::Object(_) => {
            let raw = url.get("raw").and_then(|v| v.as_str()).map(|s| s.to_string());
            raw.unwrap_or_else(|| {
                let host = url
                    .get("host")
                    .and_then(|v| v.as_array())
                    .map(|arr| join_strs(arr))
                    .unwrap_or_default();
                let path = url
                    .get("path")
                    .and_then(|v| v.as_array())
                    .map(|arr| join_strs(arr))
                    .unwrap_or_default();
                let protocol = url.get("protocol").and_then(|v| v.as_str()).unwrap_or("https");
                if path.is_empty() {
                    format!("{protocol}://{host}")
                } else {
                    format!("{protocol}://{host}/{path}")
                }
            })
        }
        _ => String::new(),
    };

    if let Some(query) = url.get("query").and_then(|v| v.as_array()) {
        for q in query {
            let Some(key) = q.get("key").and_then(|v| v.as_str()) else {
                continue;
            };
            if key.is_empty() {
                continue;
            }
            params.push(json!({
                "name": key,
                "value": q.get("value").and_then(|v| v.as_str()).unwrap_or(""),
                "type": "query",
                "enabled": q.get("disabled").and_then(|v| v.as_bool()) != Some(true)
            }));
        }
    }

    let mut base = raw;
    if let Some(pos) = base.find('?') {
        // url.query 缺失时，把 raw 里的 query 拆出来，避免重复
        if params.is_empty() {
            let query = base[pos + 1..].to_string();
            for pair in query.split('&') {
                if pair.is_empty() {
                    continue;
                }
                let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                params.push(json!({
                    "name": urldecode(k),
                    "value": urldecode(v),
                    "type": "query",
                    "enabled": true
                }));
            }
        }
        base.truncate(pos);
    }
    if base.is_empty() {
        issues.push(issue("request.url", "URL 为空", None));
    }
    (base, params)
}

fn join_strs(arr: &[J]) -> String {
    arr.iter()
        .filter_map(|v| v.as_str())
        .collect::<Vec<_>>()
        .join("/")
}

fn urldecode(s: &str) -> String {
    percent_encoding::percent_decode_str(s)
        .decode_utf8_lossy()
        .to_string()
}

fn convert_headers(value: Option<&J>) -> Vec<J> {
    value
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|h| {
                    let key = h.get("key").and_then(|v| v.as_str())?;
                    if key.is_empty() {
                        return None;
                    }
                    Some(json!({
                        "name": key,
                        "value": h.get("value").and_then(|v| v.as_str()).unwrap_or(""),
                        "description": text_of(h.get("description")).unwrap_or_default(),
                        "enabled": h.get("disabled").and_then(|v| v.as_bool()) != Some(true)
                    }))
                })
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// body
// ---------------------------------------------------------------------------

fn convert_body(value: Option<&J>, issues: &mut Vec<J>) -> J {
    let Some(body) = value else {
        return empty_body();
    };
    let mode = body.get("mode").and_then(|v| v.as_str()).unwrap_or("none");
    let mut out = empty_body();

    match mode {
        "raw" => {
            let raw = body.get("raw").and_then(|v| v.as_str()).unwrap_or("");
            let language = body
                .get("options")
                .and_then(|o| o.get("raw"))
                .and_then(|r| r.get("language"))
                .and_then(|l| l.as_str())
                .unwrap_or("");
            let target = match language {
                "json" => "json",
                "xml" => "xml",
                _ => "text",
            };
            out["mode"] = json!(target);
            out[target] = json!(raw);
        }
        "urlencoded" => {
            out["mode"] = json!("formUrlEncoded");
            out["formUrlEncoded"] = json!(body
                .get("urlencoded")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|e| {
                            let key = e.get("key").and_then(|v| v.as_str())?;
                            if key.is_empty() {
                                return None;
                            }
                            Some(json!({
                                "name": key,
                                "value": e.get("value").and_then(|v| v.as_str()).unwrap_or(""),
                                "enabled": e.get("disabled").and_then(|v| v.as_bool()) != Some(true)
                            }))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default());
        }
        "formdata" => {
            out["mode"] = json!("multipartForm");
            out["multipartForm"] = json!(body
                .get("formdata")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|e| {
                            let key = e.get("key").and_then(|v| v.as_str())?;
                            if key.is_empty() {
                                return None;
                            }
                            let is_file = e.get("type").and_then(|v| v.as_str()) == Some("file");
                            if is_file {
                                let path = e
                                    .get("src")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                if path.is_empty() {
                                    issues.push(issue(
                                        &format!("body.formdata.{key}"),
                                        "文件字段没有 src 路径，已导入为空值",
                                        Some(key),
                                    ));
                                }
                                Some(json!({
                                    "name": key,
                                    "type": "file",
                                    "value": [path],
                                    "enabled": e.get("disabled").and_then(|v| v.as_bool()) != Some(true)
                                }))
                            } else {
                                Some(json!({
                                    "name": key,
                                    "type": "text",
                                    "value": e.get("value").and_then(|v| v.as_str()).unwrap_or(""),
                                    "contentType": e.get("contentType").and_then(|v| v.as_str()).unwrap_or(""),
                                    "enabled": e.get("disabled").and_then(|v| v.as_bool()) != Some(true)
                                }))
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default());
        }
        "file" => {
            out["mode"] = json!("file");
            let src = body.get("file").and_then(|f| f.get("src"));
            let path = match src {
                Some(J::String(s)) => s.clone(),
                Some(J::Array(arr)) => arr
                    .first()
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                _ => String::new(),
            };
            if path.is_empty() {
                issues.push(issue("body.file", "binary body 没有 src 路径", None));
            }
            out["file"] = json!([{
                "filePath": path,
                "contentType": "",
                "selected": true
            }]);
        }
        "graphql" => {
            let g = body.get("graphql").cloned().unwrap_or_else(|| json!({}));
            let query = g.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let variables = g.get("variables").and_then(|v| v.as_str()).unwrap_or("");
            if variables.trim().is_empty() {
                out["mode"] = json!("json");
                out["json"] = json!(query);
            } else {
                out["mode"] = json!("json");
                out["json"] = json!(format!(
                    "{{\n  \"query\": {},\n  \"variables\": {}\n}}",
                    json!(query),
                    variables
                ));
            }
            issues.push(issue(
                "body.graphql",
                "GraphQL 请求体已转成 JSON 文本，发送前请确认格式",
                None,
            ));
        }
        "none" | "" => {}
        other => {
            issues.push(issue(
                "body",
                &format!("不支持的 Postman body 模式：{other}"),
                None,
            ));
        }
    }
    out
}

fn empty_body() -> J {
    json!({
        "mode": "none",
        "json": "",
        "text": "",
        "xml": "",
        "sparql": "",
        "formUrlEncoded": [],
        "multipartForm": [],
        "file": []
    })
}

// ---------------------------------------------------------------------------
// auth
// ---------------------------------------------------------------------------

fn convert_auth(value: Option<&J>, issues: &mut Vec<J>) -> J {
    let Some(auth) = value else {
        return json!({ "mode": "none" });
    };
    let kind = auth.get("type").and_then(|v| v.as_str()).unwrap_or("noauth");
    let params = auth
        .get(kind)
        .cloned()
        .unwrap_or_else(|| auth.clone());

    // Postman: [{key, value, type}] → {k: v}
    let field = |name: &str| -> String {
        params
            .as_array()
            .and_then(|arr| {
                arr.iter()
                    .find(|p| p.get("key").and_then(|k| k.as_str()) == Some(name))
                    .and_then(|p| p.get("value"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .or_else(|| {
                params
                    .get(name)
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_default()
    };

    match kind {
        "noauth" | "" => json!({ "mode": "none" }),
        "basic" => json!({
            "mode": "basic",
            "basic": { "username": field("username"), "password": field("password") }
        }),
        "bearer" => json!({
            "mode": "bearer",
            "bearer": { "token": field("token") }
        }),
        "apikey" => {
            let location = field("in");
            json!({
                "mode": "apikey",
                "apikey": {
                    "key": field("key"),
                    "value": field("value"),
                    "placement": if location == "query" { "queryparams" } else { "header" }
                }
            })
        }
        "digest" => json!({
            "mode": "digest",
            "digest": { "username": field("username"), "password": field("password") }
        }),
        "ntlm" => json!({
            "mode": "ntlm",
            "ntlm": { "username": field("username"), "password": field("password"), "domain": field("domain") }
        }),
        "awsv4" => json!({
            "mode": "awsv4",
            "awsv4": {
                "accessKeyId": field("accessKey"),
                "secretAccessKey": field("secretKey"),
                "sessionToken": field("sessionToken"),
                "service": field("service"),
                "region": field("region")
            }
        }),
        "oauth2" => {
            issues.push(issue(
                "request.auth",
                "OAuth2 已按 client_credentials 映射，其它授权方式需在界面里补全",
                None,
            ));
            json!({
                "mode": "oauth2",
                "oauth2": {
                    "grantType": match field("grant_type").as_str() {
                        "password" => "password_credentials",
                        "client_credentials" => "client_credentials",
                        _ => "client_credentials"
                    },
                    "accessTokenUrl": field("accessTokenUrl"),
                    "clientId": field("clientId"),
                    "clientSecret": field("clientSecret"),
                    "scope": field("scope"),
                    "credentialsPlacement": "body",
                    "tokenPlacement": "header",
                    "tokenHeaderPrefix": "Bearer",
                    "tokenQueryKey": "access_token"
                }
            })
        }
        other => {
            issues.push(issue(
                "request.auth",
                &format!("不支持的 Postman 认证类型：{other}（已置为 none）"),
                None,
            ));
            json!({ "mode": "none" })
        }
    }
}

// ---------------------------------------------------------------------------
// 变量 / 脚本
// ---------------------------------------------------------------------------

fn convert_variables(value: Option<&J>) -> Vec<J> {
    value
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    let name = v.get("key").and_then(|x| x.as_str())?;
                    if name.is_empty() {
                        return None;
                    }
                    Some(json!({
                        "name": name,
                        "value": scalar_to_string(v.get("value")),
                        "enabled": v.get("disabled").and_then(|x| x.as_bool()) != Some(true)
                    }))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn scalar_to_string(value: Option<&J>) -> String {
    match value {
        Some(J::String(s)) => s.clone(),
        Some(J::Bool(b)) => b.to_string(),
        Some(J::Number(n)) => n.to_string(),
        Some(J::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// Postman 事件 → (before-request, tests)
fn convert_events(value: Option<&J>, issues: &mut Vec<J>) -> (String, String) {
    let Some(events) = value.and_then(|v| v.as_array()) else {
        return (String::new(), String::new());
    };
    let mut pre = Vec::new();
    let mut tests = Vec::new();
    for event in events {
        let listen = event.get("listen").and_then(|v| v.as_str()).unwrap_or("");
        let Some(exec) = event
            .get("script")
            .and_then(|s| s.get("exec"))
            .and_then(|v| v.as_array())
        else {
            continue;
        };
        let code = exec
            .iter()
            .filter_map(|line| line.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if code.trim().is_empty() {
            continue;
        }
        let translated = translate_pm_script(&code);
        match listen {
            "prerequest" => pre.push(translated),
            "test" => tests.push(translated),
            other => {
                issues.push(issue(
                    "event",
                    &format!("未识别的 Postman 事件监听器：{other}（已按 test 处理）"),
                    None,
                ));
                tests.push(translated);
            }
        }
    }
    let pre_code = pre.join("\n");
    let test_code = tests.join("\n");
    if pre_code.contains("pm.") || test_code.contains("pm.") {
        issues.push(issue(
            "event",
            "脚本里仍有未映射的 pm.* 调用，需要手工调整后才能运行",
            None,
        ));
    }
    (pre_code, test_code)
}

/// `pm.*` 常见调用 → Bruno 脚本 API。只做文本级替换，复杂用法留给用户。
pub fn translate_pm_script(code: &str) -> String {
    let mut out = code.to_string();
    let rules: &[(&str, &str)] = &[
        ("pm.response.code", "res.getStatus()"),
        ("pm.response.status", "res.getStatusText()"),
        ("pm.response.responseTime", "res.getResponseTime()"),
        ("pm.response.text()", "res.getBodyText()"),
        ("pm.response.json()", "res.getBody()"),
        ("pm.response.headers.get(", "res.getHeader("),
        ("pm.environment.set(", "bru.setVar("),
        ("pm.environment.get(", "bru.getVar("),
        ("pm.collectionVariables.set(", "bru.setVar("),
        ("pm.collectionVariables.get(", "bru.getVar("),
        ("pm.variables.set(", "bru.setVar("),
        ("pm.variables.get(", "bru.getVar("),
        ("pm.expect(", "expect("),
        ("pm.test(", "test("),
        ("pm.sendRequest(", "bru.sendRequest("),
        ("pm.iterationData.get(", "bru.getVar("),
    ];
    for (from, to) in rules {
        out = out.replace(from, to);
    }
    out
}

fn issue(path: &str, message: &str, source: Option<&str>) -> J {
    json!({
        "severity": "warning",
        "path": path,
        "message": message,
        "sourceItem": source.unwrap_or("")
    })
}

// ---------------------------------------------------------------------------
// description 可能是字符串或 { content, type }
// ---------------------------------------------------------------------------

fn text_of(value: Option<&J>) -> Option<String> {
    match value {
        Some(J::String(s)) => Some(s.clone()),
        Some(J::Object(_)) => value
            .and_then(|v| v.get("content"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> J {
        json!({
            "info": {
                "name": "Shop API",
                "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
            },
            "auth": { "type": "bearer", "bearer": [{ "key": "token", "value": "{{token}}" }] },
            "variable": [{ "key": "baseUrl", "value": "http://127.0.0.1:18081" }],
            "item": [
                {
                    "name": "Orders",
                    "item": [
                        {
                            "name": "List Orders",
                            "request": {
                                "method": "GET",
                                "url": {
                                    "raw": "{{baseUrl}}/orders?limit=10&page=2",
                                    "host": ["{{baseUrl}}"],
                                    "path": ["orders"],
                                    "query": [
                                        { "key": "limit", "value": "10" },
                                        { "key": "page", "value": "2", "disabled": true }
                                    ]
                                },
                                "header": [
                                    { "key": "Accept", "value": "application/json" },
                                    { "key": "X-Skip", "value": "1", "disabled": true }
                                ]
                            },
                            "event": [{
                                "listen": "test",
                                "script": { "exec": [
                                    "pm.test('ok', function () {",
                                    "  pm.expect(pm.response.code).to.equal(200);",
                                    "});"
                                ] }
                            }]
                        }
                    ]
                },
                {
                    "name": "Create Order",
                    "request": {
                        "method": "POST",
                        "url": "{{baseUrl}}/orders",
                        "body": {
                            "mode": "raw",
                            "raw": "{\"item\":\"coffee\"}",
                            "options": { "raw": { "language": "json" } }
                        },
                        "auth": {
                            "type": "apikey",
                            "apikey": [
                                { "key": "key", "value": "X-Api-Key" },
                                { "key": "value", "value": "secret" },
                                { "key": "in", "value": "query" }
                            ]
                        }
                    }
                }
            ]
        })
    }

    #[test]
    fn converts_tree_headers_body_and_scripts() {
        let out = postman_to_bruno(&sample()).expect("convert");
        let c = &out["collection"];
        assert_eq!(c["name"], "Shop API");
        assert_eq!(c["items"].as_array().unwrap().len(), 2);

        // 文件夹不能带 request 键
        let folder = &c["items"][0];
        assert_eq!(folder["type"], "folder");
        assert!(folder.get("request").is_none());

        let list = &folder["items"][0];
        assert_eq!(list["type"], "http-request");
        assert!(list.get("items").is_none());
        let req = &list["request"];
        assert_eq!(req["url"], "{{baseUrl}}/orders");
        assert_eq!(req["params"].as_array().unwrap().len(), 2);
        assert_eq!(req["params"][1]["enabled"], false);
        assert_eq!(req["headers"][1]["enabled"], false);
        assert!(req["tests"].as_str().unwrap().contains("test('ok'"));
        assert!(req["tests"].as_str().unwrap().contains("res.getStatus()"));

        let create = &c["items"][1]["request"];
        assert_eq!(create["body"]["mode"], "json");
        assert_eq!(create["body"]["json"], "{\"item\":\"coffee\"}");
        assert_eq!(create["auth"]["mode"], "apikey");
        assert_eq!(create["auth"]["apikey"]["placement"], "queryparams");

        // 集合级认证与变量
        assert_eq!(c["root"]["request"]["auth"]["mode"], "bearer");
        assert_eq!(
            c["root"]["request"]["vars"]["req"][0]["name"],
            "baseUrl"
        );
    }

    #[test]
    fn splits_query_from_raw_url() {
        let mut issues = Vec::new();
        let (url, params) = convert_url(&json!("http://x.dev/a?b=1&c=hello%20world"), &mut issues);
        assert_eq!(url, "http://x.dev/a");
        assert_eq!(params.len(), 2);
        assert_eq!(params[1]["value"], "hello world");
    }

    #[test]
    fn translates_common_pm_calls() {
        let code = translate_pm_script(
            "pm.test('a', () => pm.expect(pm.response.code).to.equal(200)); pm.environment.set('x','1');",
        );
        assert!(code.contains("test('a'"));
        assert!(code.contains("expect(res.getStatus())"));
        assert!(code.contains("bru.setVar('x','1')"));
        assert!(!code.contains("pm."));
    }
}
