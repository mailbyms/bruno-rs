//! HTTP 请求执行引擎：脚本 → 插值 → 认证 → reqwest 发送 → 响应规范化。
//!
//! 覆盖：
//! - 认证：basic / bearer / apikey / digest（401 挑战重试）/ awsv4 / wsse /
//!   oauth2（client_credentials、password、refresh_token）；ntlm / oauth1 / edgegrid 仍为占位
//! - 请求体：json / text / xml / sparql / form-urlencoded / multipart-form / file
//! - 脚本：before-request（可改请求）、after-response、tests（QuickJS，见 `scripts`）

use crate::interpolate::{interpolate, run_assertions, AssertionResult, Vars};
use crate::model::{Auth, AuthTyped, HttpRequest};
use crate::scripts::{error_result, run_script, ScriptTestResult};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendRequestInput {
    pub item: serde_json::Value,
    pub collection: serde_json::Value,
    pub environment: Option<serde_json::Value>,
    pub variables: Option<serde_json::Value>,
    /// 由命令层按偏好设置 + cookie jar 填好；缺省时只读 item.settings
    #[serde(default)]
    pub options: Option<RequestOptions>,
}

/// 请求级运行参数（偏好设置、cookie、代理、客户端证书）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestOptions {
    /// 毫秒；0 表示不限时
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub follow_redirects: Option<bool>,
    #[serde(default)]
    pub max_redirects: Option<u32>,
    #[serde(default)]
    pub verify_tls: Option<bool>,
    /// proxy URL，如 http://127.0.0.1:7890
    #[serde(default)]
    pub proxy: Option<String>,
    #[serde(default)]
    pub client_cert_pem: Option<String>,
    #[serde(default)]
    pub client_key_pem: Option<String>,
    /// 由 cookie jar 生成的 Cookie 头
    #[serde(default)]
    pub cookie_header: Option<String>,
    /// 按最终 URL 动态取 Cookie 头（插值后才知道真实 host，Runner 里每请求都要重算）。
    /// 不参与序列化，只在本进程内传递。
    #[serde(skip)]
    pub cookie_hook: CookieHook,
}

/// `Arc<dyn Fn>` 的 Debug 包装，让 `RequestOptions` 仍可 derive Debug
#[derive(Clone, Default)]
pub struct CookieHook(pub Option<std::sync::Arc<dyn Fn(&str) -> Option<String> + Send + Sync>>);

impl std::fmt::Debug for CookieHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() {
            "CookieHook(<fn>)"
        } else {
            "CookieHook(null)"
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutedResponse {
    pub state: String,
    pub status: u16,
    pub status_text: String,
    pub data: serde_json::Value,
    pub data_buffer: String,
    pub headers: serde_json::Value,
    pub size: usize,
    pub duration: u64,
    pub timeline: Vec<serde_json::Value>,
    pub stream: bool,
    pub request_sent: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub assertion_results: Vec<AssertionResult>,
    /// tests 脚本（`test()`）结果
    pub test_results: Vec<ScriptTestResult>,
    /// before-request 脚本里的 `test()`
    pub pre_request_test_results: Vec<ScriptTestResult>,
    /// after-response 脚本里的 `test()`
    pub post_response_test_results: Vec<ScriptTestResult>,
    /// 脚本 `bru.setVar` / `req.setVar` 的增量，前端合并进 runtimeVariables
    pub runtime_variables: HashMap<String, String>,
    pub script_logs: Vec<String>,
    pub script_errors: Vec<String>,
    /// 响应的 Set-Cookie，由命令层写入 cookie jar
    #[serde(default)]
    pub set_cookies: Vec<String>,
}

/// 结构化请求：为 digest 重试与 awsv4 签名提供可重建的中间形态
#[derive(Debug, Clone)]
struct Prepared {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: BodyPayload,
}

#[derive(Debug, Clone)]
enum BodyPayload {
    None,
    Bytes(Vec<u8>),
    Form(Vec<(String, String)>),
    Multipart(Vec<PartSpec>),
}

#[derive(Debug, Clone)]
struct PartSpec {
    name: String,
    text: Option<String>,
    file: Option<(Vec<u8>, String, String)>, // bytes, filename, content-type
}

fn parse_item_request(item: &serde_json::Value) -> anyhow::Result<HttpRequest> {
    let raw = item.get("request").ok_or_else(|| anyhow::anyhow!("item 缺少 request"))?;
    let method = raw.get("method").and_then(|v| v.as_str()).unwrap_or("GET").to_string();
    let url = raw.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let headers = raw
        .get("headers")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|h| {
                    let name = h.get("name")?.as_str()?.to_string();
                    if name.is_empty() {
                        return None;
                    }
                    let value = h.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let disabled = h.get("enabled").and_then(|v| v.as_bool()) == Some(false)
                        || h.get("disabled").and_then(|v| v.as_bool()) == Some(true);
                    Some(crate::model::HttpHeader {
                        name,
                        value,
                        description: None,
                        disabled: Some(disabled),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let auth = raw.get("auth").and_then(parse_ui_auth);

    let assertions = raw
        .get("assertions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    let name = a.get("name")?.as_str()?;
                    let value = a.get("value").and_then(|v| v.as_str()).unwrap_or("");
                    let mut parts = value.splitn(2, ' ');
                    let operator = parts.next().unwrap_or("eq").to_string();
                    let expected = parts.next().map(|s| s.to_string());
                    Some(crate::model::Assertion {
                        expression: name.to_string(),
                        operator,
                        value: expected,
                        disabled: a.get("disabled").and_then(|v| v.as_bool()),
                        description: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let mut request = HttpRequest {
        info: crate::model::Info {
            name: item.get("name").and_then(|v| v.as_str()).map(|s| s.to_string()),
            r#type: item.get("type").and_then(|v| v.as_str()).map(|s| s.to_string()),
            seq: item.get("seq").and_then(|v| v.as_i64()),
            ..Default::default()
        },
        http: Some(crate::model::HttpDetails {
            method: Some(method),
            url: Some(url),
            headers,
            params: vec![],
            body: None,
            auth,
        }),
        runtime: crate::model::Runtime {
            assertions,
            ..Default::default()
        },
        settings: crate::model::HttpSettings::default(),
        docs: None,
        websocket: None,
        grpc: None,
    };
    request.runtime.scripts = vec![];
    Ok(request)
}

/// 前端 auth 是嵌套形状 `{mode, [mode]: {...}}`；同时兼容旧的扁平形状
fn auth_config<'a>(value: &'a serde_json::Value, mode: &str) -> serde_json::Value {
    if let Some(cfg) = value.get(mode) {
        if cfg.is_object() {
            return cfg.clone();
        }
    }
    let mut flat = value.clone();
    if let Some(obj) = flat.as_object_mut() {
        obj.remove("mode");
    }
    flat
}

pub(crate) fn parse_ui_auth(value: &serde_json::Value) -> Option<Auth> {
    let mode = value.get("mode").and_then(|v| v.as_str()).unwrap_or("none");
    if mode == "none" {
        return None;
    }
    if mode == "inherit" {
        return Some(Auth::Inherit("inherit".into()));
    }
    let cfg = auth_config(value, mode);
    let s = |key: &str| cfg.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let typed = match mode {
        "basic" => AuthTyped::Basic {
            username: Some(s("username")),
            password: Some(s("password")),
        },
        "bearer" => AuthTyped::Bearer { token: Some(s("token")) },
        "apikey" => AuthTyped::ApiKey {
            key: Some(s("key")),
            value: Some(s("value")),
            placement: cfg.get("placement").and_then(|v| v.as_str()).map(|x| {
                if x == "queryparams" {
                    "query".to_string()
                } else {
                    x.to_string()
                }
            }),
        },
        "digest" => AuthTyped::Digest {
            username: Some(s("username")),
            password: Some(s("password")),
        },
        "ntlm" => AuthTyped::Ntlm {
            username: Some(s("username")),
            password: Some(s("password")),
            domain: Some(s("domain")),
        },
        "wsse" => AuthTyped::Wsse {
            username: Some(s("username")),
            password: Some(s("password")),
        },
        "awsv4" => AuthTyped::AwsV4 {
            access_key_id: cfg.get("accessKeyId").and_then(|v| v.as_str()).map(|s| s.to_string()),
            secret_access_key: cfg.get("secretAccessKey").and_then(|v| v.as_str()).map(|s| s.to_string()),
            session_token: cfg.get("sessionToken").and_then(|v| v.as_str()).map(|s| s.to_string()),
            service: cfg.get("service").and_then(|v| v.as_str()).map(|s| s.to_string()),
            region: cfg.get("region").and_then(|v| v.as_str()).map(|s| s.to_string()),
            profile_name: cfg.get("profileName").and_then(|v| v.as_str()).map(|s| s.to_string()),
        },
        "oauth2" => AuthTyped::OAuth2(cfg),
        "oauth1" => AuthTyped::OAuth1(cfg),
        "akamai-edgegrid" => AuthTyped::EdgeGrid(cfg),
        _ => AuthTyped::Unknown,
    };
    Some(Auth::Typed(typed))
}

fn collect_vars(input: &SendRequestInput) -> Vars {
    let mut layers: Vec<Vars> = Vec::new();
    if let Some(root) = input.collection.get("root") {
        // 前端集合对象有两种形状：`root.vars.req`（内存集合）与
        // `root.request.vars.req`（main:collection-tree-loaded 的 payload）
        let holder = root.get("request").unwrap_or(root);
        let vars = holder
            .get("vars")
            .and_then(|v| v.get("req").or(Some(v)))
            .or_else(|| holder.get("variables"));
        layers.push(json_vars(vars));
    }
    if let Some(env) = &input.environment {
        layers.push(json_vars(env.get("variables")));
    }
    if let Some(req) = input.item.get("request") {
        if let Some(vars) = req.get("vars").and_then(|v| v.get("req")) {
            layers.push(json_vars(Some(vars)));
        }
    }
    if let Some(vars) = &input.variables {
        layers.push(json_vars(Some(vars)));
    }
    crate::interpolate::merge_vars(&layers)
}

pub(crate) fn json_vars(value: Option<&serde_json::Value>) -> Vars {
    let mut out = Vars::new();
    let Some(value) = value else { return out };
    if let Some(map) = value.as_object() {
        for (k, v) in map {
            if let Some(s) = v.as_str() {
                out.insert(k.clone(), s.to_string());
            } else if v.is_number() || v.is_boolean() {
                out.insert(k.clone(), v.to_string());
            }
        }
    }
    if let Some(arr) = value.as_array() {
        for entry in arr {
            let name = entry.get("name").and_then(|v| v.as_str());
            let val: Option<String> = entry
                .get("value")
                .and_then(|v| v.as_str().map(|s| s.to_string()).or_else(|| v.as_i64().map(|n| n.to_string())));
            if let (Some(name), Some(val)) = (name, val) {
                if entry.get("enabled").and_then(|v| v.as_bool()) != Some(false) {
                    out.insert(name.to_string(), val);
                }
            }
        }
    }
    out
}

fn script_text(item: &serde_json::Value, path: &[&str]) -> String {
    let mut cur = item.get("request");
    for part in path {
        cur = cur.and_then(|v| v.get(*part));
    }
    cur.and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn collection_path(input: &SendRequestInput) -> String {
    input
        .collection
        .get("pathname")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            input
                .collection
                .get("path")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default()
}

pub async fn execute_request(input: SendRequestInput) -> anyhow::Result<ExecutedResponse> {
    let started = Instant::now();
    let mut vars = collect_vars(&input);
    let base = collection_path(&input);
    let mut item = input.item.clone();
    let mut pre_tests: Vec<ScriptTestResult> = vec![];
    let mut logs: Vec<String> = vec![];
    let mut script_errors: Vec<String> = vec![];
    let mut body_override: Option<String> = None;
    let mut script_vars: HashMap<String, String> = HashMap::new();

    // ---------- before-request ----------
    let before_code = script_text(&item, &["script", "req"]);
    if !before_code.trim().is_empty() {
        let payload = serde_json::json!({
            "request": item.get("request").cloned().unwrap_or(serde_json::json!({})),
            "vars": vars,
            "collectionPath": base,
        });
        match run_script(&before_code, &payload) {
            Ok(out) => {
                if let Some(req) = out.request {
                    if let Some(obj) = item.get_mut("request") {
                        *obj = req;
                    }
                }
                for (k, v) in out.vars {
                    vars.insert(k.clone(), v.clone());
                    script_vars.insert(k, v);
                }
                logs.extend(out.logs);
                script_errors.extend(out.errors);
                pre_tests.extend(out.tests);
                body_override = out.body_override;
            }
            Err(e) => {
                pre_tests.push(error_result("before-request 脚本执行失败", &e));
                script_errors.push(e);
            }
        }
    }

    // ---------- 解析 + 插值 ----------
    let req = parse_item_request(&item)?;
    let http = req.http.as_ref().ok_or_else(|| anyhow::anyhow!("请求缺少 http 段"))?;
    let method = http.method.clone().unwrap_or_else(|| "GET".into()).to_uppercase();
    let mut url = interpolate(http.url.as_deref().unwrap_or(""), &vars);
    if url.is_empty() {
        anyhow::bail!("请求 URL 为空");
    }
    let mut headers: Vec<(String, String)> = http
        .headers
        .iter()
        .filter(|h| h.disabled != Some(true) && !h.name.is_empty())
        .map(|h| (h.name.clone(), interpolate(&h.value, &vars)))
        .collect();

    // cookie jar：用户手写的 Cookie 头优先，否则用 jar 里的
    let jar_cookie = input
        .options
        .as_ref()
        .and_then(|o| match &o.cookie_hook.0 {
            // 动态 hook：拿最终 URL 现算，Runner 里每个请求都不同 host
            Some(hook) => hook(&url),
            None => o.cookie_header.clone(),
        })
        .filter(|c| !c.is_empty());
    if let Some(cookie) = jar_cookie {
        if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("cookie")) {
            headers.push(("cookie".into(), cookie));
        }
    }

    // query 参数（params.type=query）并入 URL
    if let Some(params) = item
        .get("request")
        .and_then(|r| r.get("params"))
        .and_then(|v| v.as_array())
    {
        let pairs: Vec<(String, String)> = params
            .iter()
            .filter(|p| p.get("enabled").and_then(|v| v.as_bool()) != Some(false))
            .filter(|p| p.get("type").and_then(|v| v.as_str()).unwrap_or("query") == "query")
            .filter_map(|p| {
                let name = p.get("name").and_then(|v| v.as_str())?;
                if name.is_empty() {
                    return None;
                }
                Some((
                    name.to_string(),
                    interpolate(p.get("value").and_then(|v| v.as_str()).unwrap_or(""), &vars),
                ))
            })
            .collect();
        // 没有参数时不要触碰 URL，否则 url crate 会留下一个空的 `?`
        if !pairs.is_empty() {
            let mut parsed = url::Url::parse(&url)?;
            {
                let mut query = parsed.query_pairs_mut();
                for (k, v) in &pairs {
                    query.append_pair(k, v);
                }
            }
            url = parsed.to_string();
        }
    }

    // ---------- 认证：先处理不需要请求体的 ----------
    let mut extra_headers: Vec<(String, String)> = Vec::new();
    let mut digest_auth: Option<(String, String)> = None;
    let client = build_client(&item, input.options.as_ref())?;

    if let Some(Auth::Typed(auth)) = &http.auth {
        match auth {
            AuthTyped::Basic { username, password } => {
                let u = interpolate(username.as_deref().unwrap_or(""), &vars);
                let p = interpolate(password.as_deref().unwrap_or(""), &vars);
                let token = base64::engine::general_purpose::STANDARD.encode(format!("{u}:{p}"));
                extra_headers.push(("authorization".into(), format!("Basic {token}")));
            }
            AuthTyped::Bearer { token } => {
                let t = interpolate(token.as_deref().unwrap_or(""), &vars);
                extra_headers.push(("authorization".into(), format!("Bearer {t}")));
            }
            AuthTyped::ApiKey { key, value, placement } => {
                let k = interpolate(key.as_deref().unwrap_or(""), &vars);
                let v = interpolate(value.as_deref().unwrap_or(""), &vars);
                if placement.as_deref() == Some("query") {
                    let mut parsed = url::Url::parse(&url)?;
                    parsed.query_pairs_mut().append_pair(&k, &v);
                    url = parsed.to_string();
                } else {
                    extra_headers.push((k, v));
                }
            }
            AuthTyped::Digest { username, password } => {
                digest_auth = Some((
                    interpolate(username.as_deref().unwrap_or(""), &vars),
                    interpolate(password.as_deref().unwrap_or(""), &vars),
                ));
            }
            AuthTyped::Wsse { username, password } => {
                let u = interpolate(username.as_deref().unwrap_or(""), &vars);
                let p = interpolate(password.as_deref().unwrap_or(""), &vars);
                if let Some(header) = wsse_header(&u, &p) {
                    extra_headers.push(("x-wsse".into(), header));
                }
            }
            AuthTyped::OAuth2(cfg) => match fetch_oauth2_token(&client, cfg, &vars).await {
                Ok(token) => {
                    let placement = cfg.get("tokenPlacement").and_then(|v| v.as_str()).unwrap_or("header");
                    if placement == "url" {
                        let key = cfg.get("tokenQueryKey").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or("access_token");
                        let mut parsed = url::Url::parse(&url)?;
                        parsed.query_pairs_mut().append_pair(key, &token);
                        url = parsed.to_string();
                    } else {
                        let prefix = cfg
                            .get("tokenHeaderPrefix")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .unwrap_or("Bearer");
                        extra_headers.push(("authorization".into(), format!("{prefix} {token}")));
                    }
                }
                Err(e) => script_errors.push(format!("OAuth2 获取 token 失败：{e}")),
            },
            AuthTyped::AwsV4 {
                access_key_id,
                secret_access_key,
                session_token,
                service,
                region,
                ..
            } => {
                // 签名依赖最终 body，延后到 body 构建之后
                let _ = (access_key_id, secret_access_key, session_token, service, region);
            }
            _ => {}
        }
    }

    // ---------- body ----------
    let body = build_body(&item, &vars, &base, body_override.as_deref(), &mut headers)?;

    // ---------- awsv4 签名（需要 payload hash） ----------
    if let Some(Auth::Typed(AuthTyped::AwsV4 {
        access_key_id,
        secret_access_key,
        session_token,
        service,
        region,
        ..
    })) = &http.auth
    {
        let ak = interpolate(access_key_id.as_deref().unwrap_or(""), &vars);
        let sk = interpolate(secret_access_key.as_deref().unwrap_or(""), &vars);
        let token = session_token.as_deref().map(|t| interpolate(t, &vars)).filter(|t| !t.is_empty());
        if ak.is_empty() || sk.is_empty() {
            script_errors.push("AWSv4：accessKeyId / secretAccessKey 为空，已跳过签名".into());
        } else {
            let payload_hash = match &body {
                BodyPayload::Bytes(bytes) => hex::encode(Sha256::digest(bytes)),
                BodyPayload::Form(_) | BodyPayload::Multipart(_) => "UNSIGNED-PAYLOAD".to_string(),
                BodyPayload::None => hex::encode(Sha256::digest(b"")),
            };
            // 参与签名的 x-amz-* 头必须先放进 extra，签名与实际发送才一致
            let (amz_date, _) = utc_now();
            extra_headers.push(("x-amz-date".into(), amz_date));
            extra_headers.push(("x-amz-content-sha256".into(), payload_hash.clone()));
            if let Some(token) = token.as_deref() {
                extra_headers.push(("x-amz-security-token".into(), token.to_string()));
            }
            match aws_v4_authorization(
                &method,
                &url,
                &headers,
                &extra_headers,
                &payload_hash,
                &ak,
                &sk,
                token.as_deref(),
                service.as_deref(),
                region.as_deref(),
            ) {
                Ok((auth_header, _, _)) => {
                    extra_headers.push(("authorization".into(), auth_header));
                }
                Err(e) => script_errors.push(format!("AWSv4 签名失败：{e}")),
            }
        }
    }

    let prepared = Prepared {
        method: method.clone(),
        url: url.clone(),
        headers: headers.clone(),
        body,
    };

    // ---------- 发送 ----------
    let mut response = match send_prepared(&client, &prepared, &extra_headers).await {
        Ok(r) => r,
        Err(e) => return Ok(error_response(&started, e.to_string())),
    };

    // digest：401 挑战后重试一次
    if response.status().as_u16() == 401 {
        if let Some((username, password)) = digest_auth {
            if let Some(challenge) = response
                .headers()
                .get("www-authenticate")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
            {
                if let Some(auth_header) = digest_authorization(&challenge, &username, &password, &method, &url) {
                    let mut retry_headers = extra_headers.clone();
                    retry_headers.push(("authorization".into(), auth_header));
                    match send_prepared(&client, &prepared, &retry_headers).await {
                        Ok(r) => response = r,
                        Err(e) => return Ok(error_response(&started, e.to_string())),
                    }
                }
            }
        }
    }

    let status = response.status().as_u16();
    let status_text = response.status().canonical_reason().unwrap_or("").to_string();
    let response_headers: HashMap<String, String> = response
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    // 多个 Set-Cookie 会被 reqwest 合并在一个头里，按行拆开
    let set_cookies: Vec<String> = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|raw| {
            raw.split("\n")
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .collect();
    let headers_json = serde_json::to_value(&response_headers).unwrap_or_default();
    let body_bytes = response.bytes().await?.to_vec();
    let size = body_bytes.len();
    let data_buffer = base64::engine::general_purpose::STANDARD.encode(&body_bytes);
    let body_text = String::from_utf8_lossy(&body_bytes).to_string();
    let data = serde_json::from_str::<serde_json::Value>(&body_text)
        .unwrap_or(serde_json::Value::String(body_text.clone()));

    // ---------- 响应后脚本 ----------
    let response_payload = serde_json::json!({
        "status": status,
        "statusText": status_text,
        "data": data,
        "bodyText": body_text,
        "headers": response_headers,
        "duration": started.elapsed().as_millis() as u64,
        "size": size,
    });
    let mut test_results: Vec<ScriptTestResult> = vec![];
    let mut post_tests: Vec<ScriptTestResult> = vec![];

    let tests_code = script_text(&item, &["tests"]);
    if !tests_code.trim().is_empty() {
        let payload = serde_json::json!({ "response": response_payload, "vars": vars, "collectionPath": base });
        match run_script(&tests_code, &payload) {
            Ok(out) => {
                test_results.extend(out.tests);
                logs.extend(out.logs);
                script_errors.extend(out.errors);
            }
            Err(e) => test_results.push(error_result("tests 脚本执行失败", &e)),
        }
    }
    let after_code = script_text(&item, &["script", "res"]);
    if !after_code.trim().is_empty() {
        let payload = serde_json::json!({ "response": response_payload, "vars": vars, "collectionPath": base });
        match run_script(&after_code, &payload) {
            Ok(out) => {
                post_tests.extend(out.tests);
                logs.extend(out.logs);
                script_errors.extend(out.errors);
                if let Some(body) = out.body_override {
                    let _ = body;
                }
                for (k, v) in out.vars {
                    script_vars.insert(k, v);
                }
            }
            Err(e) => post_tests.push(error_result("after-response 脚本执行失败", &e)),
        }
    }

    let assertion_results = run_assertions(
        &req.runtime.assertions,
        status,
        &status_text,
        &body_text,
        if data.is_object() { Some(&data) } else { None },
    );

    let sent_headers: Vec<serde_json::Value> = headers
        .iter()
        .chain(extra_headers.iter())
        .map(|(k, v)| serde_json::json!({ "name": k, "value": v }))
        .collect();

    Ok(ExecutedResponse {
        state: "success".into(),
        status,
        status_text,
        data,
        data_buffer,
        headers: headers_json,
        size,
        duration: started.elapsed().as_millis() as u64,
        timeline: vec![],
        stream: false,
        request_sent: serde_json::json!({
            "url": url,
            "method": method,
            "headers": sent_headers,
            "timestamp": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_millis())
                .unwrap_or(0),
        }),
        error: None,
        assertion_results,
        test_results,
        pre_request_test_results: pre_tests,
        post_response_test_results: post_tests,
        runtime_variables: script_vars,
        script_logs: logs,
        script_errors,
        set_cookies,
    })
}

fn error_response(started: &Instant, error: String) -> ExecutedResponse {
    ExecutedResponse {
        state: "error".into(),
        status: 0,
        status_text: String::new(),
        data: serde_json::Value::Null,
        data_buffer: String::new(),
        headers: serde_json::json!({}),
        size: 0,
        duration: started.elapsed().as_millis() as u64,
        timeline: vec![],
        stream: false,
        request_sent: serde_json::json!({}),
        error: Some(error),
        assertion_results: vec![],
        test_results: vec![],
        pre_request_test_results: vec![],
        post_response_test_results: vec![],
        runtime_variables: HashMap::new(),
        script_logs: vec![],
        script_errors: vec![],
        set_cookies: vec![],
    }
}

/// 按请求设置 + 偏好设置构建 HTTP client。
/// 优先级：`input.options`（偏好设置注入） > `item.settings`（请求级） > 默认值。
fn build_client(
    item: &serde_json::Value,
    options: Option<&RequestOptions>,
) -> anyhow::Result<reqwest::Client> {
    let opts = options.cloned().unwrap_or_default();
    let setting_timeout = item
        .pointer("/settings/timeout")
        .and_then(|v| v.as_u64())
        .filter(|v| *v > 0);
    let timeout_ms = opts.timeout_ms.or(setting_timeout).unwrap_or(0);
    let follow = opts
        .follow_redirects
        .or_else(|| item.pointer("/settings/followRedirects").and_then(|v| v.as_bool()))
        .unwrap_or(true);
    let max_redirects = opts
        .max_redirects
        .or_else(|| {
            item.pointer("/settings/maxRedirects")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32)
        })
        .unwrap_or(10);
    let verify = opts.verify_tls.unwrap_or(true);

    let mut builder = reqwest::Client::builder();
    if timeout_ms > 0 {
        builder = builder.timeout(std::time::Duration::from_millis(timeout_ms));
    }
    builder = builder.redirect(if follow {
        reqwest::redirect::Policy::limited(max_redirects.max(1) as usize)
    } else {
        reqwest::redirect::Policy::none()
    });
    if !verify {
        builder = builder.danger_accept_invalid_certs(true);
    }
    if let Some(proxy) = opts.proxy.as_deref().filter(|p| !p.trim().is_empty()) {
        builder = builder.proxy(reqwest::Proxy::all(proxy.trim())?);
    }
    if let (Some(cert), Some(key)) = (opts.client_cert_pem.as_deref(), opts.client_key_pem.as_deref()) {
        let pem = format!("{cert}\n{key}");
        if let Ok(identity) = reqwest::Identity::from_pem(pem.as_bytes()) {
            builder = builder.identity(identity);
        }
    }
    builder.build().map_err(|e| anyhow::anyhow!(e))
}

fn resolve_path(base: &str, raw: &str) -> PathBuf {
    let path = Path::new(raw);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        PathBuf::from(base).join(path)
    }
}

/// 构建请求体；同时补齐 content-type（不覆盖用户显式设置的同名头）
fn build_body(
    item: &serde_json::Value,
    vars: &Vars,
    base: &str,
    body_override: Option<&str>,
    headers: &mut Vec<(String, String)>,
) -> anyhow::Result<BodyPayload> {
    let has_header = |headers: &Vec<(String, String)>, name: &str| {
        headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
    };
    if let Some(raw) = body_override {
        if !has_header(headers, "content-type") {
            headers.push(("content-type".into(), "text/plain".into()));
        }
        return Ok(BodyPayload::Bytes(raw.as_bytes().to_vec()));
    }
    let Some(body) = item.get("request").and_then(|r| r.get("body")) else {
        return Ok(BodyPayload::None);
    };
    let mode = body.get("mode").and_then(|v| v.as_str()).unwrap_or("none");
    match mode {
        "json" | "text" | "xml" | "sparql" => {
            let raw = body.get(mode).and_then(|v| v.as_str()).unwrap_or("");
            let text = interpolate(raw, vars);
            if !has_header(headers, "content-type") {
                let ct = match mode {
                    "json" => "application/json",
                    "xml" => "application/xml",
                    "sparql" => "application/sparql-query",
                    _ => "text/plain",
                };
                headers.push(("content-type".into(), ct.into()));
            }
            Ok(BodyPayload::Bytes(text.into_bytes()))
        }
        "formUrlEncoded" => {
            let mut form: Vec<(String, String)> = Vec::new();
            if let Some(arr) = body.get("formUrlEncoded").and_then(|v| v.as_array()) {
                for entry in arr {
                    if entry.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
                        continue;
                    }
                    let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    if name.is_empty() {
                        continue;
                    }
                    form.push((
                        name.to_string(),
                        interpolate(entry.get("value").and_then(|v| v.as_str()).unwrap_or(""), vars),
                    ));
                }
            }
            if !has_header(headers, "content-type") {
                headers.push(("content-type".into(), "application/x-www-form-urlencoded".into()));
            }
            Ok(BodyPayload::Form(form))
        }
        "multipartForm" => {
            let mut parts = Vec::new();
            if let Some(arr) = body.get("multipartForm").and_then(|v| v.as_array()) {
                for entry in arr {
                    if entry.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
                        continue;
                    }
                    let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    if name.is_empty() {
                        continue;
                    }
                    let kind = entry.get("type").and_then(|v| v.as_str()).unwrap_or("text");
                    let content_type = entry
                        .get("contentType")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if kind == "file" {
                        let paths: Vec<String> = match entry.get("value") {
                            Some(serde_json::Value::Array(arr)) => arr
                                .iter()
                                .filter_map(|v| v.as_str())
                                .map(|s| interpolate(s, vars))
                                .collect(),
                            Some(serde_json::Value::String(s)) => vec![interpolate(s, vars)],
                            _ => vec![],
                        };
                        for raw in paths {
                            if raw.is_empty() {
                                continue;
                            }
                            let path = resolve_path(base, &raw);
                            match std::fs::read(&path) {
                                Ok(bytes) => {
                                    let filename = path
                                        .file_name()
                                        .and_then(|x| x.to_str())
                                        .unwrap_or("file")
                                        .to_string();
                                    parts.push(PartSpec {
                                        name: name.to_string(),
                                        text: None,
                                        file: Some((bytes, filename, content_type.clone())),
                                    });
                                }
                                Err(e) => {
                                    return Err(anyhow::anyhow!("读取上传文件失败 {}：{e}", path.display()))
                                }
                            }
                        }
                    } else {
                        let value = entry.get("value").and_then(|v| v.as_str()).unwrap_or("");
                        parts.push(PartSpec {
                            name: name.to_string(),
                            text: Some(interpolate(value, vars)),
                            file: None,
                        });
                    }
                }
            }
            Ok(BodyPayload::Multipart(parts))
        }
        "file" => {
            let selected = body
                .get("file")
                .and_then(|v| v.as_array())
                .and_then(|arr| {
                    arr.iter()
                        .find(|e| e.get("selected").and_then(|v| v.as_bool()) == Some(true))
                        .or_else(|| arr.first())
                });
            let Some(entry) = selected else {
                return Ok(BodyPayload::None);
            };
            let raw = entry.get("filePath").and_then(|v| v.as_str()).unwrap_or("");
            if raw.is_empty() {
                return Ok(BodyPayload::None);
            }
            let path = resolve_path(base, &interpolate(raw, vars));
            let bytes = std::fs::read(&path)
                .map_err(|e| anyhow::anyhow!("读取请求体文件失败 {}：{e}", path.display()))?;
            if !has_header(headers, "content-type") {
                let declared = entry.get("contentType").and_then(|v| v.as_str()).unwrap_or("");
                let ct = if declared.is_empty() {
                    guess_content_type(&path).to_string()
                } else {
                    declared.to_string()
                };
                headers.push(("content-type".into(), ct));
            }
            Ok(BodyPayload::Bytes(bytes))
        }
        _ => Ok(BodyPayload::None),
    }
}

fn guess_content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "json" => "application/json",
        "xml" => "application/xml",
        "html" | "htm" => "text/html",
        "txt" | "md" => "text/plain",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

async fn send_prepared(
    client: &reqwest::Client,
    prepared: &Prepared,
    extra: &[(String, String)],
) -> anyhow::Result<reqwest::Response> {
    let mut builder = client.request(prepared.method.parse()?, &prepared.url);
    for (k, v) in prepared.headers.iter().chain(extra.iter()) {
        if let (Ok(name), Ok(value)) = (
            reqwest::header::HeaderName::from_bytes(k.as_bytes()),
            reqwest::header::HeaderValue::from_str(v),
        ) {
            builder = builder.header(name, value);
        }
    }
    builder = match &prepared.body {
        BodyPayload::None => builder,
        BodyPayload::Bytes(bytes) => builder.body(bytes.clone()),
        BodyPayload::Form(pairs) => {
            let mut serializer = url::form_urlencoded::Serializer::new(String::new());
            for (k, v) in pairs {
                serializer.append_pair(k, v);
            }
            builder.body(serializer.finish())
        }
        BodyPayload::Multipart(parts) => {
            let mut form = reqwest::multipart::Form::new();
            for part in parts {
                if let Some(text) = &part.text {
                    form = form.text(part.name.clone(), text.clone());
                } else if let Some((bytes, filename, content_type)) = &part.file {
                    let mut p = reqwest::multipart::Part::bytes(bytes.clone()).file_name(filename.clone());
                    if !content_type.is_empty() {
                        p = p.mime_str(content_type)?;
                    }
                    form = form.part(part.name.clone(), p);
                }
            }
            builder.multipart(form)
        }
    };
    Ok(builder.send().await?)
}

// ============================================================================
// Digest
// ============================================================================

fn parse_auth_params(header: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let value = header
        .strip_prefix("Digest ")
        .or_else(|| header.strip_prefix("digest "))
        .unwrap_or(header);
    let mut chars = value.chars().peekable();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut fields: Vec<String> = Vec::new();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                current.push(c);
            }
            ',' if !in_quotes => {
                fields.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        fields.push(current);
    }
    for field in fields {
        let field = field.trim();
        if let Some((k, v)) = field.split_once('=') {
            out.insert(k.trim().to_string(), v.trim().trim_matches('"').to_string());
        }
    }
    out
}

fn digest_authorization(
    challenge: &str,
    username: &str,
    password: &str,
    method: &str,
    url: &str,
) -> Option<String> {
    let params = parse_auth_params(challenge);
    let realm = params.get("realm")?.clone();
    let nonce = params.get("nonce")?.clone();
    let qop = params.get("qop").cloned().unwrap_or_default();
    let opaque = params.get("opaque").cloned();
    let algorithm = params.get("algorithm").cloned().unwrap_or_else(|| "MD5".into());
    if !algorithm.to_uppercase().starts_with("MD5") {
        return None; // SHA-256 变体暂不支持
    }
    let uri = url::Url::parse(url)
        .map(|u| {
            let mut p = u.path().to_string();
            if let Some(q) = u.query() {
                p.push('?');
                p.push_str(q);
            }
            p
        })
        .unwrap_or_else(|_| "/".into());
    let md5 = |s: &str| {
        use md5::Digest as _;
        hex::encode(md5::Md5::digest(s.as_bytes()))
    };
    let ha1 = md5(&format!("{username}:{realm}:{password}"));
    let ha2 = md5(&format!("{method}:{uri}"));
    let cnonce = hex::encode(uuid::Uuid::new_v4().as_bytes());
    let nc = "00000001";
    let response = if qop.split(',').any(|q| q.trim() == "auth") {
        md5(&format!("{ha1}:{nonce}:{nc}:{cnonce}:auth:{ha2}"))
    } else {
        md5(&format!("{ha1}:{nonce}:{ha2}"))
    };
    let mut header = format!(
        "Digest username=\"{username}\", realm=\"{realm}\", nonce=\"{nonce}\", uri=\"{uri}\", response=\"{response}\", algorithm={algorithm}"
    );
    if qop.split(',').any(|q| q.trim() == "auth") {
        header.push_str(&format!(", qop=auth, nc={nc}, cnonce=\"{cnonce}\""));
    }
    if let Some(opaque) = opaque {
        header.push_str(&format!(", opaque=\"{opaque}\""));
    }
    Some(header)
}

// ============================================================================
// WSSE
// ============================================================================

pub(crate) fn wsse_header(username: &str, password: &str) -> Option<String> {
    use sha1::{Digest as _, Sha1};
    if username.is_empty() && password.is_empty() {
        return None;
    }
    let nonce_bytes = uuid::Uuid::new_v4().as_bytes().to_vec();
    let nonce = base64::engine::general_purpose::STANDARD.encode(&nonce_bytes);
    let (created, _) = utc_now();
    let mut hasher = Sha1::new();
    hasher.update(&nonce_bytes);
    hasher.update(created.as_bytes());
    hasher.update(password.as_bytes());
    let digest = base64::engine::general_purpose::STANDARD.encode(hasher.finalize());
    Some(format!(
        "UsernameToken Username=\"{username}\", PasswordDigest=\"{digest}\", Nonce=\"{nonce}\", Created=\"{created}\""
    ))
}

// ============================================================================
// AWS SigV4
// ============================================================================

fn utc_now() -> (String, String) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let amz = format!("{y:04}{m:02}{d:02}T{hh:02}{mm:02}{ss:02}Z");
    let date = format!("{y:04}{m:02}{d:02}");
    (amz, date)
}

/// Howard Hinnant 的 civil_from_days
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn hmac_sha256(key: &[u8], data: &str) -> Vec<u8> {
    use hmac::{Hmac, KeyInit, Mac};
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC key");
    mac.update(data.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

#[allow(clippy::too_many_arguments)]
fn aws_v4_authorization(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    extra: &[(String, String)],
    payload_hash: &str,
    access_key: &str,
    secret_key: &str,
    session_token: Option<&str>,
    service: Option<&str>,
    region: Option<&str>,
) -> anyhow::Result<(String, String, String)> {
    let parsed = url::Url::parse(url)?;
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("URL 缺少 host"))?
        .to_string();
    let region = region
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| infer_region(&host));
    let service = service
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| infer_service(&host));
    let (amz_date, date) = utc_now();

    // canonical query：按 key 排序，RFC3986 编码
    let mut query: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    query.sort();
    let canonical_query = query
        .iter()
        .map(|(k, v)| format!("{}={}", uri_encode(k), uri_encode(v)))
        .collect::<Vec<_>>()
        .join("&");

    let canonical_uri = if parsed.path().is_empty() { "/" } else { parsed.path() };

    // 参与签名的头：host + 用户头 + 额外头（排除 authorization）
    let mut all: Vec<(String, String)> = vec![("host".into(), host.clone())];
    for (k, v) in headers.iter().chain(extra.iter()) {
        let lower = k.to_ascii_lowercase();
        if lower == "authorization" || lower == "host" {
            continue;
        }
        if lower == "content-length" {
            continue;
        }
        all.push((lower, v.trim().to_string()));
    }
    all.sort_by(|a, b| a.0.cmp(&b.0));
    let canonical_headers = all
        .iter()
        .map(|(k, v)| format!("{k}:{v}\n"))
        .collect::<String>();
    let signed_headers = all
        .iter()
        .map(|(k, _)| k.clone())
        .collect::<Vec<_>>()
        .join(";");

    let canonical_request = format!(
        "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
    );
    let scope = format!("{date}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        hex::encode(Sha256::digest(canonical_request.as_bytes()))
    );
    let k_date = hmac_sha256(format!("AWS4{secret_key}").as_bytes(), &date);
    let k_region = hmac_sha256(&k_date, &region);
    let k_service = hmac_sha256(&k_region, &service);
    let k_signing = hmac_sha256(&k_service, "aws4_request");
    let signature = hex::encode(hmac_sha256(&k_signing, &string_to_sign));

    let mut credential = access_key.to_string();
    let _ = session_token;
    credential.push_str(&format!("/{scope}"));
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={credential}, SignedHeaders={signed_headers}, Signature={signature}"
    );
    Ok((authorization, amz_date, signed_headers))
}

fn infer_region(host: &str) -> String {
    // s3.us-east-1.amazonaws.com / execute-api.eu-west-1.amazonaws.com
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() >= 4 {
        let candidate = parts[1];
        if candidate.contains('-') && candidate.chars().next().map(|c| c.is_ascii_alphabetic()) == Some(true) {
            return candidate.to_string();
        }
    }
    "us-east-1".into()
}

fn infer_service(host: &str) -> String {
    if host.starts_with("s3") || host.contains(".s3.") {
        "s3".into()
    } else if host.starts_with("execute-api") {
        "execute-api".into()
    } else if host.starts_with("dynamodb") {
        "dynamodb".into()
    } else {
        "s3".into()
    }
}

/// AWS SigV4 要求的 RFC3986 编码（空格 %20，保留 -_.~）
fn uri_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

// ============================================================================
// OAuth2
// ============================================================================

pub(crate) async fn fetch_oauth2_token(
    client: &reqwest::Client,
    cfg: &serde_json::Value,
    vars: &Vars,
) -> anyhow::Result<String> {
    let json = oauth2_token_json(client, cfg, vars).await?;
    let token = if cfg.get("tokenSource").and_then(|v| v.as_str()) == Some("id_token") {
        json.get("id_token").and_then(|v| v.as_str())
    } else {
        json.get("access_token").and_then(|v| v.as_str()).or_else(|| json.get("id_token").and_then(|v| v.as_str()))
    };
    token
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("token 响应缺少 access_token"))
}

/// 换取 token 并返回完整响应（供 UI 的「获取 Token」按钮复用）
pub async fn oauth2_token_json(
    client: &reqwest::Client,
    cfg: &serde_json::Value,
    vars: &Vars,
) -> anyhow::Result<serde_json::Value> {
    let get = |k: &str| {
        cfg.get(k)
            .and_then(|v| v.as_str())
            .map(|s| interpolate(s, vars))
            .unwrap_or_default()
    };
    let grant = get("grantType");
    let grant = if grant.is_empty() { "client_credentials".to_string() } else { grant };
    let token_url = get("accessTokenUrl");
    if token_url.is_empty() {
        anyhow::bail!("缺少 accessTokenUrl");
    }
    let client_id = get("clientId");
    let client_secret = get("clientSecret");
    let mut form: Vec<(String, String)> = vec![("grant_type".into(), grant.clone())];
    match grant.as_str() {
        "client_credentials" => {
            form.push(("client_id".into(), client_id.clone()));
            form.push(("client_secret".into(), client_secret.clone()));
            let scope = get("scope");
            if !scope.is_empty() {
                form.push(("scope".into(), scope));
            }
        }
        "password" => {
            form.push(("username".into(), get("username")));
            form.push(("password".into(), get("password")));
            if !client_id.is_empty() {
                form.push(("client_id".into(), client_id.clone()));
            }
            if !client_secret.is_empty() {
                form.push(("client_secret".into(), client_secret.clone()));
            }
            let scope = get("scope");
            if !scope.is_empty() {
                form.push(("scope".into(), scope));
            }
        }
        "refresh_token" => {
            let token = get("refreshToken");
            if token.is_empty() {
                anyhow::bail!("refresh_token grant 缺少 refreshToken");
            }
            form.push(("refresh_token".into(), token));
            if !client_id.is_empty() {
                form.push(("client_id".into(), client_id.clone()));
            }
        }
        other => anyhow::bail!("暂不支持的 grant_type：{other}"),
    }
    if let Some(extra) = cfg.get("additionalParameters").and_then(|v| v.as_array()) {
        for entry in extra {
            if entry.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
                continue;
            }
            let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if name.is_empty() {
                continue;
            }
            form.push((name.to_string(), interpolate(entry.get("value").and_then(|v| v.as_str()).unwrap_or(""), vars)));
        }
    }

    let mut request = client.post(&token_url);
    if cfg.get("credentialsPlacement").and_then(|v| v.as_str()) == Some("basic_auth_header") {
        request = request.basic_auth(&client_id, Some(&client_secret));
        form.retain(|(k, _)| k != "client_id" && k != "client_secret");
    }
    let response = request.form(&form).send().await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("token 端点返回 {status}：{text}");
    }
    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| anyhow::anyhow!("token 响应不是 JSON：{e}；{text}"))?;
    Ok(json)
}

/// 供 UI 直接使用的入口：自建 client，不插值
pub async fn fetch_oauth2_token_response(cfg: &serde_json::Value) -> anyhow::Result<serde_json::Value> {
    let client = reqwest::Client::builder().build()?;
    oauth2_token_json(&client, cfg, &Vars::new()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_digest_challenge_with_quoted_comma() {
        let header = r#"Digest realm="test", qop="auth,auth-int", nonce="abc", opaque="xyz", algorithm=MD5"#;
        let params = parse_auth_params(header);
        assert_eq!(params.get("realm").map(String::as_str), Some("test"));
        assert_eq!(params.get("qop").map(String::as_str), Some("auth,auth-int"));
        let auth = digest_authorization(header, "u", "p", "GET", "http://example.com/a?b=1").unwrap();
        assert!(auth.contains("qop=auth"));
        assert!(auth.contains("uri=\"/a?b=1\""));
        assert!(auth.contains("opaque=\"xyz\""));
    }

    #[test]
    fn uri_encode_matches_rfc3986() {
        assert_eq!(uri_encode("a b/c"), "a%20b%2Fc");
        assert_eq!(uri_encode("a-_.~"), "a-_.~");
    }

    #[test]
    fn utc_formats_are_consistent() {
        let (amz, date) = utc_now();
        assert_eq!(amz.len(), 16);
        assert!(amz.ends_with('Z'));
        assert_eq!(&amz[..8], date);
    }

    #[test]
    fn aws_signature_is_deterministic_shape() {
        let (auth, amz, signed) = aws_v4_authorization(
            "GET",
            "https://examplebucket.s3.amazonaws.com/test.txt",
            &[],
            &[],
            &hex::encode(Sha256::digest(b"")),
            "AKIDEXAMPLE",
            "SECRET",
            None,
            Some("s3"),
            Some("us-east-1"),
        )
        .unwrap();
        assert!(auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/"));
        assert!(signed.contains("host"));
        assert!(amz.ends_with('Z'));
    }
}
