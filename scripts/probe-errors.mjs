// 抓取页面运行时异常栈：先注册监听，再通过真实鼠标事件点开指定请求
import fs from 'node:fs';

const PORT = process.env.CDP_PORT || 9236;
const target = (process.argv[2] || 'Create Order').trim();

const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
const page = list.find((x) => x.type === 'page');
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));

let id = 0;
const pending = new Map();
const exceptions = [];
ws.onmessage = (e) => {
  const msg = JSON.parse(e.data);
  if (msg.id && pending.has(msg.id)) {
    pending.get(msg.id)(msg);
    pending.delete(msg.id);
    return;
  }
  if (msg.method === 'Runtime.exceptionThrown') {
    const d = msg.params.exceptionDetails;
    exceptions.push({
      text: d.text,
      desc: d.exception?.description,
      url: d.url,
      line: d.lineNumber,
      stack: d.stackTrace?.callFrames?.slice(0, 40).map((f) => `${f.functionName} @ ${f.url}:${f.lineNumber}`)
    });
  }
  if (msg.method === 'Runtime.consoleAPICalled' && msg.params.type === 'error') {
    exceptions.push({ console: msg.params.args.map((a) => a.value ?? a.description).join(' ') });
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

await call('Runtime.enable');
await new Promise((r) => setTimeout(r, 300));

const pos = await ev(`(() => {
  const row = [...document.querySelectorAll('[data-sidebar-row]')].find((e) => e.innerText.includes(${JSON.stringify(target)}));
  if (!row) return null;
  const r = row.getBoundingClientRect();
  return JSON.stringify({ x: r.x + 120, y: r.y + r.height / 2 });
})()`);
console.log('row position:', pos);
const p = JSON.parse(pos || 'null');
if (p) {
  for (const type of ['mouseMoved', 'mousePressed', 'mouseReleased']) {
    await call('Input.dispatchMouseEvent', { type, x: p.x, y: p.y, button: 'left', clickCount: 1 });
  }
  await new Promise((r) => setTimeout(r, 1500));
}
console.log('exceptions:', JSON.stringify(exceptions, null, 2));
console.log('page text:', (await ev('document.body.innerText.slice(0,300)')));
fs.writeFileSync('target/errors.json', JSON.stringify(exceptions, null, 2));
ws.close();
