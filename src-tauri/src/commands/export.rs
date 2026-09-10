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
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
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

/// Exports every active customer's Markdown journal into its own
/// subfolder (named by `short_code`, which is unique -- unlike `name`) under
/// `dest_dir`. `export::markdown::export_markdown` creates each subfolder
/// itself (see its own `std::fs::create_dir_all`), same as the
/// single-customer `export_markdown` command above already relies on.
///
/// Aborts on the first customer whose export fails (propagated via `?`)
/// rather than collecting per-customer errors and continuing -- see
/// `.report-export-all.md` for the reasoning (unlike a CSV import row typo,
/// a failure here is much more likely to be a systemic problem such as a
/// full disk or a permissions error that will just as surely doom every
/// other customer's export too, so failing loudly beats silently completing
/// a partial batch).
#[tauri::command]
pub fn export_markdown_all_customers(
    state: State<AppState>,
    from_utc: Option<String>,
    to_utc: Option<String>,
    dest_dir: String,
) -> Result<(), AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let (data_dir, late_entry_threshold_hours) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        (config.data_dir.clone(), config.late_entry_threshold_hours)
    };

    let customers = db::customers::list(&conn, false)?;
    let base_dir = std::path::Path::new(&dest_dir);
    for customer in customers {
        let filter = EntryFilter {
            customer_id: Some(customer.id),
            system_id: None,
            category: None,
            tag: None,
            from_utc: from_utc.clone(),
            to_utc: to_utc.clone(),
        };
        let customer_dir = base_dir.join(&customer.short_code);
        crate::export::markdown::export_markdown(
            &conn,
            &data_dir,
            &customer_dir,
            customer.id,
            &filter,
            late_entry_threshold_hours,
        )?;
    }
    Ok(())
}

/// Exports every active customer's PDF manual as a separate file (named
/// `<short_code>.pdf`) inside `dest_dir`. Same per-customer content as
/// `export_pdf` above, same abort-on-first-failure reasoning as
/// `export_markdown_all_customers`.
#[tauri::command]
pub fn export_pdf_all_customers(
    state: State<AppState>,
    from_utc: Option<String>,
    to_utc: Option<String>,
    dest_dir: String,
) -> Result<(), AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let (data_dir, late_entry_threshold_hours) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        (config.data_dir.clone(), config.late_entry_threshold_hours)
    };
    let tz = time::system_timezone()?;
    let (now_utc, now_tz) = time::now_with_tz(&tz);
    let generated_at_display = time::format_timestamp_for_display(&now_utc, &now_tz)?;

    std::fs::create_dir_all(&dest_dir)?;
    let base_dir = std::path::Path::new(&dest_dir);

    let customers = db::customers::list(&conn, false)?;
    for customer in customers {
        let systems = db::systems::list_by_customer(&conn, customer.id, false)?;
        let filter = EntryFilter {
            customer_id: Some(customer.id),
            system_id: None,
            category: None,
            tag: None,
            from_utc: from_utc.clone(),
            to_utc: to_utc.clone(),
        };
        let mut entries = db::entries::list(&conn, &filter)?;
        entries.reverse(); // see export_pdf above for why

        let sections = build_pdf_sections(&systems, entries, late_entry_threshold_hours)?;
        let pdf_bytes = export::pdf::render_manual_pdf(
            &data_dir,
            &customer.name,
            generated_at_display.clone(),
            sections,
        )?;
        let dest_path = base_dir.join(format!("{}.pdf", customer.short_code));
        std::fs::write(&dest_path, pdf_bytes)
            .map_err(|e| AppError::Io(format!("PDF konnte nicht geschrieben werden: {e}")))?;
    }
    Ok(())
}

/// Exports the "continuous manual" PDF (cover page, table of contents,
/// organized by system, embedded images, header/footer with customer name
/// and creation date) for one customer, optionally filtered by system
/// and/or time range. See the design spec's "Export" section under
/// Core Workflows.
#[tauri::command]
pub fn export_pdf(
    state: State<AppState>,
    customer_id: i64,
    system_id: Option<i64>,
    from_utc: Option<String>,
    to_utc: Option<String>,
    dest_path: String,
) -> Result<(), AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let (data_dir, late_entry_threshold_hours) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        (config.data_dir.clone(), config.late_entry_threshold_hours)
    };
    let tz = time::system_timezone()?;

    let customer = db::customers::get(&conn, customer_id)?;
    let systems = db::systems::list_by_customer(&conn, customer_id, false)?;

    let filter = EntryFilter {
        customer_id: Some(customer_id),
        system_id,
        category: None,
        tag: None,
        from_utc,
        to_utc,
    };
    let mut entries = db::entries::list(&conn, &filter)?;
    // `list` orders performed_at_utc DESC; the manual reads as a chronological
    // "grown history" per system (see the design spec's Systemansicht), so we
    // want ascending (oldest first) before grouping.
    entries.reverse();

    let sections = build_pdf_sections(&systems, entries, late_entry_threshold_hours)?;

    let (now_utc, now_tz) = time::now_with_tz(&tz);
    let generated_at_display = time::format_timestamp_for_display(&now_utc, &now_tz)?;

    let pdf_bytes =
        export::pdf::render_manual_pdf(&data_dir, &customer.name, generated_at_display, sections)?;

    std::fs::write(&dest_path, pdf_bytes)
        .map_err(|e| AppError::Io(format!("PDF konnte nicht geschrieben werden: {e}")))
}

/// Groups already-fetched (and already filtered by `db::entries::list`)
/// entries into per-system PDF sections: system order first (as given by
/// `systems`, e.g. active-only, name-sorted -- see the caller), then
/// "Ohne System" last for entries with no `system_id`. Only groups that
/// actually contain at least one entry become a section -- systems the
/// filter excluded entirely don't get an empty page, and "Ohne System"
/// itself is omitted if no entry has `system_id: None`.
///
/// Split out from `export_pdf` so it's unit-testable without a Tauri
/// `AppState`/`State` -- see this module's tests for the regression
/// coverage for a customer whose only matching entry has `system_id: None`
/// (see the "IntellyTec GmbH" production bug this guards against: an
/// entry with no system assigned silently vanishing from the PDF because
/// of an unrelated `system_id` filter).
fn build_pdf_sections(
    systems: &[db::systems::System],
    entries: Vec<db::entries::Entry>,
    late_entry_threshold_hours: i64,
) -> Result<Vec<PdfSystemSection>, AppError> {
    let mut group_order: Vec<Option<i64>> = systems.iter().map(|s| Some(s.id)).collect();
    group_order.push(None);
    let mut group_names: HashMap<Option<i64>, String> = systems
        .iter()
        .map(|s| (Some(s.id), s.name.clone()))
        .collect();
    group_names.insert(None, "Ohne System".to_string());

    let mut grouped: HashMap<Option<i64>, Vec<PdfEntry>> = HashMap::new();
    for entry in entries {
        let pdf_entry = to_pdf_entry(&entry, late_entry_threshold_hours)?;
        grouped.entry(entry.system_id).or_default().push(pdf_entry);
    }

    Ok(group_order
        .into_iter()
        .filter_map(|key| {
            let pdf_entries = grouped.remove(&key)?;
            if pdf_entries.is_empty() {
                return None;
            }
            let system_name = group_names
                .get(&key)
                .cloned()
                .unwrap_or_else(|| "Ohne System".to_string());
            Some(PdfSystemSection {
                system_name,
                entries: pdf_entries,
            })
        })
        .collect())
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
    let pattern =
        PATTERN.get_or_init(|| regex::Regex::new(r"!\[[^\]]*\]\((attachments/[^)]+)\)").unwrap());
    pattern
        .captures_iter(body_md)
        .map(|caps| caps[1].to_string())
        .collect()
}

fn to_pdf_entry(
    entry: &db::entries::Entry,
    late_entry_threshold_hours: i64,
) -> Result<PdfEntry, AppError> {
    let performed_at_display =
        time::format_timestamp_for_display(&entry.performed_at_utc, &entry.performed_at_tz)?;

    let performed_at = chrono::DateTime::parse_from_rfc3339(&entry.performed_at_utc)
        .map_err(|e| AppError::InvalidTimestamp(format!("{}: {e}", entry.performed_at_utc)))?;
    let created_at = chrono::DateTime::parse_from_rfc3339(&entry.created_at_utc)
        .map_err(|e| AppError::InvalidTimestamp(format!("{}: {e}", entry.created_at_utc)))?;
    let diff_hours = (created_at - performed_at).num_hours().abs();
    let late_entry_note = if diff_hours > late_entry_threshold_hours {
        let created_display =
            time::format_timestamp_for_display(&entry.created_at_utc, &entry.created_at_tz)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::customers::{self, NewCustomer};
    use crate::db::entries::{self, NewEntry};
    use crate::db::systems::{self, NewSystem};
    use crate::db::test_support::migrated_connection;
    use rusqlite::Connection;

    fn berlin() -> chrono_tz::Tz {
        "Europe/Berlin".parse().unwrap()
    }

    fn seed_customer(conn: &Connection, name: &str) -> i64 {
        customers::create(
            conn,
            NewCustomer {
                name: name.into(),
                short_code: name.to_uppercase(),
                notes: "".into(),
            },
            &berlin(),
        )
        .unwrap()
        .id
    }

    fn seed_system(conn: &Connection, customer_id: i64, name: &str) -> i64 {
        systems::create(
            conn,
            NewSystem {
                customer_id,
                name: name.into(),
                system_type: "".into(),
                hostname: "".into(),
                ip_address: "".into(),
                notes: "".into(),
                maintenance_interval_days: None,
            },
            &berlin(),
        )
        .unwrap()
        .id
    }

    fn seed_entry(
        conn: &Connection,
        data_dir: &std::path::Path,
        customer_id: i64,
        system_id: Option<i64>,
        title: &str,
        body_md: &str,
    ) -> entries::Entry {
        entries::create(
            conn,
            data_dir,
            NewEntry {
                customer_id,
                system_id,
                title: title.into(),
                body_md: body_md.into(),
                category: Category::Wartung,
                performed_at_utc: "2026-09-08T17:47:51.102Z".into(),
                performed_at_tz: "Europe/Berlin".into(),
                tag_names: vec![],
                pending_attachments: vec![],
            },
            &berlin(),
        )
        .unwrap()
    }

    // Reproduces the exact shape of the production bug: customer "IntellyTec
    // GmbH" (customer_id=1) has three systems (one archived) and exactly one
    // entry, "Eset enfernt", with `system_id: None` -- the user never
    // assigned it to a system. `export_pdf` calls `list_by_customer(..,
    // false)` (archived excluded), so `systems` here mirrors that: two
    // active systems, no matching entries for either.
    fn seed_production_shape(
        conn: &Connection,
        data_dir: &std::path::Path,
    ) -> (i64, Vec<db::systems::System>, entries::Entry) {
        let customer_id = seed_customer(conn, "IntellyTec GmbH");
        let archived_id = seed_system(conn, customer_id, "SRV-LF-SU-60 (Navision)");
        systems::archive(conn, archived_id, &berlin()).unwrap();
        seed_system(conn, customer_id, "ESET Protect");
        seed_system(conn, customer_id, "NB-IT-KU-1");
        let active_systems = systems::list_by_customer(conn, customer_id, false).unwrap();
        assert_eq!(
            active_systems.len(),
            2,
            "archived system must be excluded, mirroring export_pdf's own query"
        );

        let entry = seed_entry(
            conn,
            data_dir,
            customer_id,
            None,
            "Eset enfernt",
            "Für Testzwecke bezüglich webp Grafiken wurde ESET vom System entfernt.",
        );
        (customer_id, active_systems, entry)
    }

    #[test]
    fn build_pdf_sections_includes_ohne_system_entry_when_customer_has_other_systems() {
        let conn = migrated_connection();
        let data_dir = tempfile::tempdir().unwrap();
        let (_customer_id, systems, _entry) = seed_production_shape(&conn, data_dir.path());

        // Mirrors export_pdf's own unfiltered query (system_id: None) --
        // the export dialog's documented default, "no restriction".
        let filter = EntryFilter {
            customer_id: None,
            system_id: None,
            category: None,
            tag: None,
            from_utc: None,
            to_utc: None,
        };
        let mut fetched = entries::list(&conn, &filter).unwrap();
        fetched.reverse();
        assert_eq!(
            fetched.len(),
            1,
            "the one system_id: None entry must still be fetched when the filter is unrestricted"
        );

        let sections = build_pdf_sections(&systems, fetched, 24).unwrap();

        assert_eq!(
            sections.len(),
            1,
            "exactly one section ('Ohne System') should be produced, not zero"
        );
        assert_eq!(sections[0].system_name, "Ohne System");
        assert_eq!(sections[0].entries.len(), 1);
        assert_eq!(sections[0].entries[0].title, "Eset enfernt");
    }

    // Proves the actual mechanism behind the reported bug: an entry with
    // `system_id: None` is (correctly, by SQL NULL semantics) excluded
    // by `db::entries::list` the moment a *specific* `system_id` filter is
    // applied -- `system_id = ?2` is never true when `system_id` is NULL.
    // This is by-design filtering behavior, not itself a bug; the real bug
    // (fixed in ExportDialog's callers, see JournalView.tsx /
    // CommandPalette.tsx) was that the frontend could silently apply such a
    // filter -- a `system_id` left over in global UI state from an
    // unrelated earlier navigation -- without the user ever choosing it.
    // This test locks in the mechanism so a future change to the filter
    // semantics doesn't silently reintroduce the same class of bug.
    #[test]
    fn a_stale_system_filter_silently_excludes_the_ohne_system_entry() {
        let conn = migrated_connection();
        let data_dir = tempfile::tempdir().unwrap();
        let (customer_id, systems, _entry) = seed_production_shape(&conn, data_dir.path());
        let eset_protect_id = systems
            .iter()
            .find(|s| s.name == "ESET Protect")
            .unwrap()
            .id;

        // Simulates the exact production symptom: the export dialog's
        // System field silently carried over an unrelated system selection.
        let filter = EntryFilter {
            customer_id: Some(customer_id),
            system_id: Some(eset_protect_id),
            category: None,
            tag: None,
            from_utc: None,
            to_utc: None,
        };
        let fetched = entries::list(&conn, &filter).unwrap();
        assert!(fetched.is_empty(), "a system_id filter must exclude the system_id: None entry -- this is what produced the empty PDF");

        let sections = build_pdf_sections(&systems, fetched, 24).unwrap();
        assert!(
            sections.is_empty(),
            "no matching entries means no sections, reproducing the near-blank content page"
        );
    }

    #[test]
    fn renders_a_real_non_empty_pdf_for_the_production_shape() {
        let conn = migrated_connection();
        let data_dir = tempfile::tempdir().unwrap();
        let (_customer_id, systems, _entry) = seed_production_shape(&conn, data_dir.path());

        let filter = EntryFilter {
            customer_id: None,
            system_id: None,
            category: None,
            tag: None,
            from_utc: None,
            to_utc: None,
        };
        let mut fetched = entries::list(&conn, &filter).unwrap();
        fetched.reverse();

        let sections = build_pdf_sections(&systems, fetched, 24).unwrap();
        assert_eq!(sections.len(), 1);

        let pdf_bytes = export::pdf::render_manual_pdf(
            data_dir.path(),
            "IntellyTec GmbH",
            "08.09.2026 19:52 CEST".to_string(),
            sections,
        )
        .unwrap();

        assert!(pdf_bytes.starts_with(b"%PDF-"));
        // Typst's default PDF output isn't stream-compressed for text
        // content, so the rendered title/body do show up as literal bytes
        // in the PDF's content streams -- a direct, no-extra-dependency way
        // to confirm the entry's content actually made it into the output
        // (as opposed to only a cover page + empty TOC).
        let haystack = String::from_utf8_lossy(&pdf_bytes);
        assert!(
            haystack.contains("Eset enfernt"),
            "entry title should appear in the rendered PDF bytes"
        );
        assert!(
            haystack.contains("Ohne System"),
            "the 'Ohne System' section heading should appear in the rendered PDF bytes"
        );
    }
}
