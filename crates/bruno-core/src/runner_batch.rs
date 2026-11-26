//! 集合 Runner：按顺序跑一个文件夹（或整个集合）里的请求，并把进度写成
//! Bruno 前端认识的 `main:run-folder-event` 事件。
//!
//! 与单请求发送共用 `execute_request`，所以脚本、断言、认证行为完全一致。

use crate::runner::{execute_request, SendRequestInput};
use serde_json::{json, Value as J};

#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    pub recursive: bool,
    pub delay_ms: u64,
    /// 只跑带这些 tag 的请求（空 = 不过滤）
    pub tags: Vec<String>,
    /// 只跑这些 uid（空 = 不过滤）
    pub selected_uids: Vec<String>,
    /// 命中后立即停止
    pub cancel_token: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// 每个请求共用的运行参数（超时/代理/cookie hook），None = 只读 item.settings
    pub defaults: Option<crate::runner::RequestOptions>,
}

fn collect_requests<'a>(items: &'a [J], recursive: bool, out: &mut Vec<&'a J>) {
    for item in items {
        let kind = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if kind == "folder" {
            if recursive {
                if let Some(children) = item.get("items").and_then(|v| v.as_array()) {
                    collect_requests(children, recursive, out);
                }
            }
            continue;
        }
        // Runner 只跑 HTTP / GraphQL；WS 与 gRPC 需要长连接，不参与批量执行
        if matches!(kind, "http-request" | "graphql-request") {
            out.push(item);
        }
    }
}

fn tags_of(item: &J) -> Vec<String> {
    item.get("tags")
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|t| t.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
pub async fn run_folder<F>(
    collection: J,
    folder: Option<J>,
    environment: Option<J>,
    runtime_variables: Option<J>,
    options: RunOptions,
    emit: F,
) -> J
where
    F: Fn(&str, J),
{
    let collection_uid = collection
        .get("uid")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let folder_uid = folder
        .as_ref()
        .and_then(|f| f.get("uid"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let root_items: Vec<J> = match &folder {
        Some(f) => f
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default(),
        None => collection
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default(),
    };

    let mut selected: Vec<&J> = Vec::new();
    collect_requests(&root_items, options.recursive, &mut selected);
    let total_found = selected.len();

    // tag / uid 过滤
    selected.retain(|item| {
        let uid = item.get("uid").and_then(|v| v.as_str()).unwrap_or("");
        let uid_ok = options.selected_uids.is_empty()
            || options.selected_uids.iter().any(|u| u == uid);
        let tags = tags_of(item);
        let tag_ok = options.tags.is_empty() || options.tags.iter().any(|t| tags.contains(t));
        uid_ok && tag_ok
    });

    let cancel_token_uid = format!("runner-{}", crate::model::new_uid());
    let started = std::time::Instant::now();

    emit(
        "main:run-folder-event",
        json!({
            "collectionUid": collection_uid,
            "folderUid": folder_uid,
            "type": "testrun-started",
            "isRecursive": options.recursive,
            "cancelTokenUid": cancel_token_uid,
        }),
    );

    let mut runtime_vars: serde_json::Map<String, J> = runtime_variables
        .as_ref()
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    let mut passed = 0usize;
    let mut failed = 0usize;
    // 被 tags/uids 过滤掉的请求计入 skipped
    let mut skipped = total_found.saturating_sub(selected.len());
    let mut cancelled = false;

    let total_selected = selected.len();
    for (index, item) in selected.into_iter().enumerate() {
        if let Some(flag) = &options.cancel_token {
            if flag.load(std::sync::atomic::Ordering::SeqCst) {
                cancelled = true;
                // 取消后没跑的请求也算 skipped
                skipped += total_selected - index;
                break;
            }
        }
        let item_uid = item.get("uid").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let request_uid = crate::model::new_uid();
        emit(
            "main:run-folder-event",
            json!({
                "collectionUid": collection_uid,
                "folderUid": folder_uid,
                "itemUid": item_uid,
                "requestUid": request_uid,
                "type": "request-queued",
                "cancelTokenUid": cancel_token_uid,
            }),
        );

        let input = SendRequestInput {
            item: item.clone(),
            collection: collection.clone(),
            environment: environment.clone(),
            variables: Some(J::Object(runtime_vars.clone())),
            options: options.defaults.clone(),
        };

        let request_sent = json!({
            "url": item.pointer("/request/url").and_then(|v| v.as_str()).unwrap_or(""),
            "method": item.pointer("/request/method").and_then(|v| v.as_str()).unwrap_or("GET"),
        });
        emit(
            "main:run-folder-event",
            json!({
                "collectionUid": collection_uid,
                "folderUid": folder_uid,
                "itemUid": item_uid,
                "requestUid": request_uid,
                "type": "request-sent",
                "requestSent": request_sent,
            }),
        );

        match execute_request(input).await {
            Ok(response) => {
                for (k, v) in &response.runtime_variables {
                    runtime_vars.insert(k.clone(), json!(v));
                }
                emit(
                    "main:run-folder-event",
                    json!({
                        "collectionUid": collection_uid,
                        "folderUid": folder_uid,
                        "itemUid": item_uid,
                        "requestUid": request_uid,
                        "type": "response-received",
                        "responseReceived": response,
                    }),
                );
                emit(
                    "main:run-folder-event",
                    json!({
                        "collectionUid": collection_uid,
                        "folderUid": folder_uid,
                        "itemUid": item_uid,
                        "requestUid": request_uid,
                        "type": "test-results",
                        "testResults": response.test_results,
                    }),
                );
                let assertions = response.assertion_results.clone();
                emit(
                    "main:run-folder-event",
                    json!({
                        "collectionUid": collection_uid,
                        "folderUid": folder_uid,
                        "itemUid": item_uid,
                        "requestUid": request_uid,
                        "type": "assertion-results",
                        "assertionResults": assertions,
                    }),
                );
                if !response.pre_request_test_results.is_empty() {
                    emit(
                        "main:run-folder-event",
                        json!({
                            "collectionUid": collection_uid,
                            "folderUid": folder_uid,
                            "itemUid": item_uid,
                            "requestUid": request_uid,
                            "type": "test-results-pre-request",
                            "preRequestTestResults": response.pre_request_test_results,
                        }),
                    );
                }
                if !response.post_response_test_results.is_empty() {
                    emit(
                        "main:run-folder-event",
                        json!({
                            "collectionUid": collection_uid,
                            "folderUid": folder_uid,
                            "itemUid": item_uid,
                            "requestUid": request_uid,
                            "type": "test-results-post-response",
                            "postResponseTestResults": response.post_response_test_results,
                        }),
                    );
                }
                let tests_failed = response.test_results.iter().any(|t| t.status != "pass")
                    || response.assertion_results.iter().any(|a| !a.passed);
                if tests_failed {
                    failed += 1;
                } else {
                    passed += 1;
                }
            }
            Err(error) => {
                failed += 1;
                emit(
                    "main:run-folder-event",
                    json!({
                        "collectionUid": collection_uid,
                        "folderUid": folder_uid,
                        "itemUid": item_uid,
                        "requestUid": request_uid,
                        "type": "error",
                        "error": format!("{error}"),
                    }),
                );
            }
        }

        if options.delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(options.delay_ms)).await;
        }
    }

    let summary = json!({
        "collectionUid": collection_uid,
        "folderUid": folder_uid,
        "type": "testrun-ended",
        "statusText": if cancelled { "cancelled" } else { "completed" },
        "runCompletionTime": started.elapsed().as_millis() as u64,
        "passed": passed,
        "failed": failed,
        "skipped": skipped,
    });
    emit("main:run-folder-event", summary.clone());

    json!({
        "collectionUid": collection_uid,
        "folderUid": folder_uid,
        "cancelled": cancelled,
        "passed": passed,
        "failed": failed,
        "skipped": skipped,
        "duration": started.elapsed().as_millis() as u64,
    })
}

/// 从集合/文件夹里收集所有请求（供 UI 预统计与单测）
pub fn collect_all_requests(items: &[J], recursive: bool) -> Vec<J> {
    let mut list = Vec::new();
    collect_requests(items, recursive, &mut list);
    list.into_iter().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(name: &str, uid: &str, tags: &[&str]) -> J {
        json!({
            "uid": uid,
            "name": name,
            "type": "http-request",
            "tags": tags,
            "request": { "method": "GET", "url": "http://127.0.0.1:1/x", "headers": [], "params": [], "auth": { "mode": "none" } }
        })
    }

    #[test]
    fn collects_requests_recursively_or_not() {
        // 传入的是「文件夹的 items」：非递归只跑同级请求，递归才进子文件夹
        let items = vec![
            request("A", "r1", &[]),
            json!({
                "uid": "f2", "name": "N", "type": "folder",
                "items": [request("B", "r2", &["smoke"])]
            }),
        ];
        assert_eq!(collect_all_requests(&items, false).len(), 1);
        assert_eq!(collect_all_requests(&items, true).len(), 2);
    }

    #[test]
    fn filters_by_tags_and_uid() {
        let tree = vec![request("A", "r1", &["smoke"]), request("B", "r2", &[])];
        let mut selected: Vec<&J> = Vec::new();
        collect_requests(&tree, true, &mut selected);
        selected.retain(|i| {
            let uid = i["uid"].as_str().unwrap_or("");
            let tags = tags_of(i);
            uid == "r2" && (tags.is_empty() || tags.contains(&"smoke".to_string()))
        });
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0]["name"], "B");
    }
}
