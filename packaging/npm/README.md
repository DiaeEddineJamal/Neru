# neru-cli

*Neru 練る — to knead, to refine through repeated work.*

The `neru` command: a local-first coding agent in your terminal. It explores your project, proposes changes you read line by line, and never acts without asking.

## Install

```sh
npm install -g neru-cli
```

Node.js 18 or newer. The package downloads the prebuilt `neru` binary for your platform from the matching [GitHub release](https://github.com/DiaeEddineJamal/Neru/releases) and checks its SHA256. Prebuilt for Windows x64, macOS (Apple silicon and Intel) and Linux x64. On Linux, `neru` needs WebKitGTK 4.1 (`sudo apt install libwebkit2gtk-4.1-0`).

Set `NERU_SKIP_DOWNLOAD=1` to skip the download during install; it then happens the first time you run `neru`.

## Use

```sh
cd your-project
neru
```

`neru` shares settings, keys and sessions with the Neru desktop app.

## Update and uninstall

```sh
npm install -g neru-cli@latest
npm uninstall -g neru-cli
```

## Other ways to install

```sh
curl -fsSL https://raw.githubusercontent.com/DiaeEddineJamal/Neru/main/install.sh | bash   # macOS, Linux
irm https://raw.githubusercontent.com/DiaeEddineJamal/Neru/main/install.ps1 | iex          # Windows
winget install Luziv.Neru.CLI                                                              # Windows
```

Source, the desktop app and issues: [github.com/DiaeEddineJamal/Neru](https://github.com/DiaeEddineJamal/Neru). MIT licensed.
