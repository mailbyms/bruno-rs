//! 导入落盘：把前端传来的 Bruno 内存集合写成 OpenCollection YAML 目录。
//!
//! 原版由 Electron 主进程按 `brunoConfig` 决定写 `.bru` 还是 `.yml`。这里
//! 只支持 yml（本项目的既定范围），忽略 `options.format`。
//!
//! 目录结构：
//! ```text
//! <location>/<collection name>/
//!   opencollection.yml
//!   <request>.yml
//!   <folder>/
//!     folder.yml
//! ```
//! 请求文件复用 `persist::save_request_file`，保证与「保存请求」走同一套映射。

use crate::persist;
use serde_json::{json, Value as J};
use std::path::{Path, PathBuf};

/// 导入结果条目：`{ name, path }`
pub fn import_collection(collection: &J, location: &str) -> Result<Vec<J>, String> {
    let location = location.trim();
    if location.is_empty() {
        return Err("导入位置不能为空".into());
    }
    let location_path = PathBuf::from(location);
    if !location_path.is_absolute() {
        return Err(format!("导入位置必须是绝对路径：{location}"));
    }
    std::fs::create_dir_all(&location_path)
        .map_err(|e| format!("创建导入目录失败 {}：{e}", location_path.display()))?;

    let collections: Vec<&J> = match collection {
        J::Array(arr) => arr.iter().collect(),
        other => vec![other],
    };

    let mut imported = Vec::new();
    for item in collections {
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Imported Collection".to_string());
        let dir = unique_dir(&location_path, &safe_name(&name, "collection"));
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("创建集合目录失败 {}：{e}", dir.display()))?;

        write_root_file(&dir, item, &name)?;
        write_items(&dir, item.get("items"))?;

        imported.push(json!({
            "name": dir.file_name().and_then(|n| n.to_str()).unwrap_or(&name),
            "path": slash(&dir)
        }));
    }
    Ok(imported)
}

fn write_root_file(dir: &Path, collection: &J, name: &str) -> Result<(), String> {
    let mut root = json!({
        "opencollection": "1.0.0",
        "info": { "name": name }
    });

    if let Some(request) = collection
        .get("root")
        .and_then(|r| r.get("request"))
        .filter(|v| v.is_object())
    {
        let defaults = defaults_to_json(request);
        if defaults.as_object().map(|o| !o.is_empty()).unwrap_or(false) {
            root["request"] = defaults;
        }
    }

    let text = serde_yml::to_string(&serde_yml::to_value(&root).map_err(|e| e.to_string())?)
        .map_err(|e| format!("序列化 opencollection.yml 失败：{e}"))?;
    std::fs::write(dir.join(crate::collection::ROOT_FILE), text)
        .map_err(|e| format!("写入 opencollection.yml 失败：{e}"))
}

/// 集合/文件夹级默认值（前端 root.request）→ 磁盘 request 段
fn defaults_to_json(request: &J) -> J {
    let mut out = serde_json::Map::new();

    if let Some(headers) = request.get("headers").and_then(|v| v.as_array()) {
        let list: Vec<J> = headers
            .iter()
            .filter_map(|h| {
                let name = h.get("name").and_then(|v| v.as_str())?;
                if name.is_empty() {
                    return None;
                }
                let mut entry = json!({
                    "name": name,
                    "value": h.get("value").and_then(|v| v.as_str()).unwrap_or("")
                });
                if h.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
                    entry["disabled"] = json!(true);
                }
                Some(entry)
            })
            .collect();
        if !list.is_empty() {
            out.insert("headers".into(), json!(list));
        }
    }

    if let Some(auth) = request.get("auth") {
        if let Some(disk) = persist::ui_auth_to_disk(auth) {
            if let Ok(value) = serde_json::to_value(&disk) {
                out.insert("auth".into(), value);
            }
        }
    }

    if let Some(vars) = request
        .get("vars")
        .and_then(|v| v.get("req"))
        .and_then(|v| v.as_array())
    {
        let list: Vec<J> = vars
            .iter()
            .filter_map(|v| {
                let name = v.get("name").and_then(|x| x.as_str())?;
                if name.is_empty() {
                    return None;
                }
                let mut entry = json!({
                    "name": name,
                    "value": v.get("value").and_then(|x| x.as_str()).unwrap_or("")
                });
                if v.get("enabled").and_then(|x| x.as_bool()) == Some(false) {
                    entry["disabled"] = json!(true);
                }
                Some(entry)
            })
            .collect();
        if !list.is_empty() {
            out.insert("variables".into(), json!(list));
        }
    }

    let mut scripts = Vec::new();
    for (kind, text) in [
        ("before-request", script_text(request, "script", "req")),
        ("after-response", script_text(request, "script", "res")),
        ("tests", request.get("tests").and_then(|v| v.as_str()).map(|s| s.to_string())),
    ] {
        if let Some(code) = text.filter(|s| !s.trim().is_empty()) {
            scripts.push(json!({ "type": kind, "code": code }));
        }
    }
    if !scripts.is_empty() {
        out.insert("scripts".into(), json!(scripts));
    }

    J::Object(out)
}

fn script_text(request: &J, group: &str, key: &str) -> Option<String> {
    request
        .get(group)
        .and_then(|s| s.get(key))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn write_items(dir: &Path, items: Option<&J>) -> Result<(), String> {
    let Some(items) = items.and_then(|v| v.as_array()) else {
        return Ok(());
    };
    let mut used = std::collections::HashSet::new();
    // 同一层里文件夹名先占位，避免请求文件与兄弟文件夹同名（Orders/ 与 Orders.yml）
    for item in items {
        if item.get("items").is_some() && item.get("request").is_none() {
            if let Some(name) = item.get("name").and_then(|v| v.as_str()) {
                used.insert(safe_name(name, "folder"));
            }
        }
    }

    for (idx, item) in items.iter().enumerate() {
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("Item {}", idx + 1));
        let seq = item.get("seq").and_then(|v| v.as_i64()).unwrap_or(idx as i64 + 1);

        // 有 items 且没有 request = 文件夹（对齐前端判定）
        let is_folder = item.get("items").is_some() && item.get("request").is_none();
        if is_folder {
            let folder_dir = unique_dir(dir, &safe_name(&name, "folder"));
            std::fs::create_dir_all(&folder_dir)
                .map_err(|e| format!("创建文件夹失败 {}：{e}", folder_dir.display()))?;
            let mut root = json!({ "info": { "name": name, "seq": seq } });
            if let Some(request) = item.get("root").and_then(|r| r.get("request")) {
                let defaults = defaults_to_json(request);
                if defaults.as_object().map(|o| !o.is_empty()).unwrap_or(false) {
                    root["request"] = defaults;
                }
            }
            let text =
                serde_yml::to_string(&serde_yml::to_value(&root).map_err(|e| e.to_string())?)
                    .map_err(|e| format!("序列化 folder.yml 失败：{e}"))?;
            std::fs::write(folder_dir.join(crate::collection::FOLDER_FILE), text)
                .map_err(|e| format!("写入 folder.yml 失败：{e}"))?;
            write_items(&folder_dir, item.get("items"))?;
            continue;
        }

        let Some(request) = item.get("request").filter(|v| v.is_object()) else {
            continue;
        };
        let mut file_name = safe_name(&name, "request");
        if !used.insert(file_name.clone()) {
            let mut n = 2;
            loop {
                let candidate = format!("{file_name}-{n}");
                if used.insert(candidate.clone()) {
                    file_name = candidate;
                    break;
                }
                n += 1;
            }
        }
        let path = dir.join(format!("{file_name}.yml"));

        let mut payload = json!({
            "name": name,
            "type": "http-request",
            "seq": seq,
            "request": request
        });
        if let Some(settings) = item.get("settings").filter(|v| v.is_object()) {
            payload["settings"] = settings.clone();
        }
        if let Some(tags) = item.get("tags").filter(|v| v.is_array()) {
            payload["tags"] = tags.clone();
        }
        persist::save_request_file(&path, &payload)?;
    }
    Ok(())
}

/// 目录名安全化：去掉路径分隔符与 Windows 非法字符，保留中文与空格
fn safe_name(name: &str, fallback: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    let short: String = trimmed.chars().take(60).collect();
    let result = short.trim().to_string();
    if result.is_empty() {
        fallback.to_string()
    } else {
        result
    }
}

fn unique_dir(parent: &Path, name: &str) -> PathBuf {
    let candidate = parent.join(name);
    if !candidate.exists() {
        return candidate;
    }
    for n in 2..1000 {
        let candidate = parent.join(format!("{name}-{n}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{name}-{}", crate::model::new_uid()))
}

fn slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collection::load_collection;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bruno-rs-import-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn collection() -> J {
        json!({
            "uid": "abc",
            "name": "Shop API",
            "items": [
                {
                    "name": "Orders",
                    "type": "folder",
                    "seq": 1,
                    "items": [{
                        "name": "List Orders",
                        "type": "http-request",
                        "seq": 1,
                        "request": {
                            "method": "GET",
                            "url": "http://127.0.0.1:1/orders",
                            "headers": [{ "name": "Accept", "value": "application/json", "enabled": true }],
                            "params": [{ "name": "limit", "value": "10", "type": "query", "enabled": true }],
                            "body": { "mode": "none", "json": "", "text": "", "xml": "", "sparql": "", "formUrlEncoded": [], "multipartForm": [], "file": [] },
                            "auth": { "mode": "none" },
                            "vars": { "req": [], "res": [] },
                            "script": { "req": "bru.setVar('a','b');", "res": "" },
                            "tests": "test('ok', function () { expect(res.getStatus()).to.equal(200); });",
                            "assertions": [],
                            "docs": ""
                        },
                        "settings": { "encodeUrl": true, "timeout": 0 }
                    }]
                },
                {
                    "name": "Create Order",
                    "type": "http-request",
                    "seq": 2,
                    "request": {
                        "method": "POST",
                        "url": "http://127.0.0.1:1/orders",
                        "headers": [],
                        "params": [],
                        "body": { "mode": "json", "json": "{\"item\":\"coffee\"}", "text": "", "xml": "", "sparql": "", "formUrlEncoded": [], "multipartForm": [], "file": [] },
                        "auth": { "mode": "apikey", "apikey": { "key": "X-Api-Key", "value": "secret", "placement": "queryparams" } },
                        "vars": { "req": [], "res": [] },
                        "script": { "req": "", "res": "" },
                        "tests": "",
                        "assertions": [],
                        "docs": ""
                    },
                    "settings": { "encodeUrl": true, "timeout": 0 }
                }
            ],
            "root": {
                "request": {
                    "headers": [],
                    "auth": { "mode": "bearer", "bearer": { "token": "{{token}}" } },
                    "vars": { "req": [{ "name": "baseUrl", "value": "http://127.0.0.1:1", "enabled": true }], "res": [] },
                    "script": { "req": "", "res": "" },
                    "tests": ""
                }
            }
        })
    }

    #[test]
    fn writes_yaml_tree_and_reloads() {
        let dir = temp_dir("tree");
        let imported = import_collection(&collection(), dir.to_str().unwrap()).expect("import");
        assert_eq!(imported.len(), 1);
        let root = PathBuf::from(imported[0]["path"].as_str().unwrap());
        assert!(root.join("opencollection.yml").is_file());
        assert!(root.join("Orders").join("folder.yml").is_file());
        assert!(root.join("Orders").join("List Orders.yml").is_file());
        assert!(root.join("Create Order.yml").is_file());

        let loaded = load_collection(&root).expect("reload");
        assert_eq!(loaded.name, "Shop API");
        assert_eq!(loaded.tree.len(), 2);

        let folder = loaded.tree.iter().find(|n| n.kind == "folder").unwrap();
        assert_eq!(folder.name, "Orders");
        assert_eq!(folder.items.len(), 1);
        assert!(folder.request.is_none(), "文件夹不能带 request 键");

        let leaf = &folder.items[0];
        assert_eq!(leaf.kind, "http-request");
        let url = leaf.request.as_ref().unwrap()["url"].as_str().unwrap();
        assert_eq!(url, "http://127.0.0.1:1/orders");

        let create = loaded
            .tree
            .iter()
            .find(|n| n.name == "Create Order")
            .expect("create order");
        let auth = &create.request.as_ref().unwrap()["auth"];
        assert_eq!(auth["mode"], "apikey");
        assert_eq!(auth["apikey"]["placement"], "queryparams");

        // 集合级默认值写进 opencollection.yml
        let root_yaml = std::fs::read_to_string(root.join("opencollection.yml")).unwrap();
        assert!(root_yaml.contains("bearer"));
        assert!(root_yaml.contains("baseUrl"));
    }

    #[test]
    fn dedupes_names_and_rejects_relative_location() {
        let dir = temp_dir("dup");
        let mut col = collection();
        col["items"][1]["name"] = json!("Orders");
        let imported = import_collection(&col, dir.to_str().unwrap()).unwrap();
        let root = PathBuf::from(imported[0]["path"].as_str().unwrap());
        // 文件夹 Orders 与请求 Orders 同名：请求落成 Orders-2.yml
        assert!(root.join("Orders").is_dir());
        assert!(root.join("Orders-2.yml").is_file());
        assert!(import_collection(&col, "relative/dir").is_err());
    }

    #[test]
    fn safe_name_keeps_chinese() {
        assert_eq!(safe_name("订单 列表", "x"), "订单 列表");
        assert_eq!(safe_name("a/b:c*", "x"), "a-b-c-");
        assert_eq!(safe_name("   ", "x"), "x");
    }
}
