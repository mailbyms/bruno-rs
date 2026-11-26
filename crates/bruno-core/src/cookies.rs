//! 简易 Cookie Jar：跨请求共享、可持久化、可被 UI 增删改。
//!
//! 不用 reqwest 的 cookie_store 是因为它不能列出全部 cookie，而 Bruno 的
//! Cookies 面板需要按域名展示/编辑。这里自己存一份，发请求时手动拼 `Cookie` 头，
//! 收到响应后解析 `Set-Cookie`。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StoredCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    #[serde(default = "default_path")]
    pub path: String,
    #[serde(default)]
    pub host_only: bool,
    #[serde(default)]
    pub secure: bool,
    #[serde(default)]
    pub http_only: bool,
    /// Unix 秒；None = 会话 cookie
    #[serde(default)]
    pub expires: Option<i64>,
    #[serde(default)]
    pub same_site: Option<String>,
}

fn default_path() -> String {
    "/".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CookieJar {
    #[serde(default)]
    pub cookies: Vec<StoredCookie>,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn host_of(url: &str) -> (String, String, bool) {
    // 返回 (host, path, is_secure)
    if let Ok(parsed) = url::Url::parse(url) {
        let secure = parsed.scheme() == "https";
        return (
            parsed.host_str().unwrap_or("").to_string(),
            parsed.path().to_string(),
            secure,
        );
    }
    (String::new(), "/".into(), false)
}

fn month_index(m: &str) -> Option<i64> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let lower = m.to_ascii_lowercase();
    MONTHS.iter().position(|x| lower.starts_with(x)).map(|i| i as i64)
}

/// 解析 RFC 1123 等常见日期（"Wed, 21 Oct 2015 07:28:00 GMT"）
fn parse_http_date(raw: &str) -> Option<i64> {
    let cleaned = raw.replace(',', " ");
    let parts: Vec<&str> = cleaned.split_whitespace().collect();
    // 找到形如 21 Oct 2015 07:28:00 的连续四段
    for i in 0..parts.len().saturating_sub(3) {
        let day = parts[i].parse::<i64>().ok();
        let month = month_index(parts[i + 1]);
        let year = parts[i + 2].parse::<i64>().ok();
        let time: Vec<&str> = parts[i + 3].split(':').collect();
        if let (Some(day), Some(month), Some(year)) = (day, month, year) {
            if time.len() >= 2 {
                let hour = time[0].parse::<i64>().unwrap_or(0);
                let minute = time[1].parse::<i64>().unwrap_or(0);
                let second = time.get(2).and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
                // 简化换算（1970 起的秒），足够做过期判断
                let mut days = 0i64;
                for y in 1970..year {
                    days += if is_leap(y) { 366 } else { 365 };
                }
                for m in 0..month {
                    days += month_days(year, m);
                }
                days += day - 1;
                return Some(days * 86400 + hour * 3600 + minute * 60 + second);
            }
        }
    }
    None
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn month_days(year: i64, month_index: i64) -> i64 {
    match month_index {
        0 | 2 | 4 | 6 | 7 | 9 | 11 => 31,
        3 | 5 | 8 | 10 => 30,
        1 => {
            if is_leap(year) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

/// 公历 → 1970 起的天数（Howard Hinnant 算法），月从 1 开始。
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn epoch_from_parts(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> i64 {
    days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + s
}

/// 解析 `2026-11-18T07:28` / `2026-11-18T07:28:00.000Z` / 带 `+08:00` 偏移的 ISO 时间。
fn parse_iso_date(raw: &str) -> Option<i64> {
    let cleaned = raw.trim();
    if cleaned.is_empty() {
        return None;
    }
    let (date_part, rest) = cleaned.split_once('T').or_else(|| cleaned.split_once(' '))?;
    let d: Vec<i64> = date_part
        .split('-')
        .filter_map(|s| s.parse::<i64>().ok())
        .collect();
    if d.len() != 3 {
        return None;
    }
    let mut time_part = rest.trim();
    let mut offset = 0i64;
    if let Some(stripped) = time_part.strip_suffix('Z').or_else(|| time_part.strip_suffix('z')) {
        time_part = stripped;
    } else if let Some(idx) = time_part.rfind(['+', '-']) {
        let sign = if time_part[idx..].starts_with('-') { -1 } else { 1 };
        let tail: Vec<i64> = time_part[idx + 1..]
            .split(':')
            .filter_map(|s| s.parse::<i64>().ok())
            .collect();
        offset = sign * (tail.first().copied().unwrap_or(0) * 3600 + tail.get(1).copied().unwrap_or(0) * 60);
        time_part = &time_part[..idx];
    }
    let t: Vec<i64> = time_part
        .split(':')
        .filter_map(|s| s.split('.').next().and_then(|v| v.parse::<i64>().ok()))
        .collect();
    Some(
        epoch_from_parts(
            d[0],
            d[1],
            d[2],
            t.first().copied().unwrap_or(0),
            t.get(1).copied().unwrap_or(0),
            t.get(2).copied().unwrap_or(0),
        ) - offset,
    )
}

/// 把前端给的过期时间统一成 Unix 秒。
/// 支持：数字（秒或毫秒，>1e12 视为毫秒）、ISO 字符串、RFC 1123 字符串、null/Infinity。
fn expires_of(v: Option<&serde_json::Value>) -> Option<i64> {
    match v? {
        serde_json::Value::Number(n) => {
            let raw = n.as_f64()?;
            if !raw.is_finite() || raw <= 0.0 {
                None
            } else if raw > 1e12 {
                Some((raw / 1000.0) as i64)
            } else {
                Some(raw as i64)
            }
        }
        serde_json::Value::String(s) => {
            let s = s.trim();
            if s.is_empty() || s.eq_ignore_ascii_case("infinity") {
                None
            } else {
                parse_iso_date(s).or_else(|| parse_http_date(s))
            }
        }
        _ => None,
    }
}

fn json_str(v: Option<&serde_json::Value>) -> Option<String> {
    v.and_then(|x| x.as_str()).map(|s| s.to_string())
}

fn json_bool(v: Option<&serde_json::Value>) -> bool {
    match v {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => s == "true",
        Some(serde_json::Value::Number(n)) => n.as_f64().unwrap_or(0.0) != 0.0,
        _ => false,
    }
}

/// 从任意前端 cookie 对象里取名字：面板用 `key`，新建面板/Electron 用 `name`。
fn cookie_name_of(v: &serde_json::Value) -> String {
    json_str(v.get("key"))
        .or_else(|| json_str(v.get("name")))
        .unwrap_or_default()
}

impl CookieJar {
    pub fn from_json(raw: &str) -> Self {
        serde_json::from_str(raw).unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// 处理一次响应的全部 Set-Cookie
    pub fn store_response(&mut self, url: &str, set_cookies: &[String]) {
        let (host, req_path, _) = host_of(url);
        if host.is_empty() {
            return;
        }
        for raw in set_cookies {
            let mut parts = raw.split(';');
            let Some(pair) = parts.next() else { continue };
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            let name = name.trim().to_string();
            if name.is_empty() {
                continue;
            }
            let mut cookie = StoredCookie {
                name,
                value: value.trim().to_string(),
                domain: host.clone(),
                path: req_path.clone(),
                host_only: true,
                secure: false,
                http_only: false,
                expires: None,
                same_site: None,
            };
            if cookie.path.is_empty() {
                cookie.path = "/".into();
            }
            for attr in parts {
                let attr = attr.trim();
                let (key, val) = match attr.split_once('=') {
                    Some((k, v)) => (k.trim().to_ascii_lowercase(), v.trim().to_string()),
                    None => (attr.to_ascii_lowercase(), String::new()),
                };
                match key.as_str() {
                    "domain" => {
                        let domain = val.trim_start_matches('.').to_lowercase();
                        if !domain.is_empty() {
                            cookie.domain = domain;
                            cookie.host_only = false;
                        }
                    }
                    "path" => {
                        if val.starts_with('/') {
                            cookie.path = val;
                        }
                    }
                    "secure" => cookie.secure = true,
                    "httponly" => cookie.http_only = true,
                    "max-age" => {
                        if let Ok(secs) = val.parse::<i64>() {
                            cookie.expires = Some(now() + secs);
                        }
                    }
                    "expires" => {
                        if let Some(ts) = parse_http_date(&val) {
                            cookie.expires = Some(ts);
                        }
                    }
                    "samesite" => {
                        if !val.is_empty() {
                            cookie.same_site = Some(val);
                        }
                    }
                    _ => {}
                }
            }
            // 过期 cookie 视为删除
            if matches!(cookie.expires, Some(ts) if ts <= now()) {
                self.cookies.retain(|c| {
                    !(c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path)
                });
                continue;
            }
            self.cookies.retain(|c| {
                !(c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path)
            });
            self.cookies.push(cookie);
        }
    }

    fn matches(cookie: &StoredCookie, host: &str, path: &str, secure: bool) -> bool {
        if cookie.secure && !secure {
            return false;
        }
        let domain_ok = if cookie.host_only {
            host.eq_ignore_ascii_case(&cookie.domain)
        } else {
            let h = host.to_ascii_lowercase();
            let d = cookie.domain.to_ascii_lowercase();
            h == d || h.ends_with(&format!(".{d}"))
        };
        if !domain_ok {
            return false;
        }
        if cookie.path == "/" {
            return true;
        }
        path == cookie.path || path.starts_with(&format!("{}/", cookie.path.trim_end_matches('/')))
    }

    /// 为请求生成 `Cookie` 头的值
    pub fn header_for(&mut self, url: &str) -> Option<String> {
        let (host, path, secure) = host_of(url);
        if host.is_empty() {
            return None;
        }
        let ts = now();
        self.cookies
            .retain(|c| !matches!(c.expires, Some(t) if t <= ts));
        let mut pairs: Vec<(usize, String)> = self
            .cookies
            .iter()
            .enumerate()
            .filter(|(_, c)| Self::matches(c, &host, &path, secure))
            .map(|(i, c)| (i, format!("{}={}", c.name, c.value)))
            .collect();
        // 路径更长的优先
        pairs.sort_by_key(|(i, _)| std::cmp::Reverse(self.cookies[*i].path.len()));
        if pairs.is_empty() {
            None
        } else {
            Some(
                pairs
                    .into_iter()
                    .map(|(_, p)| p)
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        }
    }

    /// 前端 Cookies 面板用的列表：按域名分组，二级里是行数据。
    ///
    /// 面板直接渲染 `cookie.key` / `cookie.value` / `cookie.expires`（毫秒时间戳，
    /// 交给 `new Date()`），分组在前端不做，所以这里返回 `{domain, cookies:[]}` 数组。
    pub fn list(&self) -> Vec<serde_json::Value> {
        let mut order: Vec<String> = Vec::new();
        let mut grouped: std::collections::HashMap<String, Vec<serde_json::Value>> =
            std::collections::HashMap::new();
        let mut sorted = self.cookies.clone();
        sorted.sort_by(|a, b| a.domain.cmp(&b.domain).then(a.path.cmp(&b.path)).then(a.name.cmp(&b.name)));
        for c in &sorted {
            if !grouped.contains_key(&c.domain) {
                order.push(c.domain.clone());
            }
            grouped.entry(c.domain.clone()).or_default().push(serde_json::json!({
                "key": c.name,
                "value": c.value,
                "domain": c.domain,
                "path": c.path,
                "hostOnly": c.host_only,
                "secure": c.secure,
                "httpOnly": c.http_only,
                "session": c.expires.is_none(),
                "expires": c.expires.map(|ts| ts * 1000),
            }));
        }
        order
            .into_iter()
            .map(|domain| {
                let rows = grouped.remove(&domain).unwrap_or_default();
                serde_json::json!({ "domain": domain, "cookies": rows })
            })
            .collect()
    }

    pub fn clear_domain(&mut self, domain: &str) {
        self.cookies.retain(|c| !c.domain.eq_ignore_ascii_case(domain));
    }

    pub fn delete(&mut self, domain: &str, path: &str, name: &str) {
        self.cookies.retain(|c| {
            !(c.domain.eq_ignore_ascii_case(domain) && c.path == path && c.name == name)
        });
    }

    pub fn upsert(&mut self, fallback_domain: &str, cookie: &serde_json::Value) {
        let name = cookie_name_of(cookie);
        if name.is_empty() {
            return;
        }
        let domain = json_str(cookie.get("domain"))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| fallback_domain.to_string())
            .trim_start_matches('.')
            .to_string();
        let path = json_str(cookie.get("path"))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/".to_string());
        let stored = StoredCookie {
            name,
            value: json_str(cookie.get("value")).unwrap_or_default(),
            domain: domain.clone(),
            path: path.clone(),
            host_only: json_bool(cookie.get("hostOnly")),
            secure: json_bool(cookie.get("secure")),
            http_only: json_bool(cookie.get("httpOnly")),
            expires: expires_of(cookie.get("expires"))
                .or_else(|| expires_of(cookie.get("expirationDate"))),
            same_site: json_str(cookie.get("sameSite")),
        };
        self.cookies
            .retain(|c| !(c.name == stored.name && c.domain == stored.domain && c.path == stored.path));
        self.cookies.push(stored);
    }

    pub fn modify(&mut self, domain: &str, old: &serde_json::Value, new: &serde_json::Value) {
        let old_name = cookie_name_of(old);
        let old_path = json_str(old.get("path")).unwrap_or_else(|| "/".to_string());
        let old_domain = json_str(old.get("domain")).unwrap_or_else(|| domain.to_string());
        self.delete(&old_domain, &old_path, &old_name);
        self.upsert(domain, new);
    }
}

/// 1970 起的天数 → 公历（days_from_civil 的逆运算）
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Unix 秒 → `Wed, 21 Oct 2015 07:28:00 GMT`
fn to_http_date(ts: i64) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    let days = ts.div_euclid(86400);
    let secs = ts.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
        DAYS[(days.rem_euclid(7)) as usize],
        d,
        MONTHS[(m - 1) as usize],
        y,
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// 解析用户输入的整条 cookie（"a=1; Path=/; Domain=x; Secure"）成表单字段。
/// 返回 None 表示不是合法 cookie 串（前端会 toast 报错）。
pub fn parse_raw_cookie(raw: &str) -> Option<serde_json::Value> {
    let mut parts = raw.split(';');
    let first = parts.next()?.trim();
    let (key, value) = first.split_once('=')?;
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    let mut out = serde_json::json!({ "key": key, "value": value.trim() });
    for attr in parts {
        let attr = attr.trim();
        if attr.is_empty() {
            continue;
        }
        let (k, v) = match attr.split_once('=') {
            Some((k, v)) => (k.trim().to_ascii_lowercase(), v.trim().to_string()),
            None => (attr.to_ascii_lowercase(), String::new()),
        };
        match k.as_str() {
            "path" => out["path"] = serde_json::json!(v),
            "domain" => out["domain"] = serde_json::json!(v.trim_start_matches('.')),
            "expires" => {
                if let Some(ts) = parse_http_date(&v).or_else(|| parse_iso_date(&v)) {
                    out["expires"] = serde_json::json!(ts * 1000);
                }
            }
            "max-age" => {
                if let Ok(secs) = v.parse::<i64>() {
                    out["expires"] = serde_json::json!((now() + secs) * 1000);
                }
            }
            "secure" => out["secure"] = serde_json::json!(true),
            "httponly" => out["httpOnly"] = serde_json::json!(true),
            "samesite" => out["sameSite"] = serde_json::json!(v),
            _ => {}
        }
    }
    Some(out)
}

/// 反向：表单字段 → 标准 cookie 字符串
pub fn create_raw_cookie(cookie: &serde_json::Value) -> String {
    let key = cookie_name_of(cookie);
    if key.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "{}={}",
        key,
        json_str(cookie.get("value")).unwrap_or_default()
    );
    if let Some(path) = json_str(cookie.get("path")).filter(|s| !s.is_empty()) {
        out.push_str(&format!("; Path={path}"));
    }
    if let Some(domain) = json_str(cookie.get("domain")).filter(|s| !s.is_empty()) {
        out.push_str(&format!("; Domain={domain}"));
    }
    if let Some(ts) = expires_of(cookie.get("expires")) {
        out.push_str(&format!("; Expires={}", to_http_date(ts)));
    }
    if json_bool(cookie.get("secure")) {
        out.push_str("; Secure");
    }
    if json_bool(cookie.get("httpOnly")) {
        out.push_str("; HttpOnly");
    }
    if let Some(same_site) = json_str(cookie.get("sameSite")).filter(|s| !s.is_empty()) {
        out.push_str(&format!("; SameSite={same_site}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_returns_cookies_by_domain_and_path() {
        let mut jar = CookieJar::default();
        jar.store_response(
            "https://api.example.com/v1/login",
            &["sid=abc; Path=/; HttpOnly".to_string(), "scoped=1; Path=/v1".to_string()],
        );
        assert_eq!(
            jar.header_for("https://api.example.com/v1/orders").as_deref(),
            Some("scoped=1; sid=abc")
        );
        // 非 https 不带 secure cookie；这里 cookie 本身没 secure，仍应带上
        assert!(jar.header_for("http://api.example.com/v1/orders").is_some());
        // 别的域名不带
        assert!(jar.header_for("https://other.com/").is_none());
        // path 不匹配
        assert_eq!(
            jar.header_for("https://api.example.com/other").as_deref(),
            Some("sid=abc")
        );
    }

    #[test]
    fn honours_secure_flag() {
        let mut jar = CookieJar::default();
        jar.store_response("https://a.com/", &["t=1; Secure".to_string()]);
        assert!(jar.header_for("http://a.com/").is_none());
        assert!(jar.header_for("https://a.com/").is_some());
    }

    #[test]
    fn expired_cookie_is_removed() {
        let mut jar = CookieJar::default();
        jar.store_response("https://a.com/", &["x=1; Max-Age=0".to_string()]);
        assert!(jar.cookies.is_empty());
    }

    #[test]
    fn parses_http_date_and_domain_cookie() {
        assert_eq!(parse_http_date("Wed, 21 Oct 2015 07:28:00 GMT"), Some(1445412480));
        let mut jar = CookieJar::default();
        jar.store_response("https://sub.example.com/", &["d=1; Domain=.example.com".to_string()]);
        assert!(jar.header_for("https://api.example.com/").is_some());
        assert!(jar.header_for("https://notexample.com/").is_none());
    }

    #[test]
    fn ui_crud_round_trip() {
        let mut jar = CookieJar::default();
        jar.upsert(
            "example.com",
            &serde_json::json!({ "key": "a", "value": "1", "path": "/", "domain": "example.com" }),
        );
        assert_eq!(jar.list().len(), 1);
        assert_eq!(jar.list()[0]["domain"], "example.com");
        assert_eq!(jar.list()[0]["cookies"][0]["key"], "a");
        jar.modify(
            "example.com",
            &serde_json::json!({ "key": "a", "path": "/", "domain": "example.com" }),
            &serde_json::json!({ "key": "a", "value": "2", "path": "/", "domain": "example.com" }),
        );
        assert_eq!(jar.list()[0]["cookies"][0]["value"], "2");
        jar.delete("example.com", "/", "a");
        assert!(jar.list().is_empty());
    }

    #[test]
    fn parses_and_serializes_raw_cookie() {
        let parsed = parse_raw_cookie("sid=abc; Path=/v1; Domain=.example.com; Secure; HttpOnly").unwrap();
        assert_eq!(parsed["key"], "sid");
        assert_eq!(parsed["value"], "abc");
        assert_eq!(parsed["path"], "/v1");
        assert_eq!(parsed["domain"], "example.com");
        assert_eq!(parsed["secure"], true);
        assert_eq!(parsed["httpOnly"], true);
        assert!(parse_raw_cookie("no-equals").is_none());

        let raw = create_raw_cookie(&parsed);
        assert!(raw.starts_with("sid=abc; Path=/v1; Domain=example.com;"));
        assert!(raw.contains("; Secure"));
        // 往返一致
        assert_eq!(parse_raw_cookie(&raw).unwrap()["value"], "abc");
    }

    #[test]
    fn converts_dates_both_ways() {
        assert_eq!(parse_iso_date("2015-10-21T07:28:00Z"), Some(1445412480));
        assert_eq!(parse_iso_date("2015-10-21T15:28:00+08:00"), Some(1445412480));
        assert_eq!(to_http_date(1445412480), "Wed, 21 Oct 2015 07:28:00 GMT");
        assert_eq!(expires_of(Some(&serde_json::json!(1445412480000i64))), Some(1445412480));
        assert_eq!(expires_of(Some(&serde_json::json!(1445412480i64))), Some(1445412480));
        assert_eq!(expires_of(Some(&serde_json::json!(null))), None);
    }
}
