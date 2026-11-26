//! QuickJS 脚本运行时（对齐 Bruno 的 quickjs 路径）。
//!
//! 支持 Bruno 脚本的常用子集：
//! - `bru.getVar/setVar/hasVar/deleteVar/getEnvVar/setEnvVar/interpolate/cwd`
//! - `req.getUrl/setUrl/getMethod/setMethod/getHeaders/setHeader/getBody/setBody`
//! - `res.getStatus/getStatusText/getBody/getBodyText/getHeaders/getResponseTime`
//! - `test(name, fn)` + chai 风格 `expect(...)`（equal/eql/include/a/property/lengthOf/
//!   above/least/below/most/match/true/false/ok/null/undefined/empty/status，支持 `.not`）
//! - `console.log/info/warn/error`（写入日志，不回显到宿主 stdout）
//!
//! 每个阶段使用独立 Context，变量通过 JSON 进出，避免同名 `const` 冲突。

use serde::{Deserialize, Serialize};
use serde_json::Value as J;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScriptTestResult {
    pub uid: String,
    pub status: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ScriptOutcome {
    /// 脚本修改后的 request（UI 形状）
    pub request: Option<J>,
    /// `bru.setVar` / `req.setVar` 产生的变量
    pub vars: HashMap<String, String>,
    /// `bru.setEnvVar` 产生的环境变量
    pub env_vars: HashMap<String, String>,
    /// `req.setBody` 设置的请求体（优先于 body.<mode>）
    pub body_override: Option<String>,
    pub tests: Vec<ScriptTestResult>,
    pub logs: Vec<String>,
    pub errors: Vec<String>,
}

const PRELUDE: &str = r#"
(function () {
  const input = globalThis.__INPUT__ || {};
  const vars = Object.assign({}, input.vars || {});
  const outVars = {};
  const envVars = {};
  const results = [];
  const logs = [];
  const errors = [];
  const fmt = (v) => { try { return typeof v === 'string' ? v : JSON.stringify(v); } catch (e) { return String(v); } };
  const deepEqual = (a, b) => {
    if (a === b) return true;
    if (typeof a !== typeof b) return false;
    if (a && b && typeof a === 'object') {
      const ka = Object.keys(a);
      const kb = Object.keys(b);
      if (ka.length !== kb.length) return false;
      return ka.every((k) => deepEqual(a[k], b[k]));
    }
    return false;
  };
  class Assertion {
    constructor(actual, negate) { this.actual = actual; this.negate = !!negate; }
    get to() { return this; }
    get be() { return this; }
    get been() { return this; }
    get is() { return this; }
    get that() { return this; }
    get have() { return this; }
    get has() { return this; }
    get and() { return this; }
    get not() { return new Assertion(this.actual, !this.negate); }
    _assert(cond, msg) {
      const ok = this.negate ? !cond : cond;
      if (!ok) throw new Error(msg);
      return this;
    }
    equal(v) { return this._assert(this.actual === v, 'expected ' + fmt(this.actual) + ' to equal ' + fmt(v)); }
    equals(v) { return this.equal(v); }
    eql(v) { return this._assert(deepEqual(this.actual, v), 'expected ' + fmt(this.actual) + ' to deeply equal ' + fmt(v)); }
    include(v) {
      let ok = false;
      if (typeof this.actual === 'string') ok = this.actual.indexOf(String(v)) !== -1;
      else if (Array.isArray(this.actual)) ok = this.actual.some((x) => deepEqual(x, v));
      else if (this.actual && typeof this.actual === 'object') {
        ok = (v && typeof v === 'object') ? Object.keys(v).every((k) => deepEqual(this.actual[k], v[k])) : (v in this.actual);
      }
      return this._assert(ok, 'expected ' + fmt(this.actual) + ' to include ' + fmt(v));
    }
    a(type) {
      const t = Array.isArray(this.actual) ? 'array' : (this.actual === null ? 'null' : typeof this.actual);
      return this._assert(t === String(type).toLowerCase(), 'expected type ' + t + ' to be ' + type);
    }
    an(type) { return this.a(type); }
    property(name, value) {
      const has = this.actual != null && Object.prototype.hasOwnProperty.call(this.actual, name);
      if (arguments.length > 1) return this._assert(has && deepEqual(this.actual[name], value), 'expected property ' + name + ' to equal ' + fmt(value));
      return this._assert(has, 'expected property ' + name);
    }
    lengthOf(n) {
      const len = this.actual != null ? this.actual.length : undefined;
      return this._assert(len === n, 'expected length ' + len + ' to be ' + n);
    }
    above(n) { return this._assert(this.actual > n, 'expected ' + this.actual + ' to be above ' + n); }
    least(n) { return this._assert(this.actual >= n, 'expected ' + this.actual + ' to be at least ' + n); }
    below(n) { return this._assert(this.actual < n, 'expected ' + this.actual + ' to be below ' + n); }
    most(n) { return this._assert(this.actual <= n, 'expected ' + this.actual + ' to be at most ' + n); }
    match(re) { return this._assert(re.test(String(this.actual)), 'expected ' + this.actual + ' to match ' + re); }
    get true() { return this._assert(this.actual === true, 'expected ' + fmt(this.actual) + ' to be true'); }
    get false() { return this._assert(this.actual === false, 'expected ' + fmt(this.actual) + ' to be false'); }
    get ok() { return this._assert(!!this.actual, 'expected ' + fmt(this.actual) + ' to be truthy'); }
    get null() { return this._assert(this.actual === null, 'expected ' + fmt(this.actual) + ' to be null'); }
    get undefined() { return this._assert(this.actual === undefined, 'expected ' + fmt(this.actual) + ' to be undefined'); }
    get empty() {
      const v = this.actual;
      const isEmpty = v == null || v.length === 0 || (typeof v === 'object' && Object.keys(v).length === 0);
      return this._assert(isEmpty, 'expected ' + fmt(v) + ' to be empty');
    }
    status(code) { return this.equal(code); }
    header(name, value) {
      const h = (res.getHeaders() || {});
      const key = Object.keys(h).find((x) => x.toLowerCase() === String(name).toLowerCase());
      const actual = key ? h[key] : undefined;
      if (arguments.length > 1) return this._assert(actual === value, 'expected header ' + name + ' to be ' + value + ', got ' + actual);
      return this._assert(actual !== undefined, 'expected header ' + name);
    }
  }
  globalThis.expect = (v) => new Assertion(v);
  globalThis.test = (name, fn) => {
    try {
      fn();
      results.push({ status: 'pass', description: String(name) });
    } catch (e) {
      results.push({ status: 'fail', description: String(name), error: (e && (e.message || e)) ? String(e.message || e) : 'failed' });
    }
  };
  globalThis.console = {
    log: (...a) => logs.push(a.map(fmt).join(' ')),
    info: (...a) => logs.push(a.map(fmt).join(' ')),
    warn: (...a) => logs.push('[warn] ' + a.map(fmt).join(' ')),
    error: (...a) => logs.push('[error] ' + a.map(fmt).join(' ')),
  };
  const reqImpl = Object.assign({ url: '', method: 'GET', headers: [], body: null }, input.request || {});
  globalThis.bru = {
    getVar: (k) => vars[k],
    setVar: (k, v) => { vars[k] = v; outVars[k] = v; },
    hasVar: (k) => Object.prototype.hasOwnProperty.call(vars, k),
    deleteVar: (k) => { delete vars[k]; delete outVars[k]; },
    getEnvVar: (k) => envVars[k],
    setEnvVar: (k, v) => { envVars[k] = v; },
    interpolate: (s) => String(s).replace(/{{\s*([^}]+)\s*}}/g, (m, k) => (Object.prototype.hasOwnProperty.call(vars, k) ? vars[k] : m)),
    cwd: () => input.collectionPath || '',
    getProcessEnv: (k) => (input.processEnv || {})[k],
  };
  globalThis.req = {
    getUrl: () => reqImpl.url,
    setUrl: (v) => { reqImpl.url = String(v); },
    getMethod: () => reqImpl.method,
    setMethod: (v) => { reqImpl.method = String(v).toUpperCase(); },
    getHeaders: () => {
      const o = {};
      (reqImpl.headers || []).forEach((h) => { if (h.enabled !== false) o[h.name] = h.value; });
      return o;
    },
    setHeader: (name, value) => {
      const list = reqImpl.headers || (reqImpl.headers = []);
      const found = list.find((h) => String(h.name).toLowerCase() === String(name).toLowerCase());
      if (found) { found.value = String(value); found.enabled = true; }
      else list.push({ name: String(name), value: String(value), enabled: true });
    },
    getBody: () => {
      const b = reqImpl.body;
      if (!b) return input.rawBody || '';
      if (b.mode && b[b.mode] !== undefined) return b[b.mode];
      return input.rawBody || '';
    },
    setBody: (v) => {
      const text = typeof v === 'string' ? v : JSON.stringify(v);
      reqImpl.body = { mode: 'text', text: text };
      reqImpl.__bodyOverride = text;
    },
    getVar: (k) => vars[k],
    setVar: (k, v) => { vars[k] = v; outVars[k] = v; },
  };
  const resImpl = input.response || {};
  const bodyText = () => (typeof resImpl.bodyText === 'string' ? resImpl.bodyText : '');
  const bodyValue = () => {
    if (resImpl.data !== undefined && resImpl.data !== null) {
      if (typeof resImpl.data === 'object') return resImpl.data;
      try { return JSON.parse(String(resImpl.data)); } catch (e) { return resImpl.data; }
    }
    const t = bodyText();
    try { return JSON.parse(t); } catch (e) { return t; }
  };
  globalThis.res = {
    getStatus: () => resImpl.status,
    getStatusText: () => resImpl.statusText || '',
    getBody: bodyValue,
    getBodyText: bodyText,
    getHeaders: () => resImpl.headers || {},
    getHeader: (k) => {
      const h = resImpl.headers || {};
      const key = Object.keys(h).find((x) => x.toLowerCase() === String(k).toLowerCase());
      return key ? h[key] : undefined;
    },
    getResponseTime: () => resImpl.duration || 0,
    getSize: () => resImpl.size || 0,
  };
  globalThis.__brs = {
    result: () => ({
      request: reqImpl,
      vars: outVars,
      envVars: envVars,
      results: results,
      logs: logs,
      errors: errors,
      bodyOverride: reqImpl.__bodyOverride || null,
    }),
    error: (e) => { errors.push(String(e)); },
  };
})();
"#;

fn json_to_js(value: &J) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "null".into())
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

fn new_uid() -> String {
    crate::model::new_uid()
}

/// 执行一段用户脚本。`input` 会作为 `__INPUT__` 注入。
pub fn run_script(code: &str, input: &J) -> Result<ScriptOutcome, String> {
    if code.trim().is_empty() {
        return Ok(ScriptOutcome::default());
    }
    let runtime = rquickjs::Runtime::new().map_err(|e| format!("创建 JS 运行时失败：{e}"))?;
    runtime.set_memory_limit(64 * 1024 * 1024);
    let context = rquickjs::Context::full(&runtime).map_err(|e| format!("创建 JS 上下文失败：{e}"))?;

    let wrapped = format!(
        "(function () {{\n try {{\n{code}\n }} catch (e) {{ __brs.error((e && ((e.message ? e.message + ' | ' : '') + (e.stack || String(e)))) || String(e)); }}\n}})();"
    );

    let output: String = context.with(|ctx| {
        ctx.eval::<(), _>(format!("globalThis.__INPUT__ = {};", json_to_js(input)))
            .map_err(|e| format!("注入脚本输入失败：{e}"))?;
        ctx.eval::<(), _>(PRELUDE)
            .map_err(|e| format!("JS 预置失败：{e}"))?;
        ctx.eval::<(), _>(wrapped)
            .map_err(|e| format!("脚本执行失败：{e}"))?;
        ctx.eval::<String, _>("JSON.stringify(__brs.result())")
            .map_err(|e| format!("读取脚本结果失败：{e}"))
    })?;

    let parsed: J = serde_json::from_str(&output).map_err(|e| format!("脚本结果不是 JSON：{e}"))?;
    let mut outcome = ScriptOutcome::default();
    outcome.request = parsed.get("request").cloned();
    outcome.body_override = parsed
        .get("bodyOverride")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    outcome.vars = string_map(parsed.get("vars"));
    outcome.env_vars = string_map(parsed.get("envVars"));
    outcome.logs = parsed
        .get("logs")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect())
        .unwrap_or_default();
    outcome.errors = parsed
        .get("errors")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect())
        .unwrap_or_default();
    outcome.tests = parsed
        .get("results")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|r| {
                    let description = r.get("description").and_then(|v| v.as_str())?.to_string();
                    let status = r.get("status").and_then(|v| v.as_str()).unwrap_or("fail").to_string();
                    Some(ScriptTestResult {
                        uid: new_uid(),
                        status,
                        description,
                        error: r.get("error").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(outcome)
}

fn string_map(value: Option<&J>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Some(J::Object(map)) = value {
        for (k, v) in map {
            if let Some(s) = v.as_str() {
                out.insert(k.clone(), s.to_string());
            } else if v.is_number() || v.is_boolean() {
                out.insert(k.clone(), v.to_string());
            }
        }
    }
    out
}

/// 脚本错误转成一条失败测试，供 UI 的 Tests 页签展示（不静默吞掉）
pub fn error_result(prefix: &str, error: &str) -> ScriptTestResult {
    ScriptTestResult {
        uid: new_uid(),
        status: "fail".into(),
        description: prefix.to_string(),
        error: Some(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn before_request_can_mutate_request() {
        let input = serde_json::json!({
            "request": {
                "url": "http://old",
                "method": "GET",
                "headers": [{"name": "X-A", "value": "1", "enabled": true}],
                "body": {"mode": "json", "json": "{\"a\":1}"}
            },
            "vars": {"token": "t0"},
            "collectionPath": "C:/demo"
        });
        let code = r#"
            req.setUrl('http://new/' + bru.getVar('token'));
            req.setMethod('post');
            req.setHeader('X-B', '2');
            bru.setVar('token', 't1');
        "#;
        let out = run_script(code, &input).unwrap();
        let request = out.request.unwrap();
        assert_eq!(request["url"], "http://new/t0");
        assert_eq!(request["method"], "POST");
        assert_eq!(out.vars.get("token").map(String::as_str), Some("t1"));
        let headers = request["headers"].as_array().unwrap();
        assert!(headers.iter().any(|h| h["name"] == "X-B"));
    }

    #[test]
    fn tests_collect_pass_and_fail() {
        let input = serde_json::json!({
            "response": {"status": 200, "statusText": "OK", "bodyText": "{\"id\":\"abc\",\"n\":2}", "duration": 5},
            "vars": {}
        });
        let code = r#"
            test('status is 200', function () { expect(res.getStatus()).to.equal(200); });
            test('id is string', function () { expect(res.getBody().id).to.be.a('string'); });
            test('this fails', function () { expect(res.getBody().n).to.equal(3); });
            expect(res.getBody()).to.have.property('id');
            console.log('hello', 1);
        "#;
        let out = run_script(code, &input).unwrap();
        assert_eq!(out.tests.len(), 3);
        assert_eq!(out.tests.iter().filter(|t| t.status == "pass").count(), 2);
        assert_eq!(out.tests.iter().filter(|t| t.status == "fail").count(), 1);
        assert!(out.logs.iter().any(|l| l.contains("hello")));
    }

    #[test]
    fn script_error_is_reported_not_panicking() {
        let out = run_script("throw new Error('boom')", &serde_json::json!({})).unwrap();
        assert!(out.errors.iter().any(|e| e.contains("boom")), "{:?}", out.errors);
    }

    #[test]
    fn set_body_override_is_returned() {
        let input = serde_json::json!({"request": {"body": {"mode": "json", "json": "{}"}}});
        let out = run_script("req.setBody(JSON.stringify({x:1}))", &input).unwrap();
        assert_eq!(out.body_override.as_deref(), Some(r#"{"x":1}"#));
    }
}
