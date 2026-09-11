use chrono_tz::Tz;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::error::AppError;
use crate::time::now_with_tz;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpiringItemKind {
    SslCertificate,
    Domain,
    License,
    Contract,
    Warranty,
    Sonstiges,
}

impl ExpiringItemKind {
    fn as_db_str(&self) -> &'static str {
        match self {
            ExpiringItemKind::SslCertificate => "ssl_certificate",
            ExpiringItemKind::Domain => "domain",
            ExpiringItemKind::License => "license",
            ExpiringItemKind::Contract => "contract",
            ExpiringItemKind::Warranty => "warranty",
            ExpiringItemKind::Sonstiges => "sonstiges",
        }
    }

    fn from_db_str(s: &str) -> Result<Self, AppError> {
        match s {
            "ssl_certificate" => Ok(ExpiringItemKind::SslCertificate),
            "domain" => Ok(ExpiringItemKind::Domain),
            "license" => Ok(ExpiringItemKind::License),
            "contract" => Ok(ExpiringItemKind::Contract),
            "warranty" => Ok(ExpiringItemKind::Warranty),
            "sonstiges" => Ok(ExpiringItemKind::Sonstiges),
            other => Err(AppError::Database(format!("unbekannte kind: {other}"))),
        }
    }
}

impl rusqlite::types::ToSql for ExpiringItemKind {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(rusqlite::types::ToSqlOutput::from(self.as_db_str()))
    }
}

impl rusqlite::types::FromSql for ExpiringItemKind {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        let s = value.as_str()?;
        ExpiringItemKind::from_db_str(s).map_err(|_| rusqlite::types::FromSqlError::InvalidType)
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ExpiringItem {
    pub id: i64,
    pub customer_id: i64,
    pub system_id: Option<i64>,
    pub kind: ExpiringItemKind,
    pub label: String,
    pub expires_on: String,
    pub reminder_days_before: i64,
    pub notes: String,
    pub created_at_utc: String,
    pub created_at_tz: String,
    pub updated_at_utc: String,
    pub updated_at_tz: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NewExpiringItem {
    pub customer_id: i64,
    pub system_id: Option<i64>,
    pub kind: ExpiringItemKind,
    pub label: String,
    pub expires_on: String,
    #[serde(default = "default_reminder_days")]
    pub reminder_days_before: i64,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UpdateExpiringItem {
    pub system_id: Option<i64>,
    pub kind: ExpiringItemKind,
    pub label: String,
    pub expires_on: String,
    pub reminder_days_before: i64,
    #[serde(default)]
    pub notes: String,
}

fn default_reminder_days() -> i64 {
    30
}

fn validate(label: &str, expires_on: &str) -> Result<(), AppError> {
    if label.trim().is_empty() {
        return Err(AppError::Validation(
            "Bezeichnung darf nicht leer sein.".to_string(),
        ));
    }
    if chrono::NaiveDate::parse_from_str(expires_on, "%Y-%m-%d").is_err() {
        return Err(AppError::Validation(
            "Ablaufdatum muss im Format JJJJ-MM-TT sein.".to_string(),
        ));
    }
    Ok(())
}

fn row_to_item(row: &Row) -> rusqlite::Result<ExpiringItem> {
    Ok(ExpiringItem {
        id: row.get("id")?,
        customer_id: row.get("customer_id")?,
        system_id: row.get("system_id")?,
        kind: row.get("kind")?,
        label: row.get("label")?,
        expires_on: row.get("expires_on")?,
        reminder_days_before: row.get("reminder_days_before")?,
        notes: row.get("notes")?,
        created_at_utc: row.get("created_at_utc")?,
        created_at_tz: row.get("created_at_tz")?,
        updated_at_utc: row.get("updated_at_utc")?,
        updated_at_tz: row.get("updated_at_tz")?,
    })
}

pub fn create(
    conn: &Connection,
    input: NewExpiringItem,
    tz: &Tz,
) -> Result<ExpiringItem, AppError> {
    validate(&input.label, &input.expires_on)?;
    let (now_utc, now_tz) = now_with_tz(tz);
    conn.execute(
        "INSERT INTO expiring_items (customer_id, system_id, kind, label, expires_on, reminder_days_before, notes, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?8, ?9)",
        params![
            input.customer_id,
            input.system_id,
            input.kind,
            input.label,
            input.expires_on,
            input.reminder_days_before,
            input.notes,
            now_utc,
            now_tz
        ],
    )?;
    get(conn, conn.last_insert_rowid())
}

pub fn get(conn: &Connection, id: i64) -> Result<ExpiringItem, AppError> {
    conn.query_row(
        "SELECT * FROM expiring_items WHERE id = ?1",
        params![id],
        row_to_item,
    )
    .optional()?
    .ok_or_else(|| AppError::NotFound(format!("Ablauf-Eintrag {id} nicht gefunden")))
}

/// Every expiring item across every customer, ordered soonest-first --
/// powers both the customer-scoped list (frontend filters client-side by
/// `customer_id`, same convention `list_entries`'s frontend callers
/// sometimes use) and the Dashboard's cross-customer warning list, which
/// needs every customer at once as one query (mirrors
/// `commands::systems::list_overdue_systems`'s "fan out over every
/// customer" comment, except here it's a single flat table so one query
/// suffices -- no per-customer loop needed).
pub fn list_all(conn: &Connection) -> Result<Vec<ExpiringItem>, AppError> {
    let mut stmt = conn.prepare("SELECT * FROM expiring_items ORDER BY expires_on ASC")?;
    let rows = stmt.query_map([], row_to_item)?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn update(
    conn: &Connection,
    id: i64,
    input: UpdateExpiringItem,
    tz: &Tz,
) -> Result<ExpiringItem, AppError> {
    validate(&input.label, &input.expires_on)?;
    let (now_utc, now_tz) = now_with_tz(tz);
    let changed = conn.execute(
        "UPDATE expiring_items SET system_id = ?1, kind = ?2, label = ?3, expires_on = ?4, reminder_days_before = ?5, notes = ?6, updated_at_utc = ?7, updated_at_tz = ?8 WHERE id = ?9",
        params![
            input.system_id,
            input.kind,
            input.label,
            input.expires_on,
            input.reminder_days_before,
            input.notes,
            now_utc,
            now_tz,
            id
        ],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound(format!(
            "Ablauf-Eintrag {id} nicht gefunden"
        )));
    }
    get(conn, id)
}

pub fn delete(conn: &Connection, id: i64) -> Result<(), AppError> {
    let changed = conn.execute("DELETE FROM expiring_items WHERE id = ?1", params![id])?;
    if changed == 0 {
        return Err(AppError::NotFound(format!(
            "Ablauf-Eintrag {id} nicht gefunden"
        )));
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

    fn sample(customer_id: i64) -> NewExpiringItem {
        NewExpiringItem {
            customer_id,
            system_id: None,
            kind: ExpiringItemKind::SslCertificate,
            label: "Wildcard-Zertifikat *.acme.local".to_string(),
            expires_on: "2027-03-15".to_string(),
            reminder_days_before: 30,
            notes: "".to_string(),
        }
    }

    #[test]
    fn create_then_get_roundtrips() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(&conn, sample(customer_id), &berlin()).unwrap();
        assert_eq!(created.kind, ExpiringItemKind::SslCertificate);
        assert_eq!(get(&conn, created.id).unwrap(), created);
    }

    #[test]
    fn create_rejects_blank_label() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let mut input = sample(customer_id);
        input.label = "  ".to_string();
        let err = create(&conn, input, &berlin()).unwrap_err();
        assert_eq!(err.code(), "validation");
    }

    #[test]
    fn create_rejects_malformed_date() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let mut input = sample(customer_id);
        input.expires_on = "15.03.2027".to_string();
        let err = create(&conn, input, &berlin()).unwrap_err();
        assert_eq!(err.code(), "validation");
    }

    #[test]
    fn list_all_orders_soonest_first() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let mut later = sample(customer_id);
        later.label = "Später".to_string();
        later.expires_on = "2028-01-01".to_string();
        create(&conn, later, &berlin()).unwrap();
        let mut sooner = sample(customer_id);
        sooner.label = "Bald".to_string();
        sooner.expires_on = "2026-10-01".to_string();
        create(&conn, sooner, &berlin()).unwrap();

        let all = list_all(&conn).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].label, "Bald");
        assert_eq!(all[1].label, "Später");
    }

    #[test]
    fn update_missing_item_returns_not_found() {
        let conn = migrated_connection();
        let err = update(
            &conn,
            999,
            UpdateExpiringItem {
                system_id: None,
                kind: ExpiringItemKind::Domain,
                label: "x".to_string(),
                expires_on: "2027-01-01".to_string(),
                reminder_days_before: 30,
                notes: "".to_string(),
            },
            &berlin(),
        )
        .unwrap_err();
        assert_eq!(err.code(), "not_found");
    }

    #[test]
    fn delete_removes_item() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(&conn, sample(customer_id), &berlin()).unwrap();
        delete(&conn, created.id).unwrap();
        assert!(get(&conn, created.id).is_err());
    }

    #[test]
    fn delete_missing_item_returns_not_found() {
        let conn = migrated_connection();
        let err = delete(&conn, 999).unwrap_err();
        assert_eq!(err.code(), "not_found");
    }
}
