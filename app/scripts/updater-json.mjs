#!/usr/bin/env node
// latest.json, the file installed copies of Neru read to find an update, built once from the signed
// files every platform's build uploaded to the release. The builds used to write it themselves, each
// downloading, merging and re-uploading it, and parallel builds collided ("already_exists").
//
//   node app/scripts/updater-json.mjs <tag> [repo] > latest.json     (needs the gh CLI and GH_TOKEN)
//
// The keys match what tauri-action wrote: an installer-specific key per bundle (darwin-aarch64-app,
// linux-x86_64-deb, windows-x86_64-nsis…) and a plain one per platform (darwin-aarch64, linux-x86_64,
// windows-x86_64), which the updater falls back to: the .app archive, the AppImage and the .msi.

import { execFileSync } from 'node:child_process'
import { mkdtempSync, readFileSync, readdirSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const ARCH = { aarch64: 'aarch64', arm64: 'aarch64', x64: 'x86_64', amd64: 'x86_64', x86_64: 'x86_64' }

/** The latest.json keys a signed update file serves, or none for files the updater does not use. */
function keysFor(name) {
  const arch = Object.keys(ARCH).find(part => new RegExp(`[_.-]${part}[_.-]`).test(`_${name}`))
  if (!arch) return []
  const a = ARCH[arch]
  if (name.endsWith('.app.tar.gz')) return [`darwin-${a}`, `darwin-${a}-app`]
  if (name.endsWith('.AppImage')) return [`linux-${a}`, `linux-${a}-appimage`]
  if (name.endsWith('.deb')) return [`linux-${a}-deb`]
  if (name.endsWith('.rpm')) return [`linux-${a}-rpm`]
  if (name.endsWith('.msi')) return [`windows-${a}`, `windows-${a}-msi`]
  if (name.endsWith('-setup.exe')) return [`windows-${a}-nsis`]
  return []
}

/** The whole file from the release's update files and their signatures. */
function updaterJson({ version, notes, repo, tag, signatures, now = new Date() }) {
  const platforms = {}
  for (const [name, signature] of Object.entries(signatures).sort(([a], [b]) => a.localeCompare(b))) {
    for (const key of keysFor(name)) platforms[key] = { signature: signature.trim(), url: `https://github.com/${repo}/releases/download/${tag}/${name}` }
  }
  return { version, notes, pub_date: now.toISOString(), platforms: Object.fromEntries(Object.entries(platforms).sort(([a], [b]) => a.localeCompare(b))) }
}

function gh(args) {
  return execFileSync('gh', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] })
}

{
  const [tag, repo = process.env.GITHUB_REPOSITORY] = process.argv.slice(2)
  if (!tag || !repo) {
    console.error('Usage: node app/scripts/updater-json.mjs <tag> [owner/repo]')
    process.exit(2)
  }
  const release = JSON.parse(gh(['release', 'view', tag, '--repo', repo, '--json', 'body,assets']))
  const dir = mkdtempSync(join(tmpdir(), 'neru-updater-'))
  try {
    gh(['release', 'download', tag, '--repo', repo, '--pattern', '*.sig', '--dir', dir])
    const signatures = {}
    for (const file of readdirSync(dir)) {
      const name = file.replace(/\.sig$/, '')
      // A signature without its file would point the updater at nothing.
      if (release.assets.some(asset => asset.name === name)) signatures[name] = readFileSync(join(dir, file), 'utf8')
    }
    const json = updaterJson({ version: tag.replace(/^v/, ''), notes: release.body ?? '', repo, tag, signatures })
    const needed = ['darwin-aarch64', 'darwin-x86_64', 'linux-x86_64', 'linux-aarch64', 'windows-x86_64']
    const missing = needed.filter(key => !json.platforms[key])
    if (missing.length) {
      console.error(`latest.json would have no update for ${missing.join(', ')}: a build did not upload its signed files.`)
      process.exit(1)
    }
    process.stdout.write(`${JSON.stringify(json, null, 2)}\n`)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
}
