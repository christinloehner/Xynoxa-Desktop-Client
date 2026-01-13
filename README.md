# Xynoxa Desktop Client

The Xynoxa Desktop Client is the Tauri-based sync client that keeps your local folder `~/Xynoxa` in sync with the Xynoxa Cloud. Focus: a fast, resource-efficient sync engine, robust conflict handling, and seamless OS integration.

## Official Links

Your Xynoxa starting point: website and all repos at a glance, so you can jump straight to the right codebase.

- Xynoxa Website: https://www.xynoxa.com
- Xynoxa Cloud App: https://github.com/christinloehner/Xynoxa-Cloud
- Xynoxa Desktop Client: https://github.com/christinloehner/Xynoxa-Desktop-Client
- Xynoxa Mobile Client: https://github.com/christinloehner/Xynoxa-Mobile-Client

## ATTENTION  ATTENTION  ATTENTION  ATTENTION  ATTENTION 

Warning! This application is still under development and definitely not recommended for daily use! Currently, it is only recommended to install the application for testing purposes and not to use it in production!

**Warning! Expect data loss when testing this application!**


## Tech Stack
- React + TypeScript + TailwindCSS (UI)
- Tauri + Rust (Sync Core)

## Features (Excerpt)
- File sync with delta logic (Create/Update/Move/Delete)
- Local SQLite DB for index & cursor
- OS keyring for token storage
- File watcher + debounce sync
- Group folder support

## Development
```bash
npm install
npm run dev
```

## Build (App)
```bash
npm run build-app
```

Note: Builds/deployments run via CI; nothing is built or started locally in this environment.

Generated artifacts (depending on build OS):
- Linux: `.deb`, `.rpm`, and `.AppImage`
- Windows: `.exe` (NSIS)
- macOS: `.dmg`

## Releases (GitHub)
Releases are built automatically as soon as a tag is pushed (format `vX.Y.Z`).
The GitHub Action then creates a release and attaches the artifacts.

Tag and push:
```bash
git tag v1.0.0-Alpha1
git push origin v1.0.0-Alpha1
```
