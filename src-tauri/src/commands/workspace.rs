//! 工作区：创建 / 打开 / 改名 / 关闭 / 文档 / 导入。
//!
//! 磁盘布局：每个工作区一个目录，目录里 `workspace.yml` 记录 name / collections / docs。
//! 默认工作区不收在 workspaces 列表里，集合列表存在 settings.json（保持既有行为不变）。

use crate::commands::emit_workspace_config;
use crate::state::AppState;
use bruno_core::ops;
use serde_json::{json, Value as J};
use std::path::PathBuf;
use tauri::{Emitter, State};

/// 前端 confirmWorkspaceCreation / createWorkspaceAction 用
#[tauri::command]
pub fn create_workspace(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    workspace_name: String,
    workspace_folder_name: String,
    workspace_location: String,
) -> Result<J, String> {
    let entry = state.create_workspace(&workspace_name, &workspace_folder_name, &workspace_location)?;
    let config = state.workspace_config_for(&entry.uid);
    let _ = app.emit(
        "main:workspace-opened",
        (entry.path.clone(), entry.uid.clone(), config.clone()),
    );
    Ok(json!({
        "workspaceUid": entry.uid,
        "workspacePath": entry.path,
        "workspaceConfig": config,
    }))
}

/// 打开已有工作区目录（读 workspace.yml）
#[tauri::command]
pub fn open_workspace(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    pathname: String,
) -> Result<J, String> {
    let entry = state.register_workspace(&pathname)?;
    state.set_active_workspace(&entry.uid);
    let config = state.workspace_config_for(&entry.uid);
    let _ = app.emit(
        "main:workspace-opened",
        (entry.path.clone(), entry.uid.clone(), config.clone()),
    );
    Ok(json!({
        "workspaceUid": entry.uid,
        "workspacePath": entry.path,
        "workspaceConfig": config,
    }))
}

/// 弹目录选择框并打开；用户取消返回 null
#[tauri::command]
pub async fn open_workspace_dialog(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<J>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folders(move |folders| {
        let first = folders
            .unwrap_or_default()
            .into_iter()
            .find_map(|p| p.into_path().ok());
        let _ = tx.send(first);
    });
    let picked = rx.await.map_err(|e| e.to_string())?;
    let Some(dir) = picked else { return Ok(None) };
    let pathname = ops::to_slash(&dir);
    let entry = state.register_workspace(&pathname)?;
    state.set_active_workspace(&entry.uid);
    let config = state.workspace_config_for(&entry.uid);
    let _ = app.emit(
        "main:workspace-opened",
        (entry.path.clone(), entry.uid.clone(), config.clone()),
    );
    Ok(Some(json!({
        "workspaceUid": entry.uid,
        "workspacePath": entry.path,
        "workspaceConfig": config,
    })))
}

/// 前端传的是工作区目录路径
#[tauri::command]
pub fn rename_workspace(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    workspace_path: String,
    name: String,
) -> Result<bool, String> {
    let uid = state
        .workspace_uid_by_path(&workspace_path)
        .ok_or_else(|| "工作区不在列表中".to_string())?;
    if uid == crate::state::DEFAULT_WORKSPACE_UID {
        // 默认工作区的名字固定，改它没有意义，但也不必报错
        return Ok(true);
    }
    if state.workspace_by_uid(&uid).is_none() || !state.rename_workspace(&uid, &name) {
        return Err("工作区不存在".into());
    }
    let _ = app.emit(
        "main:workspace-config-updated",
        (
            workspace_path,
            uid.clone(),
            state.workspace_config_for(&uid),
        ),
    );
    Ok(true)
}

/// 只从列表里移除，目录与文件都不动
#[tauri::command]
pub fn close_workspace(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    workspace_path: String,
) -> Result<bool, String> {
    if let Some(uid) = state.workspace_uid_by_path(&workspace_path) {
        state.close_workspace(&uid);
    }
    emit_workspace_config(&app, &state);
    Ok(true)
}

#[tauri::command]
pub fn save_workspace_docs(
    state: State<'_, AppState>,
    workspace_path: String,
    docs: String,
) -> Result<bool, String> {
    let uid = state
        .workspace_uid_by_path(&workspace_path)
        .ok_or_else(|| "工作区不在列表中".to_string())?;
    // 默认工作区没有 workspace.yml，文档留在设置里没有意义，直接忽略
    if uid == crate::state::DEFAULT_WORKSPACE_UID {
        return Ok(true);
    }
    Ok(state.save_workspace_docs(&uid, &docs))
}

/// 导入工作区 zip：解压到 extractLocation 下，再按目录注册
#[tauri::command]
pub fn import_workspace(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    zip_file_path: String,
    extract_location: String,
) -> Result<J, String> {
    let zip = ops::to_path(&zip_file_path);
    if !zip.is_file() {
        return Err(format!("找不到工作区包：{zip_file_path}"));
    }
    let stem = zip
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "workspace".into());
    let base = ops::to_path(&extract_location);
    std::fs::create_dir_all(&base).map_err(|e| format!("创建导入目录失败：{e}"))?;
    let target: PathBuf = ops::unique_path(&base.join(ops::sanitize_file_name(&stem)));
    ops::import_zip(&zip, &target)?;
    let entry = state.register_workspace(&ops::to_slash(&target))?;
    state.set_active_workspace(&entry.uid);
    let config = state.workspace_config_for(&entry.uid);
    let _ = app.emit(
        "main:workspace-opened",
        (entry.path.clone(), entry.uid.clone(), config.clone()),
    );
    Ok(json!({
        "success": true,
        "workspaceUid": entry.uid,
        "workspacePath": entry.path,
        "workspaceConfig": config,
    }))
}

/// 工作区环境导入：前端已解析成环境对象，这里落成 `<workspace>/environments/<name>.yml`
#[tauri::command]
pub fn import_workspace_environment(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    workspace_path: String,
    environment_data: J,
) -> Result<J, String> {
    let name = environment_data
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("Imported Environment")
        .to_string();
    let variables = environment_data
        .get("variables")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let env = bruno_core::envs::create(&workspace_path, &name, Some(&variables), None, None)?;
    let uid = state.active_workspace_uid();
    let _ = app.emit("main:workspace-environment-added", (uid, env.clone()));
    emit_workspace_config(&app, &state);
    Ok(env)
}

/// 启动时前端 `loadLastOpenedWorkspaces` 用它同步工作区列表
#[tauri::command]
pub fn load_last_opened_workspaces(state: State<'_, AppState>) -> Vec<J> {
    state.workspace_entries()
}

/// 新建工作区前算一个不冲突的目录名
#[tauri::command]
pub fn find_unique_folder_name(base_name: String, location: String) -> String {
    let base = ops::to_path(&location);
    let mut candidate = ops::sanitize_file_name(&base_name);
    if candidate.is_empty() {
        candidate = "workspace".into();
    }
    let mut index = 2;
    while base.join(&candidate).exists() {
        candidate = format!("{}-{index}", ops::sanitize_file_name(&base_name));
        index += 1;
    }
    candidate
}
