#!/usr/bin/env node
// Downloads the neru binary for this platform from the GitHub release that matches this
// package's version, verifies its SHA256, and unpacks it into vendor/.
// No dependencies: node:https, node:crypto and the system tar (or PowerShell on old Windows).
'use strict';

const crypto = require('node:crypto');
const fs = require('node:fs');
const https = require('node:https');
const http = require('node:http');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const REPO = 'DiaeEddineJamal/Neru';
const ROOT = __dirname;
const VENDOR = path.join(ROOT, 'vendor');
const VERSION = require('./package.json').version;

const TARGETS = {
  'win32-x64': 'x86_64-pc-windows-msvc',
  // Windows on ARM runs the x64 build under emulation.
  'win32-arm64': 'x86_64-pc-windows-msvc',
  'darwin-arm64': 'aarch64-apple-darwin',
  'darwin-x64': 'x86_64-apple-darwin',
  'linux-x64': 'x86_64-unknown-linux-gnu',
};

function binaryPath() {
  return path.join(VENDOR, process.platform === 'win32' ? 'neru.exe' : 'neru');
}

function assetName() {
  const target = TARGETS[`${process.platform}-${process.arch}`];
  if (!target) return null;
  return `neru-cli-${target}${process.platform === 'win32' ? '.zip' : '.tar.gz'}`;
}

function log(message) {
  process.stderr.write(`neru-cli: ${message}\n`);
}

// Best-effort proxy support via HTTP CONNECT, for HTTPS_PROXY / https_proxy.
function proxyFor(url) {
  const proxy =
    process.env.HTTPS_PROXY || process.env.https_proxy || process.env.ALL_PROXY || process.env.all_proxy;
  if (!proxy) return null;
  const noProxy = (process.env.NO_PROXY || process.env.no_proxy || '')
    .split(',')
    .map((s) => s.trim().replace(/^\*?\./, ''))
    .filter(Boolean);
  const host = new URL(url).hostname;
  if (noProxy.some((h) => h === '*' || host === h || host.endsWith(`.${h}`))) return null;
  try {
    return new URL(proxy.includes('://') ? proxy : `http://${proxy}`);
  } catch {
    return null;
  }
}

function connectThroughProxy(proxy, target) {
  return new Promise((resolve, reject) => {
    const lib = proxy.protocol === 'https:' ? https : http;
    const headers = { Host: `${target.hostname}:443` };
    if (proxy.username) {
      const auth = `${decodeURIComponent(proxy.username)}:${decodeURIComponent(proxy.password)}`;
      headers['Proxy-Authorization'] = `Basic ${Buffer.from(auth).toString('base64')}`;
    }
    const req = lib.request({
      host: proxy.hostname,
      port: proxy.port || (proxy.protocol === 'https:' ? 443 : 80),
      method: 'CONNECT',
      path: `${target.hostname}:443`,
      headers,
    });
    req.once('connect', (res, socket) => {
      if (res.statusCode === 200) resolve(socket);
      else {
        socket.destroy();
        reject(new Error(`proxy CONNECT failed with ${res.statusCode}`));
      }
    });
    req.once('error', reject);
    req.end();
  });
}

async function get(url, redirects = 0) {
  if (redirects > 10) throw new Error('too many redirects');
  const target = new URL(url);
  const options = { headers: { 'User-Agent': `neru-cli-npm/${VERSION}`, Accept: '*/*' } };
  const proxy = target.protocol === 'https:' ? proxyFor(url) : null;
  if (proxy) {
    const socket = await connectThroughProxy(proxy, target);
    options.createConnection = () => require('node:tls').connect({ socket, servername: target.hostname });
    options.agent = false;
  }
  const lib = target.protocol === 'http:' ? http : https;
  return new Promise((resolve, reject) => {
    const req = lib.get(target, options, (res) => {
      const status = res.statusCode || 0;
      if (status >= 300 && status < 400 && res.headers.location) {
        res.resume();
        resolve(get(new URL(res.headers.location, target).toString(), redirects + 1));
        return;
      }
      if (status !== 200) {
        res.resume();
        reject(new Error(`GET ${url} returned ${status}`));
        return;
      }
      const chunks = [];
      res.on('data', (c) => chunks.push(c));
      res.on('end', () => resolve(Buffer.concat(chunks)));
      res.on('error', reject);
    });
    req.setTimeout(120000, () => req.destroy(new Error(`GET ${url} timed out`)));
    req.on('error', reject);
  });
}

function extract(archive, dest) {
  if (archive.endsWith('.tar.gz')) {
    const r = spawnSync('tar', ['-xzf', archive, '-C', dest], { stdio: 'inherit' });
    if (r.status !== 0) throw new Error('tar could not extract the archive');
    return;
  }
  // Windows 10+ ships bsdtar, which reads zip files.
  const tar = path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'tar.exe');
  const r = spawnSync(fs.existsSync(tar) ? tar : 'tar', ['-xf', archive, '-C', dest], { stdio: 'ignore' });
  if (r.status === 0) return;
  const ps = spawnSync(
    'powershell.exe',
    [
      '-NoProfile',
      '-NonInteractive',
      '-Command',
      `Expand-Archive -LiteralPath '${archive.replace(/'/g, "''")}' -DestinationPath '${dest.replace(/'/g, "''")}' -Force`,
    ],
    { stdio: 'inherit' },
  );
  if (ps.status !== 0) throw new Error('could not extract the archive (tried tar and Expand-Archive)');
}

async function install() {
  if (process.env.NERU_SKIP_DOWNLOAD) {
    log('NERU_SKIP_DOWNLOAD is set; not downloading the binary.');
    return;
  }
  if (fs.existsSync(binaryPath())) return;

  const asset = assetName();
  if (!asset) {
    throw new Error(
      `no prebuilt neru for ${process.platform}-${process.arch}. ` +
        `Build from source: https://github.com/${REPO}#build-from-source`,
    );
  }
  const base = (process.env.NERU_DOWNLOAD_BASE || `https://github.com/${REPO}/releases/download/v${VERSION}`).replace(
    /\/+$/,
    '',
  );

  log(`downloading ${asset} (v${VERSION})`);
  const [data, sumFile] = await Promise.all([get(`${base}/${asset}`), get(`${base}/${asset}.sha256`)]);
  const expected = sumFile.toString('utf8').trim().split(/\s+/)[0].toLowerCase();
  const actual = crypto.createHash('sha256').update(data).digest('hex');
  if (!expected || expected !== actual) {
    throw new Error(`checksum mismatch for ${asset} (expected ${expected || 'nothing'}, got ${actual})`);
  }

  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'neru-cli-'));
  try {
    const archive = path.join(tmp, asset);
    fs.writeFileSync(archive, data);
    const staging = path.join(tmp, 'vendor');
    fs.mkdirSync(staging);
    extract(archive, staging);
    const exe = path.join(staging, path.basename(binaryPath()));
    if (!fs.existsSync(exe)) throw new Error(`the archive does not contain ${path.basename(exe)}`);
    if (process.platform !== 'win32') fs.chmodSync(exe, 0o755);
    if (process.platform === 'darwin') spawnSync('xattr', ['-dr', 'com.apple.quarantine', staging], { stdio: 'ignore' });

    fs.rmSync(VENDOR, { recursive: true, force: true });
    try {
      fs.renameSync(staging, VENDOR);
    } catch {
      // Different volumes: copy instead.
      fs.cpSync(staging, VENDOR, { recursive: true });
    }
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
  log(`installed neru ${VERSION}`);
}

module.exports = { install, binaryPath };

if (require.main === module) {
  install().catch((err) => {
    log(`install failed: ${err.message}`);
    if (!process.env.NERU_FROM_WRAPPER) log('neru will try again the first time you run it.');
    // Never fail `npm install` over the download; bin/neru.js retries on first run.
    process.exit(0);
  });
}
