// 临时诊断脚本：抓取 collection-opened / tree-loaded 的实际 payload 与 uid
const CDP = Number(process.env.CDP_PORT || 9236);
const FIXTURE = process.env.FIXTURE || 'C:/Users/DM/Documents/workspace/github/bruno-rs/fixtures/demo';

const list = await (await fetch(`http://127.0.0.1:${CDP}/json/list`)).json();
const target = list.find((x) => x.type === 'page');
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));

let id = 0;
const pending = new Map();
ws.onmessage = (event) => {
  const msg = JSON.parse(event.data);
  if (msg.id && pending.has(msg.id)) {
    pending.get(msg.id)(msg);
    pending.delete(msg.id);
  }
};
const call = (method, params = {}) =>
  new Promise((resolve) => {
    const i = ++id;
    pending.set(i, resolve);
    ws.send(JSON.stringify({ id: i, method, params }));
  });
const evaluate = async (expression) => {
  const reply = await call('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
  if (reply.result?.exceptionDetails) throw new Error(reply.result.exceptionDetails.text);
  return reply.result?.result?.value;
};

await evaluate(`(() => {
  window.__COLL_EVTS__ = [];
  window.ipcRenderer.on('main:collection-opened', (...args) => {
    window.__COLL_EVTS__.push({ event: 'main:collection-opened', args });
  });
  window.ipcRenderer.on('main:collection-tree-loaded', (payload) => {
    window.__COLL_EVTS__.push({
      event: 'main:collection-tree-loaded',
      collectionUid: payload?.collectionUid,
      items: (payload?.tree?.items || []).map((i) => ({ uid: i.uid, name: i.name, type: i.type, pathname: i.pathname })),
    });
  });
  return true;
})()`);

await evaluate(`window.__TAURI_INTERNALS__.invoke('open_collection', { pathname: ${JSON.stringify(FIXTURE)} })`);
await new Promise((r) => setTimeout(r, 2600));

console.log(JSON.stringify(await evaluate('window.__COLL_EVTS__ || []'), null, 2));
ws.close();
