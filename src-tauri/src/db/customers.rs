use chrono_tz::Tz;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::error::AppError;
use crate::time::now_with_tz;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Customer {
    pub id: i64,
    pub name: String,
    pub short_code: String,
    pub notes: String,
    pub created_at_utc: String,
    pub created_at_tz: String,
    pub updated_at_utc: String,
    pub updated_at_tz: String,
    pub archived_at_utc: Option<String>,
    pub archived_at_tz: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NewCustomer {
    pub name: String,
    pub short_code: String,
    pub notes: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UpdateCustomer {
    pub name: String,
    pub short_code: String,
    pub notes: String,
}

fn row_to_customer(row: &Row) -> rusqlite::Result<Customer> {
    Ok(Customer {
        id: row.get("id")?,
        name: row.get("name")?,
        short_code: row.get("short_code")?,
        notes: row.get("notes")?,
        created_at_utc: row.get("created_at_utc")?,
        created_at_tz: row.get("created_at_tz")?,
        updated_at_utc: row.get("updated_at_utc")?,
        updated_at_tz: row.get("updated_at_tz")?,
        archived_at_utc: row.get("archived_at_utc")?,
        archived_at_tz: row.get("archived_at_tz")?,
    })
}

pub fn create(conn: &Connection, input: NewCustomer, tz: &Tz) -> Result<Customer, AppError> {
    let (now_utc, now_tz) = now_with_tz(tz);
    conn.execute(
        "INSERT INTO customers (name, short_code, notes, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
         VALUES (?1, ?2, ?3, ?4, ?5, ?4, ?5)",
        params![input.name, input.short_code, input.notes, now_utc, now_tz],
    )?;
    get(conn, conn.last_insert_rowid())
}

pub fn get(conn: &Connection, id: i64) -> Result<Customer, AppError> {
    conn.query_row("SELECT * FROM customers WHERE id = ?1", params![id], row_to_customer)
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("Kunde {id} nicht gefunden")))
}

pub fn list(conn: &Connection, include_archived: bool) -> Result<Vec<Customer>, AppError> {
    let sql = if include_archived {
        "SELECT * FROM customers ORDER BY name COLLATE NOCASE"
    } else {
        "SELECT * FROM customers WHERE archived_at_utc IS NULL ORDER BY name COLLATE NOCASE"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], row_to_customer)?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn update(conn: &Connection, id: i64, input: UpdateCustomer, tz: &Tz) -> Result<Customer, AppError> {
    let (now_utc, now_tz) = now_with_tz(tz);
    let changed = conn.execute(
        "UPDATE customers SET name = ?1, short_code = ?2, notes = ?3, updated_at_utc = ?4, updated_at_tz = ?5 WHERE id = ?6",
        params![input.name, input.short_code, input.notes, now_utc, now_tz, id],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound(format!("Kunde {id} nicht gefunden")));
    }
    get(conn, id)
}

pub fn archive(conn: &Connection, id: i64, tz: &Tz) -> Result<(), AppError> {
    let (now_utc, now_tz) = now_with_tz(tz);
    let changed = conn.execute(
        "UPDATE customers SET archived_at_utc = ?1, archived_at_tz = ?2 WHERE id = ?3",
        params![now_utc, now_tz, id],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound(format!("Kunde {id} nicht gefunden")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::migrated_connection;

    fn berlin() -> Tz {
        "Europe/Berlin".parse().unwrap()
    }

    #[test]
    fn create_then_get_roundtrips() {
        let conn = migrated_connection();
        let created = create(
            &conn,
            NewCustomer { name: "ACME GmbH".into(), short_code: "ACME".into(), notes: "".into() },
            &berlin(),
        )
        .unwrap();
        assert_eq!(created.name, "ACME GmbH");
        assert_eq!(created.created_at_utc, created.updated_at_utc);
        assert!(created.archived_at_utc.is_none());
        assert_eq!(get(&conn, created.id).unwrap(), created);
    }

    #[test]
    fn get_missing_customer_returns_not_found() {
        let conn = migrated_connection();
        assert!(matches!(get(&conn, 999), Err(AppError::NotFound(_))));
    }

    #[test]
    fn list_excludes_archived_by_default() {
        let conn = migrated_connection();
        let a = create(&conn, NewCustomer { name: "Aktiv".into(), short_code: "AKT".into(), notes: "".into() }, &berlin()).unwrap();
        let b = create(&conn, NewCustomer { name: "Archiviert".into(), short_code: "ARC".into(), notes: "".into() }, &berlin()).unwrap();
        archive(&conn, b.id, &berlin()).unwrap();

        let active = list(&conn, false).unwrap();
        assert_eq!(active.iter().map(|c| c.id).collect::<Vec<_>>(), vec![a.id]);
        assert_eq!(list(&conn, true).unwrap().len(), 2);
    }

    #[test]
    fn update_changes_fields_and_keeps_created_at() {
        let conn = migrated_connection();
        let created = create(&conn, NewCustomer { name: "Alt".into(), short_code: "ALT".into(), notes: "".into() }, &berlin()).unwrap();
        let updated = update(
            &conn,
            created.id,
            UpdateCustomer { name: "Neu".into(), short_code: "NEU".into(), notes: "geändert".into() },
            &berlin(),
        )
        .unwrap();
        assert_eq!(updated.name, "Neu");
        assert_eq!(updated.short_code, "NEU");
        assert_eq!(updated.created_at_utc, created.created_at_utc);
    }

    #[test]
    fn duplicate_short_code_is_rejected() {
        let conn = migrated_connection();
        create(&conn, NewCustomer { name: "Erster".into(), short_code: "DUP".into(), notes: "".into() }, &berlin()).unwrap();
        let result = create(&conn, NewCustomer { name: "Zweiter".into(), short_code: "DUP".into(), notes: "".into() }, &berlin());
        assert!(matches!(result, Err(AppError::Database(_))));
    }
}
