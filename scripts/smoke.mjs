// 里程碑 1 UI 冒烟：侧栏集合树 → 打开请求 → 点 Send → 断言 200 + JSON body
// 用法：先启动应用并开启 CDP，再 `node scripts/smoke.mjs`
const PORT = process.env.CDP_PORT || 9236;
const EXPECT = process.env.SMOKE_EXPECT || '200 OK';

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
  if (j.result?.exceptionDetails) return { err: j.result.exceptionDetails.exception?.description };
  return j.result?.result?.value;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const fail = (msg) => {
  console.error('FAIL:', msg);
  process.exit(1);
};

await call('Runtime.enable');

// 1. 首帧：等集合树出现
let rows = [];
for (let i = 0; i < 60; i++) {
  rows = JSON.parse(
    (await ev("JSON.stringify([...document.querySelectorAll('[data-sidebar-row]')].map(e=>e.innerText.split(String.fromCharCode(10)).join(' | ')))")) || '[]'
  );
  if (rows.some((r) => /GET|POST|PUT|DELETE|PATCH/.test(r))) break;
  await sleep(250);
}
if (!rows.length) fail('侧栏没有集合行');
if (!rows.some((r) => /GET|POST|PUT|DELETE|PATCH/.test(r))) fail(`集合树里没有请求：${JSON.stringify(rows)}`);

// 2. 没有请求编辑区就点第一个请求行（启动后默认落在工作区 Overview）
const hasSend = async () =>
  Boolean(await ev("Boolean([...document.querySelectorAll('button')].find((x) => x.innerText.trim() === 'Send'))"));
if (!(await hasSend())) {
  const pos = JSON.parse(
    (await ev(`(() => {
      const row = [...document.querySelectorAll('[data-sidebar-row]')].find((e) => /GET|POST|PUT|DELETE|PATCH/.test(e.innerText));
      if (!row) return null;
      const r = row.getBoundingClientRect();
      return JSON.stringify({ x: r.x + 120, y: r.y + r.height / 2 });
    })()`)) || 'null'
  );
  if (!pos) fail('侧栏里找不到可点击的请求行');
  for (const type of ['mouseMoved', 'mousePressed', 'mouseReleased']) {
    await call('Input.dispatchMouseEvent', { type, x: pos.x, y: pos.y, button: 'left', clickCount: 1 });
  }
  for (let i = 0; i < 20 && !(await hasSend()); i++) await sleep(200);
}

// 3. 点 Send
const sendPos = await ev(`(() => {
  const b = [...document.querySelectorAll('button')].find((x) => x.innerText.trim() === 'Send');
  if (!b) return null;
  const c = b.getBoundingClientRect();
  return JSON.stringify({ x: c.x + c.width / 2, y: c.y + c.height / 2 });
})()`);
if (!sendPos) fail('找不到 Send 按钮（请求标签页可能没打开）');
const sp = JSON.parse(sendPos);
for (const type of ['mouseMoved', 'mousePressed', 'mouseReleased']) {
  await call('Input.dispatchMouseEvent', { type, x: sp.x, y: sp.y, button: 'left', clickCount: 1 });
}

// 4. 断言响应
let text = '';
for (let i = 0; i < 40; i++) {
  await sleep(250);
  text = String((await ev('document.body.innerText')) || '');
  if (text.includes(EXPECT)) break;
}
if (!text.includes(EXPECT)) fail(`响应区没有 "${EXPECT}"，当前文本片段：${text.slice(0, 400)}`);
// 响应面板是 CodeMirror，innerText 里会夹行号，所以只找键值片段
const hasBody = /"ok":\s*true/.test(text);
if (!hasBody) fail('响应体不是预期的 {"ok": true}');
if (errors.length) fail(`渲染期出现异常：${errors.join('\n')}`);

console.log('OK 侧栏行：', rows.join(' / '));
console.log(`OK 响应包含 ${EXPECT} 与 {"ok": true}`);
ws.close();
