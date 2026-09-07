use rusqlite::{params, Connection};

use crate::error::AppError;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct EntryHit {
    pub entry_id: i64,
    pub customer_id: i64,
    pub system_id: Option<i64>,
    pub title: String,
    pub snippet: String,
    pub performed_at_utc: String,
    pub performed_at_tz: String,
}

pub fn search_entries(conn: &Connection, query: &str, limit: i64) -> Result<Vec<EntryHit>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT e.id, e.customer_id, e.system_id, e.title, e.performed_at_utc, e.performed_at_tz,
                snippet(entries_fts, 1, '<mark>', '</mark>', '…', 12) AS snippet
         FROM entries_fts
         JOIN entries e ON e.id = entries_fts.rowid
         WHERE entries_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![query, limit], |row| {
        Ok(EntryHit {
            entry_id: row.get(0)?,
            customer_id: row.get(1)?,
            system_id: row.get(2)?,
            title: row.get(3)?,
            performed_at_utc: row.get(4)?,
            performed_at_tz: row.get(5)?,
            snippet: row.get(6)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectoryKind {
    Customer,
    System,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DirectoryHit {
    pub kind: DirectoryKind,
    pub id: i64,
    pub customer_id: i64,
    pub label: String,
}

pub fn search_directory(conn: &Connection, query: &str, limit: i64) -> Result<Vec<DirectoryHit>, AppError> {
    let sanitized: String = query.chars().filter(|c| *c != '%' && *c != '_').collect();
    let like = format!("%{sanitized}%");
    let mut stmt = conn.prepare(
        "SELECT 'customer' AS kind, id, id AS customer_id, name || ' (' || short_code || ')' AS label
         FROM customers
         WHERE archived_at_utc IS NULL AND (name LIKE ?1 COLLATE NOCASE OR short_code LIKE ?1 COLLATE NOCASE)
         UNION ALL
         SELECT 'system' AS kind, id, customer_id, name
         FROM systems
         WHERE archived_at_utc IS NULL AND (name LIKE ?1 COLLATE NOCASE OR hostname LIKE ?1 COLLATE NOCASE)
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![like, limit], |row| {
        let kind_str: String = row.get(0)?;
        Ok(DirectoryHit {
            kind: if kind_str == "customer" { DirectoryKind::Customer } else { DirectoryKind::System },
            id: row.get(1)?,
            customer_id: row.get(2)?,
            label: row.get(3)?,
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

    fn seed(conn: &Connection) {
        conn.execute(
            "INSERT INTO customers (id, name, short_code, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
             VALUES (1, 'ACME GmbH', 'ACME', '2026-09-07T12:00:00.000Z', 'Europe/Berlin', '2026-09-07T12:00:00.000Z', 'Europe/Berlin'),
                    (2, 'Beispiel AG', 'BSP', '2026-09-07T12:00:00.000Z', 'Europe/Berlin', '2026-09-07T12:00:00.000Z', 'Europe/Berlin')",
            [],
        ).unwrap();
        conn.execute(
            "UPDATE customers SET archived_at_utc = '2026-09-07T12:00:00.000Z', archived_at_tz = 'Europe/Berlin' WHERE id = 2",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO systems (id, customer_id, name, hostname, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
             VALUES (1, 1, 'Fileserver', 'fs01.acme.local', '2026-09-07T12:00:00.000Z', 'Europe/Berlin', '2026-09-07T12:00:00.000Z', 'Europe/Berlin')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO entries (id, customer_id, title, body_md, category, performed_at_utc, performed_at_tz, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
             VALUES (1, 1, 'Exchange Update', 'Kumulatives Update auf Server eingespielt', 'wartung',
                     '2026-09-07T12:00:00.000Z', 'Europe/Berlin', '2026-09-07T12:00:00.000Z', 'Europe/Berlin', '2026-09-07T12:00:00.000Z', 'Europe/Berlin')",
            [],
        ).unwrap();
    }

    #[test]
    fn search_entries_finds_match_with_snippet() {
        let conn = migrated_connection();
        seed(&conn);
        let hits = search_entries(&conn, "kumulatives", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entry_id, 1);
        assert!(hits[0].snippet.contains("<mark>"));
    }

    #[test]
    fn search_directory_finds_customer_by_short_code() {
        let conn = migrated_connection();
        seed(&conn);
        // "GmbH" steht nur im Kundennamen, nicht im Hostname des Systems — eindeutiger Treffer.
        let hits = search_directory(&conn, "GmbH", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, DirectoryKind::Customer);
        assert_eq!(hits[0].id, 1);
    }

    #[test]
    fn search_directory_query_matching_both_returns_customer_and_system() {
        let conn = migrated_connection();
        seed(&conn);
        // "ACME" steckt sowohl im Kundennamen/-kürzel als auch im Hostname (fs01.acme.local) — beide sind korrekte Treffer.
        let hits = search_directory(&conn, "ACME", 10).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().any(|h| h.kind == DirectoryKind::Customer && h.id == 1));
        assert!(hits.iter().any(|h| h.kind == DirectoryKind::System && h.id == 1));
    }

    #[test]
    fn search_directory_excludes_archived_customers() {
        let conn = migrated_connection();
        seed(&conn);
        let hits = search_directory(&conn, "Beispiel", 10).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn search_directory_finds_system_by_hostname_fragment() {
        let conn = migrated_connection();
        seed(&conn);
        let hits = search_directory(&conn, "fs01", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, DirectoryKind::System);
        assert_eq!(hits[0].customer_id, 1);
    }
}
