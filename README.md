# Wartungsdoku

Wartungsdoku is an offline, keyboard first desktop application for a single IT administrator to document maintenance work per customer and system. It runs fully on the local machine, keeps all data in a local SQLite database, and never requires an internet connection for its core workflow.

## Features

* Customers, systems, and a searchable journal of maintenance entries.
* A global Command Palette (Ctrl+K) as the primary way to navigate and act.
* A quick capture popup, triggered by a global hotkey, for jotting down an entry without switching windows.
* A strict timestamp model. Every timestamp stores both a UTC value and an IANA timezone name, and the moment the work was performed is always kept separate from the moment it was recorded.
* Content addressed attachment storage. Files are deduplicated by their SHA256 hash.
* Markdown editing with an export to Markdown or PDF per customer.
* Full backup and restore, including the database, attachments, configuration, and any plugin cache.
* Eight built in RMM and asset management plugin integrations: NinjaOne, Level.io, Snipe IT, Microsoft Intune, Iru (Apple MDM, formerly Kandji), Jamf Pro, Apple Business Manager, and Tactical RMM. Each is read only. Data is pulled in and linked to a local system on request, and existing fields are never overwritten automatically.
* Light, dark, and system theme.

## Tech stack

* Backend: Rust, using Tauri 2 as the application shell, rusqlite for the database, and Typst for PDF rendering.
* Frontend: React and TypeScript, built with Vite, state managed with Zustand, and CodeMirror for the Markdown editor.

## Prerequisites

* Rust, stable channel, via [rustup](https://rustup.rs).
* Node.js 22 or newer, with npm.
* Windows: the MSVC build tools for the `x86_64-pc-windows-msvc` target. Either the Visual Studio Build Tools or a full Visual Studio installation, with the "Desktop development with C++" workload.
* Linux (package names vary by distribution, Debian and Ubuntu shown as an example):

  ```sh
  sudo apt install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev \
    build-essential curl wget file libssl-dev libxdo-dev
  ```

  Some distributions name the AppIndicator package `libayatana-appindicator3-dev` instead of `libappindicator3-dev`. Tauri maintains the current, full list at <https://v2.tauri.app/start/prerequisites/>.

## Development

```bash
npm install
npm run dev
```

In a second terminal, run the Tauri application in development mode:

```bash
cd src-tauri
cargo run
```

### Checks

```bash
npm run lint
npm run build
```

```bash
cd src-tauri
cargo fmt --check
cargo clippy --all-targets --all-features
cargo test
```

## Releases

Pushing a tag matching `v*` triggers a GitHub Actions workflow that builds installers for Linux and Windows and publishes them as a GitHub Release.

## License

This is a private, unlicensed project for internal use.
