#!/usr/bin/env node
// Launches the native neru binary that install.js put in vendor/, downloading it first if the
// postinstall step was skipped or failed. Arguments, exit code and signals pass straight through.
'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { spawn, spawnSync } = require('node:child_process');

const root = path.join(__dirname, '..');
const exe = path.join(root, 'vendor', process.platform === 'win32' ? 'neru.exe' : 'neru');

if (!fs.existsSync(exe)) {
  const env = { ...process.env, NERU_FROM_WRAPPER: '1' };
  delete env.NERU_SKIP_DOWNLOAD;
  spawnSync(process.execPath, [path.join(root, 'install.js')], { stdio: 'inherit', env });
  if (!fs.existsSync(exe)) {
    process.stderr.write(
      'neru-cli: the neru binary is missing and could not be downloaded.\n' +
        'Reinstall with `npm install -g neru-cli`, or see https://github.com/DiaeEddineJamal/Neru#install-the-cli\n',
    );
    process.exit(1);
  }
}

const child = spawn(exe, process.argv.slice(2), {
  stdio: 'inherit',
  env: { ...process.env, NERU_INSTALL_KIND: 'npm' },
});

// Ctrl+C reaches the child directly (same terminal / process group) and neru handles it itself,
// so the wrapper only needs to stay alive. Other signals sent to the wrapper are passed on.
const forwarded = ['SIGTERM', 'SIGHUP', 'SIGQUIT', 'SIGBREAK'];
const handlers = {};
process.on('SIGINT', () => {});
for (const sig of forwarded) {
  handlers[sig] = () => {
    try {
      child.kill(sig);
    } catch {
      // The child already exited.
    }
  };
  try {
    process.on(sig, handlers[sig]);
  } catch {
    // Not supported on this platform.
  }
}

child.on('error', (err) => {
  process.stderr.write(`neru-cli: could not start ${exe}: ${err.message}\n`);
  process.exit(1);
});

child.on('exit', (code, signal) => {
  if (signal) {
    // Exit the way the child did, so shells see the same status.
    process.removeAllListeners(signal);
    try {
      process.kill(process.pid, signal);
      return;
    } catch {
      process.exit(1);
    }
  }
  process.exit(code === null ? 1 : code);
});
