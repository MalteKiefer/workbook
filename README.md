# Wartungsdoku

Lokale, tastaturgesteuerte Wartungsdokumentation für einen einzelnen IT-Administrator:
Tätigkeiten je Kunde und System zeitstempelgenau protokollieren — vollständig offline,
ohne Server- oder Cloud-Anbindung. Tauri 2 (Rust-Backend) mit React/TypeScript-Frontend.

Die vollständige Architektur- und Design-Spezifikation steht in
[`docs/superpowers/specs/2026-09-07-wartungsdoku-design.md`](docs/superpowers/specs/2026-09-07-wartungsdoku-design.md).

## Voraussetzungen

- **Rust** (stable) über [rustup](https://rustup.rs)
- **Node.js** + **npm**
- **Windows:** MSVC-Build-Tools für das Target `x86_64-pc-windows-msvc` — entweder die
  "Visual Studio Build Tools" oder eine volle Visual-Studio-Installation, jeweils mit der
  Workload **"Desktop development with C++"**.
- **Linux** (Debian/Ubuntu als Beispiel — genaue Paketnamen variieren je Distribution):
  ```sh
  sudo apt install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev \
    build-essential curl wget file libssl-dev libxdo-dev
  ```
  Je nach Distribution heißt das AppIndicator-Paket `libayatana-appindicator3-dev` statt
  `libappindicator3-dev`. Die aktuelle, vollständige Liste pflegt Tauri selbst unter
  <https://v2.tauri.app/start/prerequisites/> — im Zweifel dort nachsehen.

## Entwicklung

```sh
npm install
```

`@tauri-apps/cli` ist als Dev-Dependency installiert (siehe `package.json`), es gibt aber
(noch) kein eigenes `tauri`-npm-Skript. Direkt nutzbar ist der lokal installierte
CLI-Befehl:

```sh
npx tauri dev
```

Das baut das Frontend (`beforeDevCommand`/`devUrl` in `src-tauri/tauri.conf.json`) und
startet die App im Entwicklungsmodus.

Alternativ — der über die bisherige Projekt-Historie hinweg tatsächlich verwendete und
verifizierte Zwei-Schritt-Weg (siehe die Verifikationsschritte in den Plänen unter
`docs/superpowers/plans/`):

```sh
npm run build
cd src-tauri
cargo run --bin wartungsdoku
```

`npm run build` erzeugt den `dist/`-Ordner, den `cargo run` dann als bereits fertiges
Frontend einbettet.

## Produktions-Build

Für einen echten Installer/Bundle ist `cargo tauri build` (bzw. äquivalent `npx tauri
build`) der vorgesehene Weg. Ehrlicher Stand: Diese Repo wurde während ihrer gesamten
bisherigen Entstehung ausschließlich im Entwicklungsmodus gebaut und ausgeführt (`cargo
build` / `cargo run` fürs Backend, `npm run build` fürs Frontend) — kein einziger
Plan-Durchlauf in `docs/superpowers/plans/` hat bislang einen tatsächlichen
Release-/Bundle-Build durchgeführt oder verifiziert. `cargo tauri build` ist der
dokumentierte nächste Schritt, aber in dieser Repo bisher ungetestet.

## Datenverzeichnis

Default-Datenverzeichnis: das plattformübliche App-Datenverzeichnis, ermittelt über die
`dirs`-Crate (`default_data_dir()` in `src-tauri/src/config.rs`) als
`dirs::data_dir()/wartungsdoku` — in der Praxis also `%APPDATA%\wartungsdoku` unter
Windows und `~/.local/share/wartungsdoku` unter Linux.

Überschreibbar per Umgebungsvariable:

```sh
WARTUNGSDOKU_DATA_DIR=/pfad/zu/eigenem/verzeichnis
```

(siehe `resolve_data_dir()` in `src-tauri/src/config.rs`).

Ein Backup ist einfach eine Kopie des gesamten Datenverzeichnisses (`wartungsdoku.db`
samt WAL-Dateien, `attachments/`, `config.toml`) — keine gesonderte Exportfunktion nötig.

## Tests

```sh
cd src-tauri
cargo test
```

Rust-seitige Tests decken Datenschicht, Migrationen und Volltextsuche ab. Die Anzahl
wächst mit dem Projekt — alle Tests sollten grün sein.

## Projektstruktur

| Pfad | Inhalt |
|---|---|
| `src-tauri/` | Rust-Backend: Tauri-Commands, SQLite-Datenschicht, Migrationen, globale Hotkeys, Export |
| `src/` | React-Frontend des Hauptfensters |
| `src/quick-capture/` | Eigenständiges Frontend des separaten Schnellerfassungsfensters |
| `docs/superpowers/specs/` | Architektur-/Design-Spezifikation |
| `docs/superpowers/plans/` | Projektinterne, phasenweise Umsetzungspläne — nützliche Lektüre für Anschlussarbeit |

## Weiterführende Dokumentation

- [`SHORTCUTS.md`](SHORTCUTS.md) — vollständige, gegen den Code verifizierte Tastaturbelegung
- [`docs/PLUGIN_ARCHITECTURE.md`](docs/PLUGIN_ARCHITECTURE.md) — Architekturnotiz zur Plugin-Erweiterung
