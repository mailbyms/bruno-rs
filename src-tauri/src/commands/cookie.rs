//! Cookies 面板的增删改查。
//!
//! 面板数据形状是「按域名分组」的数组（`[{domain, cookies:[{key,...}]}]`），
//! 分组在 Rust 侧完成，前端只负责渲染。

use crate::state::AppState;
use bruno_core::cookies::{create_raw_cookie, parse_raw_cookie};
use serde_json::Value as J;
use tauri::{Emitter, Manager, State};

fn jar_json(state: &AppState) -> Vec<J> {
    state.cookies.lock().unwrap().list()
}

fn after_change(app: &tauri::AppHandle, state: &AppState) {
    state.save_cookies();
    let _ = app.emit("main:cookies-update", jar_json(state));
}

#[tauri::command]
pub fn add_cookie(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    domain: String,
    cookie: J,
) -> Result<(), String> {
    state.cookies.lock().unwrap().upsert(&domain, &cookie);
    after_change(&app, &state);
    Ok(())
}

#[tauri::command]
pub fn modify_cookie(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    domain: String,
    old_cookie: J,
    cookie: J,
) -> Result<(), String> {
    state.cookies.lock().unwrap().modify(&domain, &old_cookie, &cookie);
    after_change(&app, &state);
    Ok(())
}

#[tauri::command]
pub fn delete_cookie(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    domain: String,
    path: String,
    cookie_key: String,
) -> Result<(), String> {
    state.cookies.lock().unwrap().delete(&domain, &path, &cookie_key);
    after_change(&app, &state);
    Ok(())
}

#[tauri::command]
pub fn delete_cookies_for_domain(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    domain: String,
) -> Result<(), String> {
    state.cookies.lock().unwrap().clear_domain(&domain);
    after_change(&app, &state);
    Ok(())
}

/// Raw 模式：粘贴一段 cookie 字符串 → 表单字段
#[tauri::command]
pub fn get_parsed_cookie(cookie_string: String) -> J {
    parse_raw_cookie(&cookie_string).unwrap_or(J::Null)
}

/// 表单字段 → 一段 cookie 字符串
#[tauri::command]
pub fn create_cookie_string(cookie: J) -> String {
    create_raw_cookie(&cookie)
}

/// 启动时补齐：让 cookie 面板在冷启动时也有数据
pub fn bootstrap_cookies(app: &tauri::AppHandle) {
    let Some(state) = app.try_state::<AppState>() else { return };
    let _ = app.emit("main:cookies-update", jar_json(&state));
}
