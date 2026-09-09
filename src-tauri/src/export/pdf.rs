//! Assembles the comprehensive PDF manual (cover page, table of contents,
//! one section per system, embedded images, header/footer with customer name
//! and creation date) described in the "Export" section of the design spec.
//!
//! This module is deliberately ignorant of the app's database types
//! (`Entry`/`Customer`/...): its input is already-prepared display strings and
//! already-converted Typst body markup (see `PdfEntry`/`PdfSystemSection`),
//! which keeps it testable without any database or filesystem fixture beyond
//! the Typst compiler itself. The caller (`commands::export::export_pdf`)
//! owns all the `format_timestamp_for_display` / `markdown_to_typst::convert`
//! calls and the DB queries.
//!
//! Rendering uses `typst-as-lib` (a wrapper around the `typst` crate that
//! spares us implementing `typst::World` by hand) plus `typst_pdf` to turn the
//! compiled document into PDF bytes.
//!
//! Fonts: rendered with Typst's own bundled default fonts (Libertinus Serif /
//! New Computer Modern / DejaVu Sans Mono, all OFL-licensed, shipped inside
//! the `typst-assets` crate) via typst-as-lib's `typst-kit-fonts` +
//! `typst-kit-embed-fonts` cargo features (`TypstKitFontOptions`). No fonts
//! are bundled in this repository and none need to be downloaded -- this is
//! option (a)/(b) from the task's font-sourcing plan, and turned out to be a
//! built-in, first-class path rather than a fallback.
//!
//! Images: `image_relative_paths` on `PdfEntry` are paths relative to the
//! app's data directory (e.g. `attachments/a3/f2c9e1....png`, as produced by
//! `attachments::store::relative_path_for`). They are embedded for real, not
//! just listed by name: `typst-as-lib`'s `with_file_system_resolver(data_dir)`
//! lets the compiled Typst template call `#image("/" + path)` and have it
//! resolved straight from `data_dir` on disk. This is approach (2a) from the
//! task's image-embedding plan; it worked cleanly (see the module's tests),
//! so no fallback to base64-embedded bytes or filename-only listing was
//! needed.

use std::path::Path;

use typst::foundations::{Dict, IntoValue, Value};
use typst_as_lib::typst_kit_options::TypstKitFontOptions;
use typst_as_lib::TypstEngine;
use typst_layout::PagedDocument;

use crate::error::AppError;

static TEMPLATE: &str = include_str!("./templates/manual.typ");

/// One entry rendered into the manual. All fields are already display-ready:
/// this module does not format timestamps, convert Markdown, or otherwise
/// interpret the data -- see the module docs.
pub struct PdfEntry {
    pub title: String,
    pub category_label: String,
    pub performed_at_display: String,
    /// e.g. "Nachträglich erfasst: 09.09.2026 08:00 CEST" -- only `Some(..)`
    /// when `created_at` is meaningfully later than `performed_at`.
    pub late_entry_note: Option<String>,
    pub tags: Vec<String>,
    /// Already converted via `markdown_to_typst::convert()`.
    pub body_typst: String,
    /// Extracted from the entry's original `body_md`'s `![..](attachments/..)`
    /// references, in order of appearance; relative to `data_dir`.
    pub image_relative_paths: Vec<String>,
}

/// One system's worth of entries. `system_name` is `"Ohne System"` for
/// entries with no `system_id`, per the caller's grouping.
pub struct PdfSystemSection {
    pub system_name: String,
    pub entries: Vec<PdfEntry>,
}

impl IntoValue for PdfEntry {
    fn into_value(self) -> Value {
        let mut dict = Dict::new();
        dict.insert("title".into(), self.title.into_value());
        dict.insert("category_label".into(), self.category_label.into_value());
        dict.insert(
            "performed_at_display".into(),
            self.performed_at_display.into_value(),
        );
        dict.insert("late_entry_note".into(), self.late_entry_note.into_value());
        dict.insert("tags".into(), self.tags.into_value());
        dict.insert("body_typst".into(), self.body_typst.into_value());
        dict.insert("image_paths".into(), self.image_relative_paths.into_value());
        Value::Dict(dict)
    }
}

impl IntoValue for PdfSystemSection {
    fn into_value(self) -> Value {
        let mut dict = Dict::new();
        dict.insert("system_name".into(), self.system_name.into_value());
        dict.insert("entries".into(), self.entries.into_value());
        Value::Dict(dict)
    }
}

struct ManualInput {
    customer_name: String,
    generated_at: String,
    sections: Vec<PdfSystemSection>,
}

impl From<ManualInput> for Dict {
    fn from(value: ManualInput) -> Self {
        let mut dict = Dict::new();
        dict.insert("customer_name".into(), value.customer_name.into_value());
        dict.insert("generated_at".into(), value.generated_at.into_value());
        dict.insert("sections".into(), value.sections.into_value());
        dict
    }
}

/// Renders the full customer manual to PDF bytes.
///
/// `data_dir` is used only as the root for resolving `image_relative_paths`
/// (via `#image("/" + path)` in the template) -- it is not read for anything
/// else.
pub fn render_manual_pdf(
    data_dir: &Path,
    customer_name: &str,
    generated_at_display: String,
    sections: Vec<PdfSystemSection>,
) -> Result<Vec<u8>, AppError> {
    let input = ManualInput {
        customer_name: customer_name.to_string(),
        generated_at: generated_at_display,
        sections,
    };

    let engine = TypstEngine::builder()
        .main_file(TEMPLATE)
        .search_fonts_with(TypstKitFontOptions::new().include_system_fonts(false))
        .with_file_system_resolver(data_dir)
        .build();

    let warned = engine.compile_with_input(input);
    let doc: PagedDocument = warned
        .output
        .map_err(|e| AppError::Io(format!("PDF-Vorlage konnte nicht kompiliert werden: {e}")))?;

    let pdf_bytes = typst_pdf::pdf(&doc, &Default::default())
        .map_err(|e| AppError::Io(format!("PDF-Erzeugung fehlgeschlagen: {e:?}")))?;

    Ok(pdf_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry(title: &str) -> PdfEntry {
        PdfEntry {
            title: title.to_string(),
            category_label: "Wartung".to_string(),
            performed_at_display: "07.09.2026 14:32 CEST".to_string(),
            late_entry_note: None,
            tags: vec!["exchange".to_string(), "backup".to_string()],
            body_typst: "Kurzer Bericht mit *fett* und _kursiv_.".to_string(),
            image_relative_paths: vec![],
        }
    }

    #[test]
    fn renders_a_minimal_manual_to_real_pdf_bytes() {
        let data_dir = tempfile::tempdir().unwrap();
        let sections = vec![
            PdfSystemSection {
                system_name: "Fileserver FS01".to_string(),
                entries: vec![sample_entry("Update eingespielt")],
            },
            PdfSystemSection {
                system_name: "Ohne System".to_string(),
                entries: vec![sample_entry("Telefonat mit Kunde")],
            },
        ];

        let pdf_bytes = render_manual_pdf(
            data_dir.path(),
            "ACME GmbH",
            "07.09.2026 15:00 CEST".to_string(),
            sections,
        )
        .expect("PDF-Rendering sollte erfolgreich sein");

        assert!(!pdf_bytes.is_empty());
        assert!(pdf_bytes.starts_with(b"%PDF-"));
    }

    #[test]
    fn renders_late_entry_note_and_tags_without_error() {
        let data_dir = tempfile::tempdir().unwrap();
        let mut entry = sample_entry("Nachträglich dokumentiert");
        entry.late_entry_note = Some("Nachträglich erfasst: 09.09.2026 08:00 CEST".to_string());

        let sections = vec![PdfSystemSection {
            system_name: "Ohne System".to_string(),
            entries: vec![entry],
        }];

        let pdf_bytes = render_manual_pdf(
            data_dir.path(),
            "ACME GmbH",
            "07.09.2026 15:00 CEST".to_string(),
            sections,
        )
        .expect("PDF-Rendering sollte erfolgreich sein");

        assert!(pdf_bytes.starts_with(b"%PDF-"));
    }

    #[test]
    fn embeds_a_real_image_resolved_from_data_dir() {
        let data_dir = tempfile::tempdir().unwrap();
        let attachments_dir = data_dir.path().join("attachments").join("ab");
        std::fs::create_dir_all(&attachments_dir).unwrap();
        // Smallest possible valid PNG (1x1 transparent pixel).
        let png_bytes: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        std::fs::write(attachments_dir.join("abc123.png"), png_bytes).unwrap();

        let mut entry = sample_entry("Mit Screenshot");
        entry.image_relative_paths = vec!["attachments/ab/abc123.png".to_string()];

        let sections = vec![PdfSystemSection {
            system_name: "Ohne System".to_string(),
            entries: vec![entry],
        }];

        let pdf_bytes = render_manual_pdf(
            data_dir.path(),
            "ACME GmbH",
            "07.09.2026 15:00 CEST".to_string(),
            sections,
        )
        .expect("PDF-Rendering mit eingebettetem Bild sollte erfolgreich sein");

        assert!(pdf_bytes.starts_with(b"%PDF-"));
    }

    #[test]
    fn fails_gracefully_when_referenced_image_is_missing() {
        let data_dir = tempfile::tempdir().unwrap();
        let mut entry = sample_entry("Fehlendes Bild");
        entry.image_relative_paths = vec!["attachments/zz/does-not-exist.png".to_string()];

        let sections = vec![PdfSystemSection {
            system_name: "Ohne System".to_string(),
            entries: vec![entry],
        }];

        let result = render_manual_pdf(
            data_dir.path(),
            "ACME GmbH",
            "07.09.2026 15:00 CEST".to_string(),
            sections,
        );
        assert!(matches!(result, Err(AppError::Io(_))));
    }

    #[test]
    fn empty_sections_still_produce_a_valid_cover_page_pdf() {
        let data_dir = tempfile::tempdir().unwrap();
        let pdf_bytes = render_manual_pdf(
            data_dir.path(),
            "ACME GmbH",
            "07.09.2026 15:00 CEST".to_string(),
            vec![],
        )
        .expect("PDF-Rendering ohne Einträge sollte erfolgreich sein");
        assert!(pdf_bytes.starts_with(b"%PDF-"));
    }
}
