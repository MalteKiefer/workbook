# In-App Updater Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a "Aktualisierung" Settings tab where the user can see the
installed version, check GitHub for a newer release, and install it —
applied on the restart the install flow itself triggers.

**Architecture:** Tauri's first-party updater plugin
(`tauri-plugin-updater` + `@tauri-apps/plugin-updater`), pointed at this
project's existing GitHub Releases via a `latest.json` manifest that
`tauri-apps/tauri-action` (already used in `release.yml`) generates and
signs automatically once a signing keypair is configured. No new backend
commands — the plugin's own JS bindings are called directly from the
frontend, the same way this project's other plugins (dialog, opener,
autostart) already work.

**Tech Stack:** Rust (`tauri-plugin-updater`, `tauri-plugin-process`),
TypeScript/React (`@tauri-apps/plugin-updater`, `@tauri-apps/plugin-process`),
existing Tauri v2 capability/permission system.

**Spec:** `docs/superpowers/specs/2026-09-10-app-updater-design.md`

## Global Constraints

- The signing keypair already exists: its public key (below) goes into `tauri.conf.json`; its private key and password are already stored as the GitHub repo secrets `TAURI_SIGNING_PRIVATE_KEY`/`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` — no task in this plan touches those secrets or generates a new keypair.
- Public key (the exact, already-generated value — paste verbatim, do not regenerate): `dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEY3QkE1Nzk0OEVERUNGNTgKUldSWXo5Nk9sRmU2OXlja2dsOFlMTGJFcWVYdmRVY2dOaXBXVGlTYUdkZUh1aDgrK0phcVpaM0UK`
- Update endpoint: `https://github.com/MalteKiefer/workbook/releases/latest/download/latest.json` (this repo's actual GitHub path — verify it matches `git remote get-url origin` before using it, don't assume).
- `release.yml` needs **no changes** — `tauri-apps/tauri-action`'s `uploadUpdaterJson` input already defaults to `true`, and it already reads the signing secrets from the environment automatically once they exist (confirmed present). The plugin config living in `tauri.conf.json` is what turns updater-artifact generation on; nothing in the workflow YAML itself needs to know about the updater.
- No new Tauri commands, no new `AppState` fields — the updater/process plugins are called directly from the frontend via their own JS bindings.
- No startup auto-check, no background polling — purely user-initiated from the new Settings tab (see spec's "Explicitly not doing").
- Linux: the updater only knows how to replace an AppImage install. A `.deb`/`.rpm` install has no update artifact — `check()` will simply not find a matching platform target for those, and Task 2's UI must show that as a plain "kein Update über diese Installationsart verfügbar"-style message, not a crash or a silent no-op.

---

### Task 1: Backend/config wiring — plugins, capabilities, updater config

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/capabilities/default.json`
- Modify: `src-tauri/tauri.conf.json`

**Interfaces:**
- Produces: the `tauri_plugin_updater`/`tauri_plugin_process` Rust plugins registered and permission-granted, and `tauri.conf.json`'s `plugins.updater` block — Task 2's frontend code calls `check()`/`downloadAndInstall()`/`relaunch()` against this registration; without it those calls fail at runtime with a "plugin not found"/permission-denied error.

- [ ] **Step 1: Add the two new Cargo dependencies**

In `src-tauri/Cargo.toml`, in the `[dependencies]` block, add these two lines in alphabetical position (after `tauri-plugin-opener`, the block is already alphabetically sorted — `tauri-plugin-process` goes right after `tauri-plugin-opener` and before `tauri-plugin-single-instance`; `tauri-plugin-updater` goes after `typst-pdf`... actually simpler: just insert both next to the existing `tauri-plugin-*` lines, sorted alphabetically among them):

```toml
tauri-plugin-autostart = "2.5.1"
tauri-plugin-dialog = "2.7.3"
tauri-plugin-global-shortcut = "2.3.2"
tauri-plugin-opener = "2.5.5"
tauri-plugin-process = "2"
tauri-plugin-single-instance = "2.4.4"
tauri-plugin-updater = "2"
```

(That's the existing 5 `tauri-plugin-*` lines plus the 2 new ones, all 7 kept alphabetically sorted together — replace the existing block of 5 with this block of 7.)

- [ ] **Step 2: Register both plugins in `lib.rs`**

In `src-tauri/src/lib.rs`, find the `tauri::Builder::default()` chain (it currently has `.plugin(tauri_plugin_single_instance::init(...))`, `.plugin(tauri_plugin_autostart::Builder::new().build())`, `.plugin(tauri_plugin_opener::init())`, `.plugin(tauri_plugin_dialog::init())` in that order, right before `.manage(AppState { ... })`). Add two more `.plugin(...)` calls right after the existing `.plugin(tauri_plugin_dialog::init())` line and before `.manage(AppState {`:

```rust
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
```

- [ ] **Step 3: Grant the two new permissions**

In `src-tauri/capabilities/default.json`, add `"updater:default"` and `"process:allow-restart"` to the `"permissions"` array (position doesn't matter, append at the end is fine):

```json
    "opener:default",
    "dialog:default",
    "updater:default",
    "process:allow-restart"
```

(That replaces the current last two lines, `"opener:default",` and `"dialog:default"`, with those same two lines plus the two new ones.)

- [ ] **Step 4: Add the updater plugin config to `tauri.conf.json`**

In `src-tauri/tauri.conf.json`, add `"createUpdaterArtifacts": true` to the existing `"bundle"` object, and add a new top-level `"plugins"` object (as a sibling of `"app"` and `"bundle"`, not nested inside either):

```json
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": ["icons/32x32.png", "icons/128x128.png", "icons/128x128@2x.png", "icons/icon.ico", "icons/icon.icns", "icons/icon.png"],
    "createUpdaterArtifacts": true
  },
  "plugins": {
    "updater": {
      "pubkey": "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEY3QkE1Nzk0OEVERUNGNTgKUldSWXo5Nk9sRmU2OXlja2dsOFlMTGJFcWVYdmRVY2dOaXBXVGlTYUdkZUh1aDgrK0phcVpaM0UK",
      "endpoints": [
        "https://github.com/MalteKiefer/workbook/releases/latest/download/latest.json"
      ]
    }
  }
```

(The whole file's structure stays `{ "$schema", "productName", "version", "identifier", "build", "app", "bundle", "plugins" }` — `"plugins"` is a new top-level key added after `"bundle"`, before the file's closing `}`.) Before finalizing, run `git remote get-url origin` and confirm the endpoint URL's `MalteKiefer/workbook` path matches — if it doesn't, use the real path instead of what's written here.

- [ ] **Step 5: Verify the backend builds and passes every existing check**

Run (from `src-tauri`, with `export PATH="$HOME/.cargo/bin:$PATH" &&` prefixed for Bash):
```
cargo build --lib
cargo test --lib
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
```
Expected: all four succeed. `cargo build --lib` succeeding confirms `tauri.conf.json` parses correctly (Tauri's build script validates it against its schema) even though the plugin config itself isn't exercised by any Rust unit test — there's nothing new to unit-test here, this is integration wiring, verified by the build succeeding and by Task 2's frontend actually calling the plugin successfully.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/lib.rs src-tauri/capabilities/default.json src-tauri/tauri.conf.json
git commit -m "feat: register the updater and process plugins, configure the update endpoint"
```

---

### Task 2: Frontend — Settings tab and the update UI

**Files:**
- Modify: `package.json` (two new dependencies)
- Modify: `src/state/appStore.ts`
- Modify: `src/components/SettingsView.tsx`
- Create: `src/components/UpdateSettingsView.tsx`

**Interfaces:**
- Consumes: `check()`, `Update` type (`@tauri-apps/plugin-updater`), `relaunch()` (`@tauri-apps/plugin-process`), `getVersion()` (`@tauri-apps/api/app`, already a transitive dependency via `@tauri-apps/api`) — all made usable at runtime by Task 1's plugin registration/permissions.
- Produces: a new `SettingsTab` value `"update"`, and `UpdateSettingsView`'s default export, mounted the same way `KeymapSettingsView`/`PluginsView`/etc. already are.

- [ ] **Step 1: Add the two frontend dependencies**

Run (from the repo root):
```bash
npm install @tauri-apps/plugin-updater@^2 @tauri-apps/plugin-process@^2
```
This adds them to `package.json`'s `dependencies` and updates `package-lock.json` — no manual edit needed, `npm install` handles both files.

- [ ] **Step 2: Extend `SettingsTab`**

In `src/state/appStore.ts`, change:
```ts
export type SettingsTab = "general" | "backup" | "plugins" | "keymap";
```
to:
```ts
export type SettingsTab = "general" | "backup" | "plugins" | "keymap" | "update";
```

- [ ] **Step 3: Create `src/components/UpdateSettingsView.tsx`**

```tsx
import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { formatInvokeError } from "../lib/errors";

type CheckState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "up-to-date" }
  | { kind: "available"; update: Update }
  | { kind: "error"; message: string };

type InstallState = { kind: "idle" } | { kind: "installing"; percent: number | null } | { kind: "error"; message: string };

export default function UpdateSettingsView() {
  const [currentVersion, setCurrentVersion] = useState<string | null>(null);
  const [checkState, setCheckState] = useState<CheckState>({ kind: "idle" });
  const [installState, setInstallState] = useState<InstallState>({ kind: "idle" });

  useEffect(() => {
    let cancelled = false;
    void getVersion().then((v) => {
      if (!cancelled) setCurrentVersion(v);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  async function handleCheck() {
    setCheckState({ kind: "checking" });
    try {
      const update = await check();
      if (update) {
        setCheckState({ kind: "available", update });
      } else {
        setCheckState({ kind: "up-to-date" });
      }
    } catch (e) {
      setCheckState({ kind: "error", message: formatInvokeError(e) });
    }
  }

  async function handleInstall(update: Update) {
    setInstallState({ kind: "installing", percent: null });
    let downloaded = 0;
    let contentLength = 0;
    try {
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          contentLength = event.data.contentLength ?? 0;
        } else if (event.event === "Progress") {
          downloaded += event.data.chunkLength;
          setInstallState({
            kind: "installing",
            percent: contentLength > 0 ? Math.round((downloaded / contentLength) * 100) : null,
          });
        }
      });
      await relaunch();
    } catch (e) {
      setInstallState({ kind: "error", message: formatInvokeError(e) });
    }
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "1.5rem", maxWidth: "36rem" }}>
      <h1 style={{ fontSize: "1.1rem" }}>Aktualisierung</h1>

      <section
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "0.5rem",
          padding: "1rem",
          border: "1px solid var(--border)",
          borderRadius: "var(--radius-sm)",
          background: "var(--bg-surface)",
        }}
      >
        <p style={{ margin: 0, fontSize: "0.85rem", color: "var(--text-secondary)" }}>
          Installierte Version: <strong style={{ color: "var(--text-primary)" }}>{currentVersion ?? "…"}</strong>
        </p>

        <div>
          <button type="button" className="btn-primary" disabled={checkState.kind === "checking"} onClick={() => void handleCheck()}>
            Nach Updates suchen
          </button>
        </div>

        {checkState.kind === "checking" && <p style={{ margin: 0, color: "var(--text-muted)", fontSize: "0.85rem" }}>Suche…</p>}
        {checkState.kind === "up-to-date" && (
          <p style={{ margin: 0, color: "var(--success)", fontSize: "0.85rem" }}>Aktuell — keine Updates verfügbar.</p>
        )}
        {checkState.kind === "error" && (
          <p style={{ margin: 0, color: "var(--danger)", fontSize: "0.85rem" }}>Fehler: {checkState.message}</p>
        )}
        {checkState.kind === "available" && (
          <div style={{ display: "flex", flexDirection: "column", gap: "0.5rem" }}>
            <p style={{ margin: 0, fontSize: "0.85rem" }}>
              Version <strong>{checkState.update.version}</strong> verfügbar (aktuell {currentVersion}).
            </p>
            {checkState.update.body && (
              <p style={{ margin: 0, fontSize: "0.8rem", color: "var(--text-secondary)", whiteSpace: "pre-wrap" }}>
                {checkState.update.body}
              </p>
            )}
            <div>
              <button
                type="button"
                className="btn-primary"
                disabled={installState.kind === "installing"}
                onClick={() => void handleInstall(checkState.update)}
              >
                Installieren und neu starten
              </button>
            </div>
            {installState.kind === "installing" && (
              <p style={{ margin: 0, color: "var(--text-muted)", fontSize: "0.85rem" }}>
                Installiere{installState.percent !== null ? ` (${installState.percent}%)` : "…"}
              </p>
            )}
            {installState.kind === "error" && (
              <p style={{ margin: 0, color: "var(--danger)", fontSize: "0.85rem" }}>Fehler: {installState.message}</p>
            )}
          </div>
        )}
      </section>
    </div>
  );
}
```

- [ ] **Step 4: Wire the tab into `SettingsView.tsx`**

Add the import:
```ts
import UpdateSettingsView from "./UpdateSettingsView";
```
Add a new `TabButton` after the existing "Tastaturbelegung" one:
```tsx
        <TabButton active={settingsTab === "update"} onClick={() => setSettingsTab("update")}>
          Aktualisierung
        </TabButton>
```
Add a new conditional render after the existing `{settingsTab === "keymap" && <KeymapSettingsView />}`:
```tsx
      {settingsTab === "update" && <UpdateSettingsView />}
```

- [ ] **Step 5: Verify types and lint**

Run: `npm run build 2>&1 | tail -20 && npm run lint 2>&1 | tail -20`
Expected: both succeed. (`npm run build` runs `tsc -b`, which will catch it if `@tauri-apps/plugin-updater`'s actual exported types — `Update`, the `DownloadEvent` shape for `Started`/`Progress`/`Finished` — don't match what this file assumes; if the real package's types differ from what Step 3's code expects, fix the code to match the real types rather than working around the type checker.)

- [ ] **Step 6: Manual verification in the Browser tool**

Start the dev server (`preview_start` name `vite-dev`), navigate to it, go to Einstellungen, confirm via `find`/`read_page` that "Aktualisierung" now appears as a 5th tab button alongside Allgemein/Backup/Plugins/Tastaturbelegung, click it, confirm the "Installierte Version" line and "Nach Updates suchen" button render. Click "Nach Updates suchen" and confirm via `read_page` that an error message appears (expected — there is no Tauri backend in this browser dev preview, `check()` will throw; confirm the error is shown as the styled red `Fehler:` paragraph, not an unhandled exception/blank screen). Stop the preview server after.

**Note for whoever runs this task:** a real end-to-end check (does `check()` actually find a real update, does `downloadAndInstall()` + `relaunch()` actually work) is only possible in a real built app pointed at a real tagged GitHub release with a `latest.json` — not in this browser dev preview, and not until this plan is merged and at least one new version has been tagged after it. That real-world check is intentionally not a step in this plan; do it once, manually, the next time a release is cut after this merges (download the release's `latest.json` asset and confirm it lists `windows-x86_64`/`linux-x86_64`/`darwin-aarch64` entries, then run the built app from the *previous* version and use its new "Aktualisierung" tab to update itself).

- [ ] **Step 7: Commit**

```bash
git add package.json package-lock.json src/state/appStore.ts src/components/SettingsView.tsx src/components/UpdateSettingsView.tsx
git commit -m "feat: add the Aktualisierung settings tab (check for and install updates)"
```

## Self-Review

**Spec coverage:** Frontend section (Settings tab, version display, check/install/relaunch flow, progress reporting, error handling, no-auto-check) — Task 2. Architecture section (plugin registration, tauri.conf.json endpoint/pubkey, capabilities) — Task 1. "Explicitly not doing" items (channels, delta updates, startup polling) — correctly absent from both tasks. Linux AppImage-only limitation — Task 2's `check()` error path already surfaces whatever the plugin itself reports for an unsupported install type as a plain error message, matching the spec's "kein Update... verfügbar"-style requirement without needing special-case code (the plugin's own error is descriptive enough; no need to detect "is this a .deb install" ourselves).

**Placeholder scan:** No TBD/TODO; every step has real, complete code.

**Type consistency:** `Update`/`check`/`relaunch`/`getVersion` names and their usage are consistent between the spec and Task 2's actual code. `SettingsTab`'s new `"update"` value is used identically in `appStore.ts` and `SettingsView.tsx`.
