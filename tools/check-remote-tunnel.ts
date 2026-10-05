// node tools/check-remote-tunnel.ts: encrypted round trip through a public WSS relay, no real chats.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash, randomBytes } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { open, seal } from '../mobile/src/remote/crypto.ts';

const WS = createRequire(new URL('../mobile/package.json', import.meta.url))('ws');
const WebSocketServer = WS.Server;
const directory = 'D:/Neru/.local/data/remote-tools';
const file = `${directory}/2026.9.3-cloudflared-windows-amd64.exe`;
mkdirSync(directory, { recursive: true });
if (!existsSync(file)) {
  const response = await fetch('https://github.com/cloudflare/cloudflared/releases/download/2026.9.3/cloudflared-windows-amd64.exe');
  assert(response.ok); writeFileSync(file, Buffer.from(await response.arrayBuffer()));
}
assert.equal(createHash('sha256').update(readFileSync(file)).digest('hex'), 'f096265ec2fcbe9bb6e2d64268db167ced3fcbb83d894bdb9e2fcdb26f2ea7e2');
const server = new WebSocketServer({ host: '127.0.0.1', port: 0 });
await new Promise<void>(resolve => server.once('listening', resolve));
const key = new Uint8Array(randomBytes(32));
server.on('connection', ws => ws.on('message', frame => {
  assert.equal(open(key, frame.toString()), 'hello');
  ws.send(seal(key, new Uint8Array(randomBytes(24)), 'encrypted reply'));
}));
const child = spawn(file, ['tunnel', '--no-autoupdate', '--protocol', 'http2', '--url', `http://127.0.0.1:${server.address().port}`], { windowsHide: true, cwd: directory, stdio: ['ignore', 'ignore', 'pipe'] });
let socket: WebSocket | undefined;
try {
  const endpoint = await new Promise<string>((resolve, reject) => {
    let host = '', log = ''; const timer = setTimeout(() => reject(Error('Tunnel registration timed out')), 60000);
    child.once('error', reject); child.once('exit', code => reject(Error(`Tunnel exited: ${code}`)));
    child.stderr.on('data', bytes => {
      log += bytes.toString(); host = log.match(/https:\/\/([a-z0-9-]+\.trycloudflare\.com)/)?.[1] ?? host;
      if (host && log.includes('Registered tunnel connection')) { clearTimeout(timer); resolve(`wss://${host}`); }
    });
  });
  let connected = false, error;
  for (let attempt = 0; attempt < 8 && !connected; attempt++) {
    socket = new WS(endpoint);
    try {
      await new Promise<void>((resolve, reject) => {
        const timer = setTimeout(() => reject(Error('Encrypted round trip timed out')), 10000);
        socket!.onopen = () => socket!.send(seal(key, new Uint8Array(randomBytes(24)), 'hello'));
        socket!.onerror = event => { clearTimeout(timer); reject(Error(event.message)); };
        socket!.onmessage = event => { try { assert.equal(open(key, String(event.data)), 'encrypted reply'); clearTimeout(timer); resolve(); } catch (error) { clearTimeout(timer); reject(error); } };
      });
      connected = true;
    } catch (failure) { error = failure; socket?.close(); await new Promise(resolve => setTimeout(resolve, 4000)); }
  }
  assert(connected, String(error));
  console.log('PASS: public WSS relay with encrypted request and response');
} finally { socket?.close(); child.kill(); for (const client of server.clients) client.terminate(); server.close(); }
