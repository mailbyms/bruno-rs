//! 集合文件监听：磁盘上的 `.yml` 被外部改动后，前端树要能自己刷新。
//!
//! 不引入 `notify` 依赖，用「轮询 + 指纹」实现：
//! 每 700ms 递归算一次目录指纹（文件名 + mtime + 大小），变化后整份重载集合，
//! 再走和写操作同一条 `main:collection-tree-loaded` 通道。
//! uid 按路径稳定生成，所以前端 `mergeTreeItems` 会保住草稿和已打开标签页。

use crate::state::AppState;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tauri::Manager;

/// 这些目录不参与指纹（体积大、且不影响集合结构）
const IGNORED_DIRS: [&str; 4] = ["node_modules", ".git", "dist", "target"];
/// 只有这些扩展名的变化会触发重载
const WATCHED_EXT: [&str; 4] = ["yml", "yaml", "json", "bru"];

fn wanted(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => WATCHED_EXT.contains(&ext.to_ascii_lowercase().as_str()),
        // 无扩展名文件（.env 等）只在名字命中时关注
        None => path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with(".env"))
            .unwrap_or(false),
    }
}

fn walk(dir: &Path, depth: usize, hasher: &mut DefaultHasher) {
    if depth > 8 {
        return;
    }
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = read.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if IGNORED_DIRS.iter().any(|d| name.eq_ignore_ascii_case(d)) {
                continue;
            }
            name.hash(hasher);
            walk(&path, depth + 1, hasher);
            continue;
        }
        if !wanted(&path) {
            continue;
        }
        name.hash(hasher);
        if let Ok(meta) = entry.metadata() {
            meta.len().hash(hasher);
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            mtime.hash(hasher);
        }
    }
}

fn fingerprint(dir: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    walk(dir, 0, &mut hasher);
    hasher.finish()
}

/// 开始监听某个集合目录（重复调用不会起第二个线程）
pub fn start(app: tauri::AppHandle, pathname: &str) {
    // 统一成「规范化的正斜杠路径」：state 里 path_to_uid 的键就是这个形状，
    // 否则 reload_collection 找不到已挂载的集合
    let raw = std::path::PathBuf::from(pathname);
    let canonical = raw.canonicalize().unwrap_or(raw);
    let text = canonical.to_string_lossy().replace('\\', "/");
    let text = text.strip_prefix("//?/").map(|s| s.to_string()).unwrap_or(text);
    let key = text;
    let Some(state) = app.try_state::<AppState>() else { return };
    {
        let mut map = state.watchers.lock().unwrap();
        if let Some(existing) = map.get(&key) {
            // 上一次的线程还活着就复用
            if existing.load(Ordering::SeqCst) {
                return;
            }
        }
        map.insert(key.clone(), Arc::new(AtomicBool::new(true)));
    }
    let flag = state.watchers.lock().unwrap().get(&key).cloned().unwrap();
    let dir = std::path::PathBuf::from(&key);
    if !dir.is_dir() {
        return;
    }

    std::thread::spawn(move || {
        let mut prev = fingerprint(&dir);
        while flag.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(700));
            if !flag.load(Ordering::SeqCst) {
                break;
            }
            let now = fingerprint(&dir);
            if now == prev {
                continue;
            }
            // 写文件可能分几次落盘，等稳定后再重载
            std::thread::sleep(Duration::from_millis(150));
            prev = fingerprint(&dir);
            let Some(state) = app.try_state::<AppState>() else { break };
            match state.reload_collection(&key) {
                Ok(fresh) => crate::commands::emit_tree(&app, &fresh, &key),
                Err(e) => eprintln!("[bruno-rs] watcher 重载失败：{e}"),
            }
        }
    });
}

/// 停止监听（集合被移除时调用）
pub fn stop(state: &AppState, pathname: &str) {
    let key = pathname.replace('\\', "/");
    let flag = state.watchers.lock().unwrap().remove(&key);
    if let Some(flag) = flag {
        flag.store(false, Ordering::SeqCst);
    }
}
