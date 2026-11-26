// 依次点击侧栏中匹配文本的行（用于展开文件夹 / 打开请求）
const PORT = process.env.CDP_PORT || 9236;
const targets = process.argv.slice(2);

const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
const page = list.find((x) => x.type === 'page' && x.url !== 'about:blank') || list.find((x) => x.type === 'page');
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

for (const target of targets) {
  const raw = await ev(`(() => {
    const row = [...document.querySelectorAll('[data-sidebar-row]')].find((e) => e.innerText.includes(${JSON.stringify(target)}));
    if (!row) return null;
    const chevron = row.querySelector('[data-testid*="chevron"]');
    const r = (chevron && ${JSON.stringify(target)} === ${JSON.stringify(targets[0])} ? chevron : row).getBoundingClientRect();
    return JSON.stringify({ x: r.x + r.width / 2, y: r.y + r.height / 2 });
  })()`);
  if (!raw || raw.err) {
    console.log(`未找到行：${target}`, raw?.err || '');
    continue;
  }
  const p = JSON.parse(raw);
  for (const type of ['mouseMoved', 'mousePressed', 'mouseReleased']) {
    await call('Input.dispatchMouseEvent', { type, x: p.x, y: p.y, button: 'left', clickCount: 1 });
  }
  await new Promise((r) => setTimeout(r, 800));
  console.log(`clicked ${target} @ ${p.x},${p.y}`);
}
console.log('rows:', await ev("JSON.stringify([...document.querySelectorAll('[data-sidebar-row]')].map(e=>e.innerText.split(String.fromCharCode(10)).join(' | ')))"));
console.log('text:', String(await ev('document.body.innerText')).slice(0, 400).replace(/\n/g, ' / '));
ws.close();
