// 发布构建必须启用 custom-protocol（即通过 npm run build:app / tauri build）。
#[cfg(all(not(debug_assertions), not(feature = "custom-protocol")))]
compile_error!(
    "release builds must enable the `custom-protocol` feature; run `npm run build:app`"
);

mod commands;
mod hubs;
mod paths;
mod state;
mod watcher;

use tauri::Manager;


pub fn run() {
    let state = state::AppState::init().unwrap_or_else(|error| {
        eprintln!("[bruno-rs] 数据目录初始化失败：{error}");
        std::process::exit(1);
    });

    eprintln!(
        "[bruno-rs] data_dir={} ({} override={:?})",
        state.layout.root.display(),
        paths::DATA_DIR_ENV,
        std::env::var_os(paths::DATA_DIR_ENV)
    );

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::renderer_ready,
            commands::app_ready,
            commands::get_collection_security_config,
            commands::save_collection_security_config,
            commands::browse_directory,
            commands::open_collection,
            commands::mount_collection,
            commands::add_collection_watcher,
            commands::start_workspace_watcher,
            commands::get_recent_collections,
            commands::load_request,
            commands::save_request,
            commands::send_http_request,
            commands::fetch_oauth2_credentials,
            commands::refresh_oauth2_credentials,
            commands::load_workspace_collections,
            commands::add_collection_to_workspace,
            commands::mount_workspace_scratch,
            commands::open_multiple_collections,
            commands::cancel_http_request,
            commands::convert_postman_to_bruno,
            commands::import_collection,
            commands::app_open_external,
            // 文件系统 CRUD / 集合 / 导出
            commands::fsops::new_request,
            commands::fsops::new_folder,
            commands::fsops::delete_item,
            commands::fsops::move_item,
            commands::fsops::rename_item_name,
            commands::fsops::rename_item_filename,
            commands::fsops::resequence_items,
            commands::fsops::create_collection,
            commands::fsops::clone_collection,
            commands::fsops::clone_folder,
            commands::fsops::rename_collection,
            commands::fsops::remove_collection,
            commands::fsops::remove_collection_from_workspace,
            commands::fsops::exists_sync,
            commands::fsops::resolve_path,
            commands::fsops::save_collection_root,
            commands::fsops::update_bruno_config,
            commands::fsops::get_collection_workspaces,
            commands::fsops::set_collection_workspace,
            commands::fsops::ensure_collections_folder,
            commands::fsops::is_bruno_collection_zip,
            commands::fsops::import_collection_zip,
            commands::fsops::save_folder_root,
            commands::fsops::save_file,
            commands::fsops::export_collection_zip,
            commands::fsops::export_collection_yaml,
            commands::fsops::export_collection_postman,
            commands::fsops::export_environment,
            commands::fsops::browse_directories,
            commands::fsops::browse_files,
            commands::fsops::is_directory,
            commands::fsops::show_in_folder,
            // 环境
            commands::env::create_environment,
            commands::env::save_environment,
            commands::env::delete_environment,
            commands::env::rename_environment,
            commands::env::update_environment_color,
            commands::env::save_environment_extends,
            commands::env::load_workspace_environments,
            commands::env::create_workspace_environment,
            commands::env::update_workspace_environment,
            commands::env::rename_workspace_environment,
            commands::env::delete_workspace_environment,
            commands::env::copy_workspace_environment,
            commands::env::export_workspace,
            commands::env::get_global_environments,
            commands::cookie::add_cookie,
            commands::cookie::modify_cookie,
            commands::cookie::delete_cookie,
            commands::cookie::delete_cookies_for_domain,
            commands::cookie::get_parsed_cookie,
            commands::cookie::create_cookie_string,
            commands::env::create_global_environment,
            commands::env::save_global_environment,
            commands::env::delete_global_environment,
            commands::env::rename_global_environment,
            commands::env::select_global_environment,
            commands::env::save_global_environment_extends,
            commands::env::update_global_environment_color,
            // 偏好 / 快照 / 窗口
            commands::prefs::get_preferences,
            commands::prefs::save_preferences,
            commands::prefs::snapshot_get,
            commands::prefs::snapshot_save,
            commands::prefs::snapshot_clear,
            commands::prefs::snapshot_get_tabs,
            commands::prefs::snapshot_get_collection,
            commands::prefs::snapshot_get_sidebar,
            commands::prefs::snapshot_update_ui_state,
            commands::prefs::theme_change,
            commands::prefs::get_system_proxy_variables,
            commands::prefs::refresh_system_proxy,
            commands::prefs::refresh_pac_cache,
            commands::prefs::clear_http_https_agent_cache,
            commands::prefs::window_minimize,
            commands::prefs::window_maximize,
            commands::prefs::window_is_maximized,
            commands::prefs::window_is_fullscreen,
            commands::prefs::window_close,
            commands::prefs::toggle_fullscreen,
            commands::prefs::toggle_devtools,
            commands::prefs::set_zoom_level,
            commands::prefs::refresh_screen_width,
            // Runner
            commands::run::run_collection_folder,
            // 工作区
            commands::workspace::create_workspace,
            commands::workspace::open_workspace,
            commands::workspace::open_workspace_dialog,
            commands::workspace::rename_workspace,
            commands::workspace::close_workspace,
            commands::workspace::save_workspace_docs,
            commands::workspace::import_workspace,
            commands::workspace::import_workspace_environment,
            commands::workspace::load_last_opened_workspaces,
            commands::workspace::find_unique_folder_name,
            // Git
            commands::git::git_version,
            commands::git::clone_git_repository,
            commands::git::list_remote_branches_for_url,
            commands::git::get_collection_git_remote_url,
            commands::git::connect_collection_to_git,
            commands::git::disconnect_collection_from_git,
            // WebSocket
            commands::ws::ws_start_connection,
            commands::ws::ws_send_message,
            commands::ws::ws_queue_message,
            commands::ws::ws_close_connection,
            commands::ws::ws_connection_status,
            commands::ws::ws_is_connection_active,
            // gRPC
            commands::grpc::grpc_start_connection,
            commands::grpc::grpc_send_message,
            commands::grpc::grpc_cancel,
            commands::grpc::grpc_end,
            commands::grpc::grpc_cancel_request,
            commands::grpc::grpc_end_request,
            commands::grpc::grpc_is_connection_active,
            commands::grpc::grpc_load_methods_proto,
            commands::grpc::grpc_load_methods_reflection,
            commands::grpc::grpc_generate_sample_message,
            commands::grpc::grpc_generate_grpcurl,
            commands::grpc::grpc_has_method,
        ])
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_title("Bruno RS");
            }
            // 命令行传入集合路径时保存到 state，等 renderer:ready 时回推给前端
            let args: Vec<String> = std::env::args().skip(1).collect();
            let startup = args.into_iter().find(|a| !a.starts_with('-') && std::path::Path::new(a).exists());
            if let Some(path) = startup {
                if let Some(state) = app.try_state::<state::AppState>() {
                    *state.startup_path.lock().unwrap() = Some(path);
                }
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build bruno-rs")
        .run(|_app, _event| {});
}
