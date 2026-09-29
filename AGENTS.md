# Neru workspace storage

- Keep all new Neru source files, dependencies, generated assets, databases, logs, build output, and test artifacts under `D:\Neru`.
- Before running project tooling in PowerShell, dot-source `D:\Neru\Use-NeruStorage.ps1` in the same shell. It redirects common temporary files and package, browser, Rust, and Gradle caches to `D:\Neru\.local`.
- Use `D:\Neru` as the working directory for Neru commands. Configure any additional tool-specific cache or SDK directory on D: before installing or running that tool.
- Do not relocate or delete pre-existing user files on C: without a specific request. Existing app installations and Codex-managed attachments are outside this project storage policy.
