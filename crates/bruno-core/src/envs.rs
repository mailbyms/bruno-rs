//! 环境变量文件操作。
//!
//! 三种作用域共用同一套 YAML 结构，只是目录不同：
//! - 集合环境：`<collection>/environments/*.yml`
//! - 工作区环境：`<workspace>/environments/*.yml`（Bruno 2.0 的工作区级环境）
//! - 全局环境：`<data>/global-environments.json`（原版是 electron-store）
//!
//! uid 由文件绝对路径 hash 得到（`ops::stable_uid`），保证同一文件每次读到的 uid 不变，
//! 前端按 uid 定位/保存才不会串。

use crate::ops::{stable_uid, to_path, to_slash};
use serde_json::{json, Value as J};
use serde_yml::Value as Y;
use std::path::{Path, PathBuf};

type Map = serde_yml::Mapping;

const ENV_EXT: &str = "yml";

pub fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').to_string();
    if trimmed.is_empty() {
        "environment".into()
    } else {
        trimmed
    }
}

fn env_dir(base: &str) -> PathBuf {
    to_path(base).join("environments")
}

fn var_to_disk(v: &J) -> Option<Y> {
    let name = v.get("name").and_then(|x| x.as_str())?;
    if name.is_empty() {
        return None;
    }
    let mut m = Map::new();
    m.insert(Y::String("name".into()), Y::String(name.to_string()));
    match v.get("value") {
        Some(J::String(s)) => {
            m.insert(Y::String("value".into()), Y::String(s.clone()));
        }
        Some(J::Object(_)) => {
            // 类型化值原样保留 { type, data }
            if let Ok(y) = serde_yml::to_value(v.get("value").unwrap()) {
                m.insert(Y::String("value".into()), y);
            }
        }
        Some(J::Number(n)) => {
            m.insert(Y::String("value".into()), Y::String(n.to_string()));
        }
        Some(J::Bool(b)) => {
            m.insert(Y::String("value".into()), Y::String(b.to_string()));
        }
        _ => {
            m.insert(Y::String("value".into()), Y::String(String::new()));
        }
    }
    if v.get("enabled").and_then(|x| x.as_bool()) == Some(false) {
        m.insert(Y::String("disabled".into()), Y::Bool(true));
    }
    if v.get("secret").and_then(|x| x.as_bool()) == Some(true) {
        m.insert(Y::String("secret".into()), Y::Bool(true));
    }
    if let Some(desc) = v.get("description").and_then(|x| x.as_str()) {
        if !desc.is_empty() {
            m.insert(Y::String("description".into()), Y::String(desc.to_string()));
        }
    }
    Some(Y::Mapping(m))
}

fn var_to_ui(v: &Y, index: usize, env_seed: &str) -> J {
    let map = v.as_mapping();
    let get = |k: &str| map.and_then(|m| m.get(Y::String(k.into())));
    let name = get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let value = match get("value") {
        Some(Y::String(s)) => J::String(s.clone()),
        Some(Y::Number(n)) => J::String(n.to_string()),
        Some(Y::Bool(b)) => J::String(b.to_string()),
        Some(Y::Mapping(_)) => serde_json::to_value(get("value").unwrap()).unwrap_or(J::Null),
        _ => J::String(String::new()),
    };
    let disabled = matches!(get("disabled"), Some(Y::Bool(true)));
    let secret = matches!(get("secret"), Some(Y::Bool(true)));
    let description = get("description").and_then(|x| x.as_str()).unwrap_or("");
    let mut out = json!({
        "uid": format!("{}{}", stable_uid(&format!("{env_seed}:{name}:{index}")), ""),
        "name": name,
        "value": value,
        "enabled": !disabled,
        "secret": secret,
    });
    if !description.is_empty() {
        out["description"] = json!(description);
    }
    out
}

fn env_to_ui(file: &Path, doc: &Y) -> J {
    let map = doc.as_mapping();
    let get = |k: &str| map.and_then(|m| m.get(Y::String(k.into())));
    let pathname = to_slash(file);
    let stem = file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("environment")
        .to_string();
    let name = get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| stem.clone());
    let variables = get("variables")
        .and_then(|v| v.as_sequence())
        .map(|list| {
            list.iter()
                .enumerate()
                .map(|(i, v)| var_to_ui(v, i, &pathname))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut out = json!({
        "uid": stable_uid(&pathname),
        "name": name,
        "pathname": pathname,
        "filename": file.file_name().and_then(|s| s.to_str()).unwrap_or(""),
        "variables": variables,
    });
    if let Some(color) = get("color").and_then(|v| v.as_str()) {
        out["color"] = json!(color);
    }
    if let Some(extends) = get("extends").and_then(|v| v.as_str()) {
        out["extends"] = json!(extends);
    }
    out
}

/// 读取目录下所有环境文件（按文件名排序，与集合加载一致）
pub fn list(base: &str) -> Vec<J> {
    let dir = env_dir(base);
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return out;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|x| x.to_str())
                    .map(|x| x.eq_ignore_ascii_case(ENV_EXT) || x.eq_ignore_ascii_case("yaml"))
                    .unwrap_or(false)
        })
        .collect();
    files.sort();
    for file in files {
        let Ok(raw) = std::fs::read_to_string(&file) else { continue };
        let Ok(doc) = serde_yml::from_str::<Y>(&raw) else { continue };
        out.push(env_to_ui(&file, &doc));
    }
    out
}

fn find_file(base: &str, name: &str) -> Option<PathBuf> {
    let dir = env_dir(base);
    let direct = dir.join(format!("{}.{ENV_EXT}", sanitize_filename(name)));
    if direct.is_file() {
        return Some(direct);
    }
    // 文件名与 name 不一致时按内容匹配
    for entry in std::fs::read_dir(&dir).ok()?.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.file_stem().and_then(|s| s.to_str()) == Some(name) {
            return Some(path);
        }
        let Ok(raw) = std::fs::read_to_string(&path) else { continue };
        let Ok(doc) = serde_yml::from_str::<Y>(&raw) else { continue };
        let parsed = doc
            .as_mapping()
            .and_then(|m| m.get(Y::String("name".into())))
            .and_then(|v| v.as_str());
        if parsed == Some(name) {
            return Some(path);
        }
    }
    None
}

fn write_env(file: &Path, env: &J) -> Result<(), String> {
    let mut doc = Map::new();
    doc.insert(
        Y::String("name".into()),
        Y::String(env.get("name").and_then(|v| v.as_str()).unwrap_or("environment").to_string()),
    );
    let variables: Vec<Y> = env
        .get("variables")
        .and_then(|v| v.as_array())
        .map(|list| list.iter().filter_map(var_to_disk).collect())
        .unwrap_or_default();
    doc.insert(Y::String("variables".into()), Y::Sequence(variables));
    if let Some(color) = env.get("color").and_then(|v| v.as_str()) {
        if !color.is_empty() {
            doc.insert(Y::String("color".into()), Y::String(color.to_string()));
        }
    }
    if let Some(extends) = env.get("extends").and_then(|v| v.as_str()) {
        if !extends.is_empty() {
            doc.insert(Y::String("extends".into()), Y::String(extends.to_string()));
        }
    }
    if let Some(dot_env) = env.get("dotEnvFilePath").and_then(|v| v.as_str()) {
        if !dot_env.is_empty() {
            doc.insert(Y::String("dotEnvFilePath".into()), Y::String(dot_env.to_string()));
        }
    }
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建环境目录失败：{e}"))?;
    }
    let body = serde_yml::to_string(&Y::Mapping(doc)).map_err(|e| format!("序列化环境失败：{e}"))?;
    std::fs::write(file, body).map_err(|e| format!("写入环境失败：{e}"))
}

pub fn create(
    base: &str,
    name: &str,
    variables: Option<&J>,
    color: Option<&str>,
    extends: Option<&str>,
) -> Result<J, String> {
    let dir = env_dir(base);
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建环境目录失败：{e}"))?;
    let stem = sanitize_filename(name);
    let mut file = dir.join(format!("{stem}.{ENV_EXT}"));
    for i in 2..1000 {
        if !file.exists() {
            break;
        }
        file = dir.join(format!("{stem}-{i}.{ENV_EXT}"));
    }
    let mut env = json!({
        "name": name,
        "variables": variables.cloned().unwrap_or_else(|| json!([])),
    });
    if let Some(color) = color {
        env["color"] = json!(color);
    }
    if let Some(extends) = extends {
        env["extends"] = json!(extends);
    }
    write_env(&file, &env)?;
    let raw = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    let doc: Y = serde_yml::from_str(&raw).map_err(|e| e.to_string())?;
    Ok(env_to_ui(&file, &doc))
}

pub fn save(base: &str, env: &J) -> Result<J, String> {
    let name = env
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "环境缺少 name".to_string())?;
    let file = find_file(base, name)
        .unwrap_or_else(|| env_dir(base).join(format!("{}.{ENV_EXT}", sanitize_filename(name))));
    write_env(&file, env)?;
    let raw = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    let doc: Y = serde_yml::from_str(&raw).map_err(|e| e.to_string())?;
    Ok(env_to_ui(&file, &doc))
}

pub fn delete(base: &str, name: &str) -> Result<(), String> {
    if let Some(file) = find_file(base, name) {
        std::fs::remove_file(&file).map_err(|e| format!("删除环境失败：{e}"))?;
    }
    Ok(())
}

pub fn rename(base: &str, old_name: &str, new_name: &str) -> Result<J, String> {
    let Some(old) = find_file(base, old_name) else {
        return Err(format!("环境不存在：{old_name}"));
    };
    let dir = env_dir(base);
    let stem = sanitize_filename(new_name);
    let mut new_path = dir.join(format!("{stem}.{ENV_EXT}"));
    for i in 2..1000 {
        if !new_path.exists() || new_path == old {
            break;
        }
        new_path = dir.join(format!("{stem}-{i}.{ENV_EXT}"));
    }
    let raw = std::fs::read_to_string(&old).map_err(|e| e.to_string())?;
    let mut doc: Y = serde_yml::from_str(&raw).unwrap_or(Y::Mapping(Map::new()));
    if let Some(map) = doc.as_mapping_mut() {
        map.insert(Y::String("name".into()), Y::String(new_name.to_string()));
    }
    let body = serde_yml::to_string(&doc).map_err(|e| e.to_string())?;
    std::fs::write(&new_path, body).map_err(|e| format!("重命名环境失败：{e}"))?;
    if new_path != old {
        std::fs::remove_file(&old).ok();
    }
    let after: Y = serde_yml::from_str(&std::fs::read_to_string(&new_path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    Ok(env_to_ui(&new_path, &after))
}

pub fn update_color(base: &str, name: &str, color: &str) -> Result<J, String> {
    let Some(file) = find_file(base, name) else {
        return Err(format!("环境不存在：{name}"));
    };
    let raw = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    let mut doc: Y = serde_yml::from_str(&raw).unwrap_or(Y::Mapping(Map::new()));
    if let Some(map) = doc.as_mapping_mut() {
        if color.is_empty() {
            map.remove(Y::String("color".into()));
        } else {
            map.insert(Y::String("color".into()), Y::String(color.to_string()));
        }
    }
    let body = serde_yml::to_string(&doc).map_err(|e| e.to_string())?;
    std::fs::write(&file, body).map_err(|e| e.to_string())?;
    let after: Y = serde_yml::from_str(&std::fs::read_to_string(&file).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    Ok(env_to_ui(&file, &after))
}

pub fn save_extends(base: &str, name: &str, extends: &str) -> Result<J, String> {
    let Some(file) = find_file(base, name) else {
        return Err(format!("环境不存在：{name}"));
    };
    let raw = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    let mut doc: Y = serde_yml::from_str(&raw).unwrap_or(Y::Mapping(Map::new()));
    if let Some(map) = doc.as_mapping_mut() {
        if extends.is_empty() {
            map.remove(Y::String("extends".into()));
        } else {
            map.insert(Y::String("extends".into()), Y::String(extends.to_string()));
        }
    }
    let body = serde_yml::to_string(&doc).map_err(|e| e.to_string())?;
    std::fs::write(&file, body).map_err(|e| e.to_string())?;
    let after: Y = serde_yml::from_str(&std::fs::read_to_string(&file).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    Ok(env_to_ui(&file, &after))
}

/// 导出为 JSON（前端 `export-environment` 的落盘端）
pub fn export_json(environments: &J, path: &str) -> Result<(), String> {
    let body = serde_json::to_string_pretty(environments).map_err(|e| e.to_string())?;
    crate::ops::save_text_file(path, &body)
}

// ============================================================================
// 全局环境（<data>/global-environments.json）
// ============================================================================

pub fn load_global(store: &Path) -> J {
    let raw = std::fs::read_to_string(store).unwrap_or_default();
    let parsed: J = serde_json::from_str(&raw).unwrap_or_else(|_| json!({}));
    json!({
        "environments": parsed.get("environments").cloned().unwrap_or_else(|| json!([])),
        "activeUid": parsed.get("activeUid").cloned().unwrap_or(J::Null),
    })
}

fn write_global(store: &Path, value: &J) -> Result<(), String> {
    if let Some(parent) = store.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let body = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(store, body).map_err(|e| format!("写入全局环境失败：{e}"))
}

/// 新建/更新全局环境。
///
/// 前端传参不统一：新建用 `uid`，其余动作（保存变量、改名、改色、继承）用
/// `environmentUid` 且只带局部字段。所以这里做**按键合并**而不是整体替换，
/// 否则「保存变量」会把 name/color 冲掉。
pub fn global_upsert(store: &Path, env: &J) -> Result<J, String> {
    let mut state = load_global(store);
    let uid = env
        .get("uid")
        .or_else(|| env.get("environmentUid"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            stable_uid(&format!(
                "global:{}",
                env.get("name").and_then(|v| v.as_str()).unwrap_or("")
            ))
        });

    let mut patch = env.clone();
    if let Some(obj) = patch.as_object_mut() {
        for key in ["uid", "environmentUid", "workspaceUid", "workspacePath"] {
            obj.remove(key);
        }
    }
    patch["uid"] = json!(uid);

    let list = state["environments"].as_array_mut().unwrap();
    let mut merged = list
        .iter()
        .find(|e| e.get("uid").and_then(|v| v.as_str()) == Some(uid.as_str()))
        .cloned()
        .unwrap_or_else(|| json!({}));
    if let (Some(target), Some(src)) = (merged.as_object_mut(), patch.as_object()) {
        for (k, v) in src {
            target.insert(k.clone(), v.clone());
        }
    }
    merged["uid"] = json!(uid);

    if let Some(existing) = list
        .iter_mut()
        .find(|e| e.get("uid").and_then(|v| v.as_str()) == Some(uid.as_str()))
    {
        *existing = merged.clone();
    } else {
        list.push(merged.clone());
    }
    write_global(store, &state)?;
    Ok(merged)
}

pub fn global_delete(store: &Path, uid: &str) -> Result<(), String> {
    let mut state = load_global(store);
    if let Some(list) = state["environments"].as_array_mut() {
        list.retain(|e| e.get("uid").and_then(|v| v.as_str()) != Some(uid));
    }
    if state["activeUid"].as_str() == Some(uid) {
        state["activeUid"] = J::Null;
    }
    write_global(store, &state)
}

pub fn global_select(store: &Path, uid: Option<&str>) -> Result<(), String> {
    let mut state = load_global(store);
    state["activeUid"] = uid.map(|u| json!(u)).unwrap_or(J::Null);
    write_global(store, &state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("brs-env-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn create_save_and_list_environment() {
        let dir = tmp("crud");
        let base = to_slash(&dir);
        let created = create(
            &base,
            "local",
            Some(&json!([{ "name": "baseUrl", "value": "http://a", "enabled": true }])),
            Some("#fff"),
            None,
        )
        .unwrap();
        assert_eq!(created["name"], "local");
        assert!(dir.join("environments/local.yml").is_file());
        assert_eq!(created["variables"][0]["name"], "baseUrl");

        let mut env = created.clone();
        env["variables"] = json!([
            { "name": "baseUrl", "value": "http://b", "enabled": true },
            { "name": "token", "value": "s", "enabled": true, "secret": true }
        ]);
        let saved = save(&base, &env).unwrap();
        assert_eq!(saved["variables"][1]["secret"], true);
        let raw = std::fs::read_to_string(dir.join("environments/local.yml")).unwrap();
        assert!(raw.contains("secret: true"), "{raw}");
        assert!(raw.contains("http://b"));

        let all = list(&base);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0]["name"], "local");
    }

    #[test]
    fn rename_and_delete_environment() {
        let dir = tmp("rename");
        let base = to_slash(&dir);
        create(&base, "dev", None, None, None).unwrap();
        let renamed = rename(&base, "dev", "staging").unwrap();
        assert_eq!(renamed["name"], "staging");
        assert!(!dir.join("environments/dev.yml").exists());
        assert!(dir.join("environments/staging.yml").is_file());
        delete(&base, "staging").unwrap();
        assert!(list(&base).is_empty());
    }

    #[test]
    fn global_store_round_trip() {
        let dir = tmp("global");
        let store = dir.join("global-environments.json");
        global_upsert(&store, &json!({ "name": "g", "variables": [] })).unwrap();
        let state = load_global(&store);
        assert_eq!(state["environments"].as_array().unwrap().len(), 1);
        let uid = state["environments"][0]["uid"].as_str().unwrap().to_string();
        global_select(&store, Some(&uid)).unwrap();
        assert_eq!(load_global(&store)["activeUid"], uid);
        global_delete(&store, &uid).unwrap();
        assert!(load_global(&store)["environments"].as_array().unwrap().is_empty());
    }
}

/// 按 uid（文件路径 hash）找环境文件
pub fn path_by_uid(base: &str, uid: &str) -> Option<PathBuf> {
    let dir = env_dir(base);
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if stable_uid(&to_slash(&path)) == uid {
            return Some(path);
        }
    }
    None
}

pub fn save_by_uid(base: &str, uid: &str, env: &J) -> Result<J, String> {
    let file = path_by_uid(base, uid).ok_or_else(|| format!("环境不存在：{uid}"))?;
    write_env(&file, env)?;
    let raw = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    let doc: Y = serde_yml::from_str(&raw).map_err(|e| e.to_string())?;
    Ok(env_to_ui(&file, &doc))
}

pub fn delete_by_uid(base: &str, uid: &str) -> Result<(), String> {
    if let Some(file) = path_by_uid(base, uid) {
        std::fs::remove_file(&file).map_err(|e| format!("删除环境失败：{e}"))?;
    }
    Ok(())
}

pub fn rename_by_uid(base: &str, uid: &str, new_name: &str) -> Result<J, String> {
    let file = path_by_uid(base, uid).ok_or_else(|| format!("环境不存在：{uid}"))?;
    let name = file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    rename(base, &name, new_name)
}

pub fn copy_by_uid(base: &str, uid: &str, new_name: &str) -> Result<J, String> {
    let file = path_by_uid(base, uid).ok_or_else(|| format!("环境不存在：{uid}"))?;
    let raw = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    let doc: Y = serde_yml::from_str(&raw).map_err(|e| e.to_string())?;
    let current = env_to_ui(&file, &doc);
    create(
        base,
        new_name,
        current.get("variables"),
        current.get("color").and_then(|v| v.as_str()),
        current.get("extends").and_then(|v| v.as_str()),
    )
}
