// 极简本地回显服务器，用于里程碑 1 端到端验证（不依赖外网）
import http from 'node:http';

const PORT = Number(process.env.MOCK_PORT || 18081);

http
  .createServer((req, res) => {
    const chunks = [];
    req.on('data', (c) => chunks.push(c));
    req.on('end', () => {
      const body = Buffer.concat(chunks).toString();
      const path = (req.url || '/').split('?')[0];
      // /cookie/set：下发两个 cookie，用于跨请求 cookie 验证
      if (path === '/cookie/set') {
        res.writeHead(200, {
          'content-type': 'application/json',
          'set-cookie': ['sid=abc123; Path=/; HttpOnly', 'theme=dark; Path=/']
        });
        res.end(JSON.stringify({ ok: true, set: ['sid', 'theme'] }));
        return;
      }
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(
        JSON.stringify({
          ok: true,
          method: req.method,
          url: req.url,
          headers: req.headers,
          body,
        })
      );
    });
  })
  .listen(PORT, '127.0.0.1', () => {
    console.log(`[mock] listening on http://127.0.0.1:${PORT}`);
  });
