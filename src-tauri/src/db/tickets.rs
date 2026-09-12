use chrono_tz::Tz;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::error::AppError;
use crate::time::now_with_tz;

const VALID_STATUSES: &[&str] = &["open", "in_progress", "closed"];

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Ticket {
    pub id: i64,
    pub customer_id: i64,
    pub system_id: Option<i64>,
    pub title: String,
    pub description: String,
    pub status: String,
    pub time_spent_minutes: i64,
    pub created_at_utc: String,
    pub created_at_tz: String,
    pub updated_at_utc: String,
    pub updated_at_tz: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NewTicket {
    pub customer_id: i64,
    #[serde(default)]
    pub system_id: Option<i64>,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub time_spent_minutes: i64,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UpdateTicket {
    #[serde(default)]
    pub system_id: Option<i64>,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub status: String,
    #[serde(default)]
    pub time_spent_minutes: i64,
}

fn validate_title(title: &str) -> Result<(), AppError> {
    if title.trim().is_empty() {
        return Err(AppError::Validation(
            "Titel darf nicht leer sein.".to_string(),
        ));
    }
    Ok(())
}

fn validate_status(status: &str) -> Result<(), AppError> {
    if !VALID_STATUSES.contains(&status) {
        return Err(AppError::Validation(format!(
            "Ungültiger Status: \"{status}\" -- erwartet einen von: {}",
            VALID_STATUSES.join(", ")
        )));
    }
    Ok(())
}

fn validate_time_spent_minutes(minutes: i64) -> Result<(), AppError> {
    if minutes < 0 {
        return Err(AppError::Validation(
            "Zeitaufwand darf nicht negativ sein.".to_string(),
        ));
    }
    Ok(())
}

fn row_to_ticket(row: &Row) -> rusqlite::Result<Ticket> {
    Ok(Ticket {
        id: row.get("id")?,
        customer_id: row.get("customer_id")?,
        system_id: row.get("system_id")?,
        title: row.get("title")?,
        description: row.get("description")?,
        status: row.get("status")?,
        time_spent_minutes: row.get("time_spent_minutes")?,
        created_at_utc: row.get("created_at_utc")?,
        created_at_tz: row.get("created_at_tz")?,
        updated_at_utc: row.get("updated_at_utc")?,
        updated_at_tz: row.get("updated_at_tz")?,
    })
}

pub fn create(conn: &Connection, input: NewTicket, tz: &Tz) -> Result<Ticket, AppError> {
    validate_title(&input.title)?;
    let status = if input.status.is_empty() {
        "open".to_string()
    } else {
        input.status
    };
    validate_status(&status)?;
    validate_time_spent_minutes(input.time_spent_minutes)?;
    let (now_utc, now_tz) = now_with_tz(tz);
    conn.execute(
        "INSERT INTO tickets (customer_id, system_id, title, description, status, time_spent_minutes, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?7, ?8)",
        params![
            input.customer_id, input.system_id, input.title, input.description,
            status, input.time_spent_minutes, now_utc, now_tz
        ],
    )?;
    get(conn, conn.last_insert_rowid())
}

pub fn get(conn: &Connection, id: i64) -> Result<Ticket, AppError> {
    conn.query_row(
        "SELECT * FROM tickets WHERE id = ?1",
        params![id],
        row_to_ticket,
    )
    .optional()?
    .ok_or_else(|| AppError::NotFound(format!("Ticket {id} nicht gefunden")))
}

pub fn list_for_customer(conn: &Connection, customer_id: i64) -> Result<Vec<Ticket>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT * FROM tickets WHERE customer_id = ?1 ORDER BY status = 'closed' ASC, created_at_utc DESC",
    )?;
    let rows = stmt.query_map(params![customer_id], row_to_ticket)?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn update(
    conn: &Connection,
    id: i64,
    input: UpdateTicket,
    tz: &Tz,
) -> Result<Ticket, AppError> {
    validate_title(&input.title)?;
    validate_status(&input.status)?;
    validate_time_spent_minutes(input.time_spent_minutes)?;
    let (now_utc, now_tz) = now_with_tz(tz);
    let changed = conn.execute(
        "UPDATE tickets SET system_id = ?1, title = ?2, description = ?3, status = ?4, time_spent_minutes = ?5, updated_at_utc = ?6, updated_at_tz = ?7 WHERE id = ?8",
        params![
            input.system_id, input.title, input.description, input.status,
            input.time_spent_minutes, now_utc, now_tz, id
        ],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound(format!("Ticket {id} nicht gefunden")));
    }
    get(conn, id)
}

pub fn delete(conn: &Connection, id: i64) -> Result<(), AppError> {
    let changed = conn.execute("DELETE FROM tickets WHERE id = ?1", params![id])?;
    if changed == 0 {
        return Err(AppError::NotFound(format!("Ticket {id} nicht gefunden")));
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
                name: "ACME GmbH".to_string(),
                short_code: "ACME".to_string(),
                notes: "".to_string(),
            },
            &berlin(),
        )
        .unwrap()
        .id
    }

    fn sample(customer_id: i64) -> NewTicket {
        NewTicket {
            customer_id,
            system_id: None,
            title: "Drucker klemmt".to_string(),
            description: "Papierstau im Fach 2".to_string(),
            status: "open".to_string(),
            time_spent_minutes: 0,
        }
    }

    #[test]
    fn create_then_get_roundtrips() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(&conn, sample(customer_id), &berlin()).unwrap();
        assert_eq!(created.title, "Drucker klemmt");
        assert_eq!(created.status, "open");
        assert_eq!(get(&conn, created.id).unwrap(), created);
    }

    #[test]
    fn create_defaults_empty_status_to_open() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let mut input = sample(customer_id);
        input.status = "".to_string();
        let created = create(&conn, input, &berlin()).unwrap();
        assert_eq!(created.status, "open");
    }

    #[test]
    fn create_rejects_blank_title() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let mut input = sample(customer_id);
        input.title = "  ".to_string();
        let err = create(&conn, input, &berlin()).unwrap_err();
        assert_eq!(err.code(), "validation");
    }

    #[test]
    fn create_rejects_invalid_status() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let mut input = sample(customer_id);
        input.status = "not-a-real-status".to_string();
        let err = create(&conn, input, &berlin()).unwrap_err();
        assert_eq!(err.code(), "validation");
    }

    #[test]
    fn list_for_customer_orders_open_before_closed_then_newest_first() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);

        let mut older_open = sample(customer_id);
        older_open.title = "Alt offen".to_string();
        let older_open = create(&conn, older_open, &berlin()).unwrap();

        let mut newer_open = sample(customer_id);
        newer_open.title = "Neu offen".to_string();
        let newer_open = create(&conn, newer_open, &berlin()).unwrap();

        let mut closed = sample(customer_id);
        closed.title = "Erledigt".to_string();
        closed.status = "closed".to_string();
        let closed = create(&conn, closed, &berlin()).unwrap();

        // now_with_tz() only has millisecond resolution, so three creates in the
        // same test tick share one timestamp and neither ORDER BY key is exercised.
        // Pin distinct values, with the CLOSED ticket deliberately the newest, so a
        // naive `ORDER BY created_at_utc DESC` would wrongly float it to the top.
        for (id, ts) in [
            (older_open.id, "2026-01-01T10:00:00.000Z"),
            (newer_open.id, "2026-01-02T10:00:00.000Z"),
            (closed.id, "2026-01-03T10:00:00.000Z"),
        ] {
            conn.execute(
                "UPDATE tickets SET created_at_utc = ?1 WHERE id = ?2",
                params![ts, id],
            )
            .unwrap();
        }

        let tickets = list_for_customer(&conn, customer_id).unwrap();
        assert_eq!(
            tickets.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![newer_open.id, older_open.id, closed.id]
        );
    }

    #[test]
    fn list_for_customer_excludes_other_customers() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let other_id = customers::create(
            &conn,
            NewCustomer {
                name: "Beispiel AG".to_string(),
                short_code: "BSP".to_string(),
                notes: "".to_string(),
            },
            &berlin(),
        )
        .unwrap()
        .id;
        create(&conn, sample(customer_id), &berlin()).unwrap();
        create(&conn, sample(other_id), &berlin()).unwrap();

        let tickets = list_for_customer(&conn, customer_id).unwrap();
        assert_eq!(tickets.len(), 1);
    }

    #[test]
    fn update_missing_ticket_returns_not_found() {
        let conn = migrated_connection();
        let err = update(
            &conn,
            999,
            UpdateTicket {
                system_id: None,
                title: "x".to_string(),
                description: "".to_string(),
                status: "open".to_string(),
                time_spent_minutes: 0,
            },
            &berlin(),
        )
        .unwrap_err();
        assert_eq!(err.code(), "not_found");
    }

    #[test]
    fn update_rejects_empty_status() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(&conn, sample(customer_id), &berlin()).unwrap();
        // The empty string is exactly the case NewTicket defaults to "open" and
        // UpdateTicket must reject -- this pins that deliberate asymmetry.
        let err = update(
            &conn,
            created.id,
            UpdateTicket {
                system_id: None,
                title: "x".to_string(),
                description: "".to_string(),
                status: "".to_string(),
                time_spent_minutes: 0,
            },
            &berlin(),
        )
        .unwrap_err();
        assert_eq!(err.code(), "validation");
    }

    #[test]
    fn create_rejects_negative_time_spent_minutes() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let mut input = sample(customer_id);
        input.time_spent_minutes = -5;
        let err = create(&conn, input, &berlin()).unwrap_err();
        assert_eq!(err.code(), "validation");
    }

    #[test]
    fn update_rejects_negative_time_spent_minutes() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(&conn, sample(customer_id), &berlin()).unwrap();
        let err = update(
            &conn,
            created.id,
            UpdateTicket {
                system_id: None,
                title: "x".to_string(),
                description: "".to_string(),
                status: "open".to_string(),
                time_spent_minutes: -1,
            },
            &berlin(),
        )
        .unwrap_err();
        assert_eq!(err.code(), "validation");
    }

    #[test]
    fn delete_removes_ticket() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(&conn, sample(customer_id), &berlin()).unwrap();
        delete(&conn, created.id).unwrap();
        assert!(get(&conn, created.id).is_err());
    }

    #[test]
    fn delete_missing_ticket_returns_not_found() {
        let conn = migrated_connection();
        let err = delete(&conn, 999).unwrap_err();
        assert_eq!(err.code(), "not_found");
    }

    #[test]
    fn system_id_survives_referenced_system_deletion_as_null() {
        // Confirms the ON DELETE SET NULL FK behavior from the migration
        // actually holds -- mirrors the equivalent test pattern used for
        // networks.location_id in db/networks.rs.
        use crate::db::systems::{self, NewSystem};
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let system_id = systems::create(
            &conn,
            NewSystem {
                customer_id,
                name: "Server1".to_string(),
                system_type: "Server".to_string(),
                hostname: "".to_string(),
                ip_address: "".to_string(),
                notes: "".to_string(),
                maintenance_interval_days: None,
            },
            &berlin(),
        )
        .unwrap()
        .id;
        let mut input = sample(customer_id);
        input.system_id = Some(system_id);
        let ticket = create(&conn, input, &berlin()).unwrap();
        assert_eq!(ticket.system_id, Some(system_id));

        conn.execute("DELETE FROM systems WHERE id = ?1", params![system_id])
            .unwrap();

        let reloaded = get(&conn, ticket.id).unwrap();
        assert_eq!(reloaded.system_id, None);
    }
}
