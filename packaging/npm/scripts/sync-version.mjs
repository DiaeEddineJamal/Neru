#!/usr/bin/env node
// Copies the app version (app/package.json) into this package, so `neru-cli@X.Y.Z` downloads the
// binaries from release vX.Y.Z. Pass a version (or a tag like v0.4.0) to use that instead.
//
//   node packaging/npm/scripts/sync-version.mjs
//   node packaging/npm/scripts/sync-version.mjs v0.4.0
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const pkgPath = join(here, '..', 'package.json');
const appPath = join(here, '..', '..', '..', 'app', 'package.json');

const arg = process.argv[2];
const version = arg ? arg.replace(/^v/, '') : JSON.parse(readFileSync(appPath, 'utf8')).version;
if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) {
  console.error(`sync-version: "${version}" is not a version`);
  process.exit(1);
}

const pkg = JSON.parse(readFileSync(pkgPath, 'utf8'));
if (pkg.version === version) {
  console.log(`neru-cli is already ${version}`);
} else {
  pkg.version = version;
  writeFileSync(pkgPath, `${JSON.stringify(pkg, null, 2)}\n`);
  console.log(`neru-cli -> ${version}`);
}
