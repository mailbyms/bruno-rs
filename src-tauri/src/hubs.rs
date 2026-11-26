//! WS / gRPC 连接注册表。
//!
//! 连接对象由 `bruno-core` 的 `ws` / `grpc` 模块创建，这里只负责：
//! 1. 把 AppHandle 包成事件出口；
//! 2. 记住 requestId → 连接，供 `*-cancel` / `*-end` / `*-status` 使用；
//! 3. 连接结束时从表里摘掉，并推 `connections-changed`。
//!
//! **重连竞态**：同一个 requestId 再次 start 时（前端 URL 变化、重跑请求）会先关掉旧连接
//! 再插入新连接；旧任务结束时它的 `on_exit` 也会跑，如果无条件 `remove(&id)` 会把**新**
//! 连接从表里删掉（表现：连上了但发不出消息）。所以每个连接带一个自增代号，
//! `on_exit` 只有代号仍然匹配时才摘除。

use bruno_core::grpc::{self, GrpcCall, GrpcSpec};
use bruno_core::ws::{self, WsConnection, WsSpec};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use tauri::{AppHandle, Emitter};

fn emitter(app: AppHandle) -> ws::Emit {
    Arc::new(move |event: &'static str, payload: Value| {
        let _ = app.emit(event, payload);
    })
}

/// `connections-changed` 的 payload 必须是「只含一个元素的数组」（元素是 id 数组）：
/// 前端 shim 会对数组做 `handler(...payload)` 展开，直接发 id 数组会变成多个实参。
fn emit_ids(app: &AppHandle, event: &str, ids: Vec<String>) {
    let _ = app.emit(event, json!([ids]));
}

// ---------------------------------------------------------------------------
// WebSocket
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct WsHub {
    conns: Mutex<HashMap<String, (u64, WsConnection)>>,
    generation: AtomicU64,
}

impl WsHub {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(self: &Arc<Self>, app: AppHandle, spec: WsSpec) -> Result<(), String> {
        let request_id = spec.request_id.clone();
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;

        // 同一 requestId 的旧连接先断：否则会同时存在两个 socket，消息谁收到说不清
        let previous = {
            let mut map = self.conns.lock().unwrap();
            map.remove(&request_id).map(|(_, conn)| conn)
        };
        if let Some(conn) = previous {
            conn.close();
        }

        let hub: Weak<WsHub> = Arc::downgrade(self);
        let app2 = app.clone();
        let rid = request_id.clone();
        let on_exit: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            if let Some(hub) = hub.upgrade() {
                let ids = {
                    let mut map = hub.conns.lock().unwrap();
                    // 只有当表里仍是本代连接时才摘除，避免误删重连后的新连接
                    if map.get(&rid).map(|(gen, _)| *gen) == Some(generation) {
                        map.remove(&rid);
                    }
                    map.keys().cloned().collect::<Vec<_>>()
                };
                emit_ids(&app2, "main:ws:connections-changed", ids);
            }
        });

        let conn = ws::spawn(spec, emitter(app.clone()), on_exit);
        {
            let mut map = self.conns.lock().unwrap();
            map.insert(request_id.clone(), (generation, conn));
        }
        let ids = self.conns.lock().unwrap().keys().cloned().collect::<Vec<_>>();
        emit_ids(&app, "main:ws:connections-changed", ids);
        Ok(())
    }

    pub fn with<T>(&self, request_id: &str, f: impl FnOnce(&WsConnection) -> T) -> Option<T> {
        let map = self.conns.lock().unwrap();
        map.get(request_id).map(|(_, conn)| f(conn))
    }
}

// ---------------------------------------------------------------------------
// gRPC
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct GrpcHub {
    calls: Mutex<HashMap<String, (u64, GrpcCall)>>,
    generation: AtomicU64,
}

impl GrpcHub {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(self: &Arc<Self>, app: AppHandle, spec: GrpcSpec) -> Result<(), String> {
        let request_id = spec.request_id.clone();
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;

        // 重跑同一请求：先 abort 旧调用，避免两路响应混进同一个时间线
        let previous = {
            let mut map = self.calls.lock().unwrap();
            map.remove(&request_id).map(|(_, call)| call)
        };
        if let Some(call) = previous {
            call.cancel();
        }

        let hub: Weak<GrpcHub> = Arc::downgrade(self);
        let app2 = app.clone();
        let rid = request_id.clone();
        let on_exit: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            if let Some(hub) = hub.upgrade() {
                let ids = {
                    let mut map = hub.calls.lock().unwrap();
                    if map.get(&rid).map(|(gen, _)| *gen) == Some(generation) {
                        map.remove(&rid);
                    }
                    map.keys().cloned().collect::<Vec<_>>()
                };
                emit_ids(&app2, "grpc:connections-changed", ids);
            }
        });

        let call = grpc::spawn(spec, emitter(app.clone()), on_exit);
        {
            let mut map = self.calls.lock().unwrap();
            map.insert(request_id.clone(), (generation, call));
        }
        let ids = self.calls.lock().unwrap().keys().cloned().collect::<Vec<_>>();
        emit_ids(&app, "grpc:connections-changed", ids);
        Ok(())
    }

    pub fn with<T>(&self, request_id: &str, f: impl FnOnce(&GrpcCall) -> T) -> Option<T> {
        let map = self.calls.lock().unwrap();
        map.get(request_id).map(|(_, call)| f(call))
    }
}
