use chrono_tz::Tz;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::error::AppError;
use crate::network_scan;
use crate::time::now_with_tz;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Network {
    pub id: i64,
    pub customer_id: i64,
    pub name: String,
    pub cidr: String,
    pub location_id: Option<i64>,
    pub notes: String,
    pub created_at_utc: String,
    pub created_at_tz: String,
    pub updated_at_utc: String,
    pub updated_at_tz: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NewNetwork {
    pub customer_id: i64,
    pub name: String,
    pub cidr: String,
    #[serde(default)]
    pub location_id: Option<i64>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UpdateNetwork {
    pub name: String,
    pub cidr: String,
    #[serde(default)]
    pub location_id: Option<i64>,
    #[serde(default)]
    pub notes: String,
}

fn validate(name: &str, cidr: &str) -> Result<(), AppError> {
    if name.trim().is_empty() {
        return Err(AppError::Validation(
            "Name darf nicht leer sein.".to_string(),
        ));
    }
    // Reuses the network-scan feature's own CIDR parser/bounds-checker
    // (rejects malformed input and anything broader than /20) -- see
    // network_scan::parse_cidr's own doc comment for the exact rules.
    // The parsed address list itself isn't needed here, only the
    // validation side effect (an Err on anything invalid).
    network_scan::parse_cidr(cidr)?;
    Ok(())
}

fn row_to_network(row: &Row) -> rusqlite::Result<Network> {
    Ok(Network {
        id: row.get("id")?,
        customer_id: row.get("customer_id")?,
        name: row.get("name")?,
        cidr: row.get("cidr")?,
        location_id: row.get("location_id")?,
        notes: row.get("notes")?,
        created_at_utc: row.get("created_at_utc")?,
        created_at_tz: row.get("created_at_tz")?,
        updated_at_utc: row.get("updated_at_utc")?,
        updated_at_tz: row.get("updated_at_tz")?,
    })
}

pub fn create(conn: &Connection, input: NewNetwork, tz: &Tz) -> Result<Network, AppError> {
    validate(&input.name, &input.cidr)?;
    let (now_utc, now_tz) = now_with_tz(tz);
    conn.execute(
        "INSERT INTO networks (customer_id, name, cidr, location_id, notes, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?6, ?7)",
        params![
            input.customer_id,
            input.name,
            input.cidr,
            input.location_id,
            input.notes,
            now_utc,
            now_tz
        ],
    )?;
    get(conn, conn.last_insert_rowid())
}

pub fn get(conn: &Connection, id: i64) -> Result<Network, AppError> {
    conn.query_row(
        "SELECT * FROM networks WHERE id = ?1",
        params![id],
        row_to_network,
    )
    .optional()?
    .ok_or_else(|| AppError::NotFound(format!("Netzwerk {id} nicht gefunden")))
}

pub fn list_for_customer(conn: &Connection, customer_id: i64) -> Result<Vec<Network>, AppError> {
    let mut stmt =
        conn.prepare("SELECT * FROM networks WHERE customer_id = ?1 ORDER BY name COLLATE NOCASE")?;
    let rows = stmt.query_map(params![customer_id], row_to_network)?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn update(
    conn: &Connection,
    id: i64,
    input: UpdateNetwork,
    tz: &Tz,
) -> Result<Network, AppError> {
    validate(&input.name, &input.cidr)?;
    let (now_utc, now_tz) = now_with_tz(tz);
    let changed = conn.execute(
        "UPDATE networks SET name = ?1, cidr = ?2, location_id = ?3, notes = ?4, updated_at_utc = ?5, updated_at_tz = ?6 WHERE id = ?7",
        params![
            input.name,
            input.cidr,
            input.location_id,
            input.notes,
            now_utc,
            now_tz,
            id
        ],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound(format!("Netzwerk {id} nicht gefunden")));
    }
    get(conn, id)
}

pub fn delete(conn: &Connection, id: i64) -> Result<(), AppError> {
    let changed = conn.execute("DELETE FROM networks WHERE id = ?1", params![id])?;
    if changed == 0 {
        return Err(AppError::NotFound(format!("Netzwerk {id} nicht gefunden")));
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

    fn sample(customer_id: i64) -> NewNetwork {
        NewNetwork {
            customer_id,
            name: "Hauptbüro".to_string(),
            cidr: "192.168.1.0/24".to_string(),
            location_id: None,
            notes: "".to_string(),
        }
    }

    #[test]
    fn create_then_get_roundtrips() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(&conn, sample(customer_id), &berlin()).unwrap();
        assert_eq!(created.cidr, "192.168.1.0/24");
        assert_eq!(get(&conn, created.id).unwrap(), created);
    }

    #[test]
    fn create_rejects_blank_name() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let mut input = sample(customer_id);
        input.name = "  ".to_string();
        let err = create(&conn, input, &berlin()).unwrap_err();
        assert_eq!(err.code(), "validation");
    }

    #[test]
    fn create_rejects_malformed_cidr() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let mut input = sample(customer_id);
        input.cidr = "not-a-cidr".to_string();
        let err = create(&conn, input, &berlin()).unwrap_err();
        assert_eq!(err.code(), "validation");
    }

    #[test]
    fn create_rejects_cidr_broader_than_slash_20() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let mut input = sample(customer_id);
        input.cidr = "10.0.0.0/8".to_string();
        let err = create(&conn, input, &berlin()).unwrap_err();
        assert_eq!(err.code(), "validation");
    }

    #[test]
    fn list_for_customer_orders_by_name_and_excludes_other_customers() {
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
        let mut b = sample(customer_id);
        b.name = "Serverraum".to_string();
        b.cidr = "10.0.0.0/28".to_string();
        create(&conn, b, &berlin()).unwrap();
        create(&conn, sample(customer_id), &berlin()).unwrap(); // "Hauptbüro"
        create(&conn, sample(other_id), &berlin()).unwrap();

        let networks = list_for_customer(&conn, customer_id).unwrap();
        assert_eq!(networks.len(), 2);
        assert_eq!(networks[0].name, "Hauptbüro");
        assert_eq!(networks[1].name, "Serverraum");
    }

    #[test]
    fn update_missing_network_returns_not_found() {
        let conn = migrated_connection();
        let err = update(
            &conn,
            999,
            UpdateNetwork {
                name: "x".to_string(),
                cidr: "10.0.0.0/24".to_string(),
                location_id: None,
                notes: "".to_string(),
            },
            &berlin(),
        )
        .unwrap_err();
        assert_eq!(err.code(), "not_found");
    }

    #[test]
    fn delete_removes_network() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(&conn, sample(customer_id), &berlin()).unwrap();
        delete(&conn, created.id).unwrap();
        assert!(get(&conn, created.id).is_err());
    }

    #[test]
    fn delete_missing_network_returns_not_found() {
        let conn = migrated_connection();
        let err = delete(&conn, 999).unwrap_err();
        assert_eq!(err.code(), "not_found");
    }

    #[test]
    fn location_id_roundtrips_through_create_and_update() {
        use crate::db::locations::{self, NewLocation};

        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        // migrated_connection() enables `PRAGMA foreign_keys = ON` before running
        // migrations, so FK enforcement is strict here (unlike a bare in-memory
        // connection without that pragma) -- a bare `location_id: Some(42)` with
        // no real `locations` row 42 fails the INSERT with a FK constraint
        // violation. A real Location is created first so the id is valid.
        let location_id = locations::create(
            &conn,
            NewLocation {
                customer_id,
                name: "Filiale Nord".to_string(),
                address_line1: "".to_string(),
                address_line2: "".to_string(),
                postal_code: "".to_string(),
                city: "".to_string(),
                country: "".to_string(),
                phone: "".to_string(),
                notes: "".to_string(),
            },
            &berlin(),
        )
        .unwrap()
        .id;

        let mut input = sample(customer_id);
        input.location_id = Some(location_id);
        let created = create(&conn, input, &berlin()).unwrap();
        assert_eq!(created.location_id, Some(location_id));

        let updated = update(
            &conn,
            created.id,
            UpdateNetwork {
                name: created.name.clone(),
                cidr: created.cidr.clone(),
                location_id: None,
                notes: created.notes.clone(),
            },
            &berlin(),
        )
        .unwrap();
        assert_eq!(updated.location_id, None);
    }
}
