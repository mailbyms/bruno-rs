/**
 * Bruno RS IPC shim.
 * 在 bruno-app 前端代码之前加载，提供与原版 Electron preload 完全一致的
 * window.ipcRenderer 契约：invoke / send / on（返回 unsubscribe）/ getFilePath / openExternal。
 * 通道名维持原版不变，由本文件映射到 Tauri commands 与事件。
 */

import { invoke as tauriInvoke, Channel } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

const listenFn = listen;

// 通道名 → Tauri command 名 + 参数转换
const COMMAND_MAP = {
  'renderer:ready':        { cmd: 'renderer_ready', args: () => ({}) },
  'renderer:app-ready':    { cmd: 'app_ready', args: () => ({}) },
  'renderer:get-collection-security-config': {
    cmd: 'get_collection_security_config', args: (a) => ({ pathname: a[0] })
  },
  'renderer:save-collection-security-config': {
    cmd: 'save_collection_security_config',
    args: (a) => ({ pathname: a[0], securityConfig: a[1] || {} })
  },
  'renderer:browse-directory': { cmd: 'browse_directory', args: () => ({}) },
  'renderer:open-workspace': { cmd: 'open_workspace', args: (a) => ({ pathname: a[0] }) },
  'renderer:open-workspace-dialog': { cmd: 'open_workspace_dialog', args: () => ({}) },
  'renderer:create-workspace': {
    cmd: 'create_workspace',
    args: (a) => ({ workspaceName: a[0], workspaceFolderName: a[1], workspaceLocation: a[2] })
  },
  'renderer:rename-workspace': {
    cmd: 'rename_workspace',
    args: (a) => ({ workspacePath: a[0], name: a[1] })
  },
  'renderer:close-workspace': { cmd: 'close_workspace', args: (a) => ({ workspacePath: a[0] }) },
  'renderer:save-workspace-docs': {
    cmd: 'save_workspace_docs',
    args: (a) => ({ workspacePath: a[0], docs: a[1] || '' })
  },
  'renderer:import-workspace': {
    cmd: 'import_workspace',
    args: (a) => ({ zipFilePath: a[0], extractLocation: a[1] })
  },
  'renderer:import-workspace-environment': {
    cmd: 'import_workspace_environment',
    args: (a) => ({ workspacePath: a[0], environmentData: a[1] || {} })
  },
  'renderer:get-last-opened-workspaces': { cmd: 'load_last_opened_workspaces', args: () => ({}) },
  'renderer:find-unique-folder-name': {
    cmd: 'find_unique_folder_name',
    args: (a) => ({ baseName: a[0], location: a[1] })
  },
  'renderer:mount-collection': {
    cmd: 'mount_collection',
    args: (a) => ({
      collectionUid: a[0]?.collectionUid || '',
      collectionPathname: a[0]?.collectionPathname || ''
    })
  },
  'renderer:mount-collection-v2': {
    cmd: 'mount_collection',
    args: (a) => ({
      collectionUid: a[0]?.collectionUid || '',
      collectionPathname: a[0]?.collectionPathname || ''
    })
  },
  'renderer:load-request': { cmd: 'load_request', args: (a) => ({ pathname: a[0] }) },
  'renderer:add-collection-watcher': {
    cmd: 'add_collection_watcher',
    args: (a) => ({ payload: a[0] || {} })
  },
  'renderer:start-workspace-watcher': {
    cmd: 'start_workspace_watcher',
    args: (a) => ({ workspacePath: a[0] ?? null })
  },
  'renderer:save-request': { cmd: 'save_request', args: (a) => ({ pathname: a[0], item: a[1] }) },
  'renderer:fetch-oauth2-credentials': {
    cmd: 'fetch_oauth2_credentials', args: (a) => ({ payload: a[0] || {} })
  },
  'renderer:refresh-oauth2-credentials': {
    cmd: 'refresh_oauth2_credentials', args: (a) => ({ payload: a[0] || {} })
  },
  'send-http-request': {
    cmd: 'send_http_request',
    args: (a) => ({ input: { item: a[0], collection: a[1], environment: a[2], variables: a[3] } })
  },
  'cancel-http-request': { cmd: 'cancel_http_request', args: (a) => ({ cancelTokenUid: a[0] }) },
  'renderer:get-recent-collections': { cmd: 'get_recent_collections', args: () => ({}) },
  'renderer:app-info': { cmd: 'app_info', args: () => ({}) },
  'renderer:open-collection': { cmd: 'open_collection', args: (a) => ({ pathname: a[0] }) },
  'renderer:app-open-external': { cmd: 'app_open_external', args: (a) => ({ url: a[0] }) },
  'renderer:load-workspace-collections': {
    cmd: 'load_workspace_collections', args: (a) => ({ workspacePath: a[0] })
  },
  'renderer:open-multiple-collections': {
    cmd: 'open_multiple_collections',
    args: (a) => ({ collectionPaths: a[0] || [], options: a[1] || null })
  },
  'renderer:add-collection-to-workspace': {
    cmd: 'add_collection_to_workspace',
    args: (a) => ({ workspacePath: a[0] || '', collection: a[1] || {} })
  },
  'renderer:mount-workspace-scratch': {
    cmd: 'mount_workspace_scratch',
    args: (a) => ({ workspaceUid: a[0]?.workspaceUid || '', workspacePath: a[0]?.workspacePath || '' })
  },
  'renderer:convert-postman-to-bruno': {
    cmd: 'convert_postman_to_bruno',
    args: (a) => ({ collection: a[0], options: a[1] || null })
  },
  'renderer:import-collection': {
    cmd: 'import_collection',
    args: (a) => ({
      collection: a[0],
      collectionLocation: a[1],
      options: a[2] || null
    })
  },

  // ---- 请求 / 文件夹 CRUD ----
  'renderer:new-request': {
    cmd: 'new_request',
    args: (a) => ({ pathname: a[0], item: a[1] })
  },
  'renderer:new-folder': {
    cmd: 'new_folder',
    args: (a) => ({ pathname: a[0]?.pathname, folderData: a[0]?.folderData, format: a[0]?.format ?? null })
  },
  'renderer:delete-item': {
    cmd: 'delete_item',
    args: (a) => ({ pathname: a[0], kind: a[1], collectionPathname: a[2] ?? null })
  },
  'renderer:move-item': {
    cmd: 'move_item',
    args: (a) => ({ targetDirname: a[0]?.targetDirname, sourcePathname: a[0]?.sourcePathname })
  },
  'renderer:rename-item-name': {
    cmd: 'rename_item_name',
    args: (a) => ({
      itemPath: a[0]?.itemPath,
      newName: a[0]?.newName,
      collectionPathname: a[0]?.collectionPathname ?? null
    })
  },
  'renderer:rename-item-filename': {
    cmd: 'rename_item_filename',
    args: (a) => ({
      oldPath: a[0]?.oldPath,
      newPath: a[0]?.newPath,
      newName: a[0]?.newName ?? null,
      newFilename: a[0]?.newFilename ?? null,
      collectionPathname: a[0]?.collectionPathname ?? null
    })
  },
  'renderer:resequence-items': {
    cmd: 'resequence_items',
    args: (a) => ({ items: a[0] || [], collectionPathname: a[1] })
  },
  'renderer:save-collection-root': {
    cmd: 'save_collection_root',
    args: (a) => ({ pathname: a[0], root: a[1], brunoConfig: a[2] ?? null })
  },
  'renderer:update-bruno-config': {
    cmd: 'update_bruno_config',
    args: (a) => ({ brunoConfig: a[0] || {}, pathname: a[1], root: a[2] ?? null })
  },
  'renderer:get-collection-workspaces': {
    cmd: 'get_collection_workspaces',
    args: (a) => ({ collectionPathname: a[0] })
  },
  'renderer:set-collection-workspace': {
    cmd: 'set_collection_workspace',
    args: (a) => ({ collectionUid: a[0], workspacePathname: a[1] ?? null })
  },
  'renderer:ensure-collections-folder': {
    cmd: 'ensure_collections_folder',
    args: (a) => ({ workspacePath: a[0] })
  },
  'renderer:is-bruno-collection-zip': {
    cmd: 'is_bruno_collection_zip',
    args: (a) => ({ pathname: a[0] })
  },
  'renderer:import-collection-zip': {
    cmd: 'import_collection_zip',
    args: (a) => ({ zipPath: a[0], collectionLocation: a[1] })
  },
  // 本实现不区分「大请求按需加载」：文件在挂载时已全部读入，这里直接返回请求内容
  'renderer:load-large-request': { cmd: 'load_request', args: (a) => ({ pathname: a[0]?.pathname }) },
  'renderer:load-request-via-worker': { cmd: 'load_request', args: (a) => ({ pathname: a[0]?.pathname }) },
  'renderer:save-folder-root': {
    cmd: 'save_folder_root',
    args: (a) => ({ folderData: a[0] })
  },
  'renderer:save-file': { cmd: 'save_file', args: (a) => ({ pathname: a[0], content: a[1] }) },
  'renderer:create-collection': {
    cmd: 'create_collection',
    args: (a) => ({
      collectionName: a[0],
      collectionFolderName: a[1],
      collectionLocation: a[2],
      options: a[3] ?? null
    })
  },
  'renderer:clone-collection': {
    cmd: 'clone_collection',
    args: (a) => ({
      collectionName: a[0],
      collectionFolderName: a[1],
      collectionLocation: a[2],
      previousPath: a[3]
    })
  },
  'renderer:clone-folder': {
    cmd: 'clone_folder',
    args: (a) => ({ item: a[0], targetPath: a[1], collectionPathname: a[2] })
  },
  'renderer:rename-collection': {
    cmd: 'rename_collection',
    args: (a) => ({ newName: a[0], collectionPathname: a[1] })
  },
  'renderer:remove-collection': {
    cmd: 'remove_collection',
    args: (a) => ({
      collectionPathname: a[0],
      collectionUid: a[1] ?? null,
      workspaceId: a[2] ?? null
    })
  },

  // ---- 导出 / 文件对话框 ----
  'renderer:export-collection-zip': {
    cmd: 'export_collection_zip',
    args: (a) => ({ pathname: a[0], name: a[1], targetPath: a[2] ?? null })
  },
  'renderer:export-collection-yaml': {
    cmd: 'export_collection_yaml',
    args: (a) => ({ name: a[0], content: a[1], targetPath: a[2] ?? null })
  },
  'renderer:export-collection-postman': {
    cmd: 'export_collection_postman',
    args: (a) => ({ location: a[0], fileName: a[1], content: a[2], overwrite: a[3] ?? false })
  },
  'renderer:export-environment': {
    cmd: 'export_environment',
    args: (a) => ({ payload: a[0] || {} })
  },
  'renderer:export-workspace': {
    cmd: 'export_workspace',
    args: (a) => ({ workspacePath: a[0] ?? null, workspaceName: a[1] })
  },
  'renderer:browse-directories': { cmd: 'browse_directories', args: () => ({}) },
  'renderer:browse-files': {
    cmd: 'browse_files',
    args: (a) => ({ filters: a[0]?.filters ?? a[0] ?? null })
  },
  'renderer:is-directory': { cmd: 'is_directory', args: (a) => ({ pathname: a[0] }) },
  'renderer:show-in-folder': { cmd: 'show_in_folder', args: (a) => ({ pathname: a[0] }) },

  // ---- 环境变量 ----
  'renderer:create-environment': {
    cmd: 'create_environment',
    args: (a) => ({
      collectionPathname: a[0],
      name: a[1],
      variables: a[2] ?? null,
      color: a[3] ?? null,
      inheritedEnvironmentName: a[4] ?? null
    })
  },
  'renderer:save-environment': {
    cmd: 'save_environment',
    args: (a) => ({ collectionPathname: a[0], environment: a[1] })
  },
  'renderer:delete-environment': {
    cmd: 'delete_environment',
    args: (a) => ({ collectionPathname: a[0], environmentName: a[1] })
  },
  'renderer:rename-environment': {
    cmd: 'rename_environment',
    args: (a) => ({ collectionPathname: a[0], oldName: a[1], newName: a[2] })
  },
  'renderer:update-environment-color': {
    cmd: 'update_environment_color',
    args: (a) => ({ collectionPathname: a[0], environmentName: a[1], color: a[2] })
  },
  'renderer:save-environment-extends': {
    cmd: 'save_environment_extends',
    args: (a) => ({
      collectionPathname: a[0],
      environmentName: a[1],
      inheritedEnvironmentName: a[2] ?? null
    })
  },
  'renderer:load-workspace-environments': {
    cmd: 'load_workspace_environments',
    args: (a) => ({ workspacePath: a[0] ?? null })
  },
  'renderer:create-workspace-environment': {
    cmd: 'create_workspace_environment',
    args: (a) => ({ workspacePath: a[0] ?? null, environmentName: a[1] })
  },
  'renderer:update-workspace-environment': {
    cmd: 'update_workspace_environment',
    args: (a) => ({ workspacePath: a[0] ?? null, environmentUid: a[1], environmentData: a[2] })
  },
  'renderer:rename-workspace-environment': {
    cmd: 'rename_workspace_environment',
    args: (a) => ({ workspacePath: a[0] ?? null, environmentUid: a[1], newName: a[2] })
  },
  'renderer:delete-workspace-environment': {
    cmd: 'delete_workspace_environment',
    args: (a) => ({ workspacePath: a[0] ?? null, environmentUid: a[1] })
  },
  'renderer:copy-workspace-environment': {
    cmd: 'copy_workspace_environment',
    args: (a) => ({ workspacePath: a[0] ?? null, environmentUid: a[1], newName: a[2] })
  },
  'renderer:get-global-environments': { cmd: 'get_global_environments', args: () => ({}) },
  'renderer:create-global-environment': {
    cmd: 'create_global_environment',
    args: (a) => ({ payload: a[0] || {} })
  },
  'renderer:save-global-environment': {
    cmd: 'save_global_environment',
    args: (a) => ({ payload: a[0] || {} })
  },
  'renderer:delete-global-environment': {
    cmd: 'delete_global_environment',
    args: (a) => ({ environmentUid: a[0]?.environmentUid })
  },
  'renderer:rename-global-environment': {
    cmd: 'rename_global_environment',
    args: (a) => ({ environmentUid: a[0]?.environmentUid, newName: a[0]?.name })
  },
  'renderer:select-global-environment': {
    cmd: 'select_global_environment',
    args: (a) => ({ environmentUid: a[0]?.environmentUid ?? null })
  },
  'renderer:save-global-environment-extends': {
    cmd: 'save_global_environment_extends',
    args: (a) => ({
      environmentUid: a[0]?.environmentUid,
      extends: a[0]?.extends ?? null,
      workspaceUid: a[0]?.workspaceUid ?? null,
      workspacePath: a[0]?.workspacePath ?? null
    })
  },
  'renderer:update-global-environment-color': {
    cmd: 'update_global_environment_color',
    args: (a) => ({
      environmentUid: a[0]?.environmentUid,
      color: a[0]?.color ?? null,
      workspaceUid: a[0]?.workspaceUid ?? null,
      workspacePath: a[0]?.workspacePath ?? null
    })
  },

  // ---- 文件系统辅助 ----
  'renderer:exists-sync': { cmd: 'exists_sync', args: (a) => ({ pathname: a[0] ?? '' }) },
  'renderer:resolve-path': {
    cmd: 'resolve_path',
    args: (a) => ({ relativePath: a[0] ?? '', basePath: a[1] ?? null })
  },
  'renderer:remove-collection-from-workspace': {
    cmd: 'remove_collection_from_workspace',
    args: (a) => ({
      workspaceUid: a[0] ?? null,
      workspacePath: a[1] ?? null,
      collectionPath: a[2] ?? '',
      options: a[3] || {}
    })
  },

  // ---- Cookies 面板 ----
  'renderer:add-cookie': {
    cmd: 'add_cookie',
    args: (a) => ({ domain: a[0] ?? '', cookie: a[1] || {} })
  },
  'renderer:modify-cookie': {
    cmd: 'modify_cookie',
    args: (a) => ({ domain: a[0] ?? '', oldCookie: a[1] || {}, cookie: a[2] || {} })
  },
  'renderer:delete-cookie': {
    cmd: 'delete_cookie',
    args: (a) => ({ domain: a[0] ?? '', path: a[1] ?? '/', cookieKey: a[2] ?? '' })
  },
  'renderer:delete-cookies-for-domain': {
    cmd: 'delete_cookies_for_domain',
    args: (a) => ({ domain: a[0] ?? '' })
  },
  'renderer:get-parsed-cookie': {
    cmd: 'get_parsed_cookie',
    args: (a) => ({ cookieString: a[0] ?? '' })
  },
  'renderer:create-cookie-string': {
    cmd: 'create_cookie_string',
    args: (a) => ({ cookie: a[0] || {} })
  },

  // ---- 偏好 / 快照 / 窗口 ----
  'renderer:get-preferences': { cmd: 'get_preferences', args: () => ({}) },
  'renderer:save-preferences': { cmd: 'save_preferences', args: (a) => ({ preferences: a[0] || {} }) },
  'renderer:snapshot:get': { cmd: 'snapshot_get', args: () => ({}) },
  'renderer:snapshot:save': { cmd: 'snapshot_save', args: (a) => ({ snapshot: a[0] }) },
  'renderer:snapshot:clear': { cmd: 'snapshot_clear', args: () => ({}) },
  'renderer:snapshot:get-tabs': {
    cmd: 'snapshot_get_tabs',
    args: (a) => ({ collectionPathname: a[0], workspacePathname: a[1] ?? null })
  },
  'renderer:snapshot:get-collection': {
    cmd: 'snapshot_get_collection',
    args: (a) => ({ collectionPathname: a[0], workspacePathname: a[1] ?? null })
  },
  'renderer:snapshot:get-sidebar': { cmd: 'snapshot_get_sidebar', args: () => ({}) },
  'renderer:update-ui-state-snapshot': {
    cmd: 'snapshot_update_ui_state',
    args: (a) => ({ payload: a[0] || {} })
  },
  'renderer:theme-change': { cmd: 'theme_change', args: (a) => ({ theme: a[0] ?? null, background: a[1] ?? null }) },
  'renderer:get-system-proxy-variables': { cmd: 'get_system_proxy_variables', args: () => ({}) },
  'renderer:refresh-system-proxy': { cmd: 'refresh_system_proxy', args: () => ({}) },
  'renderer:refresh-pac-cache': { cmd: 'refresh_pac_cache', args: () => ({}) },
  'renderer:clear-http-https-agent-cache': { cmd: 'clear_http_https_agent_cache', args: () => ({}) },
  'renderer:window-minimize': { cmd: 'window_minimize', args: () => ({}) },
  'renderer:window-maximize': { cmd: 'window_maximize', args: () => ({}) },
  'renderer:window-is-maximized': { cmd: 'window_is_maximized', args: () => ({}) },
  'renderer:window-is-fullscreen': { cmd: 'window_is_fullscreen', args: () => ({}) },
  'renderer:window-close': { cmd: 'window_close', args: () => ({}) },
  'renderer:toggle-fullscreen': { cmd: 'toggle_fullscreen', args: () => ({}) },
  'renderer:toggle-devtools': { cmd: 'toggle_devtools', args: () => ({}) },
  'renderer:set-zoom-level': { cmd: 'set_zoom_level', args: (a) => ({ level: a[0] }) },
  'renderer:refresh-screen-width': { cmd: 'refresh_screen_width', args: () => ({}) },

  // ---- Runner ----
  'renderer:run-collection-folder': {
    cmd: 'run_collection_folder',
    args: (a) => ({
      folder: a[0] ?? null,
      collection: a[1],
      environment: a[2] ?? null,
      runtimeVariables: a[3] ?? null,
      recursive: a[4] ?? false,
      delay: a[5] ?? 0,
      tags: a[6] ?? null,
      selectedRequestUids: a[7] ?? null
    })
  },

  // ---- Git ----
  'renderer:clone-git-repository': {
    cmd: 'clone_git_repository',
    args: (a) => ({
      url: a[0]?.url,
      path: a[0]?.path,
      branch: a[0]?.branch ?? null
    })
  },
  'renderer:list-remote-branches-for-url': {
    cmd: 'list_remote_branches_for_url',
    args: (a) => ({ payload: a[0] || {} })
  },
  'renderer:get-collection-git-remote-url': {
    cmd: 'get_collection_git_remote_url',
    args: (a) => ({ pathname: a[0] })
  },
  'renderer:connect-collection-to-git': {
    cmd: 'connect_collection_to_git',
    args: (a) => ({
      workspacePath: a[0] || '',
      collectionPath: a[1],
      remoteUrl: a[2]
    })
  },
  'renderer:disconnect-collection-from-git': {
    cmd: 'disconnect_collection_from_git',
    args: (a) => ({ workspacePath: a[0] || '', collectionPath: a[1] })
  },

  // ---- WebSocket ----
  'renderer:ws:start-connection': { cmd: 'ws_start_connection', args: (a) => ({ payload: a[0] || {} }) },
  'renderer:ws:queue-message': { cmd: 'ws_queue_message', args: (a) => ({ payload: a[0] || {} }) },
  'renderer:ws:send-message': {
    cmd: 'ws_send_message',
    args: (a) => ({ requestId: a[0], collectionUid: a[1] || '', message: a[2] ?? '' })
  },
  'renderer:ws:close-connection': { cmd: 'ws_close_connection', args: (a) => ({ requestId: a[0] }) },
  'renderer:ws:connection-status': { cmd: 'ws_connection_status', args: (a) => ({ requestId: a[0] }) },
  'renderer:ws:is-connection-active': { cmd: 'ws_is_connection_active', args: (a) => ({ requestId: a[0] }) },

  // ---- gRPC ----
  'grpc:start-connection': { cmd: 'grpc_start_connection', args: (a) => ({ payload: a[0] || {} }) },
  'grpc:send-message': {
    cmd: 'grpc_send_message',
    args: (a) => ({ requestId: a[0], collectionUid: a[1] || '', message: a[2] ?? '' })
  },
  'grpc:cancel': { cmd: 'grpc_cancel', args: (a) => ({ requestId: a[0] }) },
  'grpc:end': { cmd: 'grpc_end', args: (a) => ({ requestId: a[0] }) },
  // 两套入口并存：`grpc:cancel-request` 传 { requestId }
  'grpc:cancel-request': { cmd: 'grpc_cancel_request', args: (a) => ({ requestId: a[0]?.requestId ?? a[0] }) },
  'grpc:end-request': { cmd: 'grpc_end_request', args: (a) => ({ requestId: a[0]?.requestId ?? a[0] }) },
  'grpc:is-connection-active': { cmd: 'grpc_is_connection_active', args: (a) => ({ connectionId: a[0]?.connectionId ?? a[0] }) },
  'grpc:load-methods-proto': { cmd: 'grpc_load_methods_proto', args: (a) => ({ payload: a[0] || {} }) },
  'grpc:load-methods-reflection': { cmd: 'grpc_load_methods_reflection', args: (a) => ({ payload: a[0] || {} }) },
  'grpc:generate-sample-message': { cmd: 'grpc_generate_sample_message', args: (a) => ({ payload: a[0] || {} }) },
  'grpc:generate-grpcurl': { cmd: 'grpc_generate_grpcurl', args: (a) => ({ payload: a[0] || {} }) },
};

// 无需后端实现的通道：返回安全默认值，保证前端流程不中断
const PASS_THROUGH = [];

// 需要「空但类型正确」返回值的通道：返回 null 会让上游 .filter/.map 崩掉
const EMPTY_RESPONSES = {
  'renderer:load-unopenable-workspace-collections': [],
  'renderer:load-workspace-api-specs': [],
  'renderer:load-workspace-apispecs': [],
};

const listeners = new Map();

async function invoke(channel, ...args) {
  const mapping = COMMAND_MAP[channel];
  if (mapping) {
    const payload = mapping.args(args);
    // Tauri 2 command 单参数对象；无参数时也必须传 {}
    return tauriInvoke(mapping.cmd, payload);
  }
  if (Object.prototype.hasOwnProperty.call(EMPTY_RESPONSES, channel)) {
    return EMPTY_RESPONSES[channel];
  }
  // Mock Server 里程碑 1 未实现：返回成功形状，避免上游读 result.success 崩掉
  if (channel.startsWith('renderer:mock-server-')) {
    if (channel === 'renderer:mock-server-get-running') return [];
    return { success: true, instances: [], responses: [], mockServers: [] };
  }
  if (PASS_THROUGH.some((p) => channel === p || channel.startsWith(p))) {
    if (channel.includes('snapshot')) return null;
    return undefined;
  }
  // 未实现通道：静默 null，避免 UI 报错（里程碑 1 范围外功能）
  if (window.__BRUNO_RS_DEBUG__) console.warn('[bruno-rs] unhandled invoke:', channel, args);
  return null;
}

function send(channel, ...args) {
  if (window.__BRUNO_RS_DEBUG__) console.debug('[bruno-rs] send:', channel, args);
  // send 通道里有需要持久化的副作用（如主题背景色），能映射就照常转发
  const mapping = COMMAND_MAP[channel];
  if (mapping) {
    tauriInvoke(mapping.cmd, mapping.args(args)).catch(() => {});
  }
}

function on(channel, handler) {
  if (!listeners.has(channel)) listeners.set(channel, new Set());
  listeners.get(channel).add(handler);
  return () => {
    const set = listeners.get(channel);
    if (set) set.delete(handler);
  };
}

// 建立 Tauri 事件 → 前端 handler 的桥；Tauri 事件名与 Bruno 通道名一致
window.__EVLOG__ = window.__EVLOG__ || [];
window.__SHIM_HANDLERS__ = listeners;

function registerBridge(eventName) {
  listenFn(eventName, (event) => {
    window.__EVLOG__.push({ event: eventName, at: Date.now(), handlerCount: listeners.get(eventName)?.size || 0 });
    const set = listeners.get(eventName);
    if (!set) return;
    const payload = event.payload;
    // Bruno 的 on 回调签名：(...args)，而 emit 的 payload 可能是数组或单值
    if (Array.isArray(payload)) {
      for (const h of set) h(...payload);
    } else {
      for (const h of set) h(payload);
    }
  }).catch((e) => { window.__EVLOG__.push({ event: eventName, error: String(e) }); console.error('[bruno-rs] listen failed', eventName, e); });
}

const BRIDGED_EVENTS = [
  'main:load-preferences', 'main:load-global-environments', 'main:git-version',
  'main:workspace-opened', 'main:workspaces-ready', 'main:workspace-config-updated',
  'main:collection-opened', 'main:collection-tree-loaded', 'main:collection-tree-updated',
  'main:hydrate-app-with-ui-state-snapshot', 'main:run-request-event',
  'main:collection-loading-state-updated-v2', 'main:bruno-config-update-v2',
  'main:display-error', 'main:app-loaded', 'main:cookies-update',
  'main:startup-open-collection',
  // 集合 Runner 的进度事件流（started/queued/sent/received/results/ended）
  'main:run-folder-event',
  // WebSocket：payload 是 [requestId, collectionUid, eventData]
  'main:ws:connecting', 'main:ws:disconnecting', 'main:ws:upgrade', 'main:ws:redirect',
  'main:ws:request', 'main:ws:open', 'main:ws:message', 'main:ws:close', 'main:ws:error',
  'main:ws:connections-changed',
  // gRPC：同样三元组；connections-changed 是 [ids]
  'grpc:request', 'grpc:message', 'grpc:response', 'grpc:metadata', 'grpc:status',
  'grpc:error', 'grpc:server-end-stream', 'grpc:server-cancel-stream',
  'grpc:connections-changed', 'grpc:script-error', 'grpc:test-results',
];
for (const eventName of BRIDGED_EVENTS) registerBridge(eventName);

// 命令行启动时自动打开集合：把 Rust 事件转成 open_collection 调用
on('main:startup-open-collection', (pathname) => {
  if (pathname) {
    invoke('renderer:open-collection', pathname).catch((e) =>
      console.error('[bruno-rs] auto open collection failed', e)
    );
  }
});

window.ipcRenderer = {
  invoke,
  send,
  on,
  once(channel, handler) {
    const off = on(channel, (...args) => { off(); handler(...args); });
    return off;
  },
  getFilePath(file) { return typeof file?.path === 'string' ? file.path : ''; },
  openExternal(url) { return invoke('renderer:app-open-external', url); },
};

// 供前端 isElectron() 判断（原版检查 window.ipcRenderer）
window.__BRUNO_RS__ = true;
