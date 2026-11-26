//! 偏好设置、UI 快照与窗口控制命令。
//!
//! 快照整份存 JSON（前端负责结构），只在读取 tabs 时按集合路径取子集。

use crate::state::AppState;
use serde_json::{json, Value as J};
use tauri::{Manager, State};

#[tauri::command]
pub fn get_preferences(state: State<'_, AppState>) -> J {
    state.preferences.lock().unwrap().clone()
}

#[tauri::command]
pub fn save_preferences(state: State<'_, AppState>, preferences: J) -> Result<(), String> {
    {
        let mut current = state.preferences.lock().unwrap();
        crate::state::deep_merge(&mut current, &preferences);
    }
    state.save_preferences();
    Ok(())
}

// ============================================================================
// 快照（标签页 / 侧栏 / 环境选择恢复）
// ============================================================================

fn read_snapshot(state: &AppState) -> J {
    std::fs::read_to_string(&state.layout.snapshot)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or(J::Null)
}

fn write_snapshot(state: &AppState, snapshot: &J) -> Result<(), String> {
    let body = serde_json::to_string_pretty(snapshot).map_err(|e| e.to_string())?;
    std::fs::write(&state.layout.snapshot, body).map_err(|e| format!("保存快照失败：{e}"))
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}

/// 按集合路径（可限定工作区）在快照里找条目。
/// 有工作区参数时优先精确匹配，匹配不到回落到同路径的第一条。
fn find_collection_entry(snapshot: &J, pathname: &str, workspace: Option<&str>) -> Option<J> {
    let list = snapshot.get("collections")?.as_array()?;
    let target = normalize_path(pathname);
    let workspace = workspace.map(normalize_path);
    let mut fallback: Option<J> = None;
    for entry in list {
        let path = entry.get("pathname").and_then(|v| v.as_str()).unwrap_or("");
        if normalize_path(path) != target {
            continue;
        }
        match &workspace {
            Some(ws) => {
                let entry_ws = entry
                    .get("workspacePathname")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if normalize_path(entry_ws) == *ws {
                    return Some(entry.clone());
                }
                if fallback.is_none() {
                    fallback = Some(entry.clone());
                }
            }
            None => return Some(entry.clone()),
        }
    }
    fallback
}

#[tauri::command]
pub fn snapshot_get(state: State<'_, AppState>) -> Option<J> {
    let value = read_snapshot(&state);
    if value.is_null() {
        None
    } else {
        Some(value)
    }
}

#[tauri::command]
pub fn snapshot_save(state: State<'_, AppState>, snapshot: J) -> Result<(), String> {
    write_snapshot(&state, &snapshot)
}

#[tauri::command]
pub fn snapshot_clear(state: State<'_, AppState>) -> Result<(), String> {
    let _ = std::fs::remove_file(&state.layout.snapshot);
    Ok(())
}

#[tauri::command]
pub fn snapshot_get_tabs(
    state: State<'_, AppState>,
    collection_pathname: String,
    workspace_pathname: Option<String>,
) -> Option<J> {
    let snapshot = read_snapshot(&state);
    let entry = find_collection_entry(&snapshot, &collection_pathname, workspace_pathname.as_deref())?;
    Some(json!({
        "tabs": entry.get("tabs").cloned().unwrap_or_else(|| json!([])),
        "activeTab": entry.get("activeTab").cloned().unwrap_or(J::Null),
    }))
}

/// 集合挂载时恢复：选中环境、是否展开、上次打开的标签页
#[tauri::command]
pub fn snapshot_get_collection(
    state: State<'_, AppState>,
    collection_pathname: String,
    workspace_pathname: Option<String>,
) -> Option<J> {
    let snapshot = read_snapshot(&state);
    find_collection_entry(&snapshot, &collection_pathname, workspace_pathname.as_deref())
}

#[tauri::command]
pub fn snapshot_get_sidebar(state: State<'_, AppState>) -> Option<J> {
    let snapshot = read_snapshot(&state);
    let sidebar = snapshot.get("extras")?.get("sidebar")?;
    if sidebar.is_null() {
        None
    } else {
        Some(sidebar.clone())
    }
}

/// 前端在切换环境时按 patch 方式回写快照（整份快照由 snapshot middleware 负责）
#[tauri::command]
pub fn snapshot_update_ui_state(state: State<'_, AppState>, payload: J) -> Result<(), String> {
    let kind = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let data = payload.get("data").cloned().unwrap_or(J::Null);
    let collection_path = data
        .get("collectionPath")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let Some(collection_path) = collection_path else {
        return Ok(());
    };
    let workspace_pathname = data.get("workspacePathname").and_then(|v| v.as_str());

    let mut snapshot = read_snapshot(&state);
    if !snapshot.is_object() {
        snapshot = json!({ "collections": [] });
    }
    if snapshot.get("collections").and_then(|v| v.as_array()).is_none() {
        snapshot["collections"] = json!([]);
    }

    let existing = find_collection_entry(&snapshot, collection_path, workspace_pathname);
    let mut entry = existing.clone().unwrap_or_else(|| {
        json!({
            "pathname": collection_path,
            "workspacePathname": workspace_pathname.unwrap_or(""),
            "environment": { "collection": "", "global": "" },
            "environmentPath": "",
            "selectedEnvironment": "",
            "isOpen": true,
            "isMounted": true,
            "activeTab": J::Null,
            "tabs": [],
        })
    });

    if kind == "COLLECTION_ENVIRONMENT" {
        let environment_path = data
            .get("environmentPath")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let selected = data
            .get("selectedEnvironment")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        entry["environmentPath"] = json!(environment_path);
        entry["selectedEnvironment"] = json!(selected);
        if entry.get("environment").map(|v| v.is_object()).unwrap_or(false) {
            entry["environment"]["collection"] = json!(environment_path);
        } else {
            entry["environment"] = json!({ "collection": environment_path, "global": "" });
        }
    } else {
        return Ok(());
    }

    let list = snapshot["collections"].as_array_mut().unwrap();
    let normalized = normalize_path(collection_path);
    let ws = workspace_pathname.map(normalize_path);
    let position = list.iter().position(|item| {
        let path = item.get("pathname").and_then(|v| v.as_str()).unwrap_or("");
        if normalize_path(path) != normalized {
            return false;
        }
        match &ws {
            Some(ws) => item
                .get("workspacePathname")
                .and_then(|v| v.as_str())
                .map(normalize_path)
                .as_ref()
                == Some(ws),
            None => true,
        }
    });
    match position {
        Some(index) => list[index] = entry,
        None => list.push(entry),
    }
    write_snapshot(&state, &snapshot)
}

/// 前端切主题时把窗口底色同步过去，避免启动/切换时白闪
#[tauri::command]
pub fn theme_change(app: tauri::AppHandle, theme: Option<String>, background: Option<String>) {
    let _ = theme;
    let Some(bg) = background else { return };
    let hex = bg.trim().trim_start_matches('#');
    if hex.len() != 6 && hex.len() != 8 {
        return;
    }
    let parse = |range: std::ops::Range<usize>| u8::from_str_radix(&hex[range], 16).unwrap_or(0);
    let (r, g, b) = (parse(0..2), parse(2..4), parse(4..6));
    let a = if hex.len() == 8 { parse(6..8) } else { 255 };
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_background_color(Some(tauri::window::Color(r, g, b, a)));
    }
}

// ============================================================================
// 系统代理（偏好 → Proxy 面板展示）
// ============================================================================

fn env_proxy(keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| std::env::var(key).ok())
        .filter(|v| !v.trim().is_empty())
}

#[cfg(windows)]
fn windows_registry_proxy() -> Option<(String, String)> {
    use std::os::windows::process::CommandExt;
    // 通过 reg.exe 读 Internet Settings，避免为了两个键引入 winreg 依赖
    let query = |name: &str| -> Option<String> {
        let output = std::process::Command::new("reg")
            .args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
                "/v",
                name,
            ])
            .creation_flags(0x08000000)
            .output()
            .ok()?;
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    };
    let enabled = query("ProxyEnable")?.contains("0x1");
    if !enabled {
        return None;
    }
    let raw = query("ProxyServer")?;
    let server = raw.split("REG_SZ").nth(1)?.trim().to_string();
    if server.is_empty() {
        return None;
    }
    // `http=host:port;https=host:port` 或单个 `host:port`
    if let Some(rest) = server.strip_prefix("http=") {
        let http = rest.split(';').next().unwrap_or(rest).to_string();
        let https = server
            .split(';')
            .find_map(|part| part.trim().strip_prefix("https="))
            .map(|s| s.to_string())
            .unwrap_or_else(|| http.clone());
        Some((http, https))
    } else {
        Some((server.clone(), server))
    }
}

#[cfg(not(windows))]
fn windows_registry_proxy() -> Option<(String, String)> {
    None
}

#[tauri::command]
pub fn get_system_proxy_variables() -> J {
    let mut http = env_proxy(&["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"]);
    let mut https = env_proxy(&["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]);
    let no_proxy = env_proxy(&["NO_PROXY", "no_proxy"]);
    let mut source = if http.is_some() || https.is_some() {
        "environment"
    } else {
        "none"
    };
    if http.is_none() && https.is_none() {
        if let Some((from_reg_http, from_reg_https)) = windows_registry_proxy() {
            http = Some(from_reg_http);
            https = Some(from_reg_https);
            source = "system";
        }
    }
    json!({
        "source": source,
        "http_proxy": http,
        "https_proxy": https,
        "no_proxy": no_proxy,
        "pac_url": J::Null,
    })
}

#[tauri::command]
pub fn refresh_system_proxy() -> J {
    get_system_proxy_variables()
}

/// reqwest 的 Client 每次请求都会重建，这里没有进程级连接池需要清理
#[tauri::command]
pub fn refresh_pac_cache() -> bool {
    true
}

#[tauri::command]
pub fn clear_http_https_agent_cache() -> bool {
    true
}

// ============================================================================
// 窗口
// ============================================================================

fn window_of(app: &tauri::AppHandle) -> Result<tauri::WebviewWindow, String> {
    app.get_webview_window("main")
        .ok_or_else(|| "找不到主窗口".to_string())
}

#[tauri::command]
pub fn window_minimize(app: tauri::AppHandle) -> Result<(), String> {
    window_of(&app)?.minimize().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn window_maximize(app: tauri::AppHandle) -> Result<(), String> {
    let window = window_of(&app)?;
    if window.is_maximized().unwrap_or(false) {
        window.unmaximize().map_err(|e| e.to_string())
    } else {
        window.maximize().map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub fn window_is_maximized(app: tauri::AppHandle) -> bool {
    window_of(&app).map(|w| w.is_maximized().unwrap_or(false)).unwrap_or(false)
}

#[tauri::command]
pub fn window_is_fullscreen(app: tauri::AppHandle) -> bool {
    window_of(&app).map(|w| w.is_fullscreen().unwrap_or(false)).unwrap_or(false)
}

#[tauri::command]
pub fn window_close(app: tauri::AppHandle) -> Result<(), String> {
    window_of(&app)?.close().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn toggle_fullscreen(app: tauri::AppHandle) -> Result<(), String> {
    let window = window_of(&app)?;
    let next = !window.is_fullscreen().unwrap_or(false);
    window.set_fullscreen(next).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn toggle_devtools(app: tauri::AppHandle) -> Result<(), String> {
    let window = window_of(&app)?;
    if window.is_devtools_open() {
        window.close_devtools();
    } else {
        window.open_devtools();
    }
    Ok(())
}

#[tauri::command]
pub fn set_zoom_level(app: tauri::AppHandle, level: f64) -> Result<(), String> {
    // 前端传的是 0.5–3 的比例；WebView2 的 zoom 也是比例
    let clamped = level.clamp(0.25, 5.0);
    window_of(&app)?
        .eval(&format!("document.documentElement.style.zoom = '{clamped}'"))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn refresh_screen_width(app: tauri::AppHandle) -> u32 {
    window_of(&app)
        .ok()
        .and_then(|w| w.inner_size().ok())
        .map(|s| s.width)
        .unwrap_or(1200)
}
