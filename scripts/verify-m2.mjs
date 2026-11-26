// 里程碑 2 验证：脚本（before-request/tests）、Tests 页签、保存落盘与新字段保留
// 用法：应用以 CDP 9237 启动、mock server 在 18081，然后 node scripts/verify-m2.mjs
import fs from 'node:fs';
import path from 'node:path';

const PORT = Number(process.env.CDP_PORT || 9237);
const FIXTURE = process.env.FIXTURE_DIR || path.resolve('fixtures/scripted');
const TARGET_FILE = path.join(FIXTURE, 'scripted.yml');
const ORIGINAL_YAML = fs.existsSync(TARGET_FILE) ? fs.readFileSync(TARGET_FILE, 'utf8') : null;
const restore = () => {
  if (ORIGINAL_YAML !== null) fs.writeFileSync(TARGET_FILE, ORIGINAL_YAML);
};

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
  restore();
  console.error('FAIL:', msg);
  process.exit(1);
};

// 从 React fiber 里拿 Redux store（与应用自身用的是同一个）
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

// 1. 侧栏点 Scripted Echo
let opened = false;
for (let i = 0; i < 40; i++) {
  const rows = await ev("JSON.stringify([...document.querySelectorAll('[data-sidebar-row]')].map(e=>e.innerText))");
  if (String(rows).includes('Scripted Echo')) {
    opened = await clickText('Scripted Echo', '[data-sidebar-row]');
    break;
  }
  await sleep(250);
}
if (!opened) fail('侧栏找不到 Scripted Echo（集合是否已打开？）');

// 2. Send
for (let i = 0; i < 30; i++) {
  if (await clickText('Send')) break;
  await sleep(200);
}

// 3. 等响应 + 脚本结果
let result = null;
for (let i = 0; i < 60; i++) {
  await sleep(250);
  result = await ev(`(() => {
    const s = globalThis.__STORE__.getState();
    for (const c of s.collections.collections) {
      const walk = (items) => {
        for (const it of items || []) {
          if (it.name === 'Scripted Echo') return it;
          const found = walk(it.items);
          if (found) return found;
        }
      };
      const it = walk(c.items);
      if (it) return {
        status: it.response?.status,
        tests: (it.testResults || []).map(t => ({ status: t.status, description: t.description, error: t.error })),
        asserts: (it.assertionResults || []).length,
        runtimeVars: c.runtimeVariables || {},
        pathname: it.pathname,
        item: JSON.parse(JSON.stringify(it))
      };
    }
  })()`);
  if (result?.status === 200 && result.tests.length >= 3) break;
}
if (result?.status !== 200) fail(`没有拿到 200 响应：${JSON.stringify(result)}`);
if (result.tests.length !== 3) fail(`testResults 数量不是 3：${JSON.stringify(result.tests)}`);
if (!result.tests.every((t) => t.status === 'pass')) fail(`存在失败测试：${JSON.stringify(result.tests)}`);
if (result.runtimeVars.scriptTag !== 'abc') fail(`runtimeVariables 未回写：${JSON.stringify(result.runtimeVars)}`);

// 4. Tests 结果进入 UI：页签角标显示 3 条 tests + 1 条 assertion = 4，且无失败
await sleep(600);
const badge = String(
  (await ev("document.querySelector('.all-tests-passed')?.textContent || document.querySelector('.some-tests-failed')?.textContent || ''")) || ''
).trim();
if (badge !== '4') fail(`Tests 页签角标不是 4（3 tests + 1 assertion），实际 "${badge}"`);

// 5. 保存：改 URL 后走与 Ctrl+S 相同的 IPC，再读回文件
const saveResult = await ev(`(async () => {
  const item = ${JSON.stringify(result.item)};
  const toSave = {
    uid: item.uid,
    type: item.type,
    name: item.name,
    seq: item.seq,
    settings: item.settings,
    request: { ...item.request, url: 'http://127.0.0.1:18081/saved-by-test' }
  };
  toSave.request.assertions = item.request.assertions;
  const r = await window.ipcRenderer.invoke('renderer:save-request', item.pathname, toSave);
  const loaded = await window.ipcRenderer.invoke('renderer:load-request', item.pathname);
  return { saved: Boolean(r), loadedUrl: loaded?.request?.http?.url };
})()`);
if (!saveResult?.saved) fail(`保存失败：${JSON.stringify(saveResult)}`);
if (saveResult.loadedUrl !== 'http://127.0.0.1:18081/saved-by-test') {
  fail(`保存后 load-request 仍是旧值：${JSON.stringify(saveResult)}`);
}

const raw = fs.readFileSync(TARGET_FILE, 'utf8');
if (!raw.includes('saved-by-test')) fail(`磁盘 YAML 没有写入新 URL：\n${raw}`);
if (!raw.includes('before-request') || !raw.includes('type: tests')) {
  fail(`保存丢了脚本：\n${raw}`);
}
if (!raw.includes('expression: res.status')) fail(`保存丢了断言：\n${raw}`);
if (raw.includes('undefined')) fail(`YAML 里出现 undefined：\n${raw}`);

// 6. multipart 请求体 + 运行时变量跨请求
let mp = null;
if (!(await clickText('Multipart Upload', '[data-sidebar-row]'))) fail('侧栏找不到 Multipart Upload');
for (let i = 0; i < 30; i++) {
  if (await clickText('Send')) break;
  await sleep(200);
}
for (let i = 0; i < 60; i++) {
  await sleep(250);
  mp = await ev(`(() => {
    const s = globalThis.__STORE__.getState();
    for (const c of s.collections.collections) {
      const walk = (items) => {
        for (const it of items || []) {
          if (it.name === 'Multipart Upload') return it;
          const f = walk(it.items);
          if (f) return f;
        }
      };
      const it = walk(c.items);
      if (it && it.response) return { status: it.response.status, body: it.response.data?.body || '' };
    }
  })()`);
  if (mp?.status === 200) break;
}
if (mp?.status !== 200) fail(`multipart 请求没有 200：${JSON.stringify(mp)}`);
if (!mp.body.includes('name="title"') || !mp.body.includes('coffee')) fail(`multipart 字段缺失：${mp.body.slice(0, 300)}`);
if (!mp.body.includes('abc')) fail(`运行时变量没有跨请求生效（{{scriptTag}} 未替换）：${mp.body.slice(0, 300)}`);

if (errors.length) fail(`渲染期异常：${errors.join('\n')}`);

console.log('OK multipart 字段与运行时变量：', mp.body.slice(0, 120).replace(/\n/g, ' '));
console.log('OK Tests:', JSON.stringify(result.tests));
console.log('OK runtimeVariables.scriptTag =', result.runtimeVars.scriptTag);
console.log('OK 保存后 load-request URL =', saveResult.loadedUrl);
console.log('OK YAML 保留脚本与断言');
restore();
ws.close();
