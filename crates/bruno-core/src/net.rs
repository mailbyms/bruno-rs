//! WS / gRPC 共用的请求头与认证解析。
//!
//! HTTP 引擎（`runner.rs`）里这套逻辑是内联的，因为它要和 digest 重试、awsv4
//! 签名耦合。WS / gRPC 只需要「最终要发的头 + query」，所以单独抽一份，
//! 保持与 HTTP 引擎同样的优先级：集合/文件夹头 → 请求头 → 认证头。

use crate::interpolate::{interpolate, Vars};
use crate::model::{Auth, AuthTyped};
use serde_json::Value;

#[derive(Debug, Default)]
pub struct AuthOutcome {
    /// 需要附加到请求头 / metadata 的键值
    pub headers: Vec<(String, String)>,
    /// apikey 放在 query 时用
    pub query: Vec<(String, String)>,
    /// 不支持的认证方式等提示，前端按脚本错误展示
    pub errors: Vec<String>,
}

/// 与 HTTP 引擎同样的变量分层：globalEnv < collection < environment < request < runtime
pub fn build_vars(
    collection: &Value,
    environment: Option<&Value>,
    request: Option<&Value>,
    runtime: Option<&Value>,
) -> Vars {
    let mut layers: Vec<Vars> = Vec::new();
    if let Some(global) = collection.get("globalEnvironmentVariables") {
        layers.push(crate::runner::json_vars(Some(global)));
    }
    if let Some(root) = collection.get("root") {
        let holder = root.get("request").unwrap_or(root);
        let vars = holder
            .get("vars")
            .and_then(|v| v.get("req").or(Some(v)))
            .or_else(|| holder.get("variables"));
        layers.push(crate::runner::json_vars(vars));
    }
    if let Some(env) = environment {
        layers.push(crate::runner::json_vars(env.get("variables")));
    }
    if let Some(req) = request {
        if let Some(vars) = req.get("vars").and_then(|v| v.get("req")) {
            layers.push(crate::runner::json_vars(Some(vars)));
        }
    }
    if let Some(runtime) = runtime {
        layers.push(crate::runner::json_vars(Some(runtime)));
    }
    crate::interpolate::merge_vars(&layers)
}

/// 合并集合根、文件夹、请求三级 headers（后者覆盖前者，大小写不敏感去重）
pub fn merged_headers(request: &Value, collection: &Value, vars: &Vars) -> Vec<(String, String)> {
    fn collect(value: Option<&Value>, vars: &Vars, out: &mut Vec<(String, String)>) {
        let Some(list) = value.and_then(|v| v.as_array()) else {
            return;
        };
        for h in list {
            if h.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
                continue;
            }
            let Some(name) = h.get("name").and_then(|v| v.as_str()) else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            let value = interpolate(h.get("value").and_then(|v| v.as_str()).unwrap_or(""), vars);
            upsert(out, name, &value);
        }
    }

    fn upsert(out: &mut Vec<(String, String)>, name: &str, value: &str) {
        if let Some(slot) = out.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
            slot.1 = value.to_string();
        } else {
            out.push((name.to_string(), value.to_string()));
        }
    }

    let mut out: Vec<(String, String)> = Vec::new();
    // 集合根：内存集合是 root.request.headers，树 payload 是 root.headers
    let root = collection.get("root");
    let root_headers = root
        .and_then(|r| r.get("request"))
        .and_then(|r| r.get("headers"))
        .or_else(|| root.and_then(|r| r.get("headers")));
    collect(root_headers, vars, &mut out);
    collect(request.get("headers"), vars, &mut out);
    out
}

/// 认证 → 头/query。与 HTTP 引擎一致地支持 basic / bearer / apikey / wsse / oauth2，
/// 其余（digest / awsv4 / ntlm / oauth1 / edgegrid）在 WS / gRPC 下没有对应语义，返回提示。
pub async fn resolve_auth(auth: Option<&Value>, vars: &Vars) -> AuthOutcome {
    let mut out = AuthOutcome::default();
    let Some(auth) = auth else { return out };
    let mode = auth.get("mode").and_then(|v| v.as_str()).unwrap_or("none");
    if mode == "none" || mode == "inherit" {
        return out;
    }
    let Some(parsed) = crate::runner::parse_ui_auth(auth) else {
        return out;
    };
    let Auth::Typed(typed) = parsed else {
        return out;
    };

    match typed {
        AuthTyped::Basic { username, password } => {
            use base64::Engine;
            let u = interpolate(username.as_deref().unwrap_or(""), vars);
            let p = interpolate(password.as_deref().unwrap_or(""), vars);
            let token = base64::engine::general_purpose::STANDARD.encode(format!("{u}:{p}"));
            out.headers.push(("authorization".into(), format!("Basic {token}")));
        }
        AuthTyped::Bearer { token } => {
            let t = interpolate(token.as_deref().unwrap_or(""), vars);
            out.headers.push(("authorization".into(), format!("Bearer {t}")));
        }
        AuthTyped::ApiKey { key, value, placement } => {
            let k = interpolate(key.as_deref().unwrap_or(""), vars);
            let v = interpolate(value.as_deref().unwrap_or(""), vars);
            if placement.as_deref() == Some("query") {
                out.query.push((k, v));
            } else {
                out.headers.push((k, v));
            }
        }
        AuthTyped::Wsse { username, password } => {
            let u = interpolate(username.as_deref().unwrap_or(""), vars);
            let p = interpolate(password.as_deref().unwrap_or(""), vars);
            if let Some(header) = crate::runner::wsse_header(&u, &p) {
                out.headers.push(("x-wsse".into(), header));
            }
        }
        AuthTyped::OAuth2(cfg) => {
            let client = match reqwest::Client::builder().build() {
                Ok(c) => c,
                Err(e) => {
                    out.errors.push(format!("OAuth2：创建 HTTP 客户端失败：{e}"));
                    return out;
                }
            };
            match crate::runner::fetch_oauth2_token(&client, &cfg, vars).await {
                Ok(token) => {
                    let placement = cfg
                        .get("tokenPlacement")
                        .and_then(|v| v.as_str())
                        .unwrap_or("header");
                    if placement == "url" {
                        let key = cfg
                            .get("tokenQueryKey")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .unwrap_or("access_token");
                        out.query.push((key.to_string(), token));
                    } else {
                        let prefix = cfg
                            .get("tokenHeaderPrefix")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .unwrap_or("Bearer");
                        out.headers.push(("authorization".into(), format!("{prefix} {token}")));
                    }
                }
                Err(e) => out.errors.push(format!("OAuth2 获取 token 失败：{e}")),
            }
        }
        _ => out.errors.push(format!(
            "认证方式 `{mode}` 在 WebSocket / gRPC 请求中不支持，已跳过"
        )),
    }
    out
}
