//! Git 命令：克隆、列远端分支、读/连接/断开 remote。

use crate::commands::{emit_tree, emit_workspace_config};
use crate::state::AppState;
use bruno_core::{git, ops};
use serde_json::{json, Value as J};
use tauri::{Emitter, State};

#[tauri::command]
pub fn git_version() -> Option<String> {
    git::version()
}

#[tauri::command]
pub fn clone_git_repository(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    url: String,
    path: String,
    branch: Option<String>,
) -> Result<String, String> {
    git::clone_repo(&url, &path, branch.as_deref())?;
    match state.mount(ops::to_path(&path)) {
        Ok(collection) => {
            let config = json!({
                "name": collection.name,
                "type": "collection",
                "ignore": ["node_modules", ".git"],
            });
            let placeholder_uid = collection.uid.clone();
            let _ = app.emit(
                "main:collection-opened",
                (path.clone(), placeholder_uid, config),
            );
            emit_tree(&app, &collection, &path);
        }
        Err(e) => {
            // 克隆成功但不是 OpenCollection 集合：仍然告诉前端目录已就绪
            eprintln!("[bruno-rs] 克隆完成但无法挂载为集合：{e}");
        }
    }
    let _ = emit_workspace_config(&app, &state);
    Ok("Repository cloned successfully".into())
}

#[tauri::command]
pub fn list_remote_branches_for_url(payload: J) -> Result<Vec<String>, String> {
    let url = payload.get("url").and_then(|v| v.as_str()).unwrap_or("");
    git::list_remote_branches(url)
}

#[tauri::command]
pub fn get_collection_git_remote_url(pathname: String) -> Option<String> {
    git::collection_remote_url(&ops::to_path(&pathname))
}

#[tauri::command]
pub fn connect_collection_to_git(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    workspace_path: String,
    collection_path: String,
    remote_url: String,
) -> Result<bool, String> {
    let trimmed = remote_url.trim();
    if !git::is_valid_url(trimmed) {
        return Err("Git remote URL 无效".into());
    }
    if !state.set_collection_remote(&collection_path, Some(trimmed)) {
        return Err("集合不在工作区中，请先加入工作区".into());
    }
    let _ = workspace_path;
    let _ = emit_workspace_config(&app, &state);
    Ok(true)
}

#[tauri::command]
pub fn disconnect_collection_from_git(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    workspace_path: String,
    collection_path: String,
) -> Result<bool, String> {
    let _ = workspace_path;
    state.set_collection_remote(&collection_path, None);
    let _ = emit_workspace_config(&app, &state);
    Ok(true)
}
