use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use chrono::DateTime;
use rusqlite::Connection;

use crate::db::{
    attachments, customers,
    entries::{self, Entry, EntryFilter},
    systems,
};
use crate::error::AppError;
use crate::time;

/// Exportiert alle (per `filter` eingegrenzten) Einträge eines Kunden als
/// Markdown: ein Verzeichnis mit einer Datei je System (plus einer
/// "Ohne System"-Datei für kundenweite Einträge), Anhänge unter derselben
/// relativen `attachments/{prefix}/{hash}.ext`-Struktur mitkopiert wie sie
/// bereits in `entry.body_md` referenziert wird — dadurch bleibt `body_md`
/// unverändert lesbar, ohne jede Pfad-Neuschreibung.
pub fn export_markdown(
    conn: &Connection,
    data_dir: &Path,
    dest_dir: &Path,
    customer_id: i64,
    filter: &EntryFilter,
    late_entry_threshold_hours: i64,
) -> Result<(), AppError> {
    let customer = customers::get(conn, customer_id)?;
    let all_systems = systems::list_by_customer(conn, customer_id, true)?;
    let system_name_by_id: HashMap<i64, String> =
        all_systems.into_iter().map(|s| (s.id, s.name)).collect();

    let mut entries_list = entries::list(conn, filter)?;
    entries_list.reverse(); // list() liefert DESC; Export liest chronologisch, älteste zuerst

    let system_tz = time::system_timezone()?;
    let (generated_at_utc, generated_at_tz) = time::now_with_tz(&system_tz);
    let generated_at_display =
        time::format_timestamp_for_display(&generated_at_utc, &generated_at_tz)?;

    std::fs::create_dir_all(dest_dir)?;

    // Gruppiert nach system_id (None -> "Ohne System"), erhält innerhalb jeder
    // Gruppe die oben hergestellte chronologische Reihenfolge.
    let mut grouped: BTreeMap<Option<i64>, Vec<&Entry>> = BTreeMap::new();
    for entry in &entries_list {
        grouped.entry(entry.system_id).or_default().push(entry);
    }

    for (system_id, group_entries) in grouped {
        let system_label = match system_id {
            Some(id) => system_name_by_id
                .get(&id)
                .cloned()
                .unwrap_or_else(|| format!("System {id}")),
            None => "Ohne System".to_string(),
        };
        let file_name = format!("{}.md", sanitize_filename(&system_label));
        let file_path = dest_dir.join(&file_name);

        let mut content = String::new();
        content.push_str(&format!("# {} — {}\n\n", system_label, customer.name));
        content.push_str(&format!(
            "_Export erstellt: {generated_at_display}_\n\n---\n\n"
        ));

        for entry in group_entries {
            let performed_display = time::format_timestamp_for_display(
                &entry.performed_at_utc,
                &entry.performed_at_tz,
            )?;
            content.push_str(&format!("## {}\n\n", entry.title));
            content.push_str(&format!(
                "**Zeitpunkt:** {}  \n**Kategorie:** {}  \n**Tags:** {}\n\n",
                performed_display,
                category_label(entry.category),
                if entry.tags.is_empty() {
                    "—".to_string()
                } else {
                    entry.tags.join(", ")
                },
            ));
            if let Some(note) = late_entry_note(entry, late_entry_threshold_hours)? {
                content.push_str(&format!("_{note}_\n\n"));
            }
            content.push_str(&entry.body_md);
            content.push_str("\n\n---\n\n");

            for attachment in attachments::list_for_entry(conn, entry.id)? {
                let relative_path = crate::attachments::store::relative_path_for(
                    &attachment.sha256,
                    &attachment.original_filename,
                );
                let source = data_dir.join(&relative_path);
                let dest = dest_dir.join(&relative_path);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                if source.exists() {
                    std::fs::copy(&source, &dest)?;
                }
            }
        }

        std::fs::write(&file_path, content)?;
    }

    Ok(())
}

fn category_label(category: entries::Category) -> &'static str {
    match category {
        entries::Category::Wartung => "Wartung",
        entries::Category::Stoerung => "Störung",
        entries::Category::Aenderung => "Änderung",
        entries::Category::Installation => "Installation",
        entries::Category::Sonstiges => "Sonstiges",
    }
}

/// `None`, wenn der Eintrag zeitnah erfasst wurde; sonst ein deutschsprachiger
/// Hinweistext samt formatiertem `created_at`, sobald die Differenz zwischen
/// `created_at_utc` und `performed_at_utc` den Schwellwert erreicht oder
/// überschreitet.
fn late_entry_note(entry: &Entry, threshold_hours: i64) -> Result<Option<String>, AppError> {
    let performed = DateTime::parse_from_rfc3339(&entry.performed_at_utc)
        .map_err(|e| AppError::InvalidTimestamp(format!("{}: {e}", entry.performed_at_utc)))?;
    let created = DateTime::parse_from_rfc3339(&entry.created_at_utc)
        .map_err(|e| AppError::InvalidTimestamp(format!("{}: {e}", entry.created_at_utc)))?;
    let diff_hours = (created - performed).num_hours();
    if diff_hours >= threshold_hours {
        let display =
            time::format_timestamp_for_display(&entry.created_at_utc, &entry.created_at_tz)?;
        Ok(Some(format!("Nachträglich erfasst: {display}")))
    } else {
        Ok(None)
    }
}

fn sanitize_filename(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = sanitized.trim();
    if trimmed.is_empty() {
        "System".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::customers::{self, NewCustomer};
    use crate::db::entries::{Category, NewEntry};
    use crate::db::systems::{self, NewSystem};
    use crate::db::test_support::migrated_connection;

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
                system_type: "Server".into(),
                hostname: "".into(),
                ip_address: "".into(),
                notes: "".into(),
            },
            &berlin(),
        )
        .unwrap()
        .id
    }

    fn seed_entry(
        conn: &Connection,
        data_dir: &Path,
        customer_id: i64,
        system_id: Option<i64>,
        title: &str,
        performed_at_utc: &str,
    ) -> Entry {
        entries::create(
            conn,
            data_dir,
            NewEntry {
                customer_id,
                system_id,
                title: title.into(),
                body_md: format!("Inhalt von {title}"),
                category: Category::Wartung,
                performed_at_utc: performed_at_utc.into(),
                performed_at_tz: "Europe/Berlin".into(),
                tag_names: vec![],
                pending_attachments: vec![],
            },
            &berlin(),
        )
        .unwrap()
    }

    #[test]
    fn export_creates_one_file_per_system_plus_ohne_system() {
        let conn = migrated_connection();
        let data_dir = tempfile::tempdir().unwrap();
        let dest_dir = tempfile::tempdir().unwrap();
        let customer_id = seed_customer(&conn, "ACME");
        let sys_a = seed_system(&conn, customer_id, "Fileserver");
        let sys_b = seed_system(&conn, customer_id, "Firewall");

        seed_entry(
            &conn,
            data_dir.path(),
            customer_id,
            Some(sys_a),
            "Update Fileserver",
            "2026-09-01T10:00:00.000Z",
        );
        seed_entry(
            &conn,
            data_dir.path(),
            customer_id,
            Some(sys_b),
            "Regelwerk geändert",
            "2026-09-02T10:00:00.000Z",
        );
        seed_entry(
            &conn,
            data_dir.path(),
            customer_id,
            None,
            "Allgemeine Notiz",
            "2026-09-03T10:00:00.000Z",
        );

        export_markdown(
            &conn,
            data_dir.path(),
            dest_dir.path(),
            customer_id,
            &EntryFilter {
                customer_id: Some(customer_id),
                ..Default::default()
            },
            24,
        )
        .unwrap();

        let fileserver_md = std::fs::read_to_string(dest_dir.path().join("Fileserver.md")).unwrap();
        assert!(fileserver_md.contains("Update Fileserver"));
        assert!(!fileserver_md.contains("Regelwerk geändert"));

        let firewall_md = std::fs::read_to_string(dest_dir.path().join("Firewall.md")).unwrap();
        assert!(firewall_md.contains("Regelwerk geändert"));

        let ohne_system_md =
            std::fs::read_to_string(dest_dir.path().join("Ohne System.md")).unwrap();
        assert!(ohne_system_md.contains("Allgemeine Notiz"));
    }

    #[test]
    fn export_copies_attachment_at_same_relative_path() {
        let conn = migrated_connection();
        let data_dir = tempfile::tempdir().unwrap();
        let dest_dir = tempfile::tempdir().unwrap();
        let customer_id = seed_customer(&conn, "ACME");
        let entry = seed_entry(
            &conn,
            data_dir.path(),
            customer_id,
            None,
            "Mit Anhang",
            "2026-09-01T10:00:00.000Z",
        );

        let attachment = crate::attachments::store::attach_bytes_to_entry(
            &conn,
            data_dir.path(),
            entry.id,
            b"fake png bytes",
            "screenshot.png",
            "image/png",
            &berlin(),
        )
        .unwrap();

        export_markdown(
            &conn,
            data_dir.path(),
            dest_dir.path(),
            customer_id,
            &EntryFilter {
                customer_id: Some(customer_id),
                ..Default::default()
            },
            24,
        )
        .unwrap();

        let relative_path =
            crate::attachments::store::relative_path_for(&attachment.sha256, "screenshot.png");
        let copied = dest_dir.path().join(&relative_path);
        assert!(
            copied.exists(),
            "Anhang sollte unter {relative_path} im Export liegen"
        );
        assert_eq!(std::fs::read(&copied).unwrap(), b"fake png bytes");
    }

    #[test]
    fn late_entry_note_is_none_when_created_shortly_after_performed() {
        let entry = Entry {
            id: 1,
            customer_id: 1,
            system_id: None,
            title: "Zeitnah".into(),
            body_md: "".into(),
            category: Category::Wartung,
            performed_at_utc: "2026-09-01T10:00:00.000Z".into(),
            performed_at_tz: "Europe/Berlin".into(),
            created_at_utc: "2026-09-01T11:00:00.000Z".into(), // +1h
            created_at_tz: "Europe/Berlin".into(),
            updated_at_utc: "2026-09-01T11:00:00.000Z".into(),
            updated_at_tz: "Europe/Berlin".into(),
            tags: vec![],
        };

        assert_eq!(late_entry_note(&entry, 24).unwrap(), None);
    }

    #[test]
    fn late_entry_note_is_some_when_created_much_later_than_performed() {
        let entry = Entry {
            id: 1,
            customer_id: 1,
            system_id: None,
            title: "Nachtrag".into(),
            body_md: "".into(),
            category: Category::Wartung,
            performed_at_utc: "2026-09-01T10:00:00.000Z".into(),
            performed_at_tz: "Europe/Berlin".into(),
            created_at_utc: "2026-09-03T10:00:00.000Z".into(), // +48h
            created_at_tz: "Europe/Berlin".into(),
            updated_at_utc: "2026-09-03T10:00:00.000Z".into(),
            updated_at_tz: "Europe/Berlin".into(),
            tags: vec![],
        };

        let note = late_entry_note(&entry, 24).unwrap();
        assert!(note.is_some());
        assert!(note.unwrap().starts_with("Nachträglich erfasst:"));
    }

    #[test]
    fn category_label_covers_all_variants() {
        assert_eq!(category_label(Category::Wartung), "Wartung");
        assert_eq!(category_label(Category::Stoerung), "Störung");
        assert_eq!(category_label(Category::Aenderung), "Änderung");
        assert_eq!(category_label(Category::Installation), "Installation");
        assert_eq!(category_label(Category::Sonstiges), "Sonstiges");
    }
}
