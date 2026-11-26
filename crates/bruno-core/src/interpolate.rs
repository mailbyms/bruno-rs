//! 变量插值与声明式断言。
//! 优先级（对齐 bruno-cli/src/runner/interpolate-vars.js:63-76）：
//!   global env < collection vars < selected environment < folder vars < request vars < runtime vars
//! 右侧覆盖左侧。

use crate::model::Assertion;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type Vars = HashMap<String, String>;

/// 按优先级合并变量
pub fn merge_vars(layers: &[Vars]) -> Vars {
    let mut out = Vars::new();
    for layer in layers {
        for (k, v) in layer {
            out.insert(k.clone(), v.clone());
        }
    }
    out
}

/// 插值 {{var}} / {{process.env.X}}：未解析变量保留原样（与 Bruno 行为一致）
pub fn interpolate(input: &str, vars: &Vars) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        if i + 1 < chars.len() && chars[i] == '{' && chars[i + 1] == '{' {
            if let Some(end) = find_close(&chars, i + 2) {
                let key: String = chars[i + 2..end].iter().collect();
                let key = key.trim();
                if let Some(value) = resolve_key(key, vars) {
                    out.push_str(&value);
                } else {
                    out.push_str("{{");
                    out.push_str(key);
                    out.push_str("}}");
                }
                i = end + 2;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn find_close(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start;
    while i + 1 < chars.len() {
        if chars[i] == '}' && chars[i + 1] == '}' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn resolve_key(key: &str, vars: &Vars) -> Option<String> {
    if let Some(rest) = key.strip_prefix("process.env.") {
        return std::env::var(rest).ok();
    }
    vars.get(key).cloned()
}

/// 递归插值 JSON 值（字符串叶子插值，键不动）
pub fn interpolate_json(value: &serde_json::Value, vars: &Vars) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => serde_json::Value::String(interpolate(s, vars)),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(|v| interpolate_json(v, vars)).collect())
        }
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                out.insert(k.clone(), interpolate_json(v, vars));
            }
            serde_json::Value::Object(out)
        }
        other => other.clone(),
    }
}

// ============================================================================
// 声明式断言（runtime.assertions）
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssertionResult {
    /// 前端 TestResults 列表用 uid 作为 key
    pub uid: String,
    pub name: String,
    pub passed: bool,
    /// pass | fail（前端 Tests 页签按 status 渲染）
    pub status: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub message: String,
    /// 前端断言行显示 `lhsExpr: rhsExpr`
    #[serde(rename = "lhsExpr")]
    pub lhs_expr: String,
    #[serde(rename = "rhsExpr")]
    pub rhs_expr: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl AssertionResult {
    fn pass(expression: &str, operator: &str, expected: &str) -> Self {
        Self {
            uid: crate::model::new_uid(),
            name: format!("{expression} {operator} {expected}"),
            passed: true,
            status: "pass".into(),
            kind: "assert".into(),
            message: String::new(),
            lhs_expr: expression.to_string(),
            rhs_expr: format!("{operator} {expected}"),
            error: None,
        }
    }
    fn fail(expression: &str, operator: &str, expected: &str, message: String) -> Self {
        Self {
            uid: crate::model::new_uid(),
            name: format!("{expression} {operator} {expected}"),
            passed: false,
            status: "fail".into(),
            kind: "assert".into(),
            message: message.clone(),
            lhs_expr: expression.to_string(),
            rhs_expr: format!("{operator} {expected}"),
            error: Some(message),
        }
    }
}

/// 对响应执行断言列表。
/// expression 支持 res.status / res.statusText / res.body.<jsonpath> / res.body（原始文本）
pub fn run_assertions(
    assertions: &[Assertion],
    status: u16,
    status_text: &str,
    body_text: &str,
    body_json: Option<&serde_json::Value>,
) -> Vec<AssertionResult> {
    assertions
        .iter()
        .filter(|a| a.disabled != Some(true))
        .map(|a| {
            let actual = resolve_expression(&a.expression, status, status_text, body_text, body_json);
            let expected = a.value.clone().unwrap_or_default();
            let ok = match a.operator.as_str() {
                "eq" => actual == expected,
                "neq" => actual != expected,
                "contains" => actual.contains(&expected),
                "notContains" => !actual.contains(&expected),
                "startsWith" => actual.starts_with(&expected),
                "endsWith" => actual.ends_with(&expected),
                "isEmpty" => actual.is_empty(),
                "isNotEmpty" => !actual.is_empty(),
                "isNumber" => actual.parse::<f64>().is_ok(),
                "gt" => compare_num(&actual, &expected, |a, b| a > b),
                "gte" => compare_num(&actual, &expected, |a, b| a >= b),
                "lt" => compare_num(&actual, &expected, |a, b| a < b),
                "lte" => compare_num(&actual, &expected, |a, b| a <= b),
                "isJson" => body_json.is_some(),
                "isString" => !actual.is_empty(),
                "isBoolean" => matches!(actual.as_str(), "true" | "false"),
                "isTrue" => actual == "true",
                "isFalse" => actual == "false",
                "isNull" => actual == "null" || actual.is_empty(),
                "isDefined" => actual != "undefined" && !actual.is_empty(),
                "between" => {
                    let parts: Vec<&str> = expected.split(',').collect();
                    if parts.len() == 2 {
                        compare_num(&actual, parts[0].trim(), |a, b| a >= b)
                            && compare_num(&actual, parts[1].trim(), |a, b| a <= b)
                    } else {
                        false
                    }
                }
                "matches" => regex_like(&actual, &expected),
                "hasKey" => body_json
                    .and_then(|v| v.as_object())
                    .map(|m| m.contains_key(&expected))
                    .unwrap_or(false),
                "length" => actual.chars().count().to_string() == expected,
                _ => {
                    // 未知操作符：若 value 是 JSON 结构则做子串比较，避免误判为通过
                    actual.contains(&expected)
                }
            };
            if ok {
                AssertionResult::pass(&a.expression, &a.operator, &expected)
            } else {
                AssertionResult::fail(
                    &a.expression,
                    &a.operator,
                    &expected,
                    format!("期望 {}，实际 {}", expected, actual),
                )
            }
        })
        .collect()
}

fn compare_num(a: &str, b: &str, f: impl Fn(f64, f64) -> bool) -> bool {
    match (a.trim().parse::<f64>(), b.trim().parse::<f64>()) {
        (Ok(x), Ok(y)) => f(x, y),
        _ => false,
    }
}

/// 极简 glob 匹配：支持 * 与 ?，避免引入 regex 依赖
fn regex_like(text: &str, pattern: &str) -> bool {
    fn go(text: &[char], pat: &[char]) -> bool {
        if pat.is_empty() {
            return text.is_empty();
        }
        match pat[0] {
            '*' => go(text, &pat[1..]) || (!text.is_empty() && go(&text[1..], pat)),
            '?' => !text.is_empty() && go(&text[1..], &pat[1..]),
            c => !text.is_empty() && text[0] == c && go(&text[1..], &pat[1..]),
        }
    }
    go(&text.chars().collect::<Vec<_>>(), &pattern.chars().collect::<Vec<_>>())
}

fn resolve_expression(
    expr: &str,
    status: u16,
    status_text: &str,
    body_text: &str,
    body_json: Option<&serde_json::Value>,
) -> String {
    match expr {
        "res.status" => status.to_string(),
        "res.statusText" => status_text.to_string(),
        "res.body" | "res.bodyText" => body_text.to_string(),
        "res.responseTime" => "0".into(),
        _ => {
            if let Some(rest) = expr.strip_prefix("res.body.") {
                if let Some(value) = json_path(body_json, rest) {
                    return json_scalar(value);
                }
            }
            if let Some(rest) = expr.strip_prefix("res.headers.") {
                // headers 由调用方注入到 body_json 外的场景，这里保留空实现
                let _ = rest;
            }
            String::new()
        }
    }
}

fn json_path<'a>(root: Option<&'a serde_json::Value>, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = root?;
    for part in path.split('.') {
        // 支持 arr[0] / arr.0
        let (key, index) = if let Some(open) = part.find('[') {
            let key = &part[..open];
            let idx = part[open + 1..].trim_end_matches(']').parse::<usize>().ok();
            (key, idx)
        } else {
            (part, None)
        };
        if !key.is_empty() {
            current = current.get(key)?;
        }
        if let Some(i) = index {
            current = current.get(i)?;
        }
    }
    Some(current)
}

fn json_scalar(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".into(),
        other => other.to_string(),
    }
}
