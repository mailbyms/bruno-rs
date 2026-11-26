//! 集合 Runner：把前端 `renderer:run-collection-folder` 的入参转成核心 Runner 调用。
//!
//! 进度以 `main:run-folder-event` 事件流式回推；每个请求仍走 `execute_request`，
//! 因此 cookie / 脚本 / 断言与单请求完全一致。

use crate::state::AppState;
use bruno_core::runner::CookieHook;
use bruno_core::runner_batch::RunOptions;
use serde_json::Value as J;
use std::sync::atomic::Ordering;
use tauri::{Emitter, Manager, State};

/// 请求级参数：偏好默认值 + cookie hook。
///
/// Cookie 走 hook 而不是预填 `cookie_header`：真实 host 要插值之后才知道，
/// Runner 一次跑多个请求时每个 host 都不同。
pub fn options_for(state: &State<'_, AppState>, _item: &J) -> bruno_core::RequestOptions {
    let mut options = state.request_defaults();
    let jar = state.cookies.clone();
    options.cookie_hook = CookieHook(Some(std::sync::Arc::new(move |url: &str| {
        let mut guard = jar.lock().unwrap();
        let header = guard.header_for(url);
        drop(guard);
        header
    })));
    options
}

#[tauri::command]
pub async fn run_collection_folder(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    folder: Option<J>,
    collection: J,
    environment: Option<J>,
    runtime_variables: Option<J>,
    recursive: Option<bool>,
    delay: Option<u64>,
    tags: Option<J>,
    selected_request_uids: Option<J>,
) -> Result<J, String> {
    state.runner_cancel.store(false, Ordering::SeqCst);
    let tags_list: Vec<String> = tags
        .as_ref()
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|t| t.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let uid_list: Vec<String> = selected_request_uids
        .as_ref()
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|t| t.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let cancel_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancel_watch = cancel_flag.clone();
    let cancel_token = folder
        .as_ref()
        .and_then(|f| f.get("cancelTokenUid"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let _ = cancel_token;

    let options = RunOptions {
        recursive: recursive.unwrap_or(false),
        delay_ms: delay.unwrap_or(0),
        tags: tags_list,
        selected_uids: uid_list,
        cancel_token: Some(cancel_watch),
        // 与单请求同一套偏好/代理/cookie 行为
        defaults: Some(options_for(&state, &J::Null)),
    };

    // 外部取消：轮询 state.runner_cancel，转成核心的 atomic 标志
    let app_handle = app.clone();
    let watcher = tauri::async_runtime::spawn(async move {
        loop {
            {
                // 每秒检查一次，结束条件由调用方结束任务
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            }
            if app_handle
                .try_state::<AppState>()
                .map(|s| s.runner_cancel.load(Ordering::SeqCst))
                .unwrap_or(false)
            {
                cancel_flag.store(true, Ordering::SeqCst);
                break;
            }
        }
    });

    let result = bruno_core::run_folder(
        collection,
        folder,
        environment,
        runtime_variables,
        options,
        |channel, payload| {
            let _ = app.emit(channel, payload);
        },
    )
    .await;

    watcher.abort();
    // 汇总里带上的 runtimeVariables 由前端合并
    Ok(result)
}
