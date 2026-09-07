# Phase 6 — Export: PDF (Handbuch)

## Ziel

PDF-Export für einen Kunden (optional gefiltert nach System/Zeitraum, via
bestehender `EntryFilter`): "ein durchgehendes Handbuch mit Deckblatt,
Inhaltsverzeichnis, Gliederung nach System, eingebetteten Bildern, Kopf- und
Fußzeile mit Kundenname und Erstellungsdatum" (Spec, Abschnitt "Export" unter
Kernworkflows).

Jeder Eintrag trägt den vollständigen `performed_at`-Zeitstempel mit
ausgeschriebener Zone; weicht `created_at` um mehr als den konfigurierbaren
Schwellwert (`late_entry_threshold_hours`, Default 24h) ab, wird zusätzlich der
Erfassungszeitpunkt ausgewiesen ("Nachträglich erfasst"). Der Export selbst
trägt einen eigenen Erstellungszeitstempel im gleichen Format.

Dies ist die höchste-Unsicherheit-Aufgabe des gesamten Projekts (neue,
komplexe Abhängigkeit `typst`/`typst-as-lib`/`typst-pdf`, keine Vorarbeit im
Repo). Entsprechend ausführlich unten die tatsächlich verifizierten
API-Details statt Annahmen aus der Aufgabenbeschreibung.

## Technologie-Entscheidungen (mit tatsächlich verifizierter Begründung)

### Crates und Versionen

`typst-as-lib = "0.16.0"` (Wrapper, der die Implementierung von `typst::World`
erspart) + `typst-pdf = "=0.15.1"` + `typst = "=0.15.1"` + `typst-layout =
"=0.15.1"` (letztere zwei direkt, da `typst-as-lib` seinerseits `typst`/
`typst-layout` nicht re-exportiert — `PagedDocument` kommt aus `typst_layout`,
nicht aus `typst`). Alle vier resolven konsistent auf Typst-Version 0.15.1
(per `cargo add`/`Cargo.lock` verifiziert, kein Versionskonflikt).

### Font-Quelle: Typst-eigene eingebettete Fonts (kein Download nötig)

Der Auftrag nannte drei Eskalationsstufen (a: leer/Default probieren, b:
`typst-kit`, c: manueller Font-Download). **Ergebnis: Stufe (b), aber als
first-class Feature, nicht als Bastellösung.**

`typst-as-lib` bietet dafür ein eigenes Cargo-Feature-Paar:
`typst-kit-fonts` (aktiviert `TypstEngine::builder().search_fonts_with(..)`
und das Modul `typst_as_lib::typst_kit_options::TypstKitFontOptions`) und
`typst-kit-embed-fonts` (aktiviert `TypstKitFontOptions::include_embedded_fonts`,
Default `true`). Damit liefert `typst_kit::fonts::embedded()` — ohne dass
dieses Projekt `typst-kit` selbst als direkte Abhängigkeit braucht — Typst's
eigene mitgelieferte Fonts aus der `typst-assets`-Crate:

- **Libertinus Serif** (Regular/Bold/Italic/Semibold, OFL) — Typst's
  Standard-Textfont, deckt lateinische Zeichen inkl. deutscher Umlaute/ß ab
- **New Computer Modern** (NewCM10, OFL) — Alternative/Mathe-Font
- **DejaVu Sans Mono** (Bitstream Vera-Lizenz, freizügig) — Monospace

In `src-tauri/Cargo.toml`:

```toml
typst-as-lib = { version = "0.16.0", features = ["typst-kit-fonts", "typst-kit-embed-fonts"] }
```

In `pdf.rs`:

```rust
TypstEngine::builder()
    .main_file(TEMPLATE)
    .search_fonts_with(TypstKitFontOptions::new().include_system_fonts(false))
    .with_file_system_resolver(data_dir)
    .build()
```

`include_system_fonts(false)`, damit das Rendering nicht vom Font-Bestand der
jeweiligen Windows/Linux-Maschine abhängt (Reproduzierbarkeit, keine
Laufzeit-Systemscan-Kosten). Kein Font-Download, kein `assets/fonts/`-Ordner
nötig — im Gegensatz zur ursprünglichen Annahme im Auftrag, dieser Weg wäre
ein Fallback, ist es tatsächlich der sauberste verfügbare Pfad.

Verifiziert durch Lektüre des tatsächlichen Quellcodes von
`typst-as-lib-0.16.0` (aus dem lokalen Cargo-Registry-Cache, nicht aus dem
README) sowie durch reale, erfolgreiche `cargo test`-Läufe, die PDFs mit Text
rendern (deutsche Umlaute in `sample_entry`-Testdaten in `pdf.rs`).

### Bild-Einbettung: Dateisystem-Resolver (Stufe 2a aus dem Auftrag — hat funktioniert)

`typst-as-lib::TypstTemplateEngineBuilder::with_file_system_resolver(root)`
registriert einen `FileResolver`, der Typst-Dateireferenzen relativ zu `root`
aus dem Dateisystem auflöst. Mit `root = data_dir` und einer Bildreferenz
`#image("/" + relativer_pfad)` im Template (`relativer_pfad` z. B.
`attachments/a3/f2c9e1....png`, exakt das Format von
`attachments::store::relative_path_for`) liest Typst die Datei direkt von
Platte — kein Base64, kein manuelles Byte-Handling in Rust.

Verifiziert durch Lektüre von `typst-syntax`'s `VirtualPath`/`FileId`-Code
(Root-Pfad-Auflösung ist unabhängig vom "aktuellen Datei"-Konzept, ein
`&str`-Pfad wird immer gegen `VirtualRoot::Project` aufgelöst — die Root, die
auch dem Hauptdokument zugewiesen ist) **und** durch einen echten Test
(`pdf.rs::tests::embeds_a_real_image_resolved_from_data_dir`), der ein
1×1-PNG in ein Temp-`data_dir` schreibt, per `render_manual_pdf` referenziert
und ein gültiges PDF zurückbekommt. Das ist der stärkste Beleg in dieser
gesamten Aufgabe: Stufe 2a (bevorzugte Variante laut Auftrag) hat beim ersten
Versuch ohne Kompromisse funktioniert, Stufe 2b/2c (Base64-Einbettung bzw.
reine Dateinamen-Auflistung) waren nicht nötig.

Einschränkung: Fehlt eine referenzierte Bilddatei tatsächlich auf der Platte,
schlägt die Typst-Kompilierung fehl (`#image()` wirft einen Kompilierfehler)
und `render_manual_pdf` gibt `Err(AppError::Io(..))` zurück — der gesamte
Export bricht ab, statt das fehlende Bild stillschweigend zu überspringen.
Das ist bewusst so belassen (Fehler sichtbar statt verschluckt, siehe Spec
"Fehlerbehandlung"); durch
`pdf.rs::tests::fails_gracefully_when_referenced_image_is_missing` abgesichert.

### Markdown → Typst: `#eval(text, mode: "markup")`

Die pro Eintrag vorkonvertierte `body_typst`-Zeichenkette wird nicht direkt
als Typst-Quelltext ins Template interpoliert (Injection-Risiko, siehe
`markdown_to_typst`-Eskaping unten), sondern über Typst's eingebaute
`eval("...", mode: "markup")`-Funktion zur Laufzeit als Markup ausgewertet —
verifiziert im tatsächlichen `typst-library`-Quellcode
(`foundations/mod.rs::eval`, Parameter `mode: SyntaxMode`, Default `Code`,
`"markup"` explizit dokumentiert). Damit bleibt die eigentliche Typst-
Quelldatei (`templates/manual.typ`) statisch und wird nur einmal geparst;
nur der Eintragstext wird pro Eintrag als Teilausdruck ausgewertet.

Um zu verhindern, dass eine `#`-Überschrift *innerhalb* eines Eintragstexts
mit den Dokument-eigenen Überschriftenebenen (System = `=`, Eintragstitel =
`==`) kollidiert, wird der eval-Aufruf in einen Block mit
`#set heading(offset: 2)` gehüllt — Ebene-1-Überschriften im Eintragstext
landen dadurch auf Typst-Ebene 3, außerhalb der im Inhaltsverzeichnis
gezeigten Tiefe (`#outline(depth: 2)`).

## Bausteine

### 1. `src-tauri/src/export/markdown_to_typst.rs` (neu)

Reine, IO-freie Funktion `pub fn convert(body_md: &str) -> String`. Zeilenweise
Verarbeitung: Leerzeile → Absatzumbruch, `#`/`##`/`###` → `=`/`==`/`===`,
`-`/`*` am Zeilenanfang → `-`, `\d+\.` → `+`, sonst Inline-Konvertierung
(Bold, Italic, Inline-Code, Links) über eine kombinierte Regex mit benannten
Gruppen. Alles außerhalb erkannter Syntax wird literal escaped (`\` zuerst,
dann `# * _ \` < > @ $`), damit weder ein rohes `#`/`$` in einer Notiz noch
unerwartete Markdown-Reste die Typst-Kompilierung brechen können.

Bekannte, bewusst hingenommene Lücke: ein `=` am Zeilenanfang (in Typst
Überschriftensyntax) wird **nicht** escaped, da die Aufgabenstellung den
Escape-Satz explizit auf `# * _ \` < > @ $` festlegt. Für den Alleinnutzer
dieser App (eigene Notizen, kein Fremdinput) ist das Risiko gering; als
bekannte Lücke hier dokumentiert statt stillschweigend zu ignorieren.

Bilder werden hier bewusst **nicht** behandelt — die App erzeugt
Bildreferenzen selbst immer als
`![...](attachments/..)`, und deren Extraktion passiert separat in
`commands::export::export_pdf`, damit `pdf.rs` von den Roh-Pfaden erfährt statt
sie aus bereits konvertiertem Typst-Markup zurückparsen zu müssen.

11 Unit-Tests in `#[cfg(test)] mod tests` — Überschriften je Ebene,
Bold/Italic (beide Schreibweisen), Listen (un/geordnet), Inline-Code, Link,
Absatzumbruch, Escaping von `#`/`$`, Escaping von `\` selbst (muss zuerst
passieren, sonst Doppel-Escaping), Escaping innerhalb von Bold/Link-Text,
reiner Text ohne Sonderzeichen.

### 2. `src-tauri/src/export/pdf.rs` (neu)

`PdfEntry`/`PdfSystemSection` (Plain-Data-Structs, siehe Aufgabenvorgabe) plus
`pub fn render_manual_pdf(data_dir, customer_name, generated_at_display,
sections) -> Result<Vec<u8>, AppError>`. Kennt keine DB-/Entry-Typen — nimmt
ausschließlich bereits formatierte Anzeige-Strings und bereits konvertiertes
Typst-Markup entgegen (Aufrufer in `commands/export.rs` erledigt
`format_timestamp_for_display`/`markdown_to_typst::convert`). Baut daraus ein
`typst::foundations::Dict` (manuelle `IntoValue`-Implementierung pro Struct,
nach dem Muster aus `typst-as-lib`s eigenem `small_example.rs`), kompiliert
`templates/manual.typ` mit `typst-as-lib` + den oben beschriebenen
eingebetteten Fonts + Dateisystem-Resolver, erzeugt die PDF-Bytes über
`typst_pdf::pdf(&doc, &Default::default())`.

`templates/manual.typ`: Deckblatt (`#align(center+horizon)[...]` mit
Kundenname + Erstellungsdatum), Seitenumbruch, `#outline(title:
"Inhaltsverzeichnis", depth: 2)`, Seitenumbruch, danach
`#set page(header: .., footer: context [...])` mit Kundenname + Erstellungs-
datum links/rechts im Kopf, Seitenzahl im Fuß (aktiv erst **ab** dieser
Stelle im Dokumentfluss — Deckblatt und Inhaltsverzeichnis bleiben dadurch
kopf-/fußzeilenfrei, ohne fragile Seitenzahl-Bedingung). Danach `#for section
in sections [ = #section.system_name #for entry in section.entries [ ==
#entry.title ... ] ]` — eine Überschriftenebene 1 je System, Ebene 2 je
Eintrag, darunter Metazeile (Kategorie · Zeitpunkt · optional
"Nachträglich erfasst"-Hinweis), Tags, der per `#eval(..., mode: "markup")`
gerenderte Eintragstext, danach je referenziertem Bild ein `#image(...)`.

5 Tests in `#[cfg(test)] mod tests`, alle **echt gegen den Typst-Compiler
laufend**, kein Mock: minimales Handbuch mit zwei Systemen (PDF-Magic-Bytes-
Assertion), Eintrag mit "Nachträglich erfasst"-Hinweis und Tags, Eintrag mit
echtem eingebettetem PNG (siehe oben), Eintrag mit fehlendem Bild (erwarteter
`AppError::Io`), leere Sections-Liste (Deckblatt-only-PDF).

### 3. `src-tauri/src/commands/export.rs` (ergänzt, nicht überschrieben)

Datei existierte bereits (paralleler Markdown-Export-Agent, `export_markdown`-
Kommando). `export_pdf` wurde als zweite Funktion in dieselbe Datei ergänzt,
`export_markdown` unverändert gelassen. Baut `EntryFilter` aus den Parametern,
lädt Kunde + Systeme, gruppiert die (auf chronologisch aufsteigend
umgedrehten) Einträge nach `system_id` in Systemnamen-Reihenfolge
(`"Ohne System"` zuletzt, nur Gruppen mit mindestens einem Eintrag im
gefilterten Zeitraum werden zu einer Section), berechnet pro Eintrag
Kategorie-Label (eigene kleine Funktion, wie beim Markdown-Export —
`Category` hat keine öffentliche deutsche Label-Methode), Anzeige-Zeitstempel,
"Nachträglich erfasst"-Hinweis (`chrono::DateTime::parse_from_rfc3339`,
Vergleich gegen `late_entry_threshold_hours`) und extrahiert Bildpfade aus dem
rohen `body_md` per kleiner lokaler Regex (`!\[...\]\((attachments/[^)]+)\)`).
Schreibt die PDF-Bytes abschließend synchron mit `std::fs::write`.

### 4. Nicht Teil dieses Bausteins

- Registrierung von `pub mod export;` in `lib.rs` (bereits durch den
  parallelen Markdown-Export-Baustein benötigt, hier nur temporär für die
  lokale Verifikation eingefügt und danach exakt zurückgenommen — siehe
  "Verifikation" unten) sowie die `invoke_handler!`-Zeile für `export_pdf` —
  beide bleiben der Integrationsrunde vorbehalten (Boundaries der
  Aufgabenstellung).
- Frontend-Anbindung (Export-Dialog-Aufruf) — reines Rust-Backend-Ticket.

## Verifikation

`src-tauri/src/lib.rs` ist laut Aufgabenstellung nicht anfassbar. Da sowohl
`pdf.rs`/`markdown_to_typst.rs` als auch das bereits vorhandene
`commands/export.rs` aber nur über ein `pub mod export;` auf Crate-Root-Ebene
erreichbar sind (der `export`-Ordner ist ein neues Top-Level-Modul, nicht
unter `commands` verschachtelt), wurde diese eine Zeile **temporär** in
`lib.rs` eingefügt, `cargo check --lib`, `cargo test --lib export::` und
`cargo test --lib` (komplette Suite, 95 Tests, keine Regression) ausgeführt,
und die Zeile danach exakt wieder entfernt (`git diff -- src-tauri/src/lib.rs`
zeigt anschließend keine Änderung). So ist der komplette PDF-Baustein real
gegen den echten Compiler verifiziert, ohne die Boundary dauerhaft zu
verletzen oder parallele Agenten-Änderungen an `lib.rs` zu riskieren (kein
`git checkout`, nur ein chirurgisches Hinzufügen+Entfernen derselben Zeile).
