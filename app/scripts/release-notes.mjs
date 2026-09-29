// Release notes and CHANGELOG.md from src/changelog.json, the same entries the app shows in What's new.
//   node scripts/release-notes.mjs 0.1.0        notes for one release (GitHub release body)
//   node scripts/release-notes.mjs --changelog  writes ../CHANGELOG.md with every release
import { readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const entries = JSON.parse(readFileSync(join(here, '..', 'src', 'changelog.json'), 'utf8'))
const repo = 'https://github.com/DiaeEddineJamal/Neru'

const section = entry => [
  `## ${entry.version} · ${entry.title}`,
  `*${entry.date}*`,
  '',
  entry.summary,
  '',
  ...entry.sections.flatMap(part => [`### ${part.title}`, '', ...part.items.map(item => `- ${item}`), '']),
].join('\n')

const downloads = version => `## Download

| Platform | File |
| --- | --- |
| Windows 10 and 11 | \`Neru_${version}_x64-setup.exe\` (recommended) or \`Neru_${version}_x64_en-US.msi\` |
| macOS 11+ on Apple silicon | \`Neru_${version}_aarch64.dmg\` |
| macOS 11+ on Intel | \`Neru_${version}_x64.dmg\` |
| Linux | \`Neru_${version}_amd64.AppImage\` or \`Neru_${version}_amd64.deb\` |

Neru checks for updates on launch and offers to install new versions, so you only download it once.

**First launch.** The builds are not code-signed yet. On Windows, choose *More info → Run anyway* in SmartScreen. On macOS, right-click Neru in Applications and choose *Open*. On Linux, make the AppImage executable (\`chmod +x\`) or install the .deb.
`

const arg = process.argv[2]
if (arg === '--changelog') {
  const text = ['# Changelog', '', `Every release of Neru. The app shows the same notes in *What's new* after it updates. Downloads are on the [releases page](${repo}/releases).`, '', ...entries.map(section)].join('\n')
  writeFileSync(join(here, '..', '..', 'CHANGELOG.md'), text)
} else {
  const version = (arg ?? '').replace(/^v/, '')
  const entry = entries.find(item => item.version === version)
  if (!entry) {
    console.error(`No changelog entry for ${version}. Add it to app/src/changelog.json.`)
    process.exit(1)
  }
  process.stdout.write(`${section(entry).replace(/^## .*\n/, `# Neru ${entry.version} — ${entry.title}\n`)}\n${downloads(entry.version)}`)
}
