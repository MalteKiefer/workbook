use rusqlite::{params, Connection};

use crate::error::AppError;

pub fn find_or_create(conn: &Connection, name: &str) -> Result<i64, AppError> {
    conn.execute(
        "INSERT OR IGNORE INTO tags (name) VALUES (?1)",
        params![name],
    )?;
    conn.query_row("SELECT id FROM tags WHERE name = ?1", params![name], |r| {
        r.get(0)
    })
    .map_err(Into::into)
}

pub fn set_tags_for_entry(
    conn: &Connection,
    entry_id: i64,
    tag_names: &[String],
) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM entry_tags WHERE entry_id = ?1",
        params![entry_id],
    )?;
    for name in tag_names {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            continue;
        }
        let tag_id = find_or_create(conn, trimmed)?;
        conn.execute(
            "INSERT OR IGNORE INTO entry_tags (entry_id, tag_id) VALUES (?1, ?2)",
            params![entry_id, tag_id],
        )?;
    }
    Ok(())
}

pub fn tags_for_entry(conn: &Connection, entry_id: i64) -> Result<Vec<String>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT t.name FROM tags t JOIN entry_tags et ON et.tag_id = t.id WHERE et.entry_id = ?1 ORDER BY t.name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map(params![entry_id], |r| r.get::<_, String>(0))?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn list_all(conn: &Connection) -> Result<Vec<String>, AppError> {
    let mut stmt = conn.prepare("SELECT name FROM tags ORDER BY name COLLATE NOCASE")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// One tag together with how many entries currently use it -- the basis for
/// the tag cloud in `JournalView` (see `commands::tags::list_tags_with_counts`
/// on the frontend-facing side). A tag with zero entries left (all of them
/// retagged away from it) simply never matches the `JOIN`, so it's implicitly
/// excluded here rather than needing separate cleanup logic.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TagCount {
    pub name: String,
    pub count: i64,
}

pub fn list_all_with_counts(conn: &Connection) -> Result<Vec<TagCount>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT t.name, COUNT(et.entry_id) AS count
         FROM tags t
         JOIN entry_tags et ON et.tag_id = t.id
         GROUP BY t.id
         ORDER BY count DESC, t.name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(TagCount {
            name: r.get(0)?,
            count: r.get(1)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::migrated_connection;

    fn seed_entry(conn: &Connection) -> i64 {
        conn.execute(
            "INSERT INTO customers (id, name, short_code, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
             VALUES (1, 'ACME', 'ACME', '2026-09-07T12:00:00.000Z', 'Europe/Berlin', '2026-09-07T12:00:00.000Z', 'Europe/Berlin')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO entries (id, customer_id, title, body_md, category, performed_at_utc, performed_at_tz, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
             VALUES (1, 1, 'Titel', '', 'wartung', '2026-09-07T12:00:00.000Z', 'Europe/Berlin', '2026-09-07T12:00:00.000Z', 'Europe/Berlin', '2026-09-07T12:00:00.000Z', 'Europe/Berlin')",
            [],
        )
        .unwrap();
        1
    }

    #[test]
    fn find_or_create_is_idempotent() {
        let conn = migrated_connection();
        let first = find_or_create(&conn, "exchange").unwrap();
        let second = find_or_create(&conn, "exchange").unwrap();
        assert_eq!(first, second);
        assert_eq!(list_all(&conn).unwrap(), vec!["exchange".to_string()]);
    }

    #[test]
    fn set_tags_for_entry_attaches_and_replaces() {
        let conn = migrated_connection();
        let entry_id = seed_entry(&conn);

        set_tags_for_entry(&conn, entry_id, &["update".into(), "exchange".into()]).unwrap();
        assert_eq!(
            tags_for_entry(&conn, entry_id).unwrap(),
            vec!["exchange".to_string(), "update".to_string()]
        );

        set_tags_for_entry(&conn, entry_id, &["firewall".into()]).unwrap();
        assert_eq!(
            tags_for_entry(&conn, entry_id).unwrap(),
            vec!["firewall".to_string()]
        );
    }

    #[test]
    fn list_all_with_counts_orders_by_usage_then_name() {
        let conn = migrated_connection();
        let entry_1 = seed_entry(&conn);
        conn.execute(
            "INSERT INTO entries (id, customer_id, title, body_md, category, performed_at_utc, performed_at_tz, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
             VALUES (2, 1, 'Titel 2', '', 'wartung', '2026-09-07T12:00:00.000Z', 'Europe/Berlin', '2026-09-07T12:00:00.000Z', 'Europe/Berlin', '2026-09-07T12:00:00.000Z', 'Europe/Berlin')",
            [],
        )
        .unwrap();
        let entry_2 = 2;

        set_tags_for_entry(&conn, entry_1, &["exchange".into(), "update".into()]).unwrap();
        set_tags_for_entry(&conn, entry_2, &["exchange".into()]).unwrap();

        assert_eq!(
            list_all_with_counts(&conn).unwrap(),
            vec![
                TagCount {
                    name: "exchange".to_string(),
                    count: 2
                },
                TagCount {
                    name: "update".to_string(),
                    count: 1
                },
            ]
        );
    }

    #[test]
    fn list_all_with_counts_excludes_tags_no_longer_used_by_any_entry() {
        let conn = migrated_connection();
        let entry_id = seed_entry(&conn);
        set_tags_for_entry(&conn, entry_id, &["firewall".into()]).unwrap();

        // Retag away from "firewall" -- the tag row itself still exists
        // (find_or_create never deletes), but no entry uses it anymore.
        set_tags_for_entry(&conn, entry_id, &["exchange".into()]).unwrap();

        assert_eq!(
            list_all_with_counts(&conn).unwrap(),
            vec![TagCount {
                name: "exchange".to_string(),
                count: 1
            }]
        );
    }

    #[test]
    fn set_tags_for_entry_ignores_blank_names() {
        let conn = migrated_connection();
        let entry_id = seed_entry(&conn);
        set_tags_for_entry(&conn, entry_id, &["  ".into(), "real".into()]).unwrap();
        assert_eq!(
            tags_for_entry(&conn, entry_id).unwrap(),
            vec!["real".to_string()]
        );
    }
}
