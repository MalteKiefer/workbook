use chrono_tz::Tz;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::error::AppError;
use crate::time::now_with_tz;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct System {
    pub id: i64,
    pub customer_id: i64,
    pub name: String,
    pub system_type: String,
    pub hostname: String,
    pub ip_address: String,
    pub notes: String,
    pub created_at_utc: String,
    pub created_at_tz: String,
    pub updated_at_utc: String,
    pub updated_at_tz: String,
    pub archived_at_utc: Option<String>,
    pub archived_at_tz: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NewSystem {
    pub customer_id: i64,
    pub name: String,
    pub system_type: String,
    pub hostname: String,
    pub ip_address: String,
    pub notes: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UpdateSystem {
    pub name: String,
    pub system_type: String,
    pub hostname: String,
    pub ip_address: String,
    pub notes: String,
}

fn row_to_system(row: &Row) -> rusqlite::Result<System> {
    Ok(System {
        id: row.get("id")?,
        customer_id: row.get("customer_id")?,
        name: row.get("name")?,
        system_type: row.get("system_type")?,
        hostname: row.get("hostname")?,
        ip_address: row.get("ip_address")?,
        notes: row.get("notes")?,
        created_at_utc: row.get("created_at_utc")?,
        created_at_tz: row.get("created_at_tz")?,
        updated_at_utc: row.get("updated_at_utc")?,
        updated_at_tz: row.get("updated_at_tz")?,
        archived_at_utc: row.get("archived_at_utc")?,
        archived_at_tz: row.get("archived_at_tz")?,
    })
}

pub fn create(conn: &Connection, input: NewSystem, tz: &Tz) -> Result<System, AppError> {
    let (now_utc, now_tz) = now_with_tz(tz);
    conn.execute(
        "INSERT INTO systems (customer_id, name, system_type, hostname, ip_address, notes, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?7, ?8)",
        params![input.customer_id, input.name, input.system_type, input.hostname, input.ip_address, input.notes, now_utc, now_tz],
    )?;
    get(conn, conn.last_insert_rowid())
}

pub fn get(conn: &Connection, id: i64) -> Result<System, AppError> {
    conn.query_row(
        "SELECT * FROM systems WHERE id = ?1",
        params![id],
        row_to_system,
    )
    .optional()?
    .ok_or_else(|| AppError::NotFound(format!("System {id} nicht gefunden")))
}

pub fn list_by_customer(
    conn: &Connection,
    customer_id: i64,
    include_archived: bool,
) -> Result<Vec<System>, AppError> {
    let sql = if include_archived {
        "SELECT * FROM systems WHERE customer_id = ?1 ORDER BY name COLLATE NOCASE"
    } else {
        "SELECT * FROM systems WHERE customer_id = ?1 AND archived_at_utc IS NULL ORDER BY name COLLATE NOCASE"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![customer_id], row_to_system)?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn update(
    conn: &Connection,
    id: i64,
    input: UpdateSystem,
    tz: &Tz,
) -> Result<System, AppError> {
    let (now_utc, now_tz) = now_with_tz(tz);
    let changed = conn.execute(
        "UPDATE systems SET name = ?1, system_type = ?2, hostname = ?3, ip_address = ?4, notes = ?5, updated_at_utc = ?6, updated_at_tz = ?7 WHERE id = ?8",
        params![input.name, input.system_type, input.hostname, input.ip_address, input.notes, now_utc, now_tz, id],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound(format!("System {id} nicht gefunden")));
    }
    get(conn, id)
}

pub fn archive(conn: &Connection, id: i64, tz: &Tz) -> Result<(), AppError> {
    let (now_utc, now_tz) = now_with_tz(tz);
    let changed = conn.execute(
        "UPDATE systems SET archived_at_utc = ?1, archived_at_tz = ?2 WHERE id = ?3",
        params![now_utc, now_tz, id],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound(format!("System {id} nicht gefunden")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::customers::{self, NewCustomer};
    use crate::db::test_support::migrated_connection;

    fn berlin() -> Tz {
        "Europe/Berlin".parse().unwrap()
    }

    fn seed_customer(conn: &Connection) -> i64 {
        customers::create(
            conn,
            NewCustomer {
                name: "ACME".into(),
                short_code: "ACME".into(),
                notes: "".into(),
            },
            &berlin(),
        )
        .unwrap()
        .id
    }

    #[test]
    fn create_then_get_roundtrips() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(
            &conn,
            NewSystem {
                customer_id,
                name: "FS01".into(),
                system_type: "Server".into(),
                hostname: "fs01.acme.local".into(),
                ip_address: "10.0.0.5".into(),
                notes: "".into(),
            },
            &berlin(),
        )
        .unwrap();
        assert_eq!(created.hostname, "fs01.acme.local");
        assert_eq!(get(&conn, created.id).unwrap(), created);
    }

    #[test]
    fn create_rejects_unknown_customer() {
        let conn = migrated_connection();
        let result = create(
            &conn,
            NewSystem {
                customer_id: 999,
                name: "Ghost".into(),
                system_type: "".into(),
                hostname: "".into(),
                ip_address: "".into(),
                notes: "".into(),
            },
            &berlin(),
        );
        assert!(matches!(result, Err(AppError::Database(_))));
    }

    #[test]
    fn list_by_customer_excludes_archived_by_default() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let a = create(
            &conn,
            NewSystem {
                customer_id,
                name: "Aktiv".into(),
                system_type: "".into(),
                hostname: "".into(),
                ip_address: "".into(),
                notes: "".into(),
            },
            &berlin(),
        )
        .unwrap();
        let b = create(
            &conn,
            NewSystem {
                customer_id,
                name: "Alt".into(),
                system_type: "".into(),
                hostname: "".into(),
                ip_address: "".into(),
                notes: "".into(),
            },
            &berlin(),
        )
        .unwrap();
        archive(&conn, b.id, &berlin()).unwrap();

        let active = list_by_customer(&conn, customer_id, false).unwrap();
        assert_eq!(active.iter().map(|s| s.id).collect::<Vec<_>>(), vec![a.id]);
        assert_eq!(list_by_customer(&conn, customer_id, true).unwrap().len(), 2);
    }

    #[test]
    fn update_changes_fields_and_keeps_created_at() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(
            &conn,
            NewSystem {
                customer_id,
                name: "Alt".into(),
                system_type: "".into(),
                hostname: "".into(),
                ip_address: "".into(),
                notes: "".into(),
            },
            &berlin(),
        )
        .unwrap();
        let updated = update(
            &conn,
            created.id,
            UpdateSystem {
                name: "Neu".into(),
                system_type: "Firewall".into(),
                hostname: "fw.acme.local".into(),
                ip_address: "10.0.0.1".into(),
                notes: "".into(),
            },
            &berlin(),
        )
        .unwrap();
        assert_eq!(updated.name, "Neu");
        assert_eq!(updated.created_at_utc, created.created_at_utc);
    }
}
