//! 环境变量命令：集合环境 / 工作区环境 / 全局环境。
//!
//! 集合环境写入 `<collection>/environments/*.yml`，工作区环境写入
//! `<workspace>/environments/*.yml`（原版 Bruno 2.0 的布局），全局环境落在数据目录的
//! `global-environments.json`。

use crate::commands::emit_tree;
use crate::state::AppState;
use bruno_core::envs;
use bruno_core::ops;
use serde_json::{json, Value as J};
use tauri::{Emitter, State};

fn resolve_workspace_path(state: &State<'_, AppState>, provided: Option<String>) -> String {
    provided
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| state.layout.root.to_string_lossy().to_string())
}

fn reload_env_collection(app: &tauri::AppHandle, state: &AppState, pathname: &str) {
    if let Ok(fresh) = state.reload_collection(pathname) {
        emit_tree(app, &fresh, pathname);
    }
}

// ============================================================================
// 集合环境
// ============================================================================

#[tauri::command]
pub fn create_environment(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    collection_pathname: String,
    name: String,
    variables: Option<J>,
    color: Option<String>,
    inherited_environment_name: Option<String>,
) -> Result<J, String> {
    let env = envs::create(
        &collection_pathname,
        &name,
        variables.as_ref(),
        color.as_deref(),
        inherited_environment_name.as_deref(),
    )?;
    reload_env_collection(&app, &state, &collection_pathname);
    Ok(env)
}

#[tauri::command]
pub fn save_environment(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    collection_pathname: String,
    environment: J,
) -> Result<J, String> {
    let env = envs::save(&collection_pathname, &environment)?;
    reload_env_collection(&app, &state, &collection_pathname);
    Ok(env)
}

#[tauri::command]
pub fn delete_environment(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    collection_pathname: String,
    environment_name: String,
) -> Result<(), String> {
    envs::delete(&collection_pathname, &environment_name)?;
    reload_env_collection(&app, &state, &collection_pathname);
    Ok(())
}

#[tauri::command]
pub fn rename_environment(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    collection_pathname: String,
    old_name: String,
    new_name: String,
) -> Result<J, String> {
    let env = envs::rename(&collection_pathname, &old_name, &new_name)?;
    reload_env_collection(&app, &state, &collection_pathname);
    Ok(env)
}

#[tauri::command]
pub fn update_environment_color(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    collection_pathname: String,
    environment_name: String,
    color: String,
) -> Result<J, String> {
    let env = envs::update_color(&collection_pathname, &environment_name, &color)?;
    reload_env_collection(&app, &state, &collection_pathname);
    Ok(env)
}

#[tauri::command]
pub fn save_environment_extends(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    collection_pathname: String,
    environment_name: String,
    inherited_environment_name: Option<String>,
) -> Result<J, String> {
    let env = envs::save_extends(
        &collection_pathname,
        &environment_name,
        inherited_environment_name.as_deref().unwrap_or(""),
    )?;
    reload_env_collection(&app, &state, &collection_pathname);
    Ok(env)
}

// ============================================================================
// 工作区环境
// ============================================================================

#[tauri::command]
pub fn load_workspace_environments(state: State<'_, AppState>, workspace_path: Option<String>) -> Vec<J> {
    let base = resolve_workspace_path(&state, workspace_path);
    envs::list(&base)
}

#[tauri::command]
pub fn create_workspace_environment(
    state: State<'_, AppState>,
    workspace_path: Option<String>,
    environment_name: String,
) -> Result<J, String> {
    let base = resolve_workspace_path(&state, workspace_path);
    envs::create(&base, &environment_name, None, None, None)
}

#[tauri::command]
pub fn update_workspace_environment(
    state: State<'_, AppState>,
    workspace_path: Option<String>,
    environment_uid: String,
    environment_data: J,
) -> Result<J, String> {
    let base = resolve_workspace_path(&state, workspace_path);
    envs::save_by_uid(&base, &environment_uid, &environment_data)
}

#[tauri::command]
pub fn rename_workspace_environment(
    state: State<'_, AppState>,
    workspace_path: Option<String>,
    environment_uid: String,
    new_name: String,
) -> Result<J, String> {
    let base = resolve_workspace_path(&state, workspace_path);
    envs::rename_by_uid(&base, &environment_uid, &new_name)
}

#[tauri::command]
pub fn delete_workspace_environment(
    state: State<'_, AppState>,
    workspace_path: Option<String>,
    environment_uid: String,
) -> Result<(), String> {
    let base = resolve_workspace_path(&state, workspace_path);
    envs::delete_by_uid(&base, &environment_uid)
}

#[tauri::command]
pub fn copy_workspace_environment(
    state: State<'_, AppState>,
    workspace_path: Option<String>,
    environment_uid: String,
    new_name: String,
) -> Result<J, String> {
    let base = resolve_workspace_path(&state, workspace_path);
    envs::copy_by_uid(&base, &environment_uid, &new_name)
}

#[tauri::command]
pub async fn export_workspace(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    workspace_path: Option<String>,
    workspace_name: String,
) -> Result<J, String> {
    use tauri_plugin_dialog::DialogExt;
    let base = resolve_workspace_path(&state, workspace_path);
    let (tx, rx) = tokio::sync::oneshot::channel();
    let default_name = format!("{}.zip", ops::sanitize_file_name(&workspace_name));
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
    ops::export_zip(&ops::to_path(&base), &ops::to_path(&target))?;
    Ok(json!({ "success": true, "path": target }))
}

// ============================================================================
// 全局环境
// ============================================================================

fn global_store(state: &AppState) -> std::path::PathBuf {
    state.layout.global_environments.clone()
}

#[tauri::command]
pub fn get_global_environments(state: State<'_, AppState>) -> J {
    let value = envs::load_global(&global_store(&state));
    let active = value.get("activeUid").cloned().unwrap_or(J::Null);
    json!({
        "globalEnvironments": value.get("environments").cloned().unwrap_or_else(|| json!([])),
        "activeGlobalEnvironmentUid": active,
    })
}

#[tauri::command]
pub fn create_global_environment(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: J,
) -> Result<J, String> {
    let env = envs::global_upsert(&global_store(&state), &payload)?;
    let _ = app.emit("main:load-global-environments", get_global_environments(state));
    Ok(env)
}

#[tauri::command]
pub fn save_global_environment(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: J,
) -> Result<J, String> {
    let env = envs::global_upsert(&global_store(&state), &payload)?;
    let _ = app.emit("main:load-global-environments", get_global_environments(state));
    Ok(env)
}

#[tauri::command]
pub fn delete_global_environment(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    environment_uid: String,
) -> Result<(), String> {
    envs::global_delete(&global_store(&state), &environment_uid)?;
    let _ = app.emit("main:load-global-environments", get_global_environments(state));
    Ok(())
}

#[tauri::command]
pub fn rename_global_environment(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    environment_uid: String,
    new_name: String,
) -> Result<J, String> {
    let mut payload = json!({ "uid": environment_uid, "name": new_name });
    {
        let current = envs::load_global(&global_store(&state));
        if let Some(list) = current.get("environments").and_then(|v| v.as_array()) {
            if let Some(env) = list
                .iter()
                .find(|e| e.get("uid").and_then(|v| v.as_str()) == Some(environment_uid.as_str()))
            {
                payload = env.clone();
                payload["name"] = json!(new_name);
            }
        }
    }
    let env = envs::global_upsert(&global_store(&state), &payload)?;
    let _ = app.emit("main:load-global-environments", get_global_environments(state));
    Ok(env)
}

#[tauri::command]
pub fn select_global_environment(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    environment_uid: Option<String>,
) -> Result<(), String> {
    envs::global_select(&global_store(&state), environment_uid.as_deref())?;
    let _ = app.emit("main:load-global-environments", get_global_environments(state));
    Ok(())
}

#[tauri::command]
pub fn save_global_environment_extends(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    environment_uid: String,
    extends: Option<String>,
    _workspace_uid: Option<String>,
    _workspace_path: Option<String>,
) -> Result<J, String> {
    let payload = json!({ "environmentUid": environment_uid, "extends": extends });
    let env = envs::global_upsert(&global_store(&state), &payload)?;
    let _ = app.emit("main:load-global-environments", get_global_environments(state));
    Ok(env)
}

#[tauri::command]
pub fn update_global_environment_color(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    environment_uid: String,
    color: Option<String>,
    _workspace_uid: Option<String>,
    _workspace_path: Option<String>,
) -> Result<J, String> {
    let payload = json!({ "environmentUid": environment_uid, "color": color });
    let env = envs::global_upsert(&global_store(&state), &payload)?;
    let _ = app.emit("main:load-global-environments", get_global_environments(state));
    Ok(env)
}
