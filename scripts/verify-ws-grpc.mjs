// WebSocket / gRPC 端到端验证（IPC + UI）。
//
// 前置：
//   node scripts/ws-echo-server.mjs &                          # 127.0.0.1:18082
//   BRUNO_RS_DATA_DIR=<tmp> WEBVIEW2_USER_DATA_FOLDER=<tmp> \
//     WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9244 \
//     ./target/debug/bruno-rs.exe
//   node scripts/verify-ws-grpc.mjs
//
// gRPC 侧不依赖真实服务：用 proto 校验方法枚举 / 示例消息，用不可达端口校验错误事件。
import path from 'node:path';

const PORT = Number(process.env.CDP_PORT || 9244);
const WS_URL = process.env.WS_URL || 'ws://127.0.0.1:18082';
const COLL = path.resolve('fixtures/protocols').replace(/\\/g, '/');
const PROTO = `${COLL}/echo.proto`;

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
ws.onmessage = (e) => {
  const m = JSON.parse(e.data);
  if (m.id && pending.has(m.id)) {
    pending.get(m.id)(m);
    pending.delete(m.id);
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
  if (j.result?.exceptionDetails) {
    const d = j.result.exceptionDetails;
    throw new Error(d.exception?.description || d.text || 'eval failed');
  }
  return j.result?.result?.value;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const invoke = async (channel, ...args) => {
  const expr = `(async () => {
    try {
      return JSON.stringify({ ok: true, value: await window.ipcRenderer.invoke(${JSON.stringify(channel)}, ...${JSON.stringify(args)}) });
    } catch (e) {
      return JSON.stringify({ ok: false, error: String((e && (e.message || e.error)) || e) });
    }
  })()`;
  const raw = await Promise.race([
    ev(expr),
    new Promise((_, rej) => setTimeout(() => rej(new Error(`[${channel}] 调用超时`)), 60000))
  ]);
  const parsed = JSON.parse(raw);
  if (!parsed.ok) throw new Error(`[${channel}] ${parsed.error}`);
  return parsed.value;
};
const waitFor = async (label, expression, timeout = 15000) => {
  const started = Date.now();
  while (Date.now() - started < timeout) {
    const value = await ev(expression);
    if (value) return value;
    await sleep(150);
  }
  throw new Error(`等待超时：${label}`);
};

let pass = 0;
const failures = [];
const ok = (name, cond, detail = '') => {
  if (cond) {
    pass += 1;
    console.log(`  PASS  ${name}`);
  } else {
    failures.push(`${name}${detail ? ` :: ${detail}` : ''}`);
    console.log(`  FAIL  ${name}${detail ? ` :: ${detail}` : ''}`);
  }
};
const section = (t) => console.log(`\n== ${t} ==`);

await call('Runtime.enable');
await waitFor('应用就绪', '!!window.ipcRenderer && !!window.__BRUNO_RS__', 60000);

// 事件录像：WS / gRPC 的结果全走事件，先装上再触发
await ev(`(() => {
  if (window.__PROTO_EVENTS__) return true;
  window.__PROTO_EVENTS__ = [];
  const names = [
    'main:ws:connecting', 'main:ws:upgrade', 'main:ws:request', 'main:ws:open',
    'main:ws:message', 'main:ws:close', 'main:ws:error', 'main:ws:disconnecting',
    'main:ws:connections-changed',
    'grpc:request', 'grpc:response', 'grpc:metadata', 'grpc:status', 'grpc:error',
    'grpc:server-end-stream', 'grpc:connections-changed'
  ];
  for (const name of names) {
    window.ipcRenderer.on(name, (...args) => window.__PROTO_EVENTS__.push({ name, args }));
  }
  return true;
})()`);
const eventsOf = (name) => ev(`window.__PROTO_EVENTS__.filter((e) => e.name === ${JSON.stringify(name)})`);
const waitEvent = async (name, timeout = 20000) => {
  const started = Date.now();
  while (Date.now() - started < timeout) {
    const list = await eventsOf(name);
    if (list.length) return list[0];
    await sleep(150);
  }
  return null;
};

section('集合加载：ws / grpc 请求出现在树里');
const open = await invoke('renderer:open-collection', COLL);
ok('open-collection 返回 uid', Boolean(open?.uid), JSON.stringify(open));

const tree = await waitFor(
  '集合树',
  `(() => {
    const host = document.querySelector('#root') || document.body;
    const key = Object.keys(host).find((k) => k.startsWith('__reactContainer$'));
    const seen = new Set();
    let store = null;
    const walk = (fiber) => {
      if (!fiber || seen.has(fiber) || store) return;
      seen.add(fiber);
      const props = fiber.memoizedProps;
      const candidate = props?.store || props?.value?.store || (props?.value?.getState ? props.value : null);
      if (candidate && typeof candidate.getState === 'function') {
        const state = candidate.getState();
        if (state && state.collections && state.tabs) store = candidate;
      }
      walk(fiber.child);
      walk(fiber.sibling);
    };
    walk(key ? host[key] : null);
    if (!store) return null;
    const state = store.getState();
    const collection = (state.collections.collections || []).find((c) => (c.pathname || '').replace(/\\\\/g, '/') === ${JSON.stringify(COLL)});
    if (!collection) return null;
    // open-collection 先返回 uid，树是随后由 main:collection-tree-loaded 灌进 store 的；
    // items 还是空数组时继续等，否则这里会稳定地断到 []。
    const items = (collection.items || []).map((i) => ({ name: i.name, type: i.type }));
    if (!items.length) return null;
    window.__PROTO_COLLECTION__ = collection;
    return items;
  })()`,
  30000
);
ok(
  '树里有 websocket 请求',
  tree.some((i) => i.type === 'ws-request'),
  JSON.stringify(tree)
);
ok(
  '树里有 grpc 请求',
  tree.some((i) => i.type === 'grpc-request'),
  JSON.stringify(tree)
);

section('WebSocket：连接 → 发消息 → 收回显 → 断开');
const wsResult = await invoke('renderer:ws:start-connection', {
  request: {
    uid: 'ws-verify-uid',
    request: {
      url: WS_URL,
      headers: [{ name: 'x-client', value: 'verify', enabled: true }],
      body: { mode: 'ws', ws: [{ name: 'ping', type: 'json', content: '{"hello":"world"}' }] },
      auth: { mode: 'none' },
      vars: { req: [] }
    }
  },
  collection: await ev('window.__PROTO_COLLECTION__'),
  environment: null,
  runtimeVariables: null,
  settings: { timeout: 5000, keepAliveInterval: 0 },
  options: { connectOnly: true }
});
ok('start-connection 返回 success', wsResult?.success === true, JSON.stringify(wsResult));

const openEvent = await waitEvent('main:ws:open');
ok('收到 main:ws:open', Boolean(openEvent), JSON.stringify(openEvent));
ok(
  'open 事件是三元组 [requestId, collectionUid, eventData]',
  Array.isArray(openEvent?.args) && openEvent.args[0] === 'ws-verify-uid' && typeof openEvent.args[2] === 'object',
  JSON.stringify(openEvent)
);

const status = await invoke('renderer:ws:connection-status', 'ws-verify-uid');
ok('connection-status = connected', status?.status === 'connected', JSON.stringify(status));
const active = await invoke('renderer:ws:is-connection-active', 'ws-verify-uid');
ok('is-connection-active = {success, isActive:true}', active?.success === true && active.isActive === true, JSON.stringify(active));

const queued = await invoke('renderer:ws:queue-message', {
  item: {
    uid: 'ws-verify-uid',
    request: {
      url: WS_URL,
      headers: [],
      body: { mode: 'ws', ws: [{ name: 'ping', type: 'json', content: '{"hello":"world"}' }] },
      auth: { mode: 'none' }
    }
  },
  collection: await ev('window.__PROTO_COLLECTION__'),
  environment: null,
  runtimeVariables: null,
  selectedMessageIndex: 0
});
ok('queue-message 返回 success', queued?.success === true, JSON.stringify(queued));

const incoming = await waitFor(
  '服务端回显',
  `(window.__PROTO_EVENTS__ || []).find((e) => e.name === 'main:ws:message' && e.args?.[2]?.type === 'incoming') || null`,
  15000
);
ok(
  '收到 incoming 消息（echo: 回显）',
  typeof incoming?.args?.[2]?.message === 'string' && incoming.args[2].message.startsWith('echo:'),
  JSON.stringify(incoming)
);
const outgoing = await ev(
  `(window.__PROTO_EVENTS__ || []).some((e) => e.name === 'main:ws:message' && e.args?.[2]?.type === 'outgoing')`
);
ok('outgoing 消息也进了时间线', outgoing === true);

// send-message 的第三个参数是整条消息对象，不是裸字符串：这里专门盯住「别把 JSON 外壳发出去」
await ev('(() => { window.__PROTO_EVENTS__.length = 0; return true; })()');
const direct = await invoke('renderer:ws:send-message', 'ws-verify-uid', open?.uid, {
  name: 'direct',
  type: 'text',
  content: 'direct-ping'
});
ok('send-message 返回 success', direct?.success === true, JSON.stringify(direct));
const directEcho = await waitFor(
  'direct 回显',
  `(window.__PROTO_EVENTS__ || []).find((e) => e.name === 'main:ws:message' && e.args?.[2]?.type === 'incoming') || null`,
  15000
);
ok(
  'send-message 只发 content（不带 JSON 外壳）',
  directEcho?.args?.[2]?.message === 'echo:direct-ping',
  JSON.stringify(directEcho?.args?.[2]?.message)
);

const closed = await invoke('renderer:ws:close-connection', 'ws-verify-uid');
ok('close-connection 返回 success', closed?.success === true, JSON.stringify(closed));
await waitEvent('main:ws:close', 10000);
const afterClose = await invoke('renderer:ws:connection-status', 'ws-verify-uid');
ok('断开后 status = disconnected', afterClose?.status === 'disconnected', JSON.stringify(afterClose));

section('WebSocket：不可达地址报错而不是挂住');
await invoke('renderer:ws:start-connection', {
  request: {
    uid: 'ws-verify-bad',
    request: { url: 'ws://127.0.0.1:1', headers: [], body: { mode: 'ws', ws: [] }, auth: { mode: 'none' } }
  },
  collection: await ev('window.__PROTO_COLLECTION__'),
  environment: null,
  runtimeVariables: null,
  settings: { timeout: 3000 },
  options: { connectOnly: true }
});
const wsError = await waitFor(
  'WS 错误事件',
  `(window.__PROTO_EVENTS__ || []).find((e) => e.name === 'main:ws:error' && e.args?.[0] === 'ws-verify-bad') || null`,
  15000
);
ok('连接失败发出 main:ws:error', Boolean(wsError), JSON.stringify(wsError));

section('gRPC：proto 方法枚举 / 示例消息');
const methods = await invoke('grpc:load-methods-proto', {
  filePath: PROTO,
  collection: await ev('window.__PROTO_COLLECTION__')
});
ok('load-methods-proto 无 error', !methods?.error, JSON.stringify(methods?.error));
const paths = (methods?.methods || []).map((m) => m.path);
ok('列出 4 个方法', paths.length === 4, JSON.stringify(paths));
ok('包含 /echo.Echo/Unary', paths.includes('/echo.Echo/Unary'), JSON.stringify(paths));

const unary = (methods?.methods || []).find((m) => m.path === '/echo.Echo/Unary') || {};
const sample = await invoke('grpc:generate-sample-message', {
  methodPath: '/echo.Echo/Unary',
  existingMessage: '{"message":"keep"}',
  options: { arraySize: 2, methodMetadata: unary }
});
ok('generate-sample-message 成功', sample?.success === true, JSON.stringify(sample));
let sampleBody = null;
try {
  sampleBody = JSON.parse(sample.message);
} catch (error) {
  sampleBody = null;
}
ok(
  '示例消息保留已有值并补齐字段',
  sampleBody?.message === 'keep' && sampleBody?.count === 0,
  JSON.stringify(sampleBody)
);

const grpcurl = await invoke('grpc:generate-grpcurl', {
  request: {
    request: {
      url: '127.0.0.1:18083',
      method: '/echo.Echo/Unary',
      protoPath: 'echo.proto',
      headers: [{ name: 'x-trace', value: 'abc', enabled: true }],
      body: { mode: 'grpc', grpc: [{ name: 'message 1', content: '{"message":"hi"}' }] }
    }
  },
  collection: await ev('window.__PROTO_COLLECTION__'),
  environment: null,
  runtimeVariables: null
});
ok(
  'generate-grpcurl 产出命令',
  typeof grpcurl === 'string' && grpcurl.startsWith('grpcurl') && grpcurl.includes('/echo.Echo/Unary'),
  JSON.stringify(grpcurl)
);

section('gRPC：不可达服务 → grpc:error code 14');
await invoke('grpc:start-connection', {
  request: {
    uid: 'grpc-verify-uid',
    request: {
      url: '127.0.0.1:18083',
      method: '/echo.Echo/Unary',
      methodType: 'unary',
      protoPath: 'echo.proto',
      headers: [],
      body: { mode: 'grpc', grpc: [{ name: 'message 1', content: '{"message":"hi"}' }] },
      auth: { mode: 'none' }
    }
  },
  collection: await ev('window.__PROTO_COLLECTION__'),
  environment: null,
  runtimeVariables: null
});
const grpcError = await waitFor(
  'gRPC 错误事件',
  `(window.__PROTO_EVENTS__ || []).find((e) => e.name === 'grpc:error' && e.args?.[0] === 'grpc-verify-uid') || null`,
  25000
);
ok('grpc:error 事件到达', Boolean(grpcError), JSON.stringify(grpcError));
ok(
  '错误码为 14 (UNAVAILABLE)',
  grpcError?.args?.[2]?.error?.code === 14,
  JSON.stringify(grpcError?.args?.[2]?.error)
);
const grpcEnd = await waitEvent('grpc:server-end-stream', 10000);
ok('收到 grpc:server-end-stream', Boolean(grpcEnd));

section('UI：WS / gRPC 面板能打开');
// 侧栏行是 React 组件，必须走真实鼠标事件（合成 MouseEvent 不会触发 onClick 链）
const clickRow = async (name) => {
  const raw = await ev(`(() => {
    const row = [...document.querySelectorAll('[data-sidebar-row]')]
      .find((el) => (el.innerText || '').includes(${JSON.stringify(name)}));
    if (!row) return null;
    const r = row.getBoundingClientRect();
    return JSON.stringify({ x: r.x + 40, y: r.y + r.height / 2 });
  })()`);
  if (!raw) return false;
  const point = JSON.parse(raw);
  for (const type of ['mouseMoved', 'mousePressed', 'mouseReleased']) {
    await call('Input.dispatchMouseEvent', { type, x: point.x, y: point.y, button: 'left', clickCount: 1 });
  }
  await sleep(600);
  return true;
};
const clickedWs = await clickRow('WS Echo');
if (clickedWs) {
  const hasWsPane = await waitFor(
    'WS 面板',
    `!!document.querySelector('[data-testid="ws-connect-button"]')`,
    10000
  ).catch(() => false);
  ok('点 WS 请求打开 WS 查询面板', Boolean(hasWsPane));
} else {
  ok('点 WS 请求打开 WS 查询面板', false, '侧栏里没找到 WS Echo 行');
}

const clickedGrpc = await clickRow('gRPC Echo');
if (clickedGrpc) {
  const hasGrpcPane = await waitFor(
    'gRPC 面板',
    `!!document.querySelector('[data-testid="grpc-query-url-container"]')`,
    10000
  ).catch(() => false);
  ok('点 gRPC 请求打开 gRPC 查询面板', Boolean(hasGrpcPane));
} else {
  ok('点 gRPC 请求打开 gRPC 查询面板', false, '侧栏里没找到 gRPC Echo 行');
}

console.log(`\n${pass} 通过 / ${failures.length} 失败`);
for (const failure of failures) console.log(`  - ${failure}`);
ws.close();
process.exit(failures.length ? 1 : 0);
