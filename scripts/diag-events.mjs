// 诊断 IPC 事件桥：检查 shim 注册的 handler 数量与实际收到的事件
import http from 'node:http';
const CDP = Number(process.env.CDP_PORT || 9228);

const list = await new Promise((r) =>
  http.get(`http://127.0.0.1:${CDP}/json/list`, (res) => {
    let b = '';
    res.on('data', (c) => (b += c));
    res.on('end', () => r(JSON.parse(b)));
  })
);
const t = list.find((x) => x.type === 'page');
const ws = new WebSocket(t.webSocketDebuggerUrl);
let id = 0;
const pending = new Map();
ws.onmessage = (m) => {
  const j = JSON.parse(m.data);
  if (j.id && pending.has(j.id)) {
    pending.get(j.id)(j);
    pending.delete(j.id);
  }
};
await new Promise((r) => (ws.onopen = r));
const call = (method, params = {}) =>
  new Promise((res) => {
    const i = ++id;
    pending.set(i, res);
    ws.send(JSON.stringify({ id: i, method, params }));
  });
const evalExpr = async (expression) => {
  const j = await call('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
  return j.result?.result?.value;
};

console.log('--- shim 事件日志 ---');
console.log(JSON.stringify(await evalExpr('window.__EVLOG__ || []'), null, 2));
console.log('--- 各事件 handler 数量 ---');
console.log(
  JSON.stringify(
    await evalExpr(
      `(() => { const m = window.__SHIM_HANDLERS__ || new Map(); const o = {}; for (const [k, v] of m) o[k] = v.size; return o; })()`
    ),
    null,
    2
  )
);
console.log('--- 页面文本 ---');
console.log(await evalExpr('document.body.innerText.slice(0, 250)'));
ws.close();
