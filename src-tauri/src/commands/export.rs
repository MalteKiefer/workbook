use std::collections::HashMap;

use tauri::State;

use crate::db::entries::{Category, EntryFilter};
use crate::export::pdf::{PdfEntry, PdfSystemSection};
use crate::{db, export, time, AppError, AppState};

#[tauri::command]
pub fn export_markdown(
    state: State<AppState>,
    customer_id: i64,
    system_id: Option<i64>,
    from_utc: Option<String>,
    to_utc: Option<String>,
    dest_dir: String,
) -> Result<(), AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    let (data_dir, late_entry_threshold_hours) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        (config.data_dir.clone(), config.late_entry_threshold_hours)
    };
    let filter = EntryFilter {
        customer_id: Some(customer_id),
        system_id,
        category: None,
        tag: None,
        from_utc,
        to_utc,
    };
    crate::export::markdown::export_markdown(
        &conn,
        &data_dir,
        std::path::Path::new(&dest_dir),
        customer_id,
        &filter,
        late_entry_threshold_hours,
    )
}

/// Exports the "durchgehendes Handbuch" PDF (Deckblatt, Inhaltsverzeichnis,
/// Gliederung nach System, eingebettete Bilder, Kopf-/Fußzeile mit Kundenname
/// und Erstellungsdatum) for one customer, optionally filtered by system
/// and/or time range. See the design spec's "Export" section under
/// Kernworkflows.
#[tauri::command]
pub fn export_pdf(
    state: State<AppState>,
    customer_id: i64,
    system_id: Option<i64>,
    from_utc: Option<String>,
    to_utc: Option<String>,
    dest_path: String,
) -> Result<(), AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    let (data_dir, late_entry_threshold_hours) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        (config.data_dir.clone(), config.late_entry_threshold_hours)
    };
    let tz = time::system_timezone()?;

    let customer = db::customers::get(&conn, customer_id)?;
    let systems = db::systems::list_by_customer(&conn, customer_id, false)?;

    let filter =
        EntryFilter { customer_id: Some(customer_id), system_id, category: None, tag: None, from_utc, to_utc };
    let mut entries = db::entries::list(&conn, &filter)?;
    // `list` orders performed_at_utc DESC; the manual reads as a chronological
    // "grown history" per system (see the design spec's Systemansicht), so we
    // want ascending (oldest first) before grouping.
    entries.reverse();

    // System-name order for grouping, "Ohne System" last. Only groups that
    // actually contain an entry in the filtered range become a section --
    // systems the filter excluded entirely don't get an empty page.
    let mut group_order: Vec<Option<i64>> = systems.iter().map(|s| Some(s.id)).collect();
    group_order.push(None);
    let mut group_names: HashMap<Option<i64>, String> =
        systems.iter().map(|s| (Some(s.id), s.name.clone())).collect();
    group_names.insert(None, "Ohne System".to_string());

    let mut grouped: HashMap<Option<i64>, Vec<PdfEntry>> = HashMap::new();
    for entry in entries {
        let pdf_entry = to_pdf_entry(&entry, late_entry_threshold_hours)?;
        grouped.entry(entry.system_id).or_default().push(pdf_entry);
    }

    let sections: Vec<PdfSystemSection> = group_order
        .into_iter()
        .filter_map(|key| {
            let pdf_entries = grouped.remove(&key)?;
            if pdf_entries.is_empty() {
                return None;
            }
            let system_name = group_names.get(&key).cloned().unwrap_or_else(|| "Ohne System".to_string());
            Some(PdfSystemSection { system_name, entries: pdf_entries })
        })
        .collect();

    let (now_utc, now_tz) = time::now_with_tz(&tz);
    let generated_at_display = time::format_timestamp_for_display(&now_utc, &now_tz)?;

    let pdf_bytes = export::pdf::render_manual_pdf(&data_dir, &customer.name, generated_at_display, sections)?;

    std::fs::write(&dest_path, pdf_bytes)
        .map_err(|e| AppError::Io(format!("PDF konnte nicht geschrieben werden: {e}")))
}

fn category_label(category: Category) -> &'static str {
    match category {
        Category::Wartung => "Wartung",
        Category::Stoerung => "Störung",
        Category::Aenderung => "Änderung",
        Category::Installation => "Installation",
        Category::Sonstiges => "Sonstiges",
    }
}

/// Extracts `attachments/..` paths from `![alt](attachments/..)` Markdown
/// image references in `body_md`, in order of appearance. Deliberately not
/// part of `markdown_to_typst::convert` -- that function only handles text
/// markup, images are a separate concern resolved against `data_dir` by
/// `export::pdf::render_manual_pdf`.
fn extract_image_paths(body_md: &str) -> Vec<String> {
    static PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = PATTERN.get_or_init(|| regex::Regex::new(r"!\[[^\]]*\]\((attachments/[^)]+)\)").unwrap());
    pattern.captures_iter(body_md).map(|caps| caps[1].to_string()).collect()
}

fn to_pdf_entry(entry: &db::entries::Entry, late_entry_threshold_hours: i64) -> Result<PdfEntry, AppError> {
    let performed_at_display = time::format_timestamp_for_display(&entry.performed_at_utc, &entry.performed_at_tz)?;

    let performed_at = chrono::DateTime::parse_from_rfc3339(&entry.performed_at_utc)
        .map_err(|e| AppError::InvalidTimestamp(format!("{}: {e}", entry.performed_at_utc)))?;
    let created_at = chrono::DateTime::parse_from_rfc3339(&entry.created_at_utc)
        .map_err(|e| AppError::InvalidTimestamp(format!("{}: {e}", entry.created_at_utc)))?;
    let diff_hours = (created_at - performed_at).num_hours().abs();
    let late_entry_note = if diff_hours > late_entry_threshold_hours {
        let created_display = time::format_timestamp_for_display(&entry.created_at_utc, &entry.created_at_tz)?;
        Some(format!("Nachträglich erfasst: {created_display}"))
    } else {
        None
    };

    Ok(PdfEntry {
        title: entry.title.clone(),
        category_label: category_label(entry.category).to_string(),
        performed_at_display,
        late_entry_note,
        tags: entry.tags.clone(),
        body_typst: export::markdown_to_typst::convert(&entry.body_md),
        image_relative_paths: extract_image_paths(&entry.body_md),
    })
}
