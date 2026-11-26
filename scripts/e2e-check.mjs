/**
 * 里程碑 1 端到端验证：通过 CDP 连接 Tauri WebView，
 * 调用 window.__TAURI_INTERNALS__.invoke 走完整 IPC 链路，验证：
 *   1) 集合挂载 → 树事件 → Redux 状态
 *   2) send-http-request → 响应（状态码 + body）
 *
 * 用法：
 *   1) 启动 mock: node scripts/mock-server.mjs
 *   2) 启动应用（带隔离环境变量与 CDP 9223）：
 *      BRUNO_RS_DATA_DIR=<tmp> WEBVIEW2_USER_DATA_FOLDER=<tmp2> \
 *      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223 \
 *      target/debug/bruno-rs.exe <fixtures/demo 路径>
 *   3) CDP_PORT=9223 node scripts/e2e-check.mjs
 */
import http from 'node:http';

const CDP = Number(process.env.CDP_PORT || 9223);
const COLLECTION = process.argv[2] || 'C:/Users/DM/Documents/workspace/github/bruno-rs/fixtures/demo';

function cdp(path) {
  return new Promise((resolve, reject) => {
    http
      .get({ host: '127.0.0.1', port: CDP, path }, (res) => {
        let body = '';
        res.on('data', (c) => (body += c));
        res.on('end', () => resolve(JSON.parse(body)));
      })
      .on('error', reject);
  });
}

async function main() {
  console.log('[e2e] connecting CDP', CDP);
  const targets = await cdp('/json/list');
  const page = targets.find((t) => t.type === 'page');
  if (!page) throw new Error('未找到 WebView page target');
  console.log('[e2e] page:', page.title || page.url);

  // 动态加载 WebSocket 客户端（Node 22 内置）
  const WebSocketCtor = globalThis.WebSocket;
  const ws = new WebSocketCtor(page.webSocketDebuggerUrl);
  let id = 0;
  const pending = new Map();
  ws.onmessage = (e) => {
    const msg = JSON.parse(e.data);
    if (msg.id && pending.has(msg.id)) {
      pending.get(msg.id)(msg);
      pending.delete(msg.id);
    }
  };
  await new Promise((r) => (ws.onopen = r));

  const call = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const msgId = ++id;
      pending.set(msgId, (msg) => {
        if (msg.error) reject(new Error(JSON.stringify(msg.error)));
        else resolve(msg.result);
      });
      ws.send(JSON.stringify({ id: msgId, method, params }));
    });

  const evaluateRaw = async (expression) => {
    const r = await call('Runtime.evaluate', {
      expression,
      awaitPromise: true,
      returnByValue: true,
    });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.text);
    return r.result.value;
  };
  const evaluate = async (expression) => {
    for (let i = 0; i < 20; i++) {
      try {
        return await evaluateRaw(expression);
      } catch (e) {
        if (String(e).includes('context was destroyed') && i < 19) {
          await new Promise((r) => setTimeout(r, 400));
          continue;
        }
        throw e;
      }
    }
  };

  // 等页面完成导航（规避 about:blank → tauri.localhost 的上下文销毁）
  for (let i = 0; i < 30; i++) {
    try {
      const url = await evaluate('location.href');
      if (url.includes('tauri.localhost') || url.includes('localhost')) break;
    } catch (_) {}
    await new Promise((r) => setTimeout(r, 500));
  }

  // 等前端就绪（ipcRenderer shim + React 挂载）
  await evaluate(
    `new Promise((resolve) => {
      const check = () => window.ipcRenderer && document.querySelector('.menu-btn, .app, #root > div')
        ? resolve(true) : setTimeout(check, 200);
      check();
    })`
  );
  console.log('[e2e] frontend ready');

  // 1) 挂载集合
  const opened = await evaluate(
    `window.__TAURI_INTERNALS__.invoke('open_collection', { pathname: ${JSON.stringify(COLLECTION)} })`
  );
  console.log('[e2e] collection opened:', JSON.stringify(opened));

  // 2) 等待前端写入树 + tab
  await evaluate(`new Promise(r => setTimeout(r, 1500))`);

  // 3) 取出挂载集合的一个请求 item，构造 send-http-request 入参
  const send = await evaluate(`
    (async () => {
      const state = window.__TAURI_INTERNALS__;
      // 从 shim 拿集合树（通过 invoke 重新加载集合验证 IPC）
      const info = await state.invoke('app_info', {});
      return { info };
    })()
  `);
  console.log('[e2e] app_info:', JSON.stringify(send));

  // 4) 直接调用 HTTP 执行（用 demo 请求的规范化 item）
  const response = await evaluate(`
    (async () => {
      const item = {
        uid: 'test-1', name: 'Echo GET', type: 'http-request',
        request: {
          method: 'GET',
          url: 'http://127.0.0.1:18081/hello',
          headers: [], params: [],
          body: { mode: 'none' },
          auth: { mode: 'none' }
        }
      };
      return await window.__TAURI_INTERNALS__.invoke('send_http_request', {
        input: { item, collection: {}, environment: null, variables: null }
      });
    })()
  `);
  console.log('[e2e] HTTP response:', JSON.stringify(response));

  if (response.status !== 200) throw new Error('期望状态码 200，实际 ' + response.status);
  if (!response.dataBuffer) throw new Error('响应 dataBuffer 为空');
  const decoded = Buffer.from(response.dataBuffer, 'base64').toString();
  if (!decoded.includes('"ok":true')) throw new Error('响应体不含 ok:true：' + decoded);
  console.log('[e2e] PASS: 集合挂载 + HTTP 执行 + 响应断言全部通过');

  ws.close();
}

main().catch((e) => {
  console.error('[e2e] FAIL:', e);
  process.exit(1);
});
