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

/// Exports the aggregated compliance/audit-trail report PDF for one
/// customer: that customer's own audit-log entries together with every one
/// of its systems' entries (including archived systems -- a decommissioned
/// system's history is still part of the compliance record), merged into
/// one chronological table. Deliberately per-customer only, unlike
/// `export_pdf`/`export_markdown` above -- no "all customers" batch variant.
#[tauri::command]
pub fn export_audit_report_pdf(
    state: State<AppState>,
    customer_id: i64,
    dest_path: String,
) -> Result<(), AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;

    let customer = db::customers::get(&conn, customer_id)?;
    let rows = collect_audit_rows(&conn, customer_id)?;

    let tz = time::system_timezone()?;
    let (now_utc, now_tz) = time::now_with_tz(&tz);
    let generated_at_display = time::format_timestamp_for_display(&now_utc, &now_tz)?;

    let pdf_bytes =
        export::pdf::render_audit_report_pdf(&customer.name, generated_at_display, rows)?;

    std::fs::write(&dest_path, pdf_bytes)
        .map_err(|e| AppError::Io(format!("PDF konnte nicht geschrieben werden: {e}")))
}

/// Collects and chronologically sorts one customer's own audit-log entries
/// together with every one of its systems' entries (including archived
/// systems -- see export_audit_report_pdf's doc comment) into display-ready
/// rows. Split out from export_audit_report_pdf so it's unit-testable
/// without a Tauri State/AppState, same rationale as build_pdf_sections
/// above.
fn collect_audit_rows(
    conn: &rusqlite::Connection,
    customer_id: i64,
) -> Result<Vec<export::pdf::AuditReportRow>, AppError> {
    // Archived systems included deliberately -- a decommissioned system's
    // history is still part of the compliance record, unlike the regular
    // manual/journal exports which default to active-only.
    let systems = db::systems::list_by_customer(conn, customer_id, true)?;

    struct RawRow {
        id: i64,
        at_utc: String,
        at_tz: String,
        scope_label: String,
        action: String,
        summary: String,
    }

    let mut raw_rows: Vec<RawRow> = Vec::new();
    for entry in db::audit_log::list_for_entity(conn, "customer", customer_id)? {
        raw_rows.push(RawRow {
            id: entry.id,
            at_utc: entry.at_utc,
            at_tz: entry.at_tz,
            scope_label: "Kunde".to_string(),
            action: entry.action,
            summary: entry.summary,
        });
    }
    for system in &systems {
        for entry in db::audit_log::list_for_entity(conn, "system", system.id)? {
            raw_rows.push(RawRow {
                id: entry.id,
                at_utc: entry.at_utc,
                at_tz: entry.at_tz,
                scope_label: system.name.clone(),
                action: entry.action,
                summary: entry.summary,
            });
        }
    }

    // Sort on the raw RFC3339 UTC string, NOT the display string -- a
    // formatted "12.09.2026 14:30 CEST" does not sort chronologically as
    // text, but an untouched RFC3339 UTC timestamp does (fixed-width,
    // most-significant-first). Ascending (oldest first), matching
    // export_pdf's own "grown history" reading order for the manual export.
    //
    // Tiebreak on `id` (ascending): `list_for_entity` already returns each
    // entity's own rows DESCENDING (`at_utc DESC, id DESC`, see its own doc
    // comment on same-millisecond writes), and `Vec::sort_by` is a stable
    // sort -- without an explicit tiebreak here, two rows sharing an
    // identical `at_utc` would keep that DESCENDING relative order after
    // this merge, locally reversing just that tie group against the
    // otherwise-ascending result.
    raw_rows.sort_by(|a, b| a.at_utc.cmp(&b.at_utc).then(a.id.cmp(&b.id)));

    let mut rows = Vec::with_capacity(raw_rows.len());
    for r in raw_rows {
        let at_display = time::format_timestamp_for_display(&r.at_utc, &r.at_tz)?;
        rows.push(export::pdf::AuditReportRow {
            at_display,
            scope_label: r.scope_label,
            action: r.action,
            summary: r.summary,
        });
    }
    Ok(rows)
}

/// Exports a single `.ics` (iCalendar) file covering every active customer's
/// systems with a maintenance interval (one all-day event per system, on its
/// next computed due date) and every tracked expiring item (one all-day
/// event per item, with a `VALARM` reminder matching its own
/// `reminder_days_before`). Global across all active customers, not filtered
/// to one -- see `ics` module docs. A snapshot export like the other
/// `export_*` commands, not a live sync.
#[tauri::command]
pub fn export_calendar_ics(state: State<AppState>, dest_path: String) -> Result<(), AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;

    let tz = time::system_timezone()?;

    let customers = db::customers::list(&conn, false)?;

    let mut maintenance_events = Vec::new();
    for customer in &customers {
        let systems = db::systems::list_by_customer(&conn, customer.id, false)?;
        for system in systems {
            let Some(interval_days) = system.maintenance_interval_days else {
                continue;
            };
            let last_performed_at_utc = db::systems::latest_performed_at(&conn, system.id)?;
            let baseline_str = last_performed_at_utc
                .as_deref()
                .unwrap_or(&system.created_at_utc);
            let Ok(baseline) = chrono::DateTime::parse_from_rfc3339(baseline_str) else {
                continue;
            };
            let Some(duration) = chrono::Duration::try_days(interval_days) else {
                continue;
            };
            let Some(due_on) = baseline
                .with_timezone(&tz)
                .checked_add_signed(duration)
                .map(|dt| dt.date_naive())
            else {
                continue;
            };
            maintenance_events.push(crate::ics::MaintenanceEvent {
                system_id: system.id,
                system_name: system.name.clone(),
                customer_name: customer.name.clone(),
                due_on,
            });
        }
    }

    // Expiring items intentionally include archived customers' items (see
    // `db::expiring_items::list_all` below, which is NOT filtered to active
    // customers -- matching the Dashboard's own unfiltered
    // `list_expiring_items` behavior), so names must resolve for archived
    // customers too, not just active ones.
    let customer_name_by_id: HashMap<i64, String> = db::customers::list(&conn, true)
        .map(|cs| cs.into_iter().map(|c| (c.id, c.name)).collect())?;
    let mut expiry_events = Vec::new();
    for item in db::expiring_items::list_all(&conn)? {
        let Ok(expires_on) = chrono::NaiveDate::parse_from_str(&item.expires_on, "%Y-%m-%d") else {
            continue;
        };
        let customer_name = customer_name_by_id
            .get(&item.customer_id)
            .cloned()
            .unwrap_or_else(|| format!("Kunde #{}", item.customer_id));
        expiry_events.push(crate::ics::ExpiryEvent {
            item_id: item.id,
            label: item.label.clone(),
            customer_name,
            kind_label: expiring_item_kind_label(item.kind).to_string(),
            expires_on,
            reminder_days_before: item.reminder_days_before,
        });
    }

    let ics_content = crate::ics::build_calendar(&maintenance_events, &expiry_events);
    std::fs::write(&dest_path, ics_content).map_err(|e| {
        AppError::Io(format!(
            "Kalenderdatei konnte nicht geschrieben werden: {e}"
        ))
    })
}

/// German display label for an expiring-item kind -- mirrors
/// `src/lib/expiry.ts`'s `EXPIRING_ITEM_KIND_LABELS` map exactly (that map
/// is the frontend's source of truth for these labels; keep both in sync if
/// either ever changes).
fn expiring_item_kind_label(kind: db::expiring_items::ExpiringItemKind) -> &'static str {
    use db::expiring_items::ExpiringItemKind;
    match kind {
        ExpiringItemKind::SslCertificate => "SSL-Zertifikat",
        ExpiringItemKind::Domain => "Domain",
        ExpiringItemKind::License => "Lizenz",
        ExpiringItemKind::Contract => "Vertrag",
        ExpiringItemKind::Warranty => "Garantie",
        ExpiringItemKind::Sonstiges => "Sonstiges",
    }
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
    use rusqlite::{params, Connection};

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
                operating_system: None,
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

    // Regression coverage for collect_audit_rows: seeds one customer plus two
    // systems (one archived via systems::archive) and pins their
    // automatically-recorded audit-log rows' timestamps across a MONTH
    // boundary (2026-09-02 vs. 2026-10-01) specifically -- as formatted
    // display strings ("02.09.2026" vs. "01.10.2026") these do NOT sort
    // chronologically as text, but the raw ISO timestamps do, which is
    // exactly what this function's sort must get right (see its own comment
    // on sorting by at_utc, not at_display). Also proves archived systems'
    // entries aren't silently excluded, and that each row's scope_label
    // matches its own originating system, not a neighbor's.
    //
    // Audit-log rows aren't created directly -- this codebase has no public
    // `db::audit_log::create`-style function; they're written automatically
    // as a side effect of customers::create / systems::create /
    // systems::archive (see db/audit_log.rs and db/systems.rs). Each of
    // those calls' own audit_log::record() runs strictly after its main
    // INSERT, so conn.last_insert_rowid() immediately after the call gives
    // the new audit_log row's id -- used here to target the exact rows to
    // pin, same direct-SQL-UPDATE pattern db/tickets.rs's own ordering test
    // uses to avoid a wall-clock-based (and therefore flaky/tautological)
    // test.
    #[test]
    fn collect_audit_rows_sorts_across_month_boundary_and_includes_archived_systems() {
        let conn = migrated_connection();

        let customer_id = seed_customer(&conn, "ACME GmbH");
        let customer_audit_id = conn.last_insert_rowid();

        seed_system(&conn, customer_id, "Server Alpha");
        let system_a_audit_id = conn.last_insert_rowid();

        let system_b_id = seed_system(&conn, customer_id, "Server Beta");
        let system_b_created_audit_id = conn.last_insert_rowid();

        systems::archive(&conn, system_b_id, &berlin()).unwrap();
        let system_b_archived_audit_id = conn.last_insert_rowid();

        for (id, ts) in [
            (customer_audit_id, "2026-09-02T10:00:00.000Z"),
            (system_a_audit_id, "2026-09-20T10:00:00.000Z"),
            (system_b_created_audit_id, "2026-10-01T10:00:00.000Z"),
            (system_b_archived_audit_id, "2026-10-05T10:00:00.000Z"),
        ] {
            conn.execute(
                "UPDATE audit_log SET at_utc = ?1 WHERE id = ?2",
                params![ts, id],
            )
            .unwrap();
        }

        let rows = collect_audit_rows(&conn, customer_id).unwrap();

        assert_eq!(
            rows.len(),
            4,
            "the customer's own entry plus both systems' entries (including \
             the archived system's two) must all be present"
        );

        let at_displays: Vec<&str> = rows.iter().map(|r| r.at_display.as_str()).collect();
        // Correct chronological order (September before October). A sort on
        // the formatted display strings instead of the raw timestamps would
        // scramble this, since "01.10.2026" < "02.09.2026" < "05.10.2026" <
        // "20.09.2026" lexicographically.
        assert!(
            at_displays[0].starts_with("02.09.2026"),
            "expected the earliest (Kunde) row first, got {:?}",
            at_displays
        );
        assert!(
            at_displays[1].starts_with("20.09.2026"),
            "expected Server Alpha's row second, got {:?}",
            at_displays
        );
        assert!(
            at_displays[2].starts_with("01.10.2026"),
            "expected Server Beta's creation row third, got {:?}",
            at_displays
        );
        assert!(
            at_displays[3].starts_with("05.10.2026"),
            "expected Server Beta's archival row last, got {:?}",
            at_displays
        );

        assert_eq!(rows[0].scope_label, "Kunde");
        assert_eq!(rows[0].action, "created");
        assert_eq!(rows[1].scope_label, "Server Alpha");
        assert_eq!(rows[2].scope_label, "Server Beta");
        assert_eq!(rows[2].action, "created");
        assert_eq!(
            rows[3].scope_label, "Server Beta",
            "the archived system's own entry must keep its own name as scope_label, \
             not fall back to or swap with the other system's"
        );
        assert_eq!(rows[3].action, "archived");
    }
}
