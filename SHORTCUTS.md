# Tastaturbelegung — Wartungsdoku

Wartungsdoku ist auf vollständige Tastaturbedienung ausgelegt. Diese Datei listet die
**tatsächlich implementierte** Tastaturbelegung, gruppiert nach dem Bereich, in dem sie
aktiv ist — verifiziert gegen den aktuellen Quellcode (nicht nur gegen die
Design-Spezifikation, siehe `docs/superpowers/specs/2026-09-07-wartungsdoku-design.md`).
Wo Implementierung und Spec-Entwurf auseinanderlaufen, ist das unten explizit vermerkt.

## Globale Hotkeys (systemweit)

Systemweit aktiv — auch wenn Wartungsdoku keinen Fokus hat oder nur im Tray liegt.
Registrierung über `tauri-plugin-global-shortcut` in `src-tauri/src/hotkeys.rs`, Defaults
aus `HotkeyConfig` in `src-tauri/src/config.rs`. **Alle drei sind in `config.toml`
(Abschnitt `[hotkeys]`) frei änderbar.**

| Taste (Default) | Wirkung |
|---|---|
| `Strg+Alt+Leertaste` | Schnellerfassungsfenster öffnen |
| `Strg+Alt+F` | Hauptfenster anzeigen und fokussieren. **Hinweis:** Das öffnet aktuell *keine* eigene Such-Oberfläche — es zeigt schlicht das Hauptfenster (`window::show_and_focus_main`). Volltextsuche findet dort über die Command Palette (`Strg+K`) statt. |
| `Strg+Alt+S` | Bild aus der Zwischenablage lesen und die Schnellerfassung mit bereits eingefügtem Screenshot öffnen |

Siehe auch [Plattformhinweise](#plattformhinweise) — unter Wayland funktionieren diese
drei Hotkeys nicht zuverlässig.

## Hauptfenster

App-weit im Hauptfenster aktiv, registriert in `src/hooks/useGlobalHotkeys.ts`.

| Taste | Wirkung | Aktiv auch in Textfeldern? |
|---|---|---|
| `Strg+N` | Schnellerfassungsfenster mit aktuellem Kunden-/System-Kontext öffnen (`open_quick_capture_with_context`) | Ja |
| `Esc` | Offenes Formular/Overlay schließen; ist keines offen und die aktuelle Ansicht ist die Systemliste, geht es zurück zur Kundenliste | Ja |
| `g` dann `c` | Zu Kundenliste | Nein |
| `g` dann `s` | Zu Systemliste des aktuellen Kunden (nur wenn ein Kunde ausgewählt ist) | Nein |
| `g` dann `j` | Zum Journal | Nein |

`Esc` und `Strg+N` lösen laut Code auch dann aus, wenn ein Eingabefeld fokussiert ist —
alle anderen Bindings in dieser Tabelle (die `g`-Sequenzen) werden unterdrückt, solange
ein Textfeld fokussiert ist oder Strg/Alt/Cmd gehalten wird. Eine begonnene `g`-Sequenz
verfällt nach 800 ms ohne zweiten Tastendruck.

**Wichtige Feinheit:** `Strg+N` öffnet immer das separate Schnellerfassungsfenster, nicht
den im Hauptfenster eingebetteten Eintrags-Editor (siehe [Eintrags-Editor](#eintrags-editor-hauptfenster)
unten für den zweiten, eigenständigen Weg, einen neuen Eintrag anzulegen).

## Command Palette (`Strg+K`)

`Strg+K` öffnet/schließt die Command Palette (`src/components/CommandPalette.tsx`) und ist
**immer aktiv**, auch während ein Textfeld fokussiert ist (eigener Capture-Phase-Listener
mit `stopPropagation`, läuft vor allen anderen Tastatur-Handlern).

| Taste | Wirkung |
|---|---|
| `Strg+K` | Palette öffnen bzw. schließen |
| `Pfeil ↓` | Nächsten Treffer auswählen |
| `Pfeil ↑` | Vorherigen Treffer auswählen |
| `Enter` | Ausgewählten Eintrag ausführen (Befehl, Kunde/System oder Volltext-Treffer) |
| `Esc` | Palette schließen |

Bei leerer Eingabe zeigt die Palette statische Befehle (u. a. "Neuer Eintrag", "Zu
Kundenliste", "Zu Systemliste", "Zum Journal", "Anhänge bereinigen"); ab dem ersten
Zeichen kommen serverseitige Treffer aus Verzeichnis-Suche (Kunden/Systeme) und
Volltextsuche (Einträge) hinzu. Der Befehl "Neuer Eintrag" zeigt in der Palette den Hinweis
`Strg+N` an, öffnet bei Auswahl aber den Eintrags-Editor im Hauptfenster
(`openEntryEditor("new")`) — nicht dasselbe Fenster, das die globale `Strg+N`-Taste öffnet
(siehe Hinweis oben).

## Ansichts-spezifische Tasten

Die folgenden Bindings sind jeweils nur lokal in ihrer Ansicht aktiv, und nur wenn kein
Formular offen ist und kein Textfeld fokussiert ist.

### Kundenliste (`src/components/CustomerListView.tsx`)

| Taste | Wirkung |
|---|---|
| `j` | Nächsten Kunden auswählen |
| `k` | Vorherigen Kunden auswählen |
| `Enter` | Zum ausgewählten Kunden navigieren (dessen Systemliste öffnen) |
| `e` | Ausgewählten Kunden bearbeiten (Formular öffnen) |

### Systemliste (`src/components/SystemListView.tsx`)

| Taste | Wirkung |
|---|---|
| `j` | Nächstes System auswählen |
| `k` | Vorheriges System auswählen |
| `e` | Ausgewähltes System bearbeiten (Formular öffnen) |
| `Enter` | *Nicht gebunden* — anders als in der Kundenliste gibt es hier keine Navigation in eine weitere Ebene, `Enter` hat aktuell keine Wirkung |

### Journal (`src/components/JournalView.tsx`)

| Taste | Wirkung |
|---|---|
| `j` | Nächsten Eintrag auswählen |
| `k` | Vorherigen Eintrag auswählen |
| `Enter` | Inline-Rohtext-Vorschau des ausgewählten Eintrags ein-/ausklappen (wechselt zwischen gekürzter Vorschau und dem vollständigen `body_md` als Rohtext, *ohne* etwas zu öffnen) |
| `e` | Ausgewählten Eintrag im vollständigen Eintrags-Editor öffnen |

`Enter` und `e` tun in dieser Ansicht **nicht** dasselbe: `Enter` bleibt in der Liste und
blendet nur eine Vorschau ein, `e` öffnet den echten Editor.

## Eintrags-Editor (Hauptfenster, `src/components/EntryEditor.tsx`)

Der Editor ist ein Modal im Hauptfenster für neue oder bestehende Einträge. Geöffnet wird
er über den Command-Palette-Befehl "Neuer Eintrag", über `e` im Journal, oder über die
Buttons "+ Neuer Eintrag" / "Bearbeiten" in den jeweiligen Ansichten — **nicht** über die
globale `Strg+N`-Taste (die öffnet stattdessen die Schnellerfassung, siehe oben).

| Taste | Wirkung |
|---|---|
| `Strg+S` | Eintrag speichern (legt neu an oder aktualisiert, je nach Kontext) — nur aktiv, während der Editor offen ist |
| `Esc` | Editor verwerfen/schließen — läuft über die globale `Esc`-Behandlung in `useGlobalHotkeys.ts` (gemeinsamer `formOpen`-Zustand), nicht über einen eigenen Listener im Editor |

## Schnellerfassungsfenster (separates Fenster, `src/quick-capture/QuickCapture.tsx`)

Eigenständiges, kompaktes Fenster — **nicht** das Hauptfenster. Geöffnet per globalem
Hotkey, per `Strg+N` im Hauptfenster, oder per CLI-Flag (`--quick-capture`). Das Titelfeld
wird beim Öffnen automatisch fokussiert.

| Taste | Wirkung |
|---|---|
| `Strg+V` | Bild aus der Zwischenablage einfügen — wird als Anhang vorgemerkt und als Markdown-Referenz an der Cursorposition eingefügt |
| `Strg+S` | Eintrag speichern und Fenster schließen |
| `Esc` | Entwurf verwerfen und Fenster schließen |

## Shortcut-Übersicht (`?`)

`?` öffnet eine In-App-Übersicht der Tastaturbelegung
(`src/components/ShortcutOverview.tsx`).

| Taste | Wirkung |
|---|---|
| `?` | Shortcut-Übersicht öffnen — nur außerhalb von Textfeldern, ohne Strg/Alt/Cmd |
| `Esc` | Übersicht schließen |

> **Hinweis:** Die in der App unter `?` angezeigte Tabelle ist eine statische Kopie der
> ursprünglichen Spec-Tabelle und wurde nicht an jede seither entstandene
> Implementierungs-Feinheit angepasst — sie listet z. B. `/` weiterhin als "Suche
> fokussieren", obwohl diese Taste bewusst nicht gebunden ist (siehe unten), und
> unterscheidet nicht zwischen den je Ansicht leicht unterschiedlichen `Enter`/`e`-
> Verhalten oben. Diese Datei (`SHORTCUTS.md`) ist die gegen den Code verifizierte
> Referenz.

## Noch nicht gebunden

| Taste | Status |
|---|---|
| `/` | **Noch nicht gebunden.** Die Spec sieht `/` zum Fokussieren einer Sucheingabe vor. Mangels eines einzigen, global eindeutigen Sucheingabefelds wurde die Bindung in Phase 4d bewusst zurückgestellt (siehe `docs/superpowers/plans/2026-09-07-wartungsdoku-plan-phase4d.md`). Volltextsuche ist heute über die Command Palette (`Strg+K`) erreichbar. |

## Plattformhinweise

- **Windows und Linux/X11:** Die globalen Hotkeys oben werden direkt vom Betriebssystem
  über `tauri-plugin-global-shortcut` registriert.
- **Linux/Wayland:** Globale Hotkey-Registrierung ist dort nur über das XDG-Portal
  `org.freedesktop.portal.GlobalShortcuts` möglich, das nicht auf jedem Compositor
  verfügbar ist. Fehlt es, funktionieren die drei globalen Hotkeys oben nicht
  zuverlässig. Als Fallback — unter Wayland ebenso wie unter X11/Windows, etwa für
  eigene Tastenkombinationen über Drittwerkzeuge — stehen die CLI-Flags
  `wartungsdoku --quick-capture` und `wartungsdoku --search` bereit
  (`src-tauri/src/cli.rs`), unabhängig von der Portal-Verfügbarkeit immer implementiert.
  `--search` zeigt wie `Strg+Alt+F` nur das Hauptfenster, ohne eigene Such-UI.
- **Fenster-Kontexterfassung** (Titel des zuvor aktiven Fensters als Notiz übernehmen,
  standardmäßig deaktiviert) ist unter Windows und Linux/X11 implementiert
  (`src-tauri/src/context_capture/`), unter Wayland nicht verfügbar und wird dort
  bewusst als nicht unterstützt gemeldet statt stillschweigend übersprungen. Der
  X11-Pfad wurde auf einer Windows-Entwicklungsmaschine geschrieben und dort nie
  kompiliert — die Datei `src-tauri/src/context_capture/x11.rs` weist selbst darauf hin
  (`NOT COMPILED OR TESTED ON THIS HOST`); vor produktivem Linux-Einsatz sollte das
  einmal real geprüft werden.
