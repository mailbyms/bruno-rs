// 真实 UI 路径验证：侧栏菜单 → Import Collection → 选文件 → 填位置 → Import → 发请求
// 用法：应用以 CDP 9239 启动、mock server 在 18081，然后 node scripts/verify-import-ui.mjs
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';

const PORT = Number(process.env.CDP_PORT || 9239);
const FILE = path.resolve(
  process.env.POSTMAN_FILE || 'fixtures/postman/postman-shop.postman_collection.json'
);
const LOCATION = process.env.IMPORT_LOCATION || fs.mkdtempSync(path.join(os.tmpdir(), 'brs-ui-import-'));

const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
const page = list.find((x) => x.type === 'page' && x.url !== 'about:blank');
if (!page) {
  console.error('找不到页面目标');
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
  const j = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  if (j.result?.exceptionDetails) throw new Error(JSON.stringify(j.result.exceptionDetails));
  return j.result?.result?.value;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const fail = (msg) => {
  console.error('FAIL:', msg);
  process.exit(1);
};
const clickSel = async (selector) => {
  const pos = await ev(`(() => {
    const el = document.querySelector(${JSON.stringify(selector)});
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
const clickText = async (text, scope = 'button, [role="menuitem"], div') => {
  const pos = await ev(`(() => {
    const els = [...document.querySelectorAll(${JSON.stringify(scope)})];
    const el = els.find((e) => (e.innerText || '').trim() === ${JSON.stringify(text)})
      || els.find((e) => (e.innerText || '').trim().includes(${JSON.stringify(text)}));
    if (!el) return null;
    const r = el.getBoundingClientRect();
    if (!r.width || !r.height) return null;
    return JSON.stringify({ x: r.x + r.width / 2, y: r.y + r.height / 2 });
  })()`);
  if (!pos) return false;
  const p = JSON.parse(pos);
  for (const type of ['mouseMoved', 'mousePressed', 'mouseReleased']) {
    await call('Input.dispatchMouseEvent', { type, x: p.x, y: p.y, button: 'left', clickCount: 1 });
  }
  return true;
};

await call('Runtime.enable');
await call('DOM.enable');

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

// 等首帧
for (let i = 0; i < 40; i++) {
  if (await ev(STORE_FINDER)) break;
  await sleep(300);
}
if (!(await ev('Boolean(globalThis.__STORE__)'))) fail('找不到 Redux store');
for (let i = 0; i < 40; i++) {
  if (await ev("Boolean(document.querySelector('[data-testid=\"collections-header-add-menu\"]'))")) break;
  await sleep(300);
}

// 1. 打开 Import Collection
if (!(await clickSel('[data-testid="collections-header-add-menu"]'))) fail('找不到侧栏 add 菜单');
await sleep(600);
if (!(await clickText('Import Collection'))) fail('菜单里没有 Import Collection');
for (let i = 0; i < 30; i++) {
  if (await ev("Boolean(document.querySelector('[data-testid=\"import-collection-modal\"]'))")) break;
  await sleep(200);
}
if (!(await ev("Boolean(document.querySelector('[data-testid=\"import-collection-modal\"]'))"))) {
  fail('Import 弹窗没打开');
}
console.log('OK 弹窗已打开');

// 2. 给 input[type=file] 塞真实文件
const obj = await call('Runtime.evaluate', {
  expression: "document.querySelector('[data-testid=\"import-collection-modal\"] input[type=file]')"
});
const objectId = obj.result?.result?.objectId;
if (!objectId) fail('找不到文件输入框');
const setRes = await call('DOM.setFileInputFiles', { files: [FILE], objectId });
if (setRes.error) fail(`设置文件失败：${JSON.stringify(setRes.error)}`);

// 3. 等位置弹窗，填目录
for (let i = 0; i < 40; i++) {
  if (await ev("Boolean(document.querySelector('[data-testid=\"import-collection-browse-link\"]'))")) break;
  await sleep(250);
}
if (!(await ev("Boolean(document.querySelector('[data-testid=\"import-collection-browse-link\"]'))"))) {
  const text = String(await ev('document.body.innerText')).slice(0, 400);
  fail(`没有进入导入位置弹窗，页面文本：${text}`);
}
const filled = await ev(`(() => {
  const link = document.querySelector('[data-testid="import-collection-browse-link"]');
  const modal = link.closest('[role="dialog"]') || document.body;
  const input = [...modal.querySelectorAll('input')].find((i) => i.type !== 'file');
  if (!input) return false;
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
  setter.call(input, ${JSON.stringify(LOCATION)});
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
  return input.value;
})()`);
if (filled !== LOCATION) fail(`位置输入框没填上：${filled}`);
console.log('OK 位置：', LOCATION);

// 4. 点 Import
await sleep(300);
if (!(await clickText('Import', 'button'))) fail('找不到 Import 按钮');
let done = false;
for (let i = 0; i < 80; i++) {
  await sleep(300);
  const rows = String(await ev("JSON.stringify([...document.querySelectorAll('[data-sidebar-row]')].map(e=>e.innerText))"));
  if (rows.includes('Postman Shop')) {
    done = true;
    break;
  }
}
if (!done) fail('导入后侧栏没有出现 Postman Shop');
console.log('OK 侧栏出现 Postman Shop');

// 5. 打开请求并发送
await clickText('Postman Shop', '[data-sidebar-row]');
await sleep(400);
for (let i = 0; i < 20; i++) {
  if (await clickText('Orders', '[data-sidebar-row]')) break;
  await sleep(200);
}
await sleep(400);
let opened = false;
for (let i = 0; i < 30; i++) {
  if (await clickText('List Orders', '[data-sidebar-row]')) {
    opened = true;
    break;
  }
  await sleep(200);
}
if (!opened) fail('侧栏找不到 List Orders');
for (let i = 0; i < 30; i++) {
  if (await clickText('Send', 'button')) break;
  await sleep(200);
}
let status = null;
for (let i = 0; i < 60; i++) {
  await sleep(250);
  status = await ev(`(() => {
    const s = globalThis.__STORE__?.getState?.();
    if (!s) return null;
    for (const c of s.collections.collections) {
      const walk = (items) => {
        for (const it of items || []) {
          if (it.name === 'List Orders') return it;
          const f = walk(it.items);
          if (f) return f;
        }
      };
      const it = walk(c.items);
      if (it?.response) return it.response.status;
    }
  })()`);
  if (status === 200) break;
}
if (status !== 200) fail(`UI 导入的请求没有 200：${status}`);

if (errors.length) fail(`渲染期异常：${errors.join('\n')}`);
console.log('OK 请求 200，UI 导入链路通过');
ws.close();
