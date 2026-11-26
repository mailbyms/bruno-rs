// Postman 导入验证：convert → import 落盘 → 侧栏可见 → 发请求 → 脚本结果
// 用法：应用以 CDP 9237 启动、mock server 在 18081，然后 node scripts/verify-import.mjs
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';

const PORT = Number(process.env.CDP_PORT || 9237);
const FIXTURE = process.env.POSTMAN_FILE
  || path.resolve('fixtures/postman/postman-shop.postman_collection.json');
const LOCATION = process.env.IMPORT_LOCATION
  || fs.mkdtempSync(path.join(os.tmpdir(), 'brs-import-'));

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
  const msg = JSON.parse(e.data);
  if (msg.id && pending.has(msg.id)) {
    pending.get(msg.id)(msg);
    pending.delete(msg.id);
    return;
  }
  if (msg.method === 'Runtime.exceptionThrown') {
    errors.push(msg.params.exceptionDetails.exception?.description || msg.params.exceptionDetails.text);
  }
};
const call = (method, params = {}) =>
  new Promise((r) => {
    const i = ++id;
    pending.set(i, r);
    ws.send(JSON.stringify({ id: i, method, params }));
  });
const ev = async (expression) => {
  const j = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  if (j.result?.exceptionDetails) throw new Error(JSON.stringify(j.result.exceptionDetails));
  return j.result?.result?.value;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const fail = (msg) => {
  console.error('FAIL:', msg);
  process.exit(1);
};

const STORE_FINDER = `(() => {
  const host = document.querySelector('#root') || document.body;
  const key = Object.keys(host).find((k) => k.startsWith('__reactContainer$'));
  const root = key ? host[key] : null;
  const seen = new Set();
  let store = null;
  const walk = (fiber) => {
    if (!fiber || seen.has(fiber)) return;
    seen.add(fiber);
    const props = fiber.memoizedProps;
    for (const c of [props?.store, props?.value?.store, props?.value?.getState ? props.value : null]) {
      if (c && typeof c.getState === 'function' && c.getState()?.collections) store = c;
    }
    walk(fiber.child);
    walk(fiber.sibling);
  };
  walk(root);
  return store ? (globalThis.__STORE__ = store, true) : false;
})()`;

await call('Runtime.enable');
for (let i = 0; i < 30; i++) {
  if (await ev(STORE_FINDER)) break;
  await sleep(300);
}
if (!(await ev('Boolean(globalThis.__STORE__)'))) fail('找不到 Redux store');

const clickText = async (text, scope = 'button') => {
  const pos = await ev(`(() => {
    const el = [...document.querySelectorAll('${scope}')].find((e) => e.innerText.trim().includes(${JSON.stringify(text)}));
    if (!el) return null;
    const r = el.getBoundingClientRect();
    return JSON.stringify({ x: r.x + r.width / 2, y: r.y + r.height / 2 });
  })()`);
  if (!pos) return false;
  const p = JSON.parse(pos);
  for (const type of ['mouseMoved', 'mousePressed', 'mouseReleased']) {
    await call('Input.dispatchMouseEvent', { type, x: p.x, y: p.y, button: 'left', clickCount: 1 });
  }
  return true;
};

// 1. 转换
const raw = fs.readFileSync(FIXTURE, 'utf8');
const converted = await ev(`(async () => {
  const res = await window.ipcRenderer.invoke('renderer:convert-postman-to-bruno', ${raw});
  return JSON.stringify(res);
})()`);
const conv = JSON.parse(converted);
if (!conv?.collection) fail(`convert 返回空：${converted}`);
if (conv.collection.items?.length !== 2) fail(`item 数量不对：${JSON.stringify(conv.collection.items?.map((i) => i.name))}`);
const folder = conv.collection.items[0];
if (folder.type !== 'folder' || Object.prototype.hasOwnProperty.call(folder, 'request')) {
  fail(`文件夹形状不对：${JSON.stringify(folder).slice(0, 200)}`);
}
const leaf = folder.items[0];
if (leaf.type !== 'http-request' || Object.prototype.hasOwnProperty.call(leaf, 'items')) {
  fail(`请求形状不对：${JSON.stringify(leaf).slice(0, 200)}`);
}
if (!String(leaf.request.tests).includes('res.getStatus()')) fail('pm.* 脚本没有翻译');
console.log('OK convert：', conv.collection.name, '| items:', conv.collection.items.length, '| issues:', conv.issues.length);

// 2. 落盘
const imported = await ev(`(async () => {
  const res = await window.ipcRenderer.invoke('renderer:import-collection', ${JSON.stringify(conv.collection)}, ${JSON.stringify(LOCATION)}, { format: 'yml' });
  return JSON.stringify(res);
})()`);
const imp = JSON.parse(imported);
const target = imp?.success?.items?.[0]?.path;
if (!target) fail(`import 没有返回路径：${imported}`);
console.log('OK import：', target);

// 3. 磁盘结构
const rootFile = path.join(target.replace(/\//g, path.sep), 'opencollection.yml');
if (!fs.existsSync(rootFile)) fail(`缺少 opencollection.yml：${rootFile}`);
const folderFile = path.join(target.replace(/\//g, path.sep), 'Orders', 'folder.yml');
if (!fs.existsSync(folderFile)) fail(`缺少 Orders/folder.yml`);
const listFile = path.join(target.replace(/\//g, path.sep), 'Orders', 'List Orders.yml');
if (!fs.existsSync(listFile)) fail(`缺少 Orders/List Orders.yml`);
const createFile = path.join(target.replace(/\//g, path.sep), 'Create Order.yml');
if (!fs.existsSync(createFile)) fail(`缺少 Create Order.yml`);
const rootYaml = fs.readFileSync(rootFile, 'utf8');
if (!rootYaml.includes('bearer') || !rootYaml.includes('baseUrl')) fail(`集合默认值没写进 opencollection.yml：\n${rootYaml}`);
const leafYaml = fs.readFileSync(listFile, 'utf8');
if (!leafYaml.includes('params:') || !leafYaml.includes('limit')) fail(`query 参数没落盘：\n${leafYaml}`);
if (/\bnull\b/.test(leafYaml)) fail(`YAML 出现 null：\n${leafYaml}`);
console.log('OK 磁盘结构：opencollection.yml + folder.yml + 3 个请求文件');

// 4. 侧栏出现集合，点开文件夹与请求
let visible = false;
for (let i = 0; i < 40; i++) {
  const rows = String(await ev("JSON.stringify([...document.querySelectorAll('[data-sidebar-row]')].map(e=>e.innerText))"));
  if (rows.includes('Postman Shop')) {
    visible = true;
    break;
  }
  await sleep(250);
}
if (!visible) fail('导入后侧栏没有出现 Postman Shop（workspace-config-updated 没生效？）');

if (!(await clickText('Postman Shop', '[data-sidebar-row]'))) {
  if (!(await clickText('Orders', '[data-sidebar-row]'))) fail('侧栏找不到 Orders');
}
await sleep(500);
for (let i = 0; i < 20; i++) {
  if (await clickText('Orders', '[data-sidebar-row]')) break;
  await sleep(200);
}
await sleep(400);
let clicked = false;
for (let i = 0; i < 30; i++) {
  if (await clickText('List Orders', '[data-sidebar-row]')) {
    clicked = true;
    break;
  }
  await sleep(200);
}
if (!clicked) fail('侧栏找不到 List Orders');

// 5. 发请求
for (let i = 0; i < 30; i++) {
  if (await clickText('Send')) break;
  await sleep(200);
}
let result = null;
for (let i = 0; i < 60; i++) {
  await sleep(250);
  result = await ev(`(() => {
    const s = globalThis.__STORE__.getState();
    for (const c of s.collections.collections) {
      if (c.name !== 'Postman Shop') continue;
      const walk = (items) => {
        for (const it of items || []) {
          if (it.name === 'List Orders') return it;
          const f = walk(it.items);
          if (f) return f;
        }
      };
      const it = walk(c.items);
      if (it?.response) return {
        status: it.response.status,
        url: it.request?.url,
        tests: (it.testResults || []).map((t) => t.status),
        headers: JSON.stringify(it.request?.headers || [])
      };
    }
  })()`);
  if (result?.status === 200) break;
}
if (result?.status !== 200) fail(`List Orders 没有 200：${JSON.stringify(result)}`);
if (!result.tests.includes('pass')) fail(`导入的测试脚本没跑通：${JSON.stringify(result)}`);
if (!result.headers.includes('X-From')) fail(`header 没带过去：${result.headers}`);
console.log('OK 请求：', result.status, '| tests:', result.tests.join(','), '| url:', result.url);

// 6. 第二个请求（POST + prerequest 变量）
if (!(await clickText('Create Order', '[data-sidebar-row]'))) fail('侧栏找不到 Create Order');
for (let i = 0; i < 30; i++) {
  if (await clickText('Send')) break;
  await sleep(200);
}
let post = null;
for (let i = 0; i < 60; i++) {
  await sleep(250);
  post = await ev(`(() => {
    const s = globalThis.__STORE__.getState();
    for (const c of s.collections.collections) {
      if (c.name !== 'Postman Shop') continue;
      const walk = (items) => {
        for (const it of items || []) {
          if (it.name === 'Create Order') return it;
          const f = walk(it.items);
          if (f) return f;
        }
      };
      const it = walk(c.items);
      if (it?.response) return {
        status: it.response.status,
        body: it.response.data?.body || '',
        tests: (it.testResults || []).map((t) => t.status),
        vars: c.runtimeVariables || {}
      };
    }
  })()`);
  if (post?.status === 200) break;
}
if (post?.status !== 200) fail(`Create Order 没有 200：${JSON.stringify(post)}`);
if (!post.body.includes('coffee')) fail(`body 没发出去：${post.body.slice(0, 200)}`);
if (!post.tests.includes('pass')) fail(`POST 的测试脚本没跑通：${JSON.stringify(post.tests)}`);
if (post.vars.fromPostman !== 'yes') fail(`prerequest 的 setVar 没生效：${JSON.stringify(post.vars)}`);
console.log('OK POST：', post.status, '| tests:', post.tests.join(','), '| runtimeVars:', JSON.stringify(post.vars));

if (errors.length) fail(`渲染期异常：${errors.join('\n')}`);
console.log('\n全部通过。导入目录：', target);
ws.close();
