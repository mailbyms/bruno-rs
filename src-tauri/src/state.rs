//! 应用状态：集合索引 + 设置 + 偏好 + cookie jar。冷启动不加载任何文档（与 xmind-rs 一致）。

use crate::paths::{self, DataLayout};
use bruno_core::cookies::CookieJar;
use bruno_core::Collection;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceCollection {
    pub name: String,
    pub path: String,
    /// 原版工作区配置里的 collections[].remote
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
}

/// 默认工作区（不落 workspace.yml，集合列表存在 settings.json）
pub const DEFAULT_WORKSPACE_UID: &str = "brunorsdefaultworkspace1";

/// 用户自己建/打开的工作区，对应磁盘上的 `<dir>/workspace.yml`
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct WorkspaceEntry {
    pub uid: String,
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub collections: Vec<WorkspaceCollection>,
    #[serde(default)]
    pub docs: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub recent_collections: Vec<String>,
    /// 默认工作区里收录的集合（对应原版 workspace.yml 的 collections）
    #[serde(default)]
    pub workspace_collections: Vec<WorkspaceCollection>,
    #[serde(default = "default_theme")]
    pub theme: String,
    /// 默认工作区之外的其它工作区
    #[serde(default)]
    pub workspaces: Vec<WorkspaceEntry>,
    /// 当前激活的工作区；为空表示默认工作区
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_workspace_uid: Option<String>,
}

fn default_theme() -> String {
    "dark".into()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            recent_collections: Vec::new(),
            workspace_collections: Vec::new(),
            theme: "dark".into(),
            workspaces: Vec::new(),
            active_workspace_uid: None,
        }
    }
}

pub struct AppState {
    pub layout: DataLayout,
    pub settings: Mutex<Settings>,
    /// collectionUid -> 已挂载集合
    pub collections: Mutex<HashMap<String, Collection>>,
    /// pathname -> collectionUid
    pub path_to_uid: Mutex<HashMap<String, String>>,
    /// 命令行传入的启动集合路径（在 renderer:ready 时回推给前端）
    pub startup_path: Mutex<Option<String>>,
    /// 前端偏好设置（原版 electron-store）
    pub preferences: Mutex<serde_json::Value>,
    /// 跨请求共享的 cookie jar
    /// 跨请求共享的 cookie jar（Arc 便于把只读 hook 传给 Runner）
    pub cookies: std::sync::Arc<Mutex<CookieJar>>,
    /// 集合 Runner 的取消标志
    pub runner_cancel: AtomicBool,
    /// pathname -> 文件监听线程的存活标志
    pub watchers: Mutex<HashMap<String, std::sync::Arc<AtomicBool>>>,
    /// 活跃的 WebSocket 连接（requestId -> 连接）
    pub ws_hub: std::sync::Arc<crate::hubs::WsHub>,
    /// 活跃的 gRPC 调用（requestId -> 调用）
    pub grpc_hub: std::sync::Arc<crate::hubs::GrpcHub>,
}

impl AppState {
    pub fn init() -> anyhow::Result<Self> {
        let root = paths::data_dir();
        let layout = paths::layout(&root);
        std::fs::create_dir_all(&layout.root)?;

        let settings = if layout.settings.is_file() {
            let raw = std::fs::read_to_string(&layout.settings)?;
            serde_json::from_str(&raw).unwrap_or_default()
        } else {
            Settings::default()
        };
        // 激活的工作区已不存在（目录被删/手动改过设置）时回到默认工作区
        let mut settings = settings;
        if let Some(uid) = settings.active_workspace_uid.clone() {
            if !settings.workspaces.iter().any(|w| w.uid == uid) {
                settings.active_workspace_uid = None;
            }
        }

        // 偏好：与前端默认值深合并，老版本缺字段也不会让 UI 读到 undefined
        let mut preferences = default_preferences();
        if layout.preferences.is_file() {
            if let Ok(raw) = std::fs::read_to_string(&layout.preferences) {
                if let Ok(saved) = serde_json::from_str::<serde_json::Value>(&raw) {
                    deep_merge(&mut preferences, &saved);
                }
            }
        }

        let cookies = std::fs::read_to_string(&layout.cookies)
            .map(|raw| CookieJar::from_json(&raw))
            .unwrap_or_default();
        let _ = std::fs::write(
            &layout.log,
            format!(
                "[bruno-rs] {} data_dir={}\n",
                chrono_stamp(),
                layout.root.display()
            ),
        );

        Ok(Self {
            layout,
            settings: Mutex::new(settings),
            collections: Mutex::new(HashMap::new()),
            path_to_uid: Mutex::new(HashMap::new()),
            startup_path: Mutex::new(None),
            preferences: Mutex::new(preferences),
            cookies: std::sync::Arc::new(Mutex::new(cookies)),
            runner_cancel: AtomicBool::new(false),
            watchers: Mutex::new(HashMap::new()),
            ws_hub: std::sync::Arc::new(crate::hubs::WsHub::new()),
            grpc_hub: std::sync::Arc::new(crate::hubs::GrpcHub::new()),
        })
    }

    pub fn save_preferences(&self) {
        let prefs = self.preferences.lock().unwrap();
        let _ = std::fs::write(
            &self.layout.preferences,
            serde_json::to_string_pretty(&*prefs).unwrap_or_default(),
        );
    }

    pub fn save_cookies(&self) {
        let jar = self.cookies.lock().unwrap();
        let _ = std::fs::write(&self.layout.cookies, jar.to_json());
    }

    /// 请求级参数：把偏好设置翻译成 `RequestOptions` 的默认值
    pub fn request_defaults(&self) -> bruno_core::RequestOptions {
        let prefs = self.preferences.lock().unwrap();
        let request = prefs.get("request");
        let timeout = request
            .and_then(|r| r.get("timeout"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let verify = request
            .and_then(|r| r.get("sslVerification"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        bruno_core::RequestOptions {
            timeout_ms: if timeout > 0 { Some(timeout) } else { None },
            verify_tls: Some(verify),
            proxy: proxy_from_prefs(&prefs),
            ..Default::default()
        }
    }

    pub fn save_settings(&self) {
        let settings = self.settings.lock().unwrap();
        let _ = std::fs::write(
            &self.layout.settings,
            serde_json::to_string_pretty(&*settings).unwrap_or_default(),
        );
    }

    pub fn mount(&self, path: PathBuf) -> Result<Collection, String> {
        let canonical = path
            .canonicalize()
            .map_err(|e| format!("路径无效：{e}"))?;
        // Windows 的 canonicalize 会加 `\\?\` 长路径前缀，前端按字符串比较路径会不匹配
        let canonical = strip_verbatim(canonical);
        let pathname = canonical.to_string_lossy().replace('\\', "/");
        // 已挂载则直接返回
        if let Some(uid) = self.path_to_uid.lock().unwrap().get(&pathname) {
            if let Some(collection) = self.collections.lock().unwrap().get(uid) {
                return Ok(collection.clone());
            }
        }
        let collection = bruno_core::load_collection(&canonical)?;
        let uid = collection.uid.clone();
        self.collections
            .lock()
            .unwrap()
            .insert(uid.clone(), collection.clone());
        self.path_to_uid
            .lock()
            .unwrap()
            .insert(pathname.clone(), uid);
        self.remember(&pathname, &collection.name);
        Ok(collection)
    }

    /// 请求保存后同步内存索引（按前端传来的相对 pathname 找到所在集合）
    pub fn reload_request(&self, pathname: &str) {
        let normalized = pathname.replace('\\', "/");
        let mut collections = self.collections.lock().unwrap();
        for collection in collections.values_mut() {
            if collection.requests.contains_key(&normalized) {
                let _ = bruno_core::collection::reload_request(collection, &normalized);
                break;
            }
        }
    }

    /// 结构变化（增删改移）后整份重载集合并替换缓存。
    /// uid 按路径稳定生成，所以前端 mergeTreeItems 能保住草稿与已打开的标签页。
    pub fn reload_collection(&self, pathname: &str) -> Result<Collection, String> {
        let key = pathname.replace('\\', "/");
        let canonical = PathBuf::from(&key)
            .canonicalize()
            .map(|p| strip_verbatim(p))
            .unwrap_or_else(|_| PathBuf::from(&key));
        let uid = {
            let map = self.path_to_uid.lock().unwrap();
            map.get(&key)
                .cloned()
                .or_else(|| map.get(&canonical.to_string_lossy().replace('\\', "/")).cloned())
        };
        let mut fresh = bruno_core::load_collection(&canonical)?;
        let uid = uid.unwrap_or_else(|| fresh.uid.clone());
        fresh.uid = uid.clone();
        self.collections.lock().unwrap().insert(uid, fresh.clone());
        Ok(fresh)
    }

    /// 记入最近打开 + 当前激活工作区
    pub fn remember(&self, pathname: &str, name: &str) {
        let normalized = pathname.replace('\\', "/");
        {
            let mut settings = self.settings.lock().unwrap();
            if !settings.recent_collections.contains(&normalized) {
                settings.recent_collections.insert(0, normalized.clone());
                settings.recent_collections.truncate(20);
            }
            let entry = WorkspaceCollection {
                name: name.to_string(),
                path: normalized.clone(),
                remote: None,
            };
            match settings.active_workspace_uid.clone().as_deref() {
                // 只有默认工作区会自动收录「打开过的集合」；
                // 非默认工作区的集合由用户显式添加（add-collection-to-workspace），
                // 否则切换工作区后打开旧集合会把它们静默塞进新工作区。
                None | Some("") | Some(DEFAULT_WORKSPACE_UID) => {
                    if !settings
                        .workspace_collections
                        .iter()
                        .any(|c| c.path.eq_ignore_ascii_case(&normalized))
                    {
                        settings.workspace_collections.push(entry);
                    }
                }
                Some(_) => {}
            }
        }
        self.save_settings();
    }

    pub fn active_workspace_uid(&self) -> String {
        self.settings
            .lock()
            .unwrap()
            .active_workspace_uid
            .clone()
            .filter(|uid| !uid.is_empty())
            .unwrap_or_else(|| DEFAULT_WORKSPACE_UID.to_string())
    }

    pub fn set_active_workspace(&self, uid: &str) -> bool {
        let ok = {
            let mut settings = self.settings.lock().unwrap();
            let exists = uid == DEFAULT_WORKSPACE_UID
                || settings.workspaces.iter().any(|w| w.uid == uid);
            if exists {
                settings.active_workspace_uid = if uid == DEFAULT_WORKSPACE_UID {
                    None
                } else {
                    Some(uid.to_string())
                };
            }
            exists
        };
        if ok {
            self.save_settings();
        }
        ok
    }

    /// 所有工作区（默认工作区排第一），供 `renderer:load-workspaces` 使用
    pub fn workspace_entries(&self) -> Vec<serde_json::Value> {
        let settings = self.settings.lock().unwrap();
        let default_path = self.layout.root.to_string_lossy().replace('\\', "/");
        let mut list = vec![serde_json::json!({
            "uid": DEFAULT_WORKSPACE_UID,
            "name": "Bruno RS Workspace",
            "pathname": default_path,
            "type": "default",
            "collections": settings
                .workspace_collections
                .iter()
                .map(|c| collection_json(c))
                .collect::<Vec<_>>(),
        })];
        for ws in &settings.workspaces {
            list.push(serde_json::json!({
                "uid": ws.uid,
                "name": ws.name,
                "pathname": ws.path,
                "type": "workspace",
                "collections": ws.collections.iter().map(collection_json).collect::<Vec<_>>(),
            }));
        }
        list
    }

    /// 某个工作区的配置（前端 buildSidebarEntries 依赖 collections[].path）
    pub fn workspace_config_for(&self, uid: &str) -> serde_json::Value {
        let settings = self.settings.lock().unwrap();
        let default_path = self.layout.root.to_string_lossy().replace('\\', "/");
        if uid == DEFAULT_WORKSPACE_UID {
            return serde_json::json!({
                "name": "Bruno RS Workspace",
                "type": "default",
                "pathname": default_path,
                "collections": settings
                    .workspace_collections
                    .iter()
                    .map(collection_json)
                    .collect::<Vec<_>>(),
            });
        }
        match settings.workspaces.iter().find(|w| w.uid == uid) {
            Some(ws) => serde_json::json!({
                "name": ws.name,
                "type": "workspace",
                "pathname": ws.path,
                "docs": ws.docs,
                "collections": ws.collections.iter().map(collection_json).collect::<Vec<_>>(),
            }),
            None => serde_json::json!({
                "name": "Bruno RS Workspace",
                "type": "default",
                "pathname": default_path,
                "collections": [],
            }),
        }
    }

    /// 当前激活工作区的配置
    #[allow(dead_code)]
    pub fn workspace_config(&self) -> serde_json::Value {
        let uid = self.active_workspace_uid();
        self.workspace_config_for(&uid)
    }

    pub fn workspace_by_uid(&self, uid: &str) -> Option<WorkspaceEntry> {
        self.settings
            .lock()
            .unwrap()
            .workspaces
            .iter()
            .find(|w| w.uid == uid)
            .cloned()
    }

    /// 工作区目录（默认工作区就是数据目录）
    pub fn workspace_pathname(&self, uid: &str) -> String {
        if uid == DEFAULT_WORKSPACE_UID {
            return self.layout.root.to_string_lossy().replace('\\', "/");
        }
        self.settings
            .lock()
            .unwrap()
            .workspaces
            .iter()
            .find(|w| w.uid == uid)
            .map(|w| w.path.clone())
            .unwrap_or_default()
    }

    /// 按目录路径反查工作区 uid（默认工作区的路径是数据目录）
    pub fn workspace_uid_by_path(&self, path: &str) -> Option<String> {        let normalized = path.replace('\\', "/");
        let default_path = self.layout.root.to_string_lossy().replace('\\', "/");
        if normalized.is_empty() || normalized.eq_ignore_ascii_case(&default_path) {
            return Some(DEFAULT_WORKSPACE_UID.to_string());
        }
        self.settings
            .lock()
            .unwrap()
            .workspaces
            .iter()
            .find(|w| w.path.eq_ignore_ascii_case(&normalized))
            .map(|w| w.uid.clone())
    }

    /// 按工作区路径取集合列表（默认工作区用设置里的列表）
    pub fn collections_for_workspace_path(&self, workspace_path: &str) -> Vec<WorkspaceCollection> {
        let normalized = workspace_path.replace('\\', "/");
        let settings = self.settings.lock().unwrap();
        let default_path = self.layout.root.to_string_lossy().replace('\\', "/");
        if normalized.eq_ignore_ascii_case(&default_path) {
            return settings.workspace_collections.clone();
        }
        settings
            .workspaces
            .iter()
            .find(|w| w.path.eq_ignore_ascii_case(&normalized))
            .map(|w| w.collections.clone())
            .unwrap_or_default()
    }

    /// 把集合加入指定路径的工作区
    pub fn add_collection_to(&self, workspace_path: &str, name: &str, path: &str) {
        let normalized_ws = workspace_path.replace('\\', "/");
        let normalized = path.replace('\\', "/");
        let mut touched: Option<WorkspaceEntry> = None;
        {
            let mut settings = self.settings.lock().unwrap();
            let default_path = self.layout.root.to_string_lossy().replace('\\', "/");
            if normalized_ws.is_empty() || normalized_ws.eq_ignore_ascii_case(&default_path) {
                if !settings
                    .workspace_collections
                    .iter()
                    .any(|c| c.path.eq_ignore_ascii_case(&normalized))
                {
                    settings.workspace_collections.push(WorkspaceCollection {
                        name: name.to_string(),
                        path: normalized.clone(),
                        remote: None,
                    });
                }
            } else if let Some(ws) = settings
                .workspaces
                .iter_mut()
                .find(|w| w.path.eq_ignore_ascii_case(&normalized_ws))
            {
                if !ws
                    .collections
                    .iter()
                    .any(|c| c.path.eq_ignore_ascii_case(&normalized))
                {
                    ws.collections.push(WorkspaceCollection {
                        name: name.to_string(),
                        path: normalized.clone(),
                        remote: None,
                    });
                }
                touched = Some(ws.clone());
            }
        }
        if let Some(ws) = touched {
            self.persist_workspace(&ws);
        }
        self.save_settings();
    }

    /// 该集合是否还属于某个工作区（移除后决定要不要卸载并停止监听）
    pub fn is_collection_in_any_workspace(&self, path: &str) -> bool {
        let normalized = path.replace('\\', "/");
        let settings = self.settings.lock().unwrap();
        settings
            .workspace_collections
            .iter()
            .any(|c| c.path.eq_ignore_ascii_case(&normalized))
            || settings
                .workspaces
                .iter()
                .any(|w| w.collections.iter().any(|c| c.path.eq_ignore_ascii_case(&normalized)))
    }

    /// 从指定路径的工作区移除集合
    pub fn remove_collection_from(&self, workspace_path: &str, path: &str) {
        let normalized_ws = workspace_path.replace('\\', "/");
        let normalized = path.replace('\\', "/");
        let mut touched: Option<WorkspaceEntry> = None;
        {
            let mut settings = self.settings.lock().unwrap();
            let default_path = self.layout.root.to_string_lossy().replace('\\', "/");
            if normalized_ws.is_empty() || normalized_ws.eq_ignore_ascii_case(&default_path) {
                settings
                    .workspace_collections
                    .retain(|c| !c.path.eq_ignore_ascii_case(&normalized));
            } else if let Some(ws) = settings
                .workspaces
                .iter_mut()
                .find(|w| w.path.eq_ignore_ascii_case(&normalized_ws))
            {
                ws.collections
                    .retain(|c| !c.path.eq_ignore_ascii_case(&normalized));
                touched = Some(ws.clone());
            }
        }
        if let Some(ws) = touched {
            self.persist_workspace(&ws);
        }
        self.save_settings();
    }

    /// 创建并注册一个工作区目录（写 workspace.yml），并切为当前工作区
    pub fn create_workspace(
        &self,
        name: &str,
        folder_name: &str,
        location: &str,
    ) -> Result<WorkspaceEntry, String> {
        let folder = if folder_name.trim().is_empty() {
            name.to_string()
        } else {
            folder_name.to_string()
        };
        let dir = std::path::Path::new(location).join(&folder);
        std::fs::create_dir_all(&dir).map_err(|e| format!("创建工作区目录失败：{e}"))?;
        let path = dir.to_string_lossy().replace('\\', "/");
        let uid = bruno_core::ops::stable_uid(&path);
        let entry = WorkspaceEntry {
            uid: uid.clone(),
            name: name.to_string(),
            path: path.clone(),
            collections: Vec::new(),
            docs: String::new(),
        };
        self.persist_workspace(&entry);
        {
            let mut settings = self.settings.lock().unwrap();
            settings.workspaces.retain(|w| w.uid != uid);
            settings.workspaces.push(entry.clone());
            settings.active_workspace_uid = Some(uid);
        }
        self.save_settings();
        Ok(entry)
    }

    /// 打开已有工作区目录：读 workspace.yml，没有就按目录名补一个
    pub fn register_workspace(&self, path: &str) -> Result<WorkspaceEntry, String> {
        let dir = std::path::Path::new(path);
        if !dir.is_dir() {
            return Err(format!("目录不存在：{path}"));
        }
        let normalized = dir
            .canonicalize()
            .map(|p| strip_verbatim(p))
            .unwrap_or_else(|_| dir.to_path_buf())
            .to_string_lossy()
            .replace('\\', "/");
        let (name, collections, docs) = read_workspace_yaml(dir).unwrap_or_else(|| {
            let fallback = dir
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "Workspace".into());
            (fallback, Vec::new(), String::new())
        });
        let uid = bruno_core::ops::stable_uid(&normalized);
        let entry = WorkspaceEntry {
            uid: uid.clone(),
            name,
            path: normalized,
            collections,
            docs,
        };
        {
            let mut settings = self.settings.lock().unwrap();
            settings.workspaces.retain(|w| w.uid != uid);
            settings.workspaces.push(entry.clone());
        }
        self.save_settings();
        Ok(entry)
    }

    pub fn rename_workspace(&self, uid: &str, name: &str) -> bool {
        let mut touched: Option<WorkspaceEntry> = None;
        {
            let mut settings = self.settings.lock().unwrap();
            if let Some(ws) = settings.workspaces.iter_mut().find(|w| w.uid == uid) {
                ws.name = name.to_string();
                touched = Some(ws.clone());
            }
        }
        if let Some(ws) = touched {
            self.persist_workspace(&ws);
            self.save_settings();
            true
        } else {
            false
        }
    }

    pub fn close_workspace(&self, uid: &str) {
        {
            let mut settings = self.settings.lock().unwrap();
            settings.workspaces.retain(|w| w.uid != uid);
            if settings.active_workspace_uid.as_deref() == Some(uid) {
                settings.active_workspace_uid = None;
            }
        }
        self.save_settings();
    }

    pub fn save_workspace_docs(&self, uid: &str, docs: &str) -> bool {
        let mut touched: Option<WorkspaceEntry> = None;
        {
            let mut settings = self.settings.lock().unwrap();
            if let Some(ws) = settings.workspaces.iter_mut().find(|w| w.uid == uid) {
                ws.docs = docs.to_string();
                touched = Some(ws.clone());
            }
        }
        match touched {
            Some(ws) => {
                self.persist_workspace(&ws);
                self.save_settings();
                true
            }
            None => false,
        }
    }

    /// 把工作区写回 `<dir>/workspace.yml`（默认工作区不落盘，避免和 settings 双份真相）
    fn persist_workspace(&self, entry: &WorkspaceEntry) {
        write_workspace_yaml(entry);
    }

    /// 连接/断开 Git 远端（只改工作区配置，不碰仓库本身）
    pub fn set_collection_remote(&self, pathname: &str, remote: Option<&str>) -> bool {
        let normalized = pathname.replace('\\', "/");
        let mut matched = false;
        let mut touched: Vec<WorkspaceEntry> = Vec::new();
        {
            let mut settings = self.settings.lock().unwrap();
            for collection in settings.workspace_collections.iter_mut() {
                if collection.path.eq_ignore_ascii_case(&normalized) {
                    collection.remote = remote.map(|r| r.to_string());
                    matched = true;
                }
            }
            for ws in settings.workspaces.iter_mut() {
                let mut hit = false;
                for collection in ws.collections.iter_mut() {
                    if collection.path.eq_ignore_ascii_case(&normalized) {
                        collection.remote = remote.map(|r| r.to_string());
                        matched = true;
                        hit = true;
                    }
                }
                if hit {
                    touched.push(ws.clone());
                }
            }
        }
        for ws in &touched {
            self.persist_workspace(ws);
        }
        if matched {
            self.save_settings();
        }
        matched
    }

    pub fn remove_from_workspace(&self, pathname: &str) {
        let normalized = pathname.replace('\\', "/");
        let mut touched: Vec<WorkspaceEntry> = Vec::new();
        {
            let mut settings = self.settings.lock().unwrap();
            settings
                .workspace_collections
                .retain(|c| !c.path.eq_ignore_ascii_case(&normalized));
            settings
                .recent_collections
                .retain(|c| !c.eq_ignore_ascii_case(&normalized));
            for ws in settings.workspaces.iter_mut() {
                let before = ws.collections.len();
                ws.collections
                    .retain(|c| !c.path.eq_ignore_ascii_case(&normalized));
                if ws.collections.len() != before {
                    touched.push(ws.clone());
                }
            }
        }
        for ws in &touched {
            self.persist_workspace(ws);
        }
        self.save_settings();
    }

    pub fn rename_in_workspace(&self, pathname: &str, new_name: &str) {
        let normalized = pathname.replace('\\', "/");
        let mut touched: Vec<WorkspaceEntry> = Vec::new();
        {
            let mut settings = self.settings.lock().unwrap();
            for collection in settings.workspace_collections.iter_mut() {
                if collection.path.eq_ignore_ascii_case(&normalized) {
                    collection.name = new_name.to_string();
                }
            }
            for ws in settings.workspaces.iter_mut() {
                let mut hit = false;
                for collection in ws.collections.iter_mut() {
                    if collection.path.eq_ignore_ascii_case(&normalized) {
                        collection.name = new_name.to_string();
                        hit = true;
                    }
                }
                if hit {
                    touched.push(ws.clone());
                }
            }
        }
        for ws in &touched {
            self.persist_workspace(ws);
        }
        self.save_settings();
    }
}

fn collection_json(collection: &WorkspaceCollection) -> serde_json::Value {
    let mut value = serde_json::json!({ "name": collection.name, "path": collection.path });
    if let Some(remote) = &collection.remote {
        value["remote"] = serde_json::json!(remote);
    }
    value
}

/// 读 `<dir>/workspace.yml`：返回 (name, collections, docs)
fn read_workspace_yaml(dir: &std::path::Path) -> Option<(String, Vec<WorkspaceCollection>, String)> {
    let file = dir.join("workspace.yml");
    let raw = std::fs::read_to_string(&file).ok()?;
    let doc: serde_yml::Value = serde_yml::from_str(&raw).ok()?;
    let name = doc
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Workspace")
        .to_string();
    let docs = doc
        .get("docs")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mut collections = Vec::new();
    if let Some(list) = doc.get("collections").and_then(|v| v.as_sequence()) {
        for item in list {
            let path = item.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if path.is_empty() {
                continue;
            }
            let display = item
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            collections.push(WorkspaceCollection {
                name: display,
                path: path.replace('\\', "/"),
                remote: item
                    .get("remote")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string()),
            });
        }
    }
    Some((name, collections, docs))
}

fn write_workspace_yaml(entry: &WorkspaceEntry) {
    let dir = std::path::Path::new(&entry.path);
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let mut map = serde_yml::Mapping::new();
    map.insert("name".into(), entry.name.clone().into());
    map.insert("type".into(), "workspace".into());
    let mut list = Vec::new();
    for collection in &entry.collections {
        let mut item = serde_yml::Mapping::new();
        item.insert("name".into(), collection.name.clone().into());
        item.insert("path".into(), collection.path.clone().into());
        if let Some(remote) = &collection.remote {
            item.insert("remote".into(), remote.clone().into());
        }
        list.push(serde_yml::Value::Mapping(item));
    }
    map.insert("collections".into(), serde_yml::Value::Sequence(list));
    if !entry.docs.is_empty() {
        map.insert("docs".into(), entry.docs.clone().into());
    }
    if let Ok(body) = serde_yml::to_string(&serde_yml::Value::Mapping(map)) {
        let _ = std::fs::write(dir.join("workspace.yml"), body);
    }
}

fn proxy_from_prefs(prefs: &serde_json::Value) -> Option<String> {
    let proxy = prefs.get("proxy")?;
    // 两种形状：{ enabled, host, port, protocol } 或 { mode: 'manual' }
    let enabled = proxy
        .get("enabled")
        .and_then(|v| v.as_bool())
        .or_else(|| proxy.get("mode").and_then(|v| v.as_str()).map(|m| m == "manual"))?;
    if !enabled {
        return None;
    }
    if let Some(url) = proxy.get("url").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
        return Some(url.to_string());
    }
    let host = proxy.get("host").and_then(|v| v.as_str())?;
    let port = proxy.get("port").and_then(|v| v.as_u64()).unwrap_or(8080);
    let protocol = proxy
        .get("protocol")
        .and_then(|v| v.as_str())
        .unwrap_or("http");
    Some(format!("{protocol}://{host}:{port}"))
}

/// 前端 `app.js` 的 initialPreferences 快照。
/// 前端收到 `main:load-preferences` 后是整体替换 `state.preferences`，
/// 所以后端必须给完整对象，不能只给增量。
pub fn default_preferences() -> serde_json::Value {
    serde_json::json!({
        "request": {
            "sslVerification": true,
            "customCaCertificate": { "enabled": false, "filePath": null },
            "keepDefaultCaCertificates": { "enabled": true },
            "timeout": 0,
            "oauth2": { "useSystemBrowser": false },
            "clientCertificates": { "certs": [] }
        },
        "font": { "codeFont": "default" },
        "general": { "defaultLocation": "" },
        "onboarding": { "hasLaunchedBefore": false, "hasSeenWelcomeModal": true, "lastSeenVersion": null },
        "autoSave": { "enabled": false, "interval": 1000 },
        "cache": { "sslSession": { "enabled": false } },
        "ai": {
            "enabled": false,
            "providers": {
                "openai": { "enabled": false },
                "anthropic": { "enabled": false },
                "github-copilot": { "enabled": false }
            },
            "models": {},
            "defaultModel": "",
            "autocomplete": { "enabled": true, "model": "", "triggerMode": "debounced" },
            "security": {
                "redactHeaders": true,
                "redactBody": true,
                "redactVariables": true,
                "redactResponse": true,
                "customRedactedHeaders": [],
                "customRedactedVariables": []
            }
        },
        "proxy": { "enabled": false, "protocol": "http", "host": "", "port": 8080 },
        "display": { "theme": "dark" }
    })
}

/// 递归合并：saved 覆盖 base；对象逐层合并，数组整体替换
pub fn deep_merge(base: &mut serde_json::Value, saved: &serde_json::Value) {
    match (base, saved) {
        (serde_json::Value::Object(base_map), serde_json::Value::Object(saved_map)) => {
            for (k, v) in saved_map {
                match base_map.get_mut(k) {
                    Some(existing) => deep_merge(existing, v),
                    None => {
                        base_map.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (base_slot, saved_value) => *base_slot = saved_value.clone(),
    }
}

/// Windows：去掉 canonicalize 产生的 `\\?\` / `\\?\UNC\` 前缀
fn strip_verbatim(path: PathBuf) -> PathBuf {
    let raw = path.to_string_lossy().to_string();
    if let Some(rest) = raw.strip_prefix(r"\\?\") {
        if let Some(unc) = rest.strip_prefix(r"UNC\") {
            return PathBuf::from(format!(r"\\{unc}"));
        }
        return PathBuf::from(rest.to_string());
    }
    path
}

fn chrono_stamp() -> String {    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "0".into())
}
