//! IPC 命令：通道名与 Bruno 原版保持一致，便于前端零改动复用。
//! 参数 camelCase（Tauri 2 IPC 约定）。

pub mod cookie;
pub mod env;
pub mod fsops;
pub mod git;
pub mod grpc;
pub mod prefs;
pub mod run;
pub mod workspace;
pub mod ws;

use crate::state::AppState;
use bruno_core::{Collection, SendRequestInput, execute_request};
use serde_json::json;
use std::path::PathBuf;
use tauri::{Emitter, State};

/// 默认工作区 uid（前端 uidSchema 要求字母数字）
pub use crate::state::DEFAULT_WORKSPACE_UID;

/// 集合树推送：所有写操作完成后统一走这里
pub(crate) fn emit_tree(app: &tauri::AppHandle, collection: &Collection, pathname: &str) {
    let _ = app.emit(
        "main:collection-tree-loaded",
        collection_tree_payload(collection, pathname),
    );
}

/// 工作区配置变更：payload 是 (workspacePath, workspaceUid, config) 三元组，
/// 必须用**当前激活**工作区的 uid/path，否则前端 `workspaceConfigUpdatedEvent` 会当别人的事件忽略。
pub(crate) fn emit_workspace_config(app: &tauri::AppHandle, state: &AppState) {
    let uid = state.active_workspace_uid();
    let path = state.workspace_pathname(&uid);
    let _ = app.emit(
        "main:workspace-config-updated",
        (path, uid.clone(), state.workspace_config_for(&uid)),
    );
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: &'static str,
    pub data_dir: String,
    pub product_name: &'static str,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> Result<AppInfo, String> {
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        data_dir: state.layout.root.to_string_lossy().to_string(),
        product_name: "Bruno RS",
    })
}

/// 启动握手：等价于原版 renderer:ready。Rust 侧回推 preferences 等事件。
#[tauri::command]
pub fn renderer_ready(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let preferences = state.preferences.lock().unwrap().clone();
    app.emit("main:load-preferences", preferences)
        .map_err(|e| e.to_string())?;
    app.emit(
        "main:load-global-environments",
        env::get_global_environments(state),
    )
    .map_err(|e| e.to_string())?;
    app.emit(
        "main:git-version",
        bruno_core::git::version().unwrap_or_default(),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 前端所有 main:* 监听注册完毕后调用：此时回推启动事件不会丢。
#[tauri::command]
pub fn app_ready(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    // 命令行 / 文件关联传入的集合：先挂载并记入工作区，再回推工作区配置
    if let Some(path) = state.startup_path.lock().unwrap().clone() {
        if let Ok(collection) = state.mount(PathBuf::from(&path)) {
            // 启动路径可能来自命令行（相对路径也算），监听用规范化后的集合路径
            crate::watcher::start(
                app.clone(),
                &collection.pathname.to_string_lossy().replace('\\', "/"),
            );
        }
    }

    let active_uid = state.active_workspace_uid();
    // 先把已知工作区逐个发给前端：非激活的 type=workspace 不会触发切换，
    // 激活的那个 type 强制为 default，让前端 startupWorkspaceRestorePending 解除后切过去。
    for entry in state.workspace_entries() {
        let uid = entry
            .get("uid")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_WORKSPACE_UID)
            .to_string();
        let path = entry
            .get("pathname")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let mut config = state.workspace_config_for(&uid);
        if uid == active_uid {
            config["type"] = json!("default");
        }
        let _ = app.emit("main:workspace-opened", (path, uid, config));
    }
    // workspaces-ready 最后发：前端 restoreActiveWorkspaceFromSnapshot 会据此选上次激活的工作区
    let _ = app.emit("main:workspaces-ready", json!({}));
    let _ = app.emit("main:app-loaded", json!({ "isRunningInRosetta": false }));
    // cookie 面板与全局环境：前端监听已就绪，直接回推当前状态
    cookie::bootstrap_cookies(&app);
    let _ = app.emit(
        "main:load-global-environments",
        env::get_global_environments(state),
    );
    // renderer:ready 阶段可能早于前端 listen() 注册完成，这里补推一次
    let _ = app.emit("main:git-version", bruno_core::git::version().unwrap_or_default());
    Ok(())
}

#[tauri::command]
pub fn get_collection_security_config(pathname: String) -> Result<serde_json::Value, String> {
    Ok(bruno_core::ops::read_security_config(&pathname))
}

#[tauri::command]
pub fn save_collection_security_config(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    pathname: String,
    security_config: serde_json::Value,
) -> Result<(), String> {
    bruno_core::ops::save_security_config(&pathname, &security_config)?;
    // 让集合设置面板里的值同步回到树/前端状态
    if let Ok(fresh) = state.reload_collection(&pathname) {
        emit_tree(&app, &fresh, &pathname);
    }
    Ok(())
}

/// 打开文件对话框选择集合目录（前端 renderer:browse-directory 的等价）
#[tauri::command]
pub async fn browse_directory(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |folder| {
        let _ = tx.send(folder.map(|p| p.into_path().map(|p| p.to_string_lossy().to_string()).unwrap_or_default()));
    });
    rx.await.map_err(|e| e.to_string())
}

/// 集合 → 前端 main:collection-tree-loaded 的 payload（open_collection 与
/// open_multiple_collections 共用，避免两处形状不一致）
fn collection_tree_payload(collection: &Collection, pathname: &str) -> serde_json::Value {
    json!({
        "collectionUid": collection.uid,
        "tree": {
            "pathname": pathname,
            "brunoConfig": null,
            "root": {
                "request": bruno_core::collection::normalize_defaults(&collection.root.request),
                "docs": collection.root.docs.clone().unwrap_or(json!(null)),
                "meta": { "name": collection.name }
            },
            "items": collection.tree.clone(),
            // 环境必须带 uid/pathname：前端按 uid 定位环境，缺了会变成只读
            "environments": bruno_core::envs::list(pathname),
        }
    })
}

/// 集合 → main:collection-opened 的 brunoConfig
fn collection_config(collection: &Collection) -> serde_json::Value {
    json!({
        "name": collection.name,
        "opencollection": collection.root.version,
        "type": "collection",
        "ignore": [],
    })
}

/// 打开集合：挂载 + 回推 main:collection-opened + 树事件
#[tauri::command]
pub async fn open_collection(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    pathname: String,
) -> Result<serde_json::Value, String> {
    let collection = state.mount(PathBuf::from(&pathname))?;
    let uid = collection.uid.clone();
    let name = collection.name.clone();
    let config = collection_config(&collection);

    // main:collection-opened 必须在前端收到 preferences 之后发
    app.emit("main:collection-opened", (pathname.clone(), &uid, config))
        .map_err(|e| e.to_string())?;

    // 集合由前端 openCollectionEvent 异步写入 Redux；树事件先发，
    // 前端按 collectionUid 轮询重试，因此这里不需要固定 sleep。
    app.emit("main:collection-tree-loaded", collection_tree_payload(&collection, &pathname))
        .map_err(|e| e.to_string())?;
    app.emit(
        "main:hydrate-app-with-ui-state-snapshot",
        json!({ "pathname": pathname, "hasSnapshotEntry": false }),
    )
    .map_err(|e| e.to_string())?;
    // 外部编辑 `.yml` 时也能刷新树
    crate::watcher::start(app.clone(), &pathname);
    Ok(json!({ "uid": uid, "name": name }))
}

/// 与 open_collection 等价的显式监听通道（前端 `renderer:add-collection-watcher`，
/// 原版用来给临时 scratch 集合挂 watcher）
#[tauri::command]
pub fn add_collection_watcher(app: tauri::AppHandle, payload: serde_json::Value) -> Result<bool, String> {
    let collection_path = payload
        .get("collectionPath")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if collection_path.is_empty() {
        return Ok(false);
    }
    crate::watcher::start(app, &collection_path);
    Ok(true)
}

/// 工作区目录监听：workspace.yml 变更不影响集合树，这里只做占位。
#[tauri::command]
pub fn start_workspace_watcher(_workspace_path: Option<String>) -> Result<bool, String> {
    Ok(true)
}

/// 挂载集合（原版 renderer:mount-collection），返回临时目录字符串占位
#[tauri::command]
pub fn mount_collection(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    collection_uid: String,
    collection_pathname: String,
) -> Result<String, String> {
    // 注意：不能在持有 collections 锁的表达式里再调 state.mount()——
    // std::sync::Mutex 不可重入，会自死锁（表现为前端点集合永远转圈）。
    let cached = {
        let guard = state.collections.lock().unwrap();
        guard.get(&collection_uid).cloned()
    };
    let collection = match cached {
        Some(collection) => collection,
        None => state.mount(PathBuf::from(&collection_pathname))?,
    };
    let resolved = collection
        .pathname
        .to_string_lossy()
        .replace('\\', "/");
    crate::watcher::start(app, &resolved);
    Ok(resolved)
}

#[tauri::command]
pub fn get_recent_collections(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    Ok(state.settings.lock().unwrap().recent_collections.clone())
}

#[tauri::command]
pub fn load_request(state: State<'_, AppState>, pathname: String) -> Result<serde_json::Value, String> {
    let collections = state.collections.lock().unwrap();
    for collection in collections.values() {
        if let Some(file) = collection.requests.get(&pathname) {
            let mut value = serde_json::to_value(file).map_err(|e| e.to_string())?;
            // HTTP 请求保持磁盘形状（脚本回归依赖 request.http.url）；
            // WS / gRPC 的 `websocket:` / `grpc:` 段前端不认识，换成 UI 形状
            if file.kind != "http-request" {
                value["request"] = bruno_core::collection::normalize_request(&file.request);
            }
            return Ok(value);
        }
    }
    Err(format!("未找到请求：{pathname}"))
}

/// 保存请求：把前端编辑结果合并回 OpenCollection YAML
/// （只覆盖 info/http/runtime/settings，磁盘上的未知键与其它顶层键保留）
#[tauri::command]
pub fn save_request(
    state: State<'_, AppState>,
    pathname: String,
    item: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let target = resolve_collection_file(&state, &pathname)?;
    bruno_core::save_request_file(&target, &item)?;
    // 内存索引同步：否则重新打开还是旧内容
    state.reload_request(&pathname);
    Ok(item)
}

/// 把前端传来的相对 pathname 解析成集合内的绝对文件路径，并阻止路径穿越
fn resolve_collection_file(state: &State<'_, AppState>, pathname: &str) -> Result<PathBuf, String> {
    let candidate = PathBuf::from(pathname);
    if candidate.is_absolute() {
        // 绝对路径必须落在某个已挂载集合内
        let canonical = candidate
            .canonicalize()
            .map_err(|e| format!("路径无效：{e}"))?;
        let canonical = strip_verbatim(canonical);
        let collections = state.collections.lock().unwrap();
        for collection in collections.values() {
            if canonical.starts_with(&collection.pathname) && canonical.is_file() {
                return Ok(canonical);
            }
        }
        return Err(format!("路径不在已打开的集合内：{pathname}"));
    }
    let collections = state.collections.lock().unwrap();
    for collection in collections.values() {
        let candidate = collection.pathname.join(pathname);
        if candidate.is_file() {
            let canonical = candidate
                .canonicalize()
                .map_err(|e| format!("路径无效：{e}"))?;
            let canonical = strip_verbatim(canonical);
            if canonical.starts_with(&collection.pathname) {
                return Ok(canonical);
            }
        }
    }
    Err(format!("找不到请求文件：{pathname}"))
}

fn strip_verbatim(path: PathBuf) -> PathBuf {
    let raw = path.to_string_lossy().to_string();
    if let Some(rest) = raw.strip_prefix(r"\\?\") {
        if let Some(unc) = rest.strip_prefix(r"UNC\") {
            return PathBuf::from(format!(r"\\{unc}"));
        }
        return PathBuf::from(rest);
    }
    path
}

/// UI「获取 OAuth2 Token」：payload = { itemUid, request, collection }
/// 返回前端 `collectionAddOauth2CredentialsByUrl` 需要的完整形状
#[tauri::command]
pub async fn fetch_oauth2_credentials(payload: serde_json::Value) -> Result<serde_json::Value, String> {
    oauth2_credentials(payload, false).await
}

#[tauri::command]
pub async fn refresh_oauth2_credentials(payload: serde_json::Value) -> Result<serde_json::Value, String> {
    oauth2_credentials(payload, true).await
}

async fn oauth2_credentials(payload: serde_json::Value, refresh: bool) -> Result<serde_json::Value, String> {
    let request = payload.get("request").cloned().unwrap_or_default();
    let collection_uid = payload
        .get("collection")
        .and_then(|c| c.get("uid"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let auth = request.get("auth").cloned().unwrap_or_default();
    let mut cfg = auth.get("oauth2").cloned().unwrap_or_else(|| auth.clone());
    if let Some(obj) = cfg.as_object_mut() {
        obj.remove("mode");
        if refresh {
            obj.insert("grantType".into(), json!("refresh_token"));
        }
    }
    let url = cfg
        .get("accessTokenUrl")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if url.is_empty() {
        return Err("OAuth2 缺少 accessTokenUrl".into());
    }
    let credentials = bruno_core::fetch_oauth2_token_response(&cfg)
        .await
        .map_err(|e| format!("{e}"))?;
    // credentialsId 用 URL 的短哈希，保证同一端点复用同一份缓存
    let credentials_id = {
        use sha2::{Digest, Sha256};
        hex::encode(&Sha256::digest(url.as_bytes())[..4])
    };
    Ok(json!({
        "credentials": credentials,
        "url": url,
        "collectionUid": collection_uid,
        "credentialsId": credentials_id,
        "debugInfo": {},
    }))
}

/// 发请求：前端 send-http-request 的 Rust 等价。
/// 这里负责三件命令层的事：套用偏好/cookie、把响应的 Set-Cookie 写回 jar、回推 Cookies 面板。
#[tauri::command]
pub async fn send_http_request(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: SendRequestInput,
) -> Result<serde_json::Value, String> {
    let mut input = input;
    if input.options.is_none() {
        input.options = Some(run::options_for(&state, &input.item));
    }
    let response = execute_request(input)
        .await
        .map_err(|e| format!("{e}"))?;
    if !response.set_cookies.is_empty() {
        let url = response
            .request_sent
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        {
            let mut jar = state.cookies.lock().unwrap();
            jar.store_response(&url, &response.set_cookies);
            let list = jar.list();
            drop(jar);
            state.save_cookies();
            let _ = app.emit("main:cookies-update", list);
        }
    }
    serde_json::to_value(response).map_err(|e| e.to_string())
}

/// 打开多个集合（工作区切换时由前端调用）：对每个路径 emit main:collection-opened
#[tauri::command]
pub async fn open_multiple_collections(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    collection_paths: Vec<String>,
    _options: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let mut opened = Vec::new();
    let mut failed = Vec::new();
    for path in &collection_paths {
        match state.mount(std::path::PathBuf::from(path)) {
            Ok(collection) => {
                let config = collection_config(&collection);
                let _ = app.emit(
                    "main:collection-opened",
                    (path.clone(), collection.uid.clone(), config),
                );
                // 前端 openCollectionEvent 还要走若干 IPC 才能把集合放进 Redux，
                // 迟到的树由前端按 collectionUid 轮询重试，这里无需固定等待。
                let _ = app.emit(
                    "main:collection-tree-loaded",
                    collection_tree_payload(&collection, path),
                );
                crate::watcher::start(
                    app.clone(),
                    &collection.pathname.to_string_lossy().replace('\\', "/"),
                );
                opened.push(path.clone());
            }
            Err(e) => failed.push(json!({ "path": path, "error": e })),
        }
    }
    Ok(json!({ "opened": opened, "failed": failed }))
}

/// 工作区集合列表：来自该工作区的 workspace.yml（默认工作区读 settings.json）
#[tauri::command]
pub fn load_workspace_collections(
    state: State<'_, AppState>,
    workspace_path: String,
) -> Result<Vec<serde_json::Value>, String> {
    // 前端切工作区时只会调这个通道，借机把后端 active 同步过来，
    // 否则后面 mount 触发的 remember/事件推送还会按旧工作区走。
    if let Some(uid) = state.workspace_uid_by_path(&workspace_path) {
        state.set_active_workspace(&uid);
    }
    let collections = state.collections_for_workspace_path(&workspace_path);
    Ok(collections
        .iter()
        .map(|c| {
            let mut value = json!({ "path": c.path, "name": c.name });
            if let Some(remote) = &c.remote {
                value["remote"] = json!(remote);
            }
            value
        })
        .collect())
}

/// 把集合加入工作区（原版 renderer:add-collection-to-workspace）：
/// 写入 settings 后回推 main:workspace-config-updated，前端据此重建侧栏。
#[tauri::command]
pub fn add_collection_to_workspace(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    workspace_path: String,
    collection: serde_json::Value,
) -> Result<(), String> {
    let path = collection
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if path.is_empty() {
        return Err("缺少集合路径".into());
    }
    let name = collection
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            std::path::Path::new(&path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Collection")
                .to_string()
        });
    state.add_collection_to(&workspace_path, &name, &path);
    let _ = emit_workspace_config(&app, &state);
    Ok(())
}

/// Postman → Bruno 集合（原版 renderer:convert-postman-to-bruno）。
/// 原版在 Electron 主进程调用 `@usebruno/converters`；这里由 Rust 完成常用子集。
#[tauri::command]
pub fn convert_postman_to_bruno(
    collection: serde_json::Value,
    _options: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    bruno_core::postman_to_bruno(&collection)
}

/// 导入落盘（原版 renderer:import-collection）：Bruno 集合 → OpenCollection YAML。
/// 返回 `{ success: { items: [{ name, path }] } }`，并立即挂载 + 刷新侧栏。
#[tauri::command]
pub fn import_collection(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    collection: serde_json::Value,
    collection_location: String,
    _options: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let imported = bruno_core::import_collection(&collection, &collection_location)?;

    let mut mounted = Vec::new();
    for item in &imported {
        let path = item
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if path.is_empty() {
            continue;
        }
        match state.mount(PathBuf::from(&path)) {
            Ok(collection) => {
                // 树事件早于前端 Redux 里的集合条目也没关系：前端按 uid 轮询重试
                let _ = app.emit(
                    "main:collection-tree-loaded",
                    collection_tree_payload(&collection, &path),
                );
                mounted.push(json!({ "name": collection.name, "path": path }));
            }
            Err(e) => eprintln!("[bruno-rs] 导入后挂载失败 {path}: {e}"),
        }
    }

    // settings 里已记录 workspace_collections，通知前端重建侧栏
    let _ = emit_workspace_config(&app, &state);

    Ok(json!({ "success": { "items": imported, "mounted": mounted } }))
}

/// 工作区 Scratch 集合目录（原版 renderer:mount-workspace-scratch）：
/// 每个工作区固定一份，首次访问时创建最小 opencollection.yml。
#[tauri::command]
pub fn mount_workspace_scratch(
    state: State<'_, AppState>,
    _workspace_uid: String,
    _workspace_path: String,
) -> Result<String, String> {
    let dir = state.layout.root.join("scratch");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建 Scratch 目录失败：{e}"))?;
    let root_file = dir.join(bruno_core::collection::ROOT_FILE);
    if !root_file.is_file() {
        std::fs::write(
            &root_file,
            "opencollection: \"1.0.0\"\ninfo:\n  name: Scratch\nbundled: false\n",
        )
        .map_err(|e| format!("写入 Scratch 集合失败：{e}"))?;
    }
    Ok(dir.to_string_lossy().replace('\\', "/"))
}

#[tauri::command]
pub fn cancel_http_request(state: State<'_, AppState>, cancel_token_uid: String) -> Result<(), String> {
    // Runner 的取消 token 以 runner- 开头；单请求的中断暂不支持（请求很快返回）
    if cancel_token_uid.starts_with("runner-") {
        state
            .runner_cancel
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
    Ok(())
}

#[tauri::command]
pub fn app_open_external(app: tauri::AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

