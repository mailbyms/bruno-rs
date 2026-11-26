// 临时诊断脚本：查看集合行 DOM，定位展开控件
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
  const nodes = [...document.querySelectorAll('*')].filter((el) =>
    el.children.length === 0 && el.textContent.trim() === 'Bruno RS Demo'
  );
  return nodes.map((el) => {
    let p = el;
    const chevrons = [...p.parentElement.parentElement.querySelectorAll('[data-testid="collection-chevron"]')];
    const rects = chevrons.map((c) => c.getBoundingClientRect().toJSON());
    const chain = [{ rects }, ...[]];
    for (let i = 0; i < 4 && p; i++) {
      chain.push({ tag: p.tagName, cls: p.className, text: p.innerText?.slice(0, 200), html: p.outerHTML.slice(0, 4000) });
      p = p.parentElement;
    }
    return chain;
  });
})()`;
const reply = await call('Runtime.evaluate', { expression, returnByValue: true });
console.log(JSON.stringify(reply.result?.result?.value, null, 2));
ws.close();
