# Xynoxa Desktop Client

Der Xynoxa Desktop Client ist der Tauri-basierte Sync‑Client, der deinen lokalen Ordner `~/Xynoxa` mit der Xynoxa Cloud synchronisiert. Fokus: schnelle, ressourcenschonende Sync‑Engine, robuste Konfliktbehandlung und nahtlose OS‑Integration.

## ATTENTION  ATTENTION  ATTENTION  ATTENTION  ATTENTION 

Warning! This application is still under development and definitely not recommended for daily use! Currently, it is only recommended to install the application for testing purposes and not to use it in production!

**Warning! Expect data loss when testing this application!**


## Tech Stack
- React + TypeScript + TailwindCSS (UI)
- Tauri + Rust (Sync Core)

## Features (Auszug)
- Datei‑Sync mit Delta‑Logik (Create/Update/Move/Delete)
- Lokale SQLite‑DB für Index & Cursor
- OS‑Keyring für Token‑Speicherung
- File‑Watcher + Debounce‑Sync
- Gruppenordner‑Support

## Development
```bash
npm install
npm run dev
```

## Build (App)
```bash
npm run build-app
```

Hinweis: Builds/Deployments laufen über die CI; lokal wird in dieser Umgebung nichts gebaut oder gestartet.

Erzeugte Artefakte (je nach Build‑OS):
- Linux: `.deb`, `.rpm` und `.AppImage`
- Windows: `.exe` (NSIS)
- macOS: `.dmg`

## Releases (GitHub)
Releases werden automatisch gebaut, sobald ein Tag gepusht wird (Format `vX.Y.Z`).
Die GitHub Action erstellt dann einen Release und hängt die Artefakte an.

Taggen und pushen:
```bash
git tag v1.0.0-Alpha1
git push origin v1.0.0-Alpha1
```
