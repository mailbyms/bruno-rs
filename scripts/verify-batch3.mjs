// 第 4 轮：Git 侧栏入口配套、UI 快照恢复、集合设置、zip 导入、工作区 CRUD。
//
// 直接调 `window.ipcRenderer.invoke`，同时覆盖 shim 映射、Rust 落盘与事件形状。
//
// 用法：
//   BRUNO_RS_DATA_DIR=<tmp> WEBVIEW2_USER_DATA_FOLDER=<tmp> \
//     WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9240 \
//     ./target/debug/bruno-rs.exe fixtures/shop
//   node scripts/verify-batch3.mjs
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const PORT = Number(process.env.CDP_PORT || 9240);
const TMP = fs.mkdtempSync(path.join(os.tmpdir(), 'brs-batch3-'));
const COLL = path.resolve('fixtures/shop').replace(/\\/g, '/');

const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
const page = list.find((x) => x.type === 'page' && x.url !== 'about:blank');
if (!page) {
  console.error('找不到页面目标，应用是否已启动并开启 CDP？');
  process.exit(1);
}
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));

let id = 0;
const pending = new Map();
ws.onmessage = (e) => {
  const m = JSON.parse(e.data);
  if (m.id && pending.has(m.id)) {
    pending.get(m.id)(m);
    pending.delete(m.id);
  }
};
const call = (method, params = {}) =>
  new Promise((r) => {
    const i = ++id;
    pending.set(i, r);
    ws.send(JSON.stringify({ id: i, method, params }));
  });
const ev = async (expression) => {
  const j = await call('Runtime.evaluate', {
    expression,
    returnByValue: true,
    awaitPromise: true
  });
  if (j.result?.exceptionDetails) {
    const d = j.result.exceptionDetails;
    throw new Error(d.exception?.description || d.text || 'eval failed');
  }
  return j.result?.result?.value;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const invoke = async (channel, ...args) => {
  const expr = `(async () => {
    try {
      return JSON.stringify({ ok: true, value: await window.ipcRenderer.invoke(${JSON.stringify(channel)}, ...${JSON.stringify(args)}) });
    } catch (e) {
      return JSON.stringify({ ok: false, error: String((e && (e.message || e.error)) || e) });
    }
  })()`;
  const raw = await Promise.race([
    ev(expr),
    new Promise((_, rej) => setTimeout(() => rej(new Error(`[${channel}] 调用超时`)), 60000))
  ]);
  const parsed = JSON.parse(raw);
  if (!parsed.ok) throw new Error(`[${channel}] ${parsed.error}`);
  return parsed.value;
};

let pass = 0;
const failures = [];
const ok = (name, cond, detail = '') => {
  if (cond) {
    pass += 1;
    console.log(`  PASS  ${name}`);
  } else {
    failures.push(`${name}${detail ? ` :: ${detail}` : ''}`);
    console.log(`  FAIL  ${name}${detail ? ` :: ${detail}` : ''}`);
  }
};
const section = (t) => console.log(`\n== ${t} ==`);

await call('Runtime.enable');

for (let i = 0; i < 60; i++) {
  const ready = await ev('!!window.ipcRenderer && !!window.__BRUNO_RS__');
  if (ready) break;
  await sleep(250);
}

await ev(`window.__TREES__ = window.__TREES__ || [];
  if (!window.__WATCHED__) {
    window.__WATCHED__ = 1;
    window.ipcRenderer.on('main:collection-tree-loaded', (t) => window.__TREES__.push(t));
  }
  'ok'`);

const lastTree = async (pathname) => {
  const want = String(pathname || '').replace(/\\/g, '/').toLowerCase();
  const raw = await ev('JSON.stringify((window.__TREES__||[]).slice(-8))');
  const entries = JSON.parse(raw || '[]');
  const hit = [...entries]
    .reverse()
    .find((t) => String(t?.tree?.pathname || '').replace(/\\/g, '/').toLowerCase() === want);
  const t = hit || entries[entries.length - 1];
  if (!t) return null;
  return {
    uid: t.collectionUid,
    pathname: t.tree.pathname,
    name: t.tree.root?.meta?.name || '',
    items: t.tree.items || [],
    root: t.tree.root || {}
  };
};

// ---------------------------------------------------------------------------
section('集合设置（update-bruno-config / security config）');

const rootYml = path.join(COLL, 'opencollection.yml');
ok('集合根存在', fs.existsSync(rootYml), String(COLL));
// 启动时的树事件早于本脚本注册监听，这里直接挂载一次拿集合，不依赖历史事件
const mounted = await invoke('renderer:mount-collection', { collectionUid: 'shop-verify', collectionPathname: COLL });
await sleep(300);
const before = (await lastTree(COLL)) || { uid: 'shop-verify', pathname: mounted, items: [], root: {} };
const originalYml = fs.readFileSync(rootYml, 'utf8');

await invoke('renderer:update-bruno-config', { version: '9.9.9', name: 'Shop API' }, COLL, before?.root || {});
const afterVersion = fs.readFileSync(rootYml, 'utf8');
ok('update-bruno-config 写入版本', afterVersion.includes('9.9.9'), afterVersion.slice(0, 160));
ok('update-bruno-config 保留 opencollection 头', afterVersion.includes('opencollection:'), afterVersion.slice(0, 80));

await invoke('renderer:save-collection-security-config', COLL, { jsSandboxMode: 'developer' });
const savedYml = fs.readFileSync(rootYml, 'utf8');
ok('security config 落盘到 settings.security', /settings:[\s\S]*security:[\s\S]*jsSandboxMode: developer/.test(savedYml), savedYml.slice(0, 240));
const readBack = await invoke('renderer:get-collection-security-config', COLL);
ok('security config 读回', readBack?.jsSandboxMode === 'developer', JSON.stringify(readBack));

await invoke('renderer:save-collection-security-config', COLL, { jsSandboxMode: 'safe' });
fs.writeFileSync(rootYml, originalYml);

// ---------------------------------------------------------------------------
section('UI 快照（侧栏 / 集合环境 / 标签页）');

await invoke('renderer:snapshot:save', {
  activeWorkspacePath: '',
  extras: { sidebar: { width: 321, collapsed: true } },
  workspaces: [],
  collections: [
    {
      pathname: COLL,
      workspacePathname: '',
      environment: { collection: 'environments/dev.yml', global: '' },
      environmentPath: 'environments/dev.yml',
      selectedEnvironment: 'dev',
      isOpen: true,
      isMounted: true,
      activeTab: { uid: 'tab-1', type: 'request' },
      tabs: [{ uid: 'tab-1', type: 'request' }]
    }
  ]
});
const sidebar = await invoke('renderer:snapshot:get-sidebar');
ok('snapshot:get-sidebar 返回宽高', sidebar?.width === 321 && sidebar?.collapsed === true, JSON.stringify(sidebar));

const collSnap = await invoke('renderer:snapshot:get-collection', COLL, '');
ok('snapshot:get-collection 返回选中环境', collSnap?.selectedEnvironment === 'dev', JSON.stringify(collSnap));
const tabsSnap = await invoke('renderer:snapshot:get-tabs', COLL, '');
ok('snapshot:get-tabs 返回标签页', Array.isArray(tabsSnap?.tabs) && tabsSnap.tabs.length === 1, JSON.stringify(tabsSnap));

await invoke('renderer:update-ui-state-snapshot', {
  type: 'COLLECTION_ENVIRONMENT',
  data: {
    collectionPath: COLL,
    workspacePathname: '',
    environmentPath: 'environments/prod.yml',
    selectedEnvironment: 'prod'
  }
});
const patched = await invoke('renderer:snapshot:get-collection', COLL, '');
ok('update-ui-state-snapshot 改选中环境', patched?.selectedEnvironment === 'prod', JSON.stringify(patched));
ok('update-ui-state-snapshot 保留标签页', Array.isArray(patched?.tabs) && patched.tabs.length === 1, JSON.stringify(patched?.tabs));

await invoke('renderer:snapshot:clear');
ok('snapshot:clear 后读不到侧栏', (await invoke('renderer:snapshot:get-sidebar')) == null, '');

// ---------------------------------------------------------------------------
section('ZIP 导出 → 校验 → 导入');

const zipPath = path.join(TMP, 'shop.zip').replace(/\\/g, '/');
await invoke('renderer:export-collection-zip', COLL, 'shop', zipPath);
ok('导出 zip 落盘', fs.existsSync(zipPath), zipPath);
ok('is-bruno-collection-zip 识别集合包', (await invoke('renderer:is-bruno-collection-zip', zipPath)) === true, '');

const importRoot = path.join(TMP, 'imported').replace(/\\/g, '/');
fs.mkdirSync(importRoot, { recursive: true });
const imported = await invoke('renderer:import-collection-zip', zipPath, importRoot);
ok('import-collection-zip 返回集合路径', typeof imported === 'string' && imported.length > 0, String(imported));
ok('导入目录里有 opencollection.yml', fs.existsSync(path.join(imported, 'opencollection.yml')), String(imported));
ok('is-bruno-collection-zip 拒绝普通 zip', (() => {
  const plain = path.join(TMP, 'plain.zip').replace(/\\/g, '/');
  fs.writeFileSync(plain, 'not a zip');
  return true;
})() && (await invoke('renderer:is-bruno-collection-zip', path.join(TMP, 'plain.zip').replace(/\\/g, '/')) === false));

// ---------------------------------------------------------------------------
section('工作区目录与文档');

const wsRoot = path.join(TMP, 'workspaces').replace(/\\/g, '/');
fs.mkdirSync(wsRoot, { recursive: true });
const uniqueName = await invoke('renderer:find-unique-folder-name', 'team', wsRoot);
ok('find-unique-folder-name 返回目录名', uniqueName === 'team', String(uniqueName));

const created = await invoke('renderer:create-workspace', 'Team WS', 'team', wsRoot);
ok('create-workspace 返回 uid/path/config', !!created?.workspaceUid && !!created?.workspacePath, JSON.stringify(created));
ok('create-workspace 落盘 workspace.yml', fs.existsSync(path.join(wsRoot, 'team', 'workspace.yml')), '');
ok('create-workspace config 带 pathname', created?.workspaceConfig?.pathname === created?.workspacePath, JSON.stringify(created?.workspaceConfig));

await invoke('renderer:save-workspace-docs', created.workspacePath, '# Team\nhello');
const wsYml = fs.readFileSync(path.join(wsRoot, 'team', 'workspace.yml'), 'utf8');
ok('save-workspace-docs 写入 workspace.yml', wsYml.includes('hello'), wsYml.slice(0, 160));

await invoke('renderer:rename-workspace', created.workspacePath, 'Team Renamed');
const renamed = fs.readFileSync(path.join(wsRoot, 'team', 'workspace.yml'), 'utf8');
ok('rename-workspace 改 workspace.yml 的 name', renamed.includes('Team Renamed'), renamed.slice(0, 120));

const listed = await invoke('renderer:get-last-opened-workspaces');
ok('get-last-opened-workspaces 含默认与新工作区', Array.isArray(listed) && listed.length >= 2, JSON.stringify(listed?.map((w) => w.name)));

const reopened = await invoke('renderer:open-workspace', path.join(wsRoot, 'team').replace(/\\/g, '/'));
ok('open-workspace 重新注册', reopened?.workspaceUid === created.workspaceUid, JSON.stringify(reopened));
ok('open-workspace 读到 docs', typeof reopened?.workspaceConfig?.docs === 'string' && reopened.workspaceConfig.docs.includes('hello'), JSON.stringify(reopened?.workspaceConfig?.docs));

const ensureDir = await invoke('renderer:ensure-collections-folder', created.workspacePath);
ok('ensure-collections-folder 建目录', fs.existsSync(path.join(created.workspacePath, 'collections')), String(ensureDir));

await invoke('renderer:close-workspace', created.workspacePath);
const afterClose = await invoke('renderer:get-last-opened-workspaces');
ok('close-workspace 从列表移除但保留目录', afterClose.length === listed.length - 1 && fs.existsSync(path.join(wsRoot, 'team')), JSON.stringify(afterClose?.map((w) => w.name)));

// ---------------------------------------------------------------------------
section('其它长尾通道');

const proxy = await invoke('renderer:get-system-proxy-variables');
ok('get-system-proxy-variables 返回形状', !!proxy && 'http_proxy' in proxy && 'source' in proxy, JSON.stringify(proxy));
ok('refresh-system-proxy 可用', !!(await invoke('renderer:refresh-system-proxy')), '');
ok('refresh-pac-cache 可用', (await invoke('renderer:refresh-pac-cache')) === true, '');
ok('clear-http-https-agent-cache 可用', (await invoke('renderer:clear-http-https-agent-cache')) === true, '');

const workspacesOfCollection = await invoke('renderer:get-collection-workspaces', COLL);
ok('get-collection-workspaces 返回数组', Array.isArray(workspacesOfCollection), JSON.stringify(workspacesOfCollection));
ok('set-collection-workspace 直接成功', (await invoke('renderer:set-collection-workspace', 'shop-verify', '')) === true, '');

const firstRequest = (before?.items || []).find((i) => i.request || i.type === 'http-request');
const requestPathname = firstRequest?.pathname || 'health.yml';
const loaded = await invoke('renderer:load-large-request', { collectionUid: 'shop-verify', pathname: requestPathname });
ok('load-large-request 返回请求内容', !!loaded, JSON.stringify(loaded)?.slice(0, 120));

// ---------------------------------------------------------------------------
console.log(`\n结果：${pass} 通过，${failures.length} 失败`);
if (failures.length) {
  failures.forEach((f) => console.log('  - ' + f));
}
ws.close();
process.exit(failures.length ? 1 : 0);
