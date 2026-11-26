// 最小 WebSocket echo 服务（不依赖任何 npm 包）。
// 收到文本帧原样回 "echo:<text>"，收到 ping 回 pong，收到 close 回 close 并断开。
// 端口可用 WS_PORT 覆盖，默认 18082。
import crypto from 'node:crypto';
import http from 'node:http';

const PORT = Number(process.env.WS_PORT || 18082);
const GUID = '258EAFA5-E914-47DA-95CA-C5AB0DC85B11';

function encodeFrame(opcode, payload) {
  const length = payload.length;
  let header;
  if (length < 126) {
    header = Buffer.from([0x80 | opcode, length]);
  } else if (length < 65536) {
    header = Buffer.alloc(4);
    header[0] = 0x80 | opcode;
    header[1] = 126;
    header.writeUInt16BE(length, 2);
  } else {
    header = Buffer.alloc(10);
    header[0] = 0x80 | opcode;
    header[1] = 127;
    header.writeBigUInt64BE(BigInt(length), 2);
  }
  return Buffer.concat([header, payload]);
}

function decodeFrames(buffer, onFrame, onError) {
  let offset = 0;
  while (offset + 2 <= buffer.length) {
    const first = buffer[offset];
    const second = buffer[offset + 1];
    const opcode = first & 0x0f;
    const masked = (second & 0x80) !== 0;
    let length = second & 0x7f;
    let cursor = offset + 2;
    if (length === 126) {
      if (cursor + 2 > buffer.length) break;
      length = buffer.readUInt16BE(cursor);
      cursor += 2;
    } else if (length === 127) {
      if (cursor + 8 > buffer.length) break;
      length = Number(buffer.readBigUInt64BE(cursor));
      cursor += 8;
    }
    const maskKey = masked ? buffer.subarray(cursor, cursor + 4) : null;
    if (masked) cursor += 4;
    if (cursor + length > buffer.length) break;
    const payload = Buffer.from(buffer.subarray(cursor, cursor + length));
    if (masked) {
      for (let i = 0; i < payload.length; i += 1) payload[i] ^= maskKey[i % 4];
    }
    offset = cursor + length;
    try {
      onFrame(opcode, payload);
    } catch (error) {
      onError(error);
      return offset;
    }
  }
  return offset;
}

const server = http.createServer((req, res) => {
  res.writeHead(200, { 'content-type': 'text/plain' });
  res.end('ws-echo-server\n');
});

server.on('upgrade', (req, socket) => {
  const key = req.headers['sec-websocket-key'];
  if (!key) {
    socket.destroy();
    return;
  }
  const accept = crypto.createHash('sha1').update(key + GUID).digest('base64');
  socket.write(
    ['HTTP/1.1 101 Switching Protocols', 'Upgrade: websocket', 'Connection: Upgrade', `Sec-WebSocket-Accept: ${accept}`, '', ''].join(
      '\r\n'
    )
  );

  let pending = Buffer.alloc(0);
  const close = () => {
    try {
      socket.end();
    } catch {
      /* 已断开 */
    }
  };

  socket.on('data', (chunk) => {
    pending = Buffer.concat([pending, chunk]);
    const consumed = decodeFrames(
      pending,
      (opcode, payload) => {
        if (opcode === 0x1) {
          console.log('[ws] text:', payload.toString('utf8'));
          socket.write(encodeFrame(0x1, Buffer.from(`echo:${payload.toString('utf8')}`, 'utf8')));
        } else if (opcode === 0x2) {
          console.log('[ws] binary:', payload.length, 'bytes');
          socket.write(encodeFrame(0x2, payload));
        } else if (opcode === 0x8) {
          socket.write(encodeFrame(0x8, Buffer.alloc(0)));
          close();
        } else if (opcode === 0x9) {
          socket.write(encodeFrame(0xa, payload));
        }
      },
      (error) => console.error('[ws] frame error', error)
    );
    pending = pending.subarray(consumed);
  });
  socket.on('error', () => close());
  socket.on('close', close);
});

server.listen(PORT, '127.0.0.1', () => {
  console.log(`ws echo server listening on ws://127.0.0.1:${PORT}`);
});
