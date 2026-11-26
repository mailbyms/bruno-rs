// 重载页面并收集启动期间的异常/错误日志
const PORT = process.env.CDP_PORT || 9236;
const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
const page = list.find((x) => x.type === 'page' && x.url !== 'about:blank');
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));

let id = 0;
const pending = new Map();
const logs = [];
ws.onmessage = (e) => {
  const msg = JSON.parse(e.data);
  if (msg.id && pending.has(msg.id)) {
    pending.get(msg.id)(msg);
    pending.delete(msg.id);
    return;
  }
  if (msg.method === 'Runtime.exceptionThrown') {
    const d = msg.params.exceptionDetails;
    logs.push({ kind: 'exception', text: d.exception?.description || d.text });
  }
  if (msg.method === 'Runtime.consoleAPICalled' && ['error', 'warning'].includes(msg.params.type)) {
    logs.push({ kind: msg.params.type, text: msg.params.args.map((a) => a.value ?? a.description).join(' ') });
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
await call('Page.enable');
await call('Page.reload', { ignoreCache: true });
await new Promise((r) => setTimeout(r, 6000));

console.log('startup logs:', JSON.stringify(logs, null, 2));
console.log('rows:', await ev("JSON.stringify([...document.querySelectorAll('[data-sidebar-row]')].map(e=>e.innerText.split(String.fromCharCode(10)).join(' | ')))"));
console.log('text:', String(await ev('document.body.innerText')).slice(0, 240).replace(/\n/g, ' / '));
ws.close();
