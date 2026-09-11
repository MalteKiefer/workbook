use chrono_tz::Tz;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::error::AppError;
use crate::time::now_with_tz;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Location {
    pub id: i64,
    pub customer_id: i64,
    pub name: String,
    pub address_line1: String,
    pub address_line2: String,
    pub postal_code: String,
    pub city: String,
    pub country: String,
    pub phone: String,
    pub notes: String,
    pub created_at_utc: String,
    pub created_at_tz: String,
    pub updated_at_utc: String,
    pub updated_at_tz: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NewLocation {
    pub customer_id: i64,
    pub name: String,
    #[serde(default)]
    pub address_line1: String,
    #[serde(default)]
    pub address_line2: String,
    #[serde(default)]
    pub postal_code: String,
    #[serde(default)]
    pub city: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub phone: String,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UpdateLocation {
    pub name: String,
    #[serde(default)]
    pub address_line1: String,
    #[serde(default)]
    pub address_line2: String,
    #[serde(default)]
    pub postal_code: String,
    #[serde(default)]
    pub city: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub phone: String,
    #[serde(default)]
    pub notes: String,
}

fn validate_name(name: &str) -> Result<(), AppError> {
    if name.trim().is_empty() {
        return Err(AppError::Validation(
            "Name darf nicht leer sein.".to_string(),
        ));
    }
    Ok(())
}

fn row_to_location(row: &Row) -> rusqlite::Result<Location> {
    Ok(Location {
        id: row.get("id")?,
        customer_id: row.get("customer_id")?,
        name: row.get("name")?,
        address_line1: row.get("address_line1")?,
        address_line2: row.get("address_line2")?,
        postal_code: row.get("postal_code")?,
        city: row.get("city")?,
        country: row.get("country")?,
        phone: row.get("phone")?,
        notes: row.get("notes")?,
        created_at_utc: row.get("created_at_utc")?,
        created_at_tz: row.get("created_at_tz")?,
        updated_at_utc: row.get("updated_at_utc")?,
        updated_at_tz: row.get("updated_at_tz")?,
    })
}

pub fn create(conn: &Connection, input: NewLocation, tz: &Tz) -> Result<Location, AppError> {
    validate_name(&input.name)?;
    let (now_utc, now_tz) = now_with_tz(tz);
    conn.execute(
        "INSERT INTO locations (customer_id, name, address_line1, address_line2, postal_code, city, country, phone, notes, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?10, ?11)",
        params![
            input.customer_id, input.name, input.address_line1, input.address_line2,
            input.postal_code, input.city, input.country, input.phone, input.notes,
            now_utc, now_tz
        ],
    )?;
    get(conn, conn.last_insert_rowid())
}

pub fn get(conn: &Connection, id: i64) -> Result<Location, AppError> {
    conn.query_row(
        "SELECT * FROM locations WHERE id = ?1",
        params![id],
        row_to_location,
    )
    .optional()?
    .ok_or_else(|| AppError::NotFound(format!("Standort {id} nicht gefunden")))
}

pub fn list_for_customer(conn: &Connection, customer_id: i64) -> Result<Vec<Location>, AppError> {
    let mut stmt = conn
        .prepare("SELECT * FROM locations WHERE customer_id = ?1 ORDER BY name COLLATE NOCASE")?;
    let rows = stmt.query_map(params![customer_id], row_to_location)?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn update(
    conn: &Connection,
    id: i64,
    input: UpdateLocation,
    tz: &Tz,
) -> Result<Location, AppError> {
    validate_name(&input.name)?;
    let (now_utc, now_tz) = now_with_tz(tz);
    let changed = conn.execute(
        "UPDATE locations SET name = ?1, address_line1 = ?2, address_line2 = ?3, postal_code = ?4, city = ?5, country = ?6, phone = ?7, notes = ?8, updated_at_utc = ?9, updated_at_tz = ?10 WHERE id = ?11",
        params![
            input.name, input.address_line1, input.address_line2, input.postal_code,
            input.city, input.country, input.phone, input.notes, now_utc, now_tz, id
        ],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound(format!("Standort {id} nicht gefunden")));
    }
    get(conn, id)
}

pub fn delete(conn: &Connection, id: i64) -> Result<(), AppError> {
    let changed = conn.execute("DELETE FROM locations WHERE id = ?1", params![id])?;
    if changed == 0 {
        return Err(AppError::NotFound(format!("Standort {id} nicht gefunden")));
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

    fn sample(customer_id: i64) -> NewLocation {
        NewLocation {
            customer_id,
            name: "Hauptsitz".to_string(),
            address_line1: "Musterstr. 1".to_string(),
            address_line2: "".to_string(),
            postal_code: "12345".to_string(),
            city: "Berlin".to_string(),
            country: "Deutschland".to_string(),
            phone: "030 1234567".to_string(),
            notes: "".to_string(),
        }
    }

    #[test]
    fn create_then_get_roundtrips() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(&conn, sample(customer_id), &berlin()).unwrap();
        assert_eq!(created.city, "Berlin");
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
        b.name = "Zweigstelle".to_string();
        create(&conn, b, &berlin()).unwrap();
        create(&conn, sample(customer_id), &berlin()).unwrap(); // "Hauptsitz"
        create(&conn, sample(other_id), &berlin()).unwrap();

        let locations = list_for_customer(&conn, customer_id).unwrap();
        assert_eq!(locations.len(), 2);
        assert_eq!(locations[0].name, "Hauptsitz");
        assert_eq!(locations[1].name, "Zweigstelle");
    }

    #[test]
    fn update_missing_location_returns_not_found() {
        let conn = migrated_connection();
        let err = update(
            &conn,
            999,
            UpdateLocation {
                name: "x".to_string(),
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
        .unwrap_err();
        assert_eq!(err.code(), "not_found");
    }

    #[test]
    fn delete_removes_location() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(&conn, sample(customer_id), &berlin()).unwrap();
        delete(&conn, created.id).unwrap();
        assert!(get(&conn, created.id).is_err());
    }

    #[test]
    fn delete_missing_location_returns_not_found() {
        let conn = migrated_connection();
        let err = delete(&conn, 999).unwrap_err();
        assert_eq!(err.code(), "not_found");
    }
}
