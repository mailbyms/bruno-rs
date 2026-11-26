// 临时诊断脚本：通过 CDP 从 React fiber 中读取 Redux store 状态
const CDP = Number(process.env.CDP_PORT || 9236);

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

const expression = `(() => {
  const host = document.querySelector('#root') || document.body;
  const key = Object.keys(host).find((k) => k.startsWith('__reactContainer$'));
  const root = key ? host[key] : null;
  const seen = new Set();
  const candidates = [];
  let store = null;
  const walk = (fiber) => {
    if (!fiber || seen.has(fiber)) return;
    seen.add(fiber);
    const props = fiber.memoizedProps;
    const maybeStores = [
      props?.store,
      props?.value?.store,
      props?.value?.getState ? props.value : null,
    ].filter(Boolean);
    for (const candidate of maybeStores) {
      if (typeof candidate.getState !== 'function') continue;
      try {
        const state = candidate.getState();
        candidates.push({ keys: Object.keys(state || {}), type: typeof state });
        if (state && state.collections && state.tabs) {
          store = candidate;
        }
      } catch (e) {
        candidates.push({ error: String(e) });
      }
    }
    walk(fiber.child);
    walk(fiber.sibling);
  };
  walk(root);
  if (!store) {
    return { error: 'store not found', candidates, rootTag: host.tagName, keys: Object.keys(host).filter((k) => k.startsWith('__react')) };
  }
  const state = store.getState();
  if (!state || !state.collections) {
    return { error: 'unexpected state shape', keys: Object.keys(state || {}), stateType: typeof state };
  }
  return {
    collections: state.collections.collections.map((c) => ({
      uid: c.uid,
      name: c.name,
      pathname: c.pathname,
      collapsed: c.collapsed,
      items: (c.items || []).map((i) => ({
        uid: i.uid,
        name: i.name,
        type: i.type,
        collapsed: i.collapsed,
        pathname: i.pathname,
      })),
    })),
    workspaces: state.workspaces.workspaces.map((w) => ({
      uid: w.uid,
      pathname: w.pathname,
      active: state.workspaces.activeWorkspaceUid === w.uid,
      collections: w.collections,
    })),
    tabs: state.tabs.tabs.map((t) => ({ uid: t.uid, type: t.type, collectionUid: t.collectionUid })),
  };
})()`;

const reply = await call('Runtime.evaluate', { expression, returnByValue: true });
if (reply.result?.exceptionDetails) {
  console.error(JSON.stringify(reply.result.exceptionDetails, null, 2));
} else {
  console.log(JSON.stringify(reply.result?.result?.value, null, 2));
}
ws.close();
