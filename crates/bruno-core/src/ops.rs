//! 集合 / 请求 / 文件夹的写操作：CRUD、集合根与文件夹根保存、ZIP 导出。
//!
//! 约定：
//! - 输入路径可能来自前端 `path.join`（Windows 上是反斜杠），统一走 `to_path` 归一化；
//!   返回给前端的一律是 `/` 分隔的相对路径，和 `collection.rs` 的 `relative_path` 保持一致。
//! - 同名冲突不报错，按 `-2`、`-3` 递增（原版 Electron 也是静默处理）。
//! - 落盘都走「临时文件 + rename」，避免半截文件被 watcher 读成损坏。

use crate::collection::settings_to_ui;
use crate::model::HttpRequest;
use serde_json::{json, Value as J};
use serde_yml::Value as Y;
use std::path::{Path, PathBuf};

type Map = serde_yml::Mapping;

pub const ROOT_FILE: &str = "opencollection.yml";
pub const FOLDER_FILE: &str = "folder.yml";
const DEFAULT_GITIGNORE: &str = "node_modules\n.git\n.DS_Store\n";

// ============================================================================
// 路径
// ============================================================================

pub fn to_path(input: &str) -> PathBuf {
    if std::path::MAIN_SEPARATOR == '\\' {
        PathBuf::from(input.replace('/', "\\"))
    } else {
        PathBuf::from(input)
    }
}

pub fn to_slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn stem_of(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string()
}

fn rel_path(root: &Path, path: &Path) -> String {
    to_slash(path.strip_prefix(root).unwrap_or(path))
}

/// 稳定 uid：同一路径每次加载都得到同一个 21 位 id（环境变量按 uid 定位，不能每次变）
pub fn stable_uid(seed: &str) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in seed.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let mut out = String::with_capacity(21);
    for i in 0..21 {
        let idx = ((hash >> ((i % 10) * 6)) as usize)
            .wrapping_add(i * 37)
            % ALPHABET.len();
        out.push(ALPHABET[idx] as char);
    }
    out
}

/// 目标已存在时追加 `-2`、`-3`…；目录没有扩展名，文件保留扩展名
pub fn unique_path(desired: &Path) -> PathBuf {
    if !desired.exists() {
        return desired.to_path_buf();
    }
    let parent = desired.parent().unwrap_or_else(|| Path::new("."));
    let is_dir_target = desired.extension().is_none();
    let (base, ext) = if is_dir_target {
        (desired.file_name().and_then(|s| s.to_str()).unwrap_or("item").to_string(), String::new())
    } else {
        (
            stem_of(desired),
            desired
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default(),
        )
    };
    for i in 2..1000 {
        let candidate = parent.join(format!("{base}-{i}{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    desired.to_path_buf()
}

// ============================================================================
// YAML 读写
// ============================================================================

fn read_yaml(path: &Path) -> Result<Y, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
    let value: Y = serde_yml::from_str(&raw).map_err(|e| format!("解析 {} 失败：{e}", path.display()))?;
    Ok(if value.is_mapping() { value } else { Y::Mapping(Map::new()) })
}

fn write_text_atomic(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
    }
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("yml")
    ));
    std::fs::write(&tmp, content).map_err(|e| format!("写入临时文件失败：{e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("替换文件失败：{e}")
    })
}

fn write_yaml(path: &Path, value: &Y) -> Result<(), String> {
    let body = serde_yml::to_string(value).map_err(|e| format!("序列化 YAML 失败：{e}"))?;
    write_text_atomic(path, &body)
}

fn key(name: &str) -> Y {
    Y::String(name.to_string())
}

fn set_str(map: &mut Map, name: &str, value: &str) {
    map.insert(key(name), Y::String(value.to_string()));
}

fn take_map(root: &mut Map, name: &str) -> Map {
    match root.remove(key(name)) {
        Some(Y::Mapping(m)) => m,
        _ => Map::new(),
    }
}

// ============================================================================
// 前端 request 结构 → YAML 片段
// ============================================================================

fn headers_to_disk(headers: Option<&J>) -> Option<Y> {
    let list = headers?.as_array()?;
    let items: Vec<Y> = list
        .iter()
        .filter_map(|h| {
            let name = h.get("name").and_then(|v| v.as_str())?;
            if name.is_empty() {
                return None;
            }
            let mut m = Map::new();
            set_str(&mut m, "name", name);
            set_str(&mut m, "value", h.get("value").and_then(|v| v.as_str()).unwrap_or(""));
            if h.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
                m.insert(key("disabled"), Y::Bool(true));
            }
            let description = h.get("description").and_then(|v| v.as_str()).unwrap_or("");
            if !description.is_empty() {
                set_str(&mut m, "description", description);
            }
            Some(Y::Mapping(m))
        })
        .collect();
    if items.is_empty() {
        None
    } else {
        Some(Y::Sequence(items))
    }
}

fn vars_to_disk(vars: Option<&J>) -> Option<Y> {
    // 兼容 { req: [...] } 与直接数组两种输入
    let list = vars
        .and_then(|v| v.get("req").or(Some(v)))
        .and_then(|v| v.as_array())?;
    let items: Vec<Y> = list
        .iter()
        .filter_map(|v| {
            let name = v.get("name").and_then(|x| x.as_str())?;
            if name.is_empty() {
                return None;
            }
            let mut m = Map::new();
            set_str(&mut m, "name", name);
            set_str(&mut m, "value", v.get("value").and_then(|x| x.as_str()).unwrap_or(""));
            if v.get("enabled").and_then(|x| x.as_bool()) == Some(false) {
                m.insert(key("disabled"), Y::Bool(true));
            }
            let description = v.get("description").and_then(|x| x.as_str()).unwrap_or("");
            if !description.is_empty() {
                set_str(&mut m, "description", description);
            }
            Some(Y::Mapping(m))
        })
        .collect();
    if items.is_empty() {
        None
    } else {
        Some(Y::Sequence(items))
    }
}

fn scripts_to_disk(script: Option<&J>, tests: Option<&str>) -> Option<Y> {
    let mut items: Vec<Y> = Vec::new();
    let push = |items: &mut Vec<Y>, kind: &str, code: Option<&str>| {
        let Some(code) = code else { return };
        if code.trim().is_empty() {
            return;
        }
        let mut m = Map::new();
        set_str(&mut m, "type", kind);
        set_str(&mut m, "code", code);
        items.push(Y::Mapping(m));
    };
    push(
        &mut items,
        "before-request",
        script.and_then(|s| s.get("req")).and_then(|v| v.as_str()),
    );
    push(
        &mut items,
        "after-response",
        script.and_then(|s| s.get("res")).and_then(|v| v.as_str()),
    );
    push(&mut items, "tests", tests);
    if items.is_empty() {
        None
    } else {
        Some(Y::Sequence(items))
    }
}

/// 把前端 `request`（headers/auth/vars/script/tests）合并进 YAML 的根映射
fn merge_request_defaults(root: &mut Map, request: &J) {
    let mut section = take_map(root, "request");
    match headers_to_disk(request.get("headers")) {
        Some(v) => {
            section.insert(key("headers"), v);
        }
        None => {
            section.remove(key("headers"));
        }
    }
    match request.get("auth").and_then(crate::persist::ui_auth_to_disk) {
        Some(y) => {
            section.insert(key("auth"), y);
        }
        None => {
            section.remove(key("auth"));
        }
    }
    match vars_to_disk(request.get("vars")) {
        Some(v) => {
            let mut vars = take_map(&mut section, "vars");
            vars.insert(key("req"), v);
            section.insert(key("vars"), Y::Mapping(vars));
        }
        None => {
            section.remove(key("vars"));
        }
    }
    match scripts_to_disk(
        request.get("script"),
        request.get("tests").and_then(|v| v.as_str()),
    ) {
        Some(v) => {
            section.insert(key("script"), v);
        }
        None => {
            section.remove(key("script"));
        }
    }
    if !section.is_empty() {
        root.insert(key("request"), Y::Mapping(section));
    }
}

fn merge_docs(root: &mut Map, docs: Option<&J>) {
    match docs {
        Some(J::String(s)) if !s.trim().is_empty() => {
            root.insert(key("docs"), Y::String(s.clone()));
        }
        Some(J::Object(_)) => {
            root.insert(
                key("docs"),
                serde_yml::to_value(docs.unwrap()).unwrap_or(Y::Null),
            );
        }
        _ => {}
    }
}

// ============================================================================
// 请求节点
// ============================================================================

/// 从一个请求 yml 生成前端树节点（新文件创建后用它回给前端 / 发事件）
pub fn node_from_file(abs: &Path, root: &Path) -> Result<J, String> {
    let raw = std::fs::read_to_string(abs).map_err(|e| format!("读取请求失败：{e}"))?;
    let req: HttpRequest = serde_yml::from_str(&raw).map_err(|e| format!("解析请求失败：{e}"))?;
    let filename = stem_of(abs);
    let name = req
        .info
        .name
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| filename.clone());
    Ok(json!({
        "uid": crate::model::new_uid(),
        "name": name,
        // http / websocket / grpc 三类分别落到前端的 http-request / ws-request / grpc-request
        "type": req.kind(),
        "seq": req.info.seq.unwrap_or(1),
        "tags": req.info.tags.clone(),
        "filename": filename,
        "pathname": rel_path(root, abs),
        "request": crate::collection::normalize_request(&req),
        "settings": if req.kind() == "http-request" {
            serde_json::Value::from(settings_to_ui(&req.settings))
        } else {
            serde_json::Value::Null
        },
    }))
}

/// 新建请求：`pathname` 带扩展名；同名时自动加 `-2`
pub fn new_request(pathname: &str, item: &J) -> Result<J, String> {
    let desired = to_path(pathname);
    // 前端传的是「集合内的完整文件路径」；传成目录会让 unique_path 造出
    // 一个同级的 `xxx-2` 文件，直接报错更容易发现
    if desired.is_dir() {
        return Err(format!(
            "新建请求的路径是目录而不是文件：{}",
            desired.display()
        ));
    }
    let target = unique_path(&desired);
    crate::persist::save_request_file(&target, item)?;
    let root = item
        .get("collectionPathname")
        .and_then(|v| v.as_str())
        .map(to_path);
    // 没有 collectionPathname 时用文件所在目录兜底（前端只关心 pathname 字段）
    let root = root.unwrap_or_else(|| target.parent().unwrap_or(Path::new(".")).to_path_buf());
    node_from_file(&target, &root)
}

/// 新建文件夹：写 folder.yml
pub fn new_folder(pathname: &str, folder_data: &J) -> Result<J, String> {
    let desired = to_path(pathname);
    let target = unique_path(&desired);
    std::fs::create_dir_all(&target).map_err(|e| format!("创建文件夹失败：{e}"))?;
    let meta = folder_data.get("meta").cloned().unwrap_or(json!({}));
    let name = meta
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| target.file_name().and_then(|s| s.to_str()).unwrap_or("New Folder"));
    let mut doc = Map::new();
    let mut info = Map::new();
    set_str(&mut info, "name", name);
    if let Some(seq) = meta.get("seq").and_then(|v| v.as_i64()) {
        info.insert(key("seq"), Y::Number(seq.into()));
    }
    doc.insert(key("info"), Y::Mapping(info));
    if let Some(request) = folder_data.get("request") {
        merge_request_defaults(&mut doc, request);
    }
    write_yaml(&target.join(FOLDER_FILE), &Y::Mapping(doc))?;
    Ok(json!({
        "uid": crate::model::new_uid(),
        "name": name,
        "type": "folder",
        "seq": meta.get("seq").and_then(|v| v.as_i64()).unwrap_or(1),
        "pathname": to_slash(&target),
        "filename": target.file_name().and_then(|s| s.to_str()).unwrap_or(""),
    }))
}

pub fn delete_item(pathname: &str, kind: &str) -> Result<(), String> {
    let path = to_path(pathname);
    if !path.exists() {
        return Ok(());
    }
    if kind == "folder" || path.is_dir() {
        std::fs::remove_dir_all(&path).map_err(|e| format!("删除文件夹失败：{e}"))
    } else {
        std::fs::remove_file(&path).map_err(|e| format!("删除文件失败：{e}"))
    }
}

/// 移动（拖拽）：返回前端需要对齐的新 pathname
pub fn move_item(target_dir: &str, source: &str) -> Result<J, String> {
    let src = to_path(source);
    if !src.exists() {
        return Err(format!("源路径不存在：{}", src.display()));
    }
    let dir = to_path(target_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("目标目录不可用：{e}"))?;
    let file_name = src
        .file_name()
        .ok_or_else(|| "源路径没有文件名".to_string())?
        .to_string_lossy()
        .to_string();
    let mut dest = dir.join(&file_name);
    // 同目录移动 = 无操作
    if let (Ok(a), Ok(b)) = (src.canonicalize(), dest.canonicalize()) {
        if a == b {
            return Ok(json!({ "newPathname": to_slash(&src) }));
        }
    }
    if dest.exists() {
        if dest.is_dir() {
            let stem = dest.file_name().and_then(|s| s.to_str()).unwrap_or("item").to_string();
            for i in 2..1000 {
                let candidate = dir.join(format!("{stem}-{i}"));
                if !candidate.exists() {
                    dest = candidate;
                    break;
                }
            }
        } else {
            dest = unique_path(&dest);
        }
    }
    std::fs::rename(&src, &dest).map_err(|e| format!("移动失败：{e}"))?;
    Ok(json!({ "newPathname": to_slash(&dest) }))
}

pub fn rename_item_name(pathname: &str, new_name: &str) -> Result<(), String> {
    let path = to_path(pathname);
    let file = if path.is_dir() { path.join(FOLDER_FILE) } else { path };
    if !file.is_file() {
        return Ok(());
    }
    let mut doc = read_yaml(&file)?;
    let root = doc.as_mapping_mut().ok_or("YAML 根不是映射")?;
    let mut info = take_map(root, "info");
    set_str(&mut info, "name", new_name);
    root.insert(key("info"), Y::Mapping(info));
    write_yaml(&file, &doc)
}

pub fn rename_item_filename(old: &str, new: &str, new_name: &str) -> Result<String, String> {
    let src = to_path(old);
    let mut dest = to_path(new);
    if !src.exists() {
        return Err(format!("路径不存在：{}", src.display()));
    }
    // 仅大小写变化（Windows 大小写不敏感）时 canonicalize 相等，直接改名
    let same_file = matches!((src.canonicalize(), dest.canonicalize()), (Ok(a), Ok(b)) if a == b);
    if !same_file && dest.exists() {
        dest = if src.is_dir() {
            unique_path(&dest)
        } else {
            unique_path(&dest)
        };
    }
    if src != dest {
        std::fs::rename(&src, &dest).map_err(|e| format!("重命名失败：{e}"))?;
    }
    if !new_name.trim().is_empty() {
        rename_item_name(&to_slash(&dest), new_name)?;
    }
    Ok(to_slash(&dest))
}

pub fn resequence(items: &J) -> Result<(), String> {
    let Some(list) = items.as_array() else {
        return Ok(());
    };
    for item in list {
        let Some(pathname) = item.get("pathname").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(seq) = item.get("seq").and_then(|v| v.as_i64()) else {
            continue;
        };
        let path = to_path(pathname);
        let file = if path.is_dir() { path.join(FOLDER_FILE) } else { path };
        if !file.is_file() {
            continue;
        }
        let mut doc = read_yaml(&file)?;
        let Some(root) = doc.as_mapping_mut() else { continue };
        let mut info = take_map(root, "info");
        info.insert(key("seq"), Y::Number(seq.into()));
        root.insert(key("info"), Y::Mapping(info));
        write_yaml(&file, &doc)?;
    }
    Ok(())
}

// ============================================================================
// 集合
// ============================================================================

/// 新建集合：返回目录路径（前端随后用 main:collection-opened 打开）
pub fn create_collection(name: &str, folder_name: &str, location: &str) -> Result<String, String> {
    let parent = to_path(location);
    std::fs::create_dir_all(&parent).map_err(|e| format!("目标目录不可用：{e}"))?;
    let desired = parent.join(folder_name);
    // 已存在且为空目录时复用（与 Electron 一致）
    let dir = if desired.is_dir() {
        let empty = std::fs::read_dir(&desired)
            .map(|mut it| it.next().is_none())
            .unwrap_or(false);
        if empty {
            desired
        } else {
            unique_path(&desired)
        }
    } else {
        unique_path(&desired)
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建集合目录失败：{e}"))?;
    let mut doc = Map::new();
    doc.insert(key("opencollection"), Y::String("1.0.0".into()));
    let mut info = Map::new();
    set_str(&mut info, "name", name);
    doc.insert(key("info"), Y::Mapping(info));
    write_yaml(&dir.join(ROOT_FILE), &Y::Mapping(doc))?;
    write_text_atomic(&dir.join(".gitignore"), DEFAULT_GITIGNORE)?;
    Ok(to_slash(&dir))
}

pub fn clone_collection(
    name: &str,
    folder_name: &str,
    location: &str,
    previous: &str,
) -> Result<String, String> {
    let dir = create_collection(name, folder_name, location)?;
    let target = to_path(&dir);
    let source = to_path(previous);
    copy_tree(&source, &target, true)?;
    // 根配置由 create_collection 写好，复制时跳过，避免覆盖新名字
    let _ = std::fs::remove_file(target.join(ROOT_FILE));
    std::fs::copy(source.join(ROOT_FILE), target.join(ROOT_FILE))
        .map_err(|e| format!("复制集合配置失败：{e}"))?;
    // 覆盖 info.name
    rename_item_name(&to_slash(&target), name)?;
    Ok(dir)
}

pub fn clone_folder(target: &str, source: &str) -> Result<J, String> {
    let src = to_path(source);
    let desired = to_path(target);
    std::fs::create_dir_all(desired.parent().unwrap_or(Path::new(".")))
        .map_err(|e| format!("创建父目录失败：{e}"))?;
    let dest = unique_path(&desired);
    copy_tree(&src, &dest, false)?;
    Ok(json!({
        "uid": crate::model::new_uid(),
        "name": dest.file_name().and_then(|s| s.to_str()).unwrap_or("Folder"),
        "type": "folder",
        "pathname": to_slash(&dest),
        "filename": dest.file_name().and_then(|s| s.to_str()).unwrap_or(""),
    }))
}

fn copy_tree(src: &Path, dest: &Path, skip_root_config: bool) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("创建目录失败：{e}"))?;
    for entry in std::fs::read_dir(src).map_err(|e| format!("读取目录失败：{e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();
        if file_name == ".git" && !skip_root_config {
            continue;
        }
        if path.is_dir() {
            if file_name == ".git" {
                continue;
            }
            copy_tree(&path, &dest.join(&file_name), false)?;
        } else if !(skip_root_config && file_name == ROOT_FILE) {
            std::fs::copy(&path, dest.join(&file_name)).map_err(|e| format!("复制文件失败：{e}"))?;
        }
    }
    Ok(())
}

pub fn save_collection_root(pathname: &str, root: &J, bruno_config: Option<&J>) -> Result<(), String> {
    let dir = to_path(pathname);
    let file = dir.join(ROOT_FILE);
    let mut doc = read_yaml(&file)?;
    let map = doc.as_mapping_mut().ok_or("集合配置不是 YAML 映射")?;
    if !map.contains_key(key("opencollection")) {
        map.insert(key("opencollection"), Y::String("1.0.0".into()));
    }
    // info：名字与版本
    let mut info = take_map(map, "info");
    if let Some(config) = bruno_config {
        if let Some(name) = config.get("name").and_then(|v| v.as_str()) {
            set_str(&mut info, "name", name);
        }
        if let Some(version) = config.get("version").and_then(|v| v.as_str()) {
            if !version.is_empty() {
                set_str(&mut info, "version", version);
            }
        }
    }
    if !info.is_empty() {
        map.insert(key("info"), Y::Mapping(info));
    }
    if let Some(request) = root.get("request") {
        merge_request_defaults(map, request);
    }
    let docs = root.get("docs").or_else(|| root.get("meta").and_then(|m| m.get("docs")));
    merge_docs(map, docs);
    write_yaml(&file, &doc)
}

pub fn save_folder_root(doc_in: &J) -> Result<(), String> {
    let folder_path = doc_in
        .get("folderPathname")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "缺少 folderPathname".to_string())?;
    let dir = to_path(folder_path);
    let file = dir.join(FOLDER_FILE);
    let mut doc = if file.is_file() {
        read_yaml(&file)?
    } else {
        Y::Mapping(Map::new())
    };
    let map = doc.as_mapping_mut().ok_or("folder.yml 不是映射")?;
    let mut info = take_map(map, "info");
    if let Some(name) = doc_in.get("name").and_then(|v| v.as_str()) {
        set_str(&mut info, "name", name);
    }
    if let Some(root) = doc_in.get("root") {
        if let Some(meta_name) = root
            .get("meta")
            .and_then(|m| m.get("name"))
            .and_then(|v| v.as_str())
        {
            set_str(&mut info, "name", meta_name);
        }
        if let Some(seq) = root.get("meta").and_then(|m| m.get("seq")).and_then(|v| v.as_i64()) {
            info.insert(key("seq"), Y::Number(seq.into()));
        }
        if let Some(tags) = root.get("meta").and_then(|m| m.get("tags")).and_then(|v| v.as_array()) {
            let list: Vec<Y> = tags
                .iter()
                .filter_map(|t| t.as_str().map(|s| Y::String(s.to_string())))
                .collect();
            info.insert(key("tags"), Y::Sequence(list));
        }
        if let Some(request) = root.get("request") {
            merge_request_defaults(map, request);
        }
        merge_docs(map, root.get("docs"));
    }
    map.insert(key("info"), Y::Mapping(info));
    write_yaml(&file, &doc)
}

pub fn save_text_file(pathname: &str, content: &str) -> Result<(), String> {
    write_text_atomic(&to_path(pathname), content)
}

/// 集合设置里的安全配置（JS 沙箱模式等）留在 `opencollection.yml` 的 `settings.security`
pub fn read_security_config(pathname: &str) -> J {
    let file = to_path(pathname).join(ROOT_FILE);
    let Ok(doc) = read_yaml(&file) else {
        return json!({ "jsSandboxMode": "safe" });
    };
    doc.get("settings")
        .and_then(|s| s.get("security"))
        .map(|security| {
            serde_json::to_value(security).unwrap_or_else(|_| json!({ "jsSandboxMode": "safe" }))
        })
        .unwrap_or_else(|| json!({ "jsSandboxMode": "safe" }))
}

pub fn save_security_config(pathname: &str, config: &J) -> Result<(), String> {
    let file = to_path(pathname).join(ROOT_FILE);
    let mut doc = read_yaml(&file)?;
    let map = doc.as_mapping_mut().ok_or("集合配置不是 YAML 映射")?;
    let mut settings = take_map(map, "settings");
    let mut security = take_map(&mut settings, "security");
    if let Some(obj) = config.as_object() {
        for (name, value) in obj {
            match serde_yml::to_value(value) {
                Ok(y) => {
                    security.insert(key(name), y);
                }
                Err(_) => {}
            }
        }
    }
    settings.insert(key("security"), Y::Mapping(security));
    map.insert(key("settings"), Y::Mapping(settings));
    write_yaml(&file, &doc)
}

/// 判断 zip 是不是 Bruno / OpenCollection 集合包（根目录或单层子目录里有 opencollection.yml / bruno.json）
pub fn is_collection_zip(zip_path: &Path) -> Result<bool, String> {
    let file = std::fs::File::open(zip_path).map_err(|e| format!("打开 zip 失败：{e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("解析 zip 失败：{e}"))?;
    for index in 0..archive.len() {
        let Ok(entry) = archive.by_index(index) else { continue };
        let name = entry.name().replace('\\', "/");
        let lower = name.to_ascii_lowercase();
        let base = lower.rsplit('/').next().unwrap_or("");
        if base == "opencollection.yml" || base == "bruno.json" || base == "collection.bru" {
            return Ok(true);
        }
    }
    Ok(false)
}

/// 解压集合包到目标目录（目标目录里直接放集合文件）
pub fn import_zip(zip_path: &Path, target_dir: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| format!("打开 zip 失败：{e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("解析 zip 失败：{e}"))?;
    // zip 内可能整体套了一层同名目录，先探测共同前缀
    let mut names: Vec<String> = Vec::new();
    for index in 0..archive.len() {
        if let Ok(entry) = archive.by_index(index) {
            names.push(entry.name().replace('\\', "/"));
        }
    }
    let prefix = common_prefix_dir(&names);
    std::fs::create_dir_all(target_dir).map_err(|e| format!("创建目录失败：{e}"))?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("读取 zip 条目失败：{e}"))?;
        let raw = entry.name().replace('\\', "/");
        let rel = raw.strip_prefix(&prefix).unwrap_or(&raw).trim_start_matches('/');
        if rel.is_empty() {
            continue;
        }
        // 阻止 zip slip
        let rel_path = PathBuf::from(rel);
        if rel_path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            continue;
        }
        let out = target_dir.join(&rel_path);
        if entry.is_dir() || rel.ends_with('/') {
            std::fs::create_dir_all(&out).map_err(|e| format!("创建目录失败：{e}"))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
        }
        let mut outfile = std::fs::File::create(&out).map_err(|e| format!("写入文件失败：{e}"))?;
        std::io::copy(&mut entry, &mut outfile).map_err(|e| format!("解压文件失败：{e}"))?;
    }
    Ok(())
}

/// 所有条目都在同一个顶层目录下时返回 `dir/`，否则返回空串
fn common_prefix_dir(names: &[String]) -> String {
    let mut first: Option<&str> = None;
    for name in names {
        let name = name.trim_start_matches('/');
        if name.is_empty() {
            continue;
        }
        let head = match name.split_once('/') {
            Some((head, _)) => head,
            // 有根级文件 → 没有共同前缀
            None => return String::new(),
        };
        match first {
            None => first = Some(head),
            Some(existing) if existing == head => {}
            _ => return String::new(),
        }
    }
    first.map(|head| format!("{head}/")).unwrap_or_default()
}

// ============================================================================
// 导出
// ============================================================================

/// 把目录打成 zip（保留相对路径，跳过 .git / node_modules）
pub fn export_zip(src_dir: &Path, zip_path: &Path) -> Result<(), String> {
    use zip::write::SimpleFileOptions;
    let file = std::fs::File::create(zip_path).map_err(|e| format!("创建 zip 失败：{e}"))?;
    let mut writer = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut stack = vec![src_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(v) => v,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if name == ".git" || name == "node_modules" {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let rel = path
                .strip_prefix(src_dir)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| name.clone());
            let mut f = std::fs::File::open(&path).map_err(|e| format!("读取文件失败：{e}"))?;
            writer
                .start_file(rel, options)
                .map_err(|e| format!("写入 zip 条目失败：{e}"))?;
            std::io::copy(&mut f, &mut writer).map_err(|e| format!("写入 zip 失败：{e}"))?;
        }
    }
    writer.finish().map_err(|e| format!("完成 zip 失败：{e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("brs-ops-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn creates_request_with_unique_name() {
        let dir = tmp("new-request");
        let item = json!({
            "name": "Ping",
            "seq": 1,
            "request": { "method": "GET", "url": "http://example.com", "headers": [], "params": [], "auth": { "mode": "none" } }
        });
        let a = new_request(&to_slash(&dir.join("ping.yml")), &item).unwrap();
        let b = new_request(&to_slash(&dir.join("ping.yml")), &item).unwrap();
        assert_eq!(a["name"], "Ping");
        assert!(dir.join("ping.yml").is_file());
        assert!(dir.join("ping-2.yml").is_file());
        assert_eq!(b["filename"], "ping-2");
        let raw = std::fs::read_to_string(dir.join("ping.yml")).unwrap();
        assert!(raw.contains("name: Ping"));
    }

    #[test]
    fn renames_file_and_updates_info_name() {
        let dir = tmp("rename");
        let item = json!({ "name": "Old", "seq": 1, "request": { "method": "GET", "url": "u", "headers": [], "params": [], "auth": { "mode": "none" } } });
        new_request(&to_slash(&dir.join("old.yml")), &item).unwrap();
        let new_path = rename_item_filename(
            &to_slash(&dir.join("old.yml")),
            &to_slash(&dir.join("new.yml")),
            "New",
        )
        .unwrap();
        assert!(new_path.ends_with("new.yml"));
        assert!(dir.join("new.yml").is_file());
        let raw = std::fs::read_to_string(dir.join("new.yml")).unwrap();
        assert!(raw.contains("name: New"), "{raw}");
    }

    #[test]
    fn saves_folder_root_and_merges_unknown_keys() {
        let dir = tmp("folder-root");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FOLDER_FILE), "info:\n  name: F\n  seq: 1\nextensions:\n  keep: me\n").unwrap();
        save_folder_root(&json!({
            "name": "F2",
            "folderPathname": to_slash(&dir),
            "root": { "meta": { "name": "F2", "seq": 3 }, "request": { "headers": [{ "name": "X", "value": "1", "enabled": false }], "auth": { "mode": "none" }, "vars": { "req": [{ "name": "v", "value": "1" }] } } }
        }))
        .unwrap();
        let raw = std::fs::read_to_string(dir.join(FOLDER_FILE)).unwrap();
        assert!(raw.contains("name: F2"));
        assert!(raw.contains("seq: 3"));
        assert!(raw.contains("disabled: true"));
        assert!(raw.contains("keep: me"));
    }

    #[test]
    fn collection_root_keeps_version_and_sets_request_defaults() {
        let dir = tmp("collection-root");
        std::fs::write(
            dir.join(ROOT_FILE),
            "opencollection: 1.0.0\ninfo:\n  name: Demo\nrequest:\n  auth:\n    type: none\n",
        )
        .unwrap();
        save_collection_root(
            &to_slash(&dir),
            &json!({ "request": { "headers": [{ "name": "X-Test", "value": "1", "enabled": true }], "auth": { "mode": "bearer", "bearer": { "token": "t" } }, "vars": { "req": [] }, "tests": "test('a', () => {})" } }),
            Some(&json!({ "name": "Demo 2", "version": "2" })),
        )
        .unwrap();
        let raw = std::fs::read_to_string(dir.join(ROOT_FILE)).unwrap();
        assert!(raw.contains("name: Demo 2"));
        assert!(raw.contains("version: '2'") || raw.contains("version: 2"));
        assert!(raw.contains("X-Test"));
        assert!(raw.contains("type: bearer"), "{raw}");
        assert!(raw.contains("tests"), "{raw}");
    }
}

/// 文件名安全化（保留中文，只替换 Windows 非法字符）
pub fn sanitize_file_name(name: &str) -> String {
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
        "untitled".into()
    } else {
        trimmed
    }
}
