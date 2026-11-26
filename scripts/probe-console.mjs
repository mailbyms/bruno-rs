// 在页面 JS 之前注入 console.error 包装，重载后读取完整错误对象
const PORT = process.env.CDP_PORT || 9236;
const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
const page = list.find((x) => x.type === 'page' && x.url !== 'about:blank');
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));

let id = 0;
const pending = new Map();
ws.onmessage = (e) => {
  const msg = JSON.parse(e.data);
  if (msg.id && pending.has(msg.id)) {
    pending.get(msg.id)(msg);
    pending.delete(msg.id);
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

await call('Page.enable');
await call('Page.addScriptToEvaluateOnNewDocument', {
  source: `
    window.__ERRLOG__ = [];
    const origError = console.error.bind(console);
    console.error = (...args) => {
      try {
        window.__ERRLOG__.push(args.map((a) => {
          if (a instanceof Error) return a.stack || String(a);
          if (typeof a === 'object' && a !== null) {
            try { return JSON.stringify(a, Object.getOwnPropertyNames(a)); } catch (_) { return String(a); }
          }
          return String(a);
        }).join(' | '));
      } catch (_) {}
      origError(...args);
    };
  `
});
await call('Page.reload', { ignoreCache: true });
await new Promise((r) => setTimeout(r, 7000));
console.log('errlog:', JSON.stringify(await ev('window.__ERRLOG__'), null, 2));
ws.close();
