//! 集合 / 请求 / 文件夹的 CRUD 命令，以及导出。
//!
//! 每个写操作完成后都会重新加载集合并推送 `main:collection-tree-loaded`：
//! 节点的 uid 是按相对路径稳定生成的，所以前端 `mergeTreeItems` 会按 uid 合并，
//! 已打开的标签页与未保存草稿都保得住。

use crate::commands::{emit_tree, emit_workspace_config, DEFAULT_WORKSPACE_UID};
use crate::state::AppState;
use bruno_core::ops;
use serde_json::{json, Value as J};
use std::path::{Path, PathBuf};
use tauri::{Emitter, State};

fn collection_root_for(state: &AppState, path: &str) -> Result<PathBuf, String> {
    let target = ops::to_path(path);
    let mut best: Option<PathBuf> = None;
    for collection in state.collections.lock().unwrap().values() {
        let root = &collection.pathname;
        if target.starts_with(root) {
            match &best {
                Some(current) if current.components().count() >= root.components().count() => {}
                _ => best = Some(root.clone()),
            }
        }
    }
    best.ok_or_else(|| format!("路径不属于任何已打开集合：{path}"))
}

fn abs_for(state: &AppState, pathname: &str, collection_pathname: Option<&str>) -> Result<PathBuf, String> {
    let candidate = ops::to_path(pathname);
    if candidate.is_absolute() {
        return Ok(candidate);
    }
    if let Some(root) = collection_pathname {
        return Ok(ops::to_path(root).join(candidate));
    }
    // 没有集合路径时，用相对 pathname 在已挂载集合里找
    let normalized = pathname.replace('\\', "/");
    for collection in state.collections.lock().unwrap().values() {
        if collection.requests.contains_key(&normalized) {
            return Ok(collection.pathname.join(candidate));
        }
        if collection
            .tree
            .iter()
            .any(|node| node.pathname == normalized || node.pathname.starts_with(&format!("{normalized}/")))
        {
            return Ok(collection.pathname.join(candidate));
        }
    }
    Err(format!("无法定位路径：{pathname}"))
}

fn reload_and_emit(app: &tauri::AppHandle, state: &AppState, collection_pathname: &str) {
    if let Ok(fresh) = state.reload_collection(collection_pathname) {
        emit_tree(app, &fresh, collection_pathname);
    }
}

#[tauri::command]
pub fn new_request(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    pathname: String,
    item: J,
) -> Result<J, String> {
    let root = collection_root_for(&state, &pathname)?;
    let node = ops::new_request(&pathname, &item)?;
    // 返回相对 pathname（前端 OPEN_REQUEST 任务按它查树）
    let mut node = node;
    if let Some(p) = node.get("pathname").and_then(|v| v.as_str()) {
        let rel = ops::to_path(p).strip_prefix(&root).map(|r| ops::to_slash(r)).unwrap_or_else(|_| p.to_string());
        node["pathname"] = json!(rel);
    }
    reload_and_emit(&app, &state, &ops::to_slash(&root));
    Ok(node)
}

#[tauri::command]
pub fn new_folder(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    pathname: String,
    folder_data: J,
    #[allow(unused_variables)] format: Option<String>,
) -> Result<J, String> {
    let root = collection_root_for(&state, &pathname)?;
    let node = ops::new_folder(&pathname, &folder_data)?;
    let mut node = node;
    if let Some(p) = node.get("pathname").and_then(|v| v.as_str()) {
        let rel = ops::to_path(p).strip_prefix(&root).map(|r| ops::to_slash(r)).unwrap_or_else(|_| p.to_string());
        node["pathname"] = json!(rel);
    }
    reload_and_emit(&app, &state, &ops::to_slash(&root));
    Ok(node)
}

#[tauri::command]
pub fn delete_item(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    pathname: String,
    kind: String,
    collection_pathname: Option<String>,
) -> Result<(), String> {
    let abs = abs_for(&state, &pathname, collection_pathname.as_deref())?;
    let root = collection_pathname
        .clone()
        .or_else(|| collection_root_for(&state, &ops::to_slash(&abs)).ok().map(|p| ops::to_slash(&p)))
        .ok_or_else(|| "缺少集合路径".to_string())?;
    ops::delete_item(&ops::to_slash(&abs), &kind)?;
    reload_and_emit(&app, &state, &root);
    Ok(())
}

#[tauri::command]
pub fn move_item(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    target_dirname: String,
    source_pathname: String,
) -> Result<J, String> {
    let src = abs_for(&state, &source_pathname, None)?;
    let target_dir = if ops::to_path(&target_dirname).is_absolute() {
        ops::to_path(&target_dirname)
    } else {
        // targetDirname 是相对集合根的目录：用集合根拼
        let root = collection_root_for(&state, &ops::to_slash(&src))?;
        root.join(ops::to_path(&target_dirname))
    };
    let result = ops::move_item(&ops::to_slash(&target_dir), &ops::to_slash(&src))?;
    if let Some(root) = collection_root_for(&state, &ops::to_slash(&src)).ok() {
        reload_and_emit(&app, &state, &ops::to_slash(&root));
    }
    Ok(result)
}

#[tauri::command]
pub fn rename_item_name(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    item_path: String,
    new_name: String,
    collection_pathname: Option<String>,
) -> Result<(), String> {
    let abs = abs_for(&state, &item_path, collection_pathname.as_deref())?;
    ops::rename_item_name(&ops::to_slash(&abs), &new_name)?;
    if let Some(root) = collection_pathname
        .clone()
        .or_else(|| collection_root_for(&state, &ops::to_slash(&abs)).ok().map(|p| ops::to_slash(&p)))
    {
        reload_and_emit(&app, &state, &root);
    }
    Ok(())
}

#[tauri::command]
pub fn rename_item_filename(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    old_path: String,
    new_path: String,
    new_name: Option<String>,
    _new_filename: Option<String>,
    collection_pathname: Option<String>,
) -> Result<J, String> {
    let old_abs = abs_for(&state, &old_path, collection_pathname.as_deref())?;
    let new_abs = abs_for(&state, &new_path, collection_pathname.as_deref())?;
    let new_path = ops::rename_item_filename(
        &ops::to_slash(&old_abs),
        &ops::to_slash(&new_abs),
        new_name.as_deref().unwrap_or(""),
    )?;
    let root = collection_pathname
        .clone()
        .or_else(|| collection_root_for(&state, &new_path).ok().map(|p| ops::to_slash(&p)));
    if let Some(root) = root {
        reload_and_emit(&app, &state, &root);
        let rel = ops::to_path(&new_path)
            .strip_prefix(ops::to_path(&root))
            .map(|p| ops::to_slash(p))
            .unwrap_or(new_path.clone());
        return Ok(json!({ "pathname": rel, "newPathname": rel }));
    }
    Ok(json!({ "pathname": new_path }))
}

#[tauri::command]
pub fn resequence_items(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    items: J,
    collection_pathname: String,
) -> Result<(), String> {
    let root = ops::to_path(&collection_pathname);
    // 前端给的是相对 pathname，拼成绝对路径再写
    let mut normalized = Vec::new();
    if let Some(list) = items.as_array() {
        for item in list {
            let Some(pathname) = item.get("pathname").and_then(|v| v.as_str()) else {
                continue;
            };
            let abs = if ops::to_path(pathname).is_absolute() {
                ops::to_path(pathname)
            } else {
                root.join(ops::to_path(pathname))
            };
            normalized.push(json!({ "pathname": ops::to_slash(&abs), "seq": item.get("seq") }));
        }
    }
    ops::resequence(&J::Array(normalized))?;
    reload_and_emit(&app, &state, &collection_pathname);
    Ok(())
}

#[tauri::command]
pub fn create_collection(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    collection_name: String,
    collection_folder_name: String,
    collection_location: String,
    _options: Option<J>,
) -> Result<J, String> {
    let dir = ops::create_collection(&collection_name, &collection_folder_name, &collection_location)?;
    let collection = state.mount(ops::to_path(&dir))?;
    let uid = collection.uid.clone();
    let config = json!({
        "name": collection.name,
        "opencollection": collection.root.version,
        "type": "collection",
        "ignore": ["node_modules", ".git"],
    });
    let _ = app.emit("main:collection-opened", (dir.clone(), uid.clone(), config));
    emit_tree(&app, &collection, &dir);
    let _ = emit_workspace_config(&app, &state);
    Ok(json!({ "uid": uid, "pathname": dir, "name": collection.name }))
}

#[tauri::command]
pub fn clone_collection(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    collection_name: String,
    collection_folder_name: String,
    collection_location: String,
    previous_path: String,
) -> Result<J, String> {
    let dir = ops::clone_collection(
        &collection_name,
        &collection_folder_name,
        &collection_location,
        &previous_path,
    )?;
    let collection = state.mount(ops::to_path(&dir))?;
    let config = json!({ "name": collection.name, "type": "collection", "ignore": ["node_modules", ".git"] });
    let _ = app.emit("main:collection-opened", (dir.clone(), collection.uid.clone(), config));
    emit_tree(&app, &collection, &dir);
    Ok(json!({ "pathname": dir, "name": collection.name }))
}

#[tauri::command]
pub fn clone_folder(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    item: J,
    target_path: String,
    collection_pathname: String,
) -> Result<J, String> {
    let source = abs_for(
        &state,
        item.get("pathname").and_then(|v| v.as_str()).unwrap_or(""),
        Some(&collection_pathname),
    )?;
    let node = ops::clone_folder(&target_path, &ops::to_slash(&source))?;
    reload_and_emit(&app, &state, &collection_pathname);
    Ok(node)
}

#[tauri::command]
pub fn rename_collection(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    new_name: String,
    collection_pathname: String,
) -> Result<(), String> {
    ops::rename_item_name(&collection_pathname, &new_name)?;
    state.rename_in_workspace(&collection_pathname, &new_name);
    let _ = app.emit(
        "main:collection-renamed",
        json!({ "collectionPathname": collection_pathname, "newName": new_name }),
    );
    let _ = emit_workspace_config(&app, &state);
    Ok(())
}

#[tauri::command]
pub fn remove_collection(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    collection_pathname: String,
    _collection_uid: Option<String>,
    _workspace_id: Option<String>,
) -> Result<(), String> {
    state.remove_from_workspace(&collection_pathname);
    let _ = emit_workspace_config(&app, &state);
    Ok(())
}

/// 侧栏「从工作区移除」：`(workspaceUid, workspacePath, collectionPath, { deleteFiles })`
#[tauri::command]
pub fn remove_collection_from_workspace(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    workspace_uid: Option<String>,
    workspace_path: Option<String>,
    collection_path: String,
    options: Option<J>,
) -> Result<(), String> {
    let _ = workspace_uid;
    let delete_files = options
        .as_ref()
        .and_then(|o| o.get("deleteFiles"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // 只从指定工作区移除；集合若还被别的工作区引用则保持挂载
    let workspace_path = workspace_path.unwrap_or_default();
    state.remove_collection_from(&workspace_path, &collection_path);
    let still_referenced = state.is_collection_in_any_workspace(&collection_path);

    if !still_referenced {
        crate::watcher::stop(&state, &collection_path);
        // 只有确认是集合根（含 opencollection.yml）才允许删盘，避免误删任意目录
        if delete_files {
            let root = ops::to_path(&collection_path);
            if root.join("opencollection.yml").is_file() {
                ops::delete_item(&collection_path, "folder")?;
            }
        }
        let mut map = state.path_to_uid.lock().unwrap();
        let normalized = collection_path.replace('\\', "/");
        let uid = map
            .iter()
            .find(|(k, _)| k.replace('\\', "/").eq_ignore_ascii_case(&normalized))
            .map(|(_, v)| v.clone());
        if let Some(uid) = uid {
            state.collections.lock().unwrap().remove(&uid);
            map.retain(|k, _| !k.replace('\\', "/").eq_ignore_ascii_case(&normalized));
        }
    }

    let _ = emit_workspace_config(&app, &state);
    Ok(())
}

/// `renderer:exists-sync`：给前端判断文件是否存在的便捷通道
#[tauri::command]
pub fn exists_sync(pathname: String) -> bool {
    ops::to_path(&pathname).exists()
}

/// `renderer:resolve-path`：把相对路径按基准目录拼成绝对路径
#[tauri::command]
pub fn resolve_path(relative_path: String, base_path: Option<String>) -> String {
    let rel = ops::to_path(&relative_path);
    if rel.is_absolute() {
        return ops::to_slash(&rel);
    }
    match base_path {
        Some(base) if !base.is_empty() => ops::to_slash(&ops::to_path(&base).join(rel)),
        _ => ops::to_slash(&rel),
    }
}

#[tauri::command]
pub fn save_collection_root(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    pathname: String,
    root: J,
    bruno_config: Option<J>,
) -> Result<(), String> {
    ops::save_collection_root(&pathname, &root, bruno_config.as_ref())?;
    reload_and_emit(&app, &state, &pathname);
    Ok(())
}

/// `renderer:update-bruno-config`：只改集合配置（版本号等），root 由前端一并带上
#[tauri::command]
pub fn update_bruno_config(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    bruno_config: J,
    pathname: String,
    root: Option<J>,
) -> Result<(), String> {
    let root = root.unwrap_or_else(|| json!({}));
    ops::save_collection_root(&pathname, &root, Some(&bruno_config))?;
    reload_and_emit(&app, &state, &pathname);
    Ok(())
}

/// 该集合当前属于哪些工作区。本实现只有默认工作区，用于「从工作区移除」后判断是否彻底关闭集合。
#[tauri::command]
pub fn get_collection_workspaces(state: State<'_, AppState>, collection_pathname: String) -> Vec<J> {
    let normalized = collection_pathname.replace('\\', "/");
    let settings = state.settings.lock().unwrap();
    settings
        .workspace_collections
        .iter()
        .filter(|c| c.path.eq_ignore_ascii_case(&normalized))
        .map(|c| {
            json!({
                "uid": DEFAULT_WORKSPACE_UID,
                "name": c.name,
                "pathname": "",
            })
        })
        .collect()
}

/// 新建集合后前端会顺手把集合归入当前工作区，真正落库走 `add-collection-to-workspace`
#[tauri::command]
pub fn set_collection_workspace(_collection_uid: String, _workspace_pathname: Option<String>) -> bool {
    true
}

/// 工作区里的 `collections/` 目录：新建集合的默认落点
#[tauri::command]
pub fn ensure_collections_folder(workspace_path: String) -> Result<String, String> {
    let dir = ops::to_path(&workspace_path).join("collections");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建工作区 collections 目录失败：{e}"))?;
    Ok(ops::to_slash(&dir))
}

#[tauri::command]
pub fn is_bruno_collection_zip(pathname: String) -> bool {
    ops::is_collection_zip(&ops::to_path(&pathname)).unwrap_or(false)
}

/// 解压集合包到 `collectionLocation/<包名>`，挂载后回推树事件
#[tauri::command]
pub fn import_collection_zip(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    zip_path: String,
    collection_location: String,
) -> Result<String, String> {
    let zip = ops::to_path(&zip_path);
    if !zip.is_file() {
        return Err(format!("找不到集合包：{zip_path}"));
    }
    let stem = zip
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "collection".into());
    let base = ops::to_path(&collection_location);
    std::fs::create_dir_all(&base).map_err(|e| format!("创建导入目录失败：{e}"))?;
    let target = ops::unique_path(&base.join(ops::sanitize_file_name(&stem)));
    ops::import_zip(&zip, &target)?;
    let pathname = ops::to_slash(&target);

    match state.mount(target.clone()) {
        Ok(collection) => {
            let config = json!({
                "name": collection.name,
                "opencollection": collection.root.version,
                "type": "collection",
                "ignore": ["node_modules", ".git"],
            });
            let _ = app.emit(
                "main:collection-opened",
                (pathname.clone(), collection.uid.clone(), config),
            );
            emit_tree(&app, &collection, &pathname);
        }
        Err(e) => {
            // 解压成功但不是 OpenCollection 集合：前端仍拿到路径，可自行决定是否打开
            eprintln!("[bruno-rs] 集合包解压完成但无法挂载：{e}");
        }
    }
    let _ = emit_workspace_config(&app, &state);
    Ok(pathname)
}

#[tauri::command]
pub fn save_folder_root(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    folder_data: J,
) -> Result<(), String> {
    ops::save_folder_root(&folder_data)?;
    if let Some(root) = folder_data.get("collectionPathname").and_then(|v| v.as_str()) {
        reload_and_emit(&app, &state, root);
    }
    Ok(())
}

#[tauri::command]
pub fn save_file(pathname: String, content: String) -> Result<(), String> {
    ops::save_text_file(&pathname, &content)
}

#[tauri::command]
pub async fn export_collection_zip(
    app: tauri::AppHandle,
    pathname: String,
    name: String,
    target_path: Option<String>,
) -> Result<J, String> {
    use tauri_plugin_dialog::DialogExt;
    // 自动化验证可以直接给目标路径，跳过系统保存对话框
    if let Some(target) = target_path.filter(|p| !p.is_empty()) {
        ops::export_zip(&ops::to_path(&pathname), &ops::to_path(&target))?;
        return Ok(json!({ "success": true, "path": target }));
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    let default_name = format!("{}.zip", ops::sanitize_file_name(&name));
    app.dialog()
        .file()
        .set_file_name(&default_name)
        .add_filter("Zip", &["zip"])
        .save_file(move |path| {
            let _ = tx.send(path.map(|p| p.into_path().map(|x| x.to_string_lossy().to_string()).unwrap_or_default()));
        });
    let target = rx.await.map_err(|e| e.to_string())?;
    let Some(target) = target else {
        return Ok(json!({ "success": false, "cancelled": true }));
    };
    ops::export_zip(&ops::to_path(&pathname), &ops::to_path(&target))?;
    Ok(json!({ "success": true, "path": target }))
}

#[tauri::command]
pub async fn export_collection_yaml(
    app: tauri::AppHandle,
    name: String,
    content: String,
    target_path: Option<String>,
) -> Result<J, String> {
    use tauri_plugin_dialog::DialogExt;
    if let Some(target) = target_path.filter(|p| !p.is_empty()) {
        ops::save_text_file(&target, &content)?;
        return Ok(json!({ "success": true, "path": target }));
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    let default_name = format!("{}.yaml", ops::sanitize_file_name(&name));
    app.dialog()
        .file()
        .set_file_name(&default_name)
        .add_filter("YAML", &["yaml", "yml"])
        .save_file(move |path| {
            let _ = tx.send(path.map(|p| p.into_path().map(|x| x.to_string_lossy().to_string()).unwrap_or_default()));
        });
    let target = rx.await.map_err(|e| e.to_string())?;
    let Some(target) = target else {
        return Ok(json!({ "success": false, "cancelled": true }));
    };
    ops::save_text_file(&target, &content)?;
    Ok(json!({ "success": true, "path": target }))
}

#[tauri::command]
pub fn export_collection_postman(
    location: String,
    file_name: String,
    content: String,
    overwrite: Option<bool>,
) -> Result<(), String> {
    let dir = ops::to_path(&location);
    if !dir.is_dir() {
        return Err(format!("导出目录不存在：{location}"));
    }
    let target = dir.join(&file_name);
    if target.exists() && overwrite != Some(true) {
        return Err(format!("{} already exists", target.display()));
    }
    ops::save_text_file(&ops::to_slash(&target), &content)
}

#[tauri::command]
pub fn export_environment(payload: J) -> Result<(), String> {
    let environments = payload
        .get("environments")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let file_path = payload.get("filePath").and_then(|v| v.as_str()).unwrap_or("");
    if file_path.is_empty() {
        return Err("缺少导出目录".into());
    }
    let dir = ops::to_path(file_path);
    std::fs::create_dir_all(&dir).map_err(|e| format!("导出目录不可用：{e}"))?;
    if let Some(list) = environments.as_array() {
        if list.len() == 1 {
            let name = list[0].get("name").and_then(|v| v.as_str()).unwrap_or("environment");
            let target = dir.join(format!("{}.json", ops::sanitize_file_name(name)));
            return ops::save_text_file(
                &ops::to_slash(&target),
                &serde_json::to_string_pretty(&list[0]).unwrap_or_default(),
            );
        }
        for env in list {
            let name = env.get("name").and_then(|v| v.as_str()).unwrap_or("environment");
            let target = dir.join(format!("{}.json", ops::sanitize_file_name(name)));
            ops::save_text_file(
                &ops::to_slash(&target),
                &serde_json::to_string_pretty(env).unwrap_or_default(),
            )?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn browse_directories(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folders(move |folders| {
        let list = folders
            .unwrap_or_default()
            .into_iter()
            .filter_map(|p| p.into_path().ok().map(|x| x.to_string_lossy().to_string()))
            .collect::<Vec<_>>();
        let _ = tx.send(list);
    });
    rx.await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browse_files(app: tauri::AppHandle, filters: Option<J>) -> Result<Vec<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut dialog = app.dialog().file();
    if let Some(list) = filters.as_ref().and_then(|f| f.as_array()) {
        for filter in list {
            let name = filter.get("name").and_then(|v| v.as_str()).unwrap_or("Files");
            let exts: Vec<String> = filter
                .get("extensions")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
                .unwrap_or_default();
            let refs: Vec<&str> = exts.iter().map(|s| s.as_str()).collect();
            if !refs.is_empty() {
                dialog = dialog.add_filter(name, &refs);
            }
        }
    }
    dialog.pick_files(move |files| {
        let list = files
            .unwrap_or_default()
            .into_iter()
            .filter_map(|p| p.into_path().ok().map(|x| x.to_string_lossy().to_string()))
            .collect::<Vec<_>>();
        let _ = tx.send(list);
    });
    rx.await.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn is_directory(pathname: String) -> bool {
    Path::new(&ops::to_path(&pathname)).is_dir()
}

#[tauri::command]
pub fn show_in_folder(app: tauri::AppHandle, pathname: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let path = ops::to_path(&pathname);
    let target = if path.is_dir() {
        path.clone()
    } else {
        path.parent().map(|p| p.to_path_buf()).unwrap_or(path.clone())
    };
    app.opener()
        .open_path(target.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}
