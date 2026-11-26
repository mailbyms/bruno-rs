// 第 1 批 + 第 2 批（除文件监听）+ 第 3 批 Git 的通道级验证。
//
// 直接调 `window.ipcRenderer.invoke`，因此同时覆盖三件事：
//   1. ipc-shim 的通道名/参数名映射是否正确（最容易写错的一层）
//   2. Rust 命令是否落盘、返回值形状是否符合前端预期
//   3. 写操作之后事件是否按前端订阅的 payload 形状回推
//
// 用法：
//   node scripts/mock-server.mjs        # 127.0.0.1:18081
//   BRUNO_RS_DATA_DIR=<tmp> WEBVIEW2_USER_DATA_FOLDER=<tmp> \
//     WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9240 \
//     ./target/debug/bruno-rs.exe fixtures/shop
//   node scripts/verify-batch12.mjs
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';

const PORT = Number(process.env.CDP_PORT || 9240);
const MOCK = process.env.MOCK_URL || 'http://127.0.0.1:18081';
const TMP = fs.mkdtempSync(path.join(os.tmpdir(), 'brs-batch12-'));

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
const errors = [];
ws.onmessage = (e) => {
  const m = JSON.parse(e.data);
  if (m.id && pending.has(m.id)) {
    pending.get(m.id)(m);
    pending.delete(m.id);
    return;
  }
  if (m.method === 'Runtime.exceptionThrown') {
    errors.push(m.params.exceptionDetails.exception?.description || m.params.exceptionDetails.text);
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

/** 以真实前端同样的方式调用 shim；失败时把 IPC 错误原文抛出来 */
const invoke = async (channel, ...args) => {
  const expr = `(async () => {
    try {
      return JSON.stringify({ ok: true, value: await window.ipcRenderer.invoke(${JSON.stringify(channel)}, ...${JSON.stringify(args)}) });
    } catch (e) {
      return JSON.stringify({ ok: false, error: String((e && (e.message || e.error)) || e) });
    }
  })()`;
  // 单通道超时：卡住的通道要报错而不是把整轮验证挂死
  const raw = await Promise.race([
    ev(expr),
    new Promise((_, rej) => setTimeout(() => rej(new Error(`[${channel}] 调用超时`)), 90000))
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

// 等首帧：集合树就绪后才继续（shim 的 invoke 在页面加载后可用）
for (let i = 0; i < 60; i++) {
  const ready = await ev('!!window.ipcRenderer && !!window.__BRUNO_RS__');
  if (ready) break;
  await sleep(250);
}
await ev(`window.__TREES__ = [];
  window.ipcRenderer.on('main:collection-tree-loaded', (t) => window.__TREES__.push(t));
  'ok'`);

/** 取最近一次 tree 事件，拼成前端 Redux 里的 collection 形状 */
const lastTree = async (pathname) => {
  const want = String(pathname || '').replace(/\\/g, '/').toLowerCase();
  const raw = await ev('JSON.stringify((window.__TREES__||[]).slice(-5))');
  const list = JSON.parse(raw || '[]');
  const hit = [...list]
    .reverse()
    .find((t) => String(t?.tree?.pathname || '').replace(/\\/g, '/').toLowerCase() === want);
  const t = hit || list[list.length - 1];
  if (!t) return null;
  return {
    uid: t.collectionUid,
    pathname: t.tree.pathname,
    name: t.tree.root?.meta?.name || 'Batch12',
    items: t.tree.items || [],
    environments: t.tree.environments || [],
    root: t.tree.root || {}
  };
};

// ---------------------------------------------------------------------------
section('集合 / 请求 / 文件夹 CRUD');
// ---------------------------------------------------------------------------
const created = await invoke('renderer:create-collection', 'Batch12', 'batch12', TMP);
ok('create-collection 返回 uid+pathname', !!created?.uid && !!created?.pathname, JSON.stringify(created));
const collPath = created?.pathname;
ok('create-collection 落盘 opencollection.yml', !!collPath && fs.existsSync(path.join(collPath, 'opencollection.yml')));

const reqItem = {
  name: 'ping',
  type: 'http-request',
  request: { url: `${MOCK}/echo`, method: 'GET', headers: [], params: [], body: { mode: 'none' } }
};
// 前端传的 pathname 是「集合内相对路径 + 扩展名」，返回值也是相对路径
const newReq = await invoke('renderer:new-request', path.join(collPath, 'ping.yml'), reqItem);
ok('new-request 返回相对 pathname', typeof newReq?.pathname === 'string' && !path.isAbsolute(newReq.pathname), JSON.stringify(newReq));
ok('new-request 文件存在', fs.existsSync(path.join(collPath, newReq?.pathname || 'missing.yml')), String(newReq?.pathname));

const folder = await invoke('renderer:new-folder', {
  pathname: path.join(collPath, 'nested'),
  folderData: { name: 'nested', type: 'folder', root: { request: {} } },
  format: 'yml'
});
ok('new-folder 返回 pathname', typeof folder?.pathname === 'string', JSON.stringify(folder));
ok('new-folder 落盘 folder.yml', fs.existsSync(path.join(collPath, 'nested', 'folder.yml')));

await invoke('renderer:rename-item-name', {
  itemPath: newReq.pathname,
  newName: 'ping-renamed',
  collectionPathname: collPath
});
const reqYml = fs.readFileSync(path.join(collPath, newReq.pathname), 'utf8');
ok('rename-item-name 只改 info.name', reqYml.includes('ping-renamed'), reqYml.slice(0, 120));

await invoke('renderer:rename-item-filename', {
  oldPath: newReq.pathname,
  newPath: 'ping-renamed.yml',
  newName: 'ping-renamed',
  newFilename: 'ping-renamed',
  collectionPathname: collPath
});
const renamedAbs = path.join(collPath, 'ping-renamed.yml');
ok('rename-item-filename 改名落盘', fs.existsSync(renamedAbs) && !fs.existsSync(path.join(collPath, newReq.pathname)), renamedAbs);

const moved = await invoke('renderer:move-item', {
  targetDirname: path.join(collPath, 'nested'),
  sourcePathname: 'ping-renamed.yml'
});
const movedPath = path.join(collPath, 'nested', 'ping-renamed.yml');
ok('move-item 移到 nested', fs.existsSync(movedPath), JSON.stringify(moved));

// 集合根设置：写 request 默认值（headers）并保留 version
await invoke('renderer:save-collection-root', collPath, {
  version: '1',
  name: 'Batch12',
  request: { headers: [{ name: 'x-coll', value: '1', enabled: true }], auth: { mode: 'none' } }
});
const collRoot = fs.readFileSync(path.join(collPath, 'opencollection.yml'), 'utf8');
ok('save-collection-root 写入 request 默认头', collRoot.includes('x-coll'), collRoot.slice(0, 200));

await invoke('renderer:save-folder-root', {
  name: 'nested',
  folderPathname: path.join(collPath, 'nested'),
  collectionPathname: collPath,
  root: { request: { headers: [{ name: 'x-folder', value: '2', enabled: true }] }, docs: 'folder docs' }
});
const folderRoot = fs.readFileSync(path.join(collPath, 'nested', 'folder.yml'), 'utf8');
ok('save-folder-root 写入 folder.yml', folderRoot.includes('x-folder'), folderRoot.slice(0, 200));

// ---------------------------------------------------------------------------
section('环境（集合 / 全局）');
// ---------------------------------------------------------------------------
const env = await invoke('renderer:create-environment', collPath, 'staging', [
  { name: 'baseUrl', value: MOCK, enabled: true }
], '#ff0000');
ok('create-environment 返回 uid', !!env?.uid, JSON.stringify(env));
ok('create-environment 落盘', fs.existsSync(path.join(collPath, 'environments', 'staging.yml')));

await invoke('renderer:save-environment', collPath, {
  ...env,
  name: 'staging',
  variables: [
    { name: 'baseUrl', value: MOCK, enabled: true },
    { name: 'token', value: 'abc', enabled: true, secret: true }
  ]
});
const envYml = fs.readFileSync(path.join(collPath, 'environments', 'staging.yml'), 'utf8');
ok('save-environment 写入变量', envYml.includes('token') && envYml.includes('baseUrl'));

await invoke('renderer:rename-environment', collPath, 'staging', 'prod');
ok('rename-environment 文件改名', fs.existsSync(path.join(collPath, 'environments', 'prod.yml')));

await invoke('renderer:update-environment-color', collPath, 'prod', '#00ff00');
ok('update-environment-color 写入颜色', fs.readFileSync(path.join(collPath, 'environments', 'prod.yml'), 'utf8').includes('00ff00'));

const treeAfterEnv = await lastTree(collPath);
ok('树事件带 environments', Array.isArray(treeAfterEnv?.environments) && treeAfterEnv.environments.length > 0,
  JSON.stringify(treeAfterEnv?.environments));

await invoke('renderer:delete-environment', collPath, 'prod');
ok('delete-environment 删掉文件', !fs.existsSync(path.join(collPath, 'environments', 'prod.yml')));

// 全局环境
const g = await invoke('renderer:create-global-environment', { name: 'g1', uid: 'g1', variables: [{ name: 'a', value: '1', enabled: true }] });
ok('create-global-environment 返回 uid', !!g?.uid, JSON.stringify(g));
await invoke('renderer:rename-global-environment', { name: 'g1-renamed', environmentUid: g?.uid });
let globals = await invoke('renderer:get-global-environments');
ok('rename-global-environment 生效',
  globals?.globalEnvironments?.some((e) => e.uid === g?.uid && e.name === 'g1-renamed'),
  JSON.stringify(globals));
await invoke('renderer:update-global-environment-color', { environmentUid: g?.uid, color: '#123456' });
globals = await invoke('renderer:get-global-environments');
ok('update-global-environment-color 生效',
  globals?.globalEnvironments?.some((e) => e.uid === g?.uid && e.color === '#123456'),
  JSON.stringify(globals));
await invoke('renderer:save-global-environment', {
  environmentUid: g?.uid,
  variables: [{ name: 'a', value: '2', enabled: true }]
});
globals = await invoke('renderer:get-global-environments');
const gEnv = globals?.globalEnvironments?.find((e) => e.uid === g?.uid);
ok('save-global-environment 只改变量、保留 name/color',
  gEnv?.name === 'g1-renamed' && gEnv?.color === '#123456' && gEnv?.variables?.[0]?.value === '2',
  JSON.stringify(gEnv));
await invoke('renderer:select-global-environment', { environmentUid: g?.uid });
globals = await invoke('renderer:get-global-environments');
ok('select-global-environment 生效', globals?.activeGlobalEnvironmentUid === g?.uid, JSON.stringify(globals?.activeGlobalEnvironmentUid));
await invoke('renderer:delete-global-environment', { environmentUid: g?.uid });
globals = await invoke('renderer:get-global-environments');
ok('delete-global-environment 生效', !globals?.globalEnvironments?.some((e) => e.uid === g?.uid));

// ---------------------------------------------------------------------------
section('偏好设置');
// ---------------------------------------------------------------------------
const prefs = await invoke('renderer:get-preferences');
ok('get-preferences 返回完整对象', !!prefs?.request && prefs.request.timeout !== undefined, JSON.stringify(prefs)?.slice(0, 200));
const beforeTimeout = prefs.request.timeout;
await invoke('renderer:save-preferences', { ...prefs, request: { ...prefs.request, timeout: 4321 } });
const prefs2 = await invoke('renderer:get-preferences');
ok('save-preferences 生效（timeout=4321）', prefs2?.request?.timeout === 4321, String(prefs2?.request?.timeout));
await invoke('renderer:save-preferences', { ...prefs2, request: { ...prefs2.request, timeout: beforeTimeout } });

// ---------------------------------------------------------------------------
section('Cookies');
// ---------------------------------------------------------------------------
const parsed = await invoke('renderer:get-parsed-cookie', `sid=xyz; Path=/api; Domain=.example.com; HttpOnly`);
ok('get-parsed-cookie 解析字段',
  parsed?.key === 'sid' && parsed?.value === 'xyz' && parsed?.path === '/api' && parsed?.domain === 'example.com' && parsed?.httpOnly === true,
  JSON.stringify(parsed));
const raw = await invoke('renderer:create-cookie-string', { key: 'sid', value: 'xyz', path: '/api', domain: 'example.com', httpOnly: true });
ok('create-cookie-string 反向拼串', typeof raw === 'string' && raw.startsWith('sid=xyz; Path=/api; Domain=example.com'), String(raw));

await invoke('renderer:add-cookie', 'example.com', { key: 'a', value: '1', path: '/', domain: 'example.com', secure: true, httpOnly: false });
let cookies = await invoke('renderer:delete-cookies-for-domain', 'example.com').then(() => null);
await invoke('renderer:add-cookie', 'example.com', { key: 'a', value: '1', path: '/', domain: 'example.com' });
await invoke('renderer:add-cookie', 'example.com', { key: 'b', value: '2', path: '/', domain: 'example.com' });
// 面板数据来自 main:cookies-update 事件；这里直接读 Redux 之外的状态不方便，
// 用 add → modify → delete 的行为串联验证（失败会抛错）
await invoke('renderer:modify-cookie', 'example.com', { key: 'a', path: '/', domain: 'example.com' }, { key: 'a', value: '9', path: '/', domain: 'example.com' });
const cookieStr = await invoke('renderer:create-cookie-string', { key: 'a', value: '9', path: '/', domain: 'example.com' });
ok('modify-cookie 后 value 已更新', cookieStr.includes('a=9'), cookieStr);
await invoke('renderer:delete-cookie', 'example.com', '/', 'a');
await invoke('renderer:delete-cookie', 'example.com', '/', 'b');
ok('cookie CRUD 未抛错', true);

// 真跨请求：先打 /cookie/set，再打 /cookie/echo，echo 应带上 sid
const cookieColl = { uid: created?.uid, pathname: collPath, name: 'Batch12', items: [], environments: [] };
const probe = (uid, url) => ({
  uid,
  pathname: '',
  name: uid,
  type: 'http-request',
  request: { url, method: 'GET', headers: [], params: [], body: { mode: 'none' } }
});
const before = await invoke('send-http-request', probe('cookie-set', `${MOCK}/cookie/set`), cookieColl, null, null);
ok('cookie/set 请求成功', String(before?.status) === '200', JSON.stringify(before?.status));
const after = await invoke('send-http-request', probe('cookie-echo', `${MOCK}/cookie/echo`), cookieColl, null, null);
// 响应体在 data（已解析）/ dataBuffer（原始文本）；requestSent.headers 是实际发出的头
const sentCookie = (after?.requestSent?.headers || []).find((h) => String(h.name).toLowerCase() === 'cookie');
ok('跨请求带上 Cookie 头', String(sentCookie?.value || '').includes('sid=abc123'), JSON.stringify(sentCookie));
let echoData = after?.data;
if (typeof echoData === 'string') {
  try {
    echoData = JSON.parse(echoData);
  } catch {}
}
ok('服务端也收到 Cookie 头', String(echoData?.headers?.cookie || '').includes('sid=abc123'), JSON.stringify(echoData?.headers?.cookie));
await invoke('renderer:delete-cookies-for-domain', '127.0.0.1');

// ---------------------------------------------------------------------------
section('Runner');
// ---------------------------------------------------------------------------
const runColl = await lastTree(collPath);
const runFolder = { uid: runColl?.uid, pathname: collPath, name: 'Batch12', items: runColl?.items || [] };
await ev(`window.__RUNEVENTS__ = []; window.ipcRenderer.on('main:run-folder-event', (e) => window.__RUNEVENTS__.push(e)); 'ok'`);
const summary = await invoke('renderer:run-collection-folder', runFolder, runColl, null, null, true, 0, null, null);
const runEvents = JSON.parse((await ev('JSON.stringify((window.__RUNEVENTS__||[]).map(e => e.type))')) || '[]');
ok('runner 返回汇总', typeof summary?.total === 'number' || typeof summary?.passed === 'number', JSON.stringify(summary));
ok('runner 事件流完整',
  runEvents.includes('testrun-started') && runEvents.includes('request-queued') && runEvents.includes('testrun-ended'),
  JSON.stringify(runEvents));

// ---------------------------------------------------------------------------
section('导出');
// ---------------------------------------------------------------------------
const zipPath = path.join(TMP, 'export.zip').replace(/\\/g, '/');
const zip = await invoke('renderer:export-collection-zip', collPath, 'batch12', zipPath);
ok('export-collection-zip 落盘', fs.existsSync(zipPath) && fs.statSync(zipPath).size > 0, JSON.stringify(zip));
const ymlPath = path.join(TMP, 'export.yml').replace(/\\/g, '/');
await invoke('renderer:export-collection-yaml', 'batch12', collRoot, ymlPath);
ok('export-collection-yaml 落盘', fs.existsSync(ymlPath) && fs.statSync(ymlPath).size > 0);
const pmPath = path.join(TMP, 'export.postman.json').replace(/\\/g, '/');
await invoke('renderer:export-collection-postman', TMP, 'export.postman.json', JSON.stringify({ info: { name: 'x', schema: 'v2.1' }, item: [] }), true);
ok('export-collection-postman 落盘', fs.existsSync(pmPath), pmPath);

// ---------------------------------------------------------------------------
section('快照');
// ---------------------------------------------------------------------------
const snapshot = { collections: [{ uid: created?.uid, pathname: collPath, activeTabUid: 'tab-1' }] };
await invoke('renderer:snapshot:save', snapshot);
const got = await invoke('renderer:snapshot:get');
ok('snapshot:save/get 往返', JSON.stringify(got)?.includes('tab-1'), JSON.stringify(got)?.slice(0, 160));
const tabs = await invoke('renderer:snapshot:get-tabs', collPath, null);
ok('snapshot:get-tabs 可调用', tabs === null || typeof tabs === 'object', JSON.stringify(tabs));
await invoke('renderer:snapshot:clear');

// ---------------------------------------------------------------------------
section('Git');
// ---------------------------------------------------------------------------
const repo = path.join(TMP, 'git-src').replace(/\\/g, '/');
fs.mkdirSync(repo, { recursive: true });
const run = (cmd, args, cwd) =>
  execFileSync(cmd, args, { cwd, stdio: 'pipe', encoding: 'utf8' });
run('git', ['init', '-q'], repo);
fs.writeFileSync(path.join(repo, 'opencollection.yml'), 'version: "1"\nname: gitcoll\n');
run('git', ['add', '.'], repo);
run('git', ['-c', 'user.email=a@b.c', '-c', 'user.name=t', 'commit', '-qm', 'init'], repo);

const evlog = JSON.parse((await ev('JSON.stringify((window.__EVLOG__||[]).map(e => e.event))')) || '[]');
ok('启动推送 main:git-version', evlog.includes('main:git-version'), JSON.stringify([...new Set(evlog)].slice(0, 12)));
ok('启动推送 main:cookies-update', evlog.includes('main:cookies-update'), '');

const cloneTarget = path.join(TMP, 'cloned').replace(/\\/g, '/');
const cloneUrl = `file:///${repo.replace(/\\/g, '/')}`;
const cloned = await invoke('renderer:clone-git-repository', { url: cloneUrl, path: cloneTarget, branch: null });
ok('clone-git-repository 克隆成功', fs.existsSync(path.join(cloneTarget, 'opencollection.yml')), JSON.stringify(cloned));

const remote = await invoke('renderer:get-collection-git-remote-url', cloneTarget);
ok('get-collection-git-remote-url 读到 remote', String(remote || '').includes('git-src') || !!remote, String(remote));

// 克隆后集合已进入工作区，connect 才能写入 remote
const connected = await invoke('renderer:connect-collection-to-git', '', cloneTarget, cloneUrl);
ok('connect-collection-to-git 返回 true', connected === true, JSON.stringify(connected));
const disconnected = await invoke('renderer:disconnect-collection-from-git', '', cloneTarget);
ok('disconnect-collection-from-git 返回 true', disconnected === true, JSON.stringify(disconnected));

// ---------------------------------------------------------------------------
console.log(`\n${pass} passed, ${failures.length} failed`);
if (errors.length) {
  console.log('\n页面异常：');
  for (const e of errors.slice(0, 10)) console.log('  -', e);
}
if (failures.length) {
  console.log('\n失败项：');
  for (const f of failures) console.log('  -', f);
  process.exit(1);
}
