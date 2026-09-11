use chrono_tz::Tz;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::error::AppError;
use crate::time::now_with_tz;
use crate::vault::crypto;

/// A vault entry with its secret/notes already decrypted -- returned to
/// the frontend only while the vault is unlocked (see
/// `commands::vault::require_unlocked`). Never serialized in its
/// still-encrypted form; there is no "encrypted DTO" type, since nothing
/// outside this module and its tests should ever see raw ciphertext.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct VaultEntry {
    pub id: i64,
    pub customer_id: i64,
    pub system_id: Option<i64>,
    pub label: String,
    pub username: String,
    pub secret: Option<String>,
    pub url: String,
    pub notes: Option<String>,
    pub created_at_utc: String,
    pub created_at_tz: String,
    pub updated_at_utc: String,
    pub updated_at_tz: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NewVaultEntry {
    pub customer_id: i64,
    pub system_id: Option<i64>,
    pub label: String,
    #[serde(default)]
    pub username: String,
    pub secret: Option<String>,
    #[serde(default)]
    pub url: String,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UpdateVaultEntry {
    pub system_id: Option<i64>,
    pub label: String,
    #[serde(default)]
    pub username: String,
    pub secret: Option<String>,
    #[serde(default)]
    pub url: String,
    pub notes: Option<String>,
}

fn validate_label(label: &str) -> Result<(), AppError> {
    if label.trim().is_empty() {
        return Err(AppError::Validation(
            "Bezeichnung darf nicht leer sein.".to_string(),
        ));
    }
    Ok(())
}

fn row_to_entry(row: &Row, key: &[u8; 32]) -> Result<VaultEntry, AppError> {
    let secret_encrypted: Option<String> = row.get("secret_encrypted")?;
    let notes_encrypted: Option<String> = row.get("notes_encrypted")?;
    Ok(VaultEntry {
        id: row.get("id")?,
        customer_id: row.get("customer_id")?,
        system_id: row.get("system_id")?,
        label: row.get("label")?,
        username: row.get("username")?,
        secret: secret_encrypted
            .map(|enc| crypto::decrypt(&enc, key))
            .transpose()?,
        url: row.get("url")?,
        notes: notes_encrypted
            .map(|enc| crypto::decrypt(&enc, key))
            .transpose()?,
        created_at_utc: row.get("created_at_utc")?,
        created_at_tz: row.get("created_at_tz")?,
        updated_at_utc: row.get("updated_at_utc")?,
        updated_at_tz: row.get("updated_at_tz")?,
    })
}

pub fn create(
    conn: &Connection,
    input: NewVaultEntry,
    key: &[u8; 32],
    tz: &Tz,
) -> Result<VaultEntry, AppError> {
    validate_label(&input.label)?;
    let secret_encrypted = input
        .secret
        .as_deref()
        .map(|s| crypto::encrypt(s, key))
        .transpose()?;
    let notes_encrypted = input
        .notes
        .as_deref()
        .map(|s| crypto::encrypt(s, key))
        .transpose()?;
    let (now_utc, now_tz) = now_with_tz(tz);
    conn.execute(
        "INSERT INTO vault_entries (customer_id, system_id, label, username, secret_encrypted, url, notes_encrypted, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?8, ?9)",
        params![
            input.customer_id,
            input.system_id,
            input.label,
            input.username,
            secret_encrypted,
            input.url,
            notes_encrypted,
            now_utc,
            now_tz
        ],
    )?;
    get(conn, conn.last_insert_rowid(), key)
}

pub fn get(conn: &Connection, id: i64, key: &[u8; 32]) -> Result<VaultEntry, AppError> {
    let row_result = conn
        .query_row(
            "SELECT * FROM vault_entries WHERE id = ?1",
            params![id],
            |row| Ok(row_to_entry(row, key)),
        )
        .optional()?;
    match row_result {
        Some(entry) => entry,
        None => Err(AppError::NotFound(format!(
            "Zugangsdaten-Eintrag {id} nicht gefunden"
        ))),
    }
}

/// Ordered by `label` -- callers filter by customer_id (and optionally
/// system_id) client-side or via a WHERE clause added at the SQL layer
/// here; kept to exactly the columns/params needed for
/// "every vault entry belonging to one customer" since that's the only
/// listing the UI needs (Task 6).
pub fn list_for_customer(
    conn: &Connection,
    customer_id: i64,
    key: &[u8; 32],
) -> Result<Vec<VaultEntry>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT * FROM vault_entries WHERE customer_id = ?1 ORDER BY label COLLATE NOCASE",
    )?;
    let rows = stmt.query_map(params![customer_id], |row| Ok(row_to_entry(row, key)))?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row??);
    }
    Ok(result)
}

pub fn update(
    conn: &Connection,
    id: i64,
    input: UpdateVaultEntry,
    key: &[u8; 32],
    tz: &Tz,
) -> Result<VaultEntry, AppError> {
    validate_label(&input.label)?;
    let secret_encrypted = input
        .secret
        .as_deref()
        .map(|s| crypto::encrypt(s, key))
        .transpose()?;
    let notes_encrypted = input
        .notes
        .as_deref()
        .map(|s| crypto::encrypt(s, key))
        .transpose()?;
    let (now_utc, now_tz) = now_with_tz(tz);
    let changed = conn.execute(
        "UPDATE vault_entries SET system_id = ?1, label = ?2, username = ?3, secret_encrypted = ?4, url = ?5, notes_encrypted = ?6, updated_at_utc = ?7, updated_at_tz = ?8 WHERE id = ?9",
        params![
            input.system_id,
            input.label,
            input.username,
            secret_encrypted,
            input.url,
            notes_encrypted,
            now_utc,
            now_tz,
            id
        ],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound(format!(
            "Zugangsdaten-Eintrag {id} nicht gefunden"
        )));
    }
    get(conn, id, key)
}

pub fn delete(conn: &Connection, id: i64) -> Result<(), AppError> {
    let changed = conn.execute("DELETE FROM vault_entries WHERE id = ?1", params![id])?;
    if changed == 0 {
        return Err(AppError::NotFound(format!(
            "Zugangsdaten-Eintrag {id} nicht gefunden"
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

    fn test_key() -> [u8; 32] {
        crypto::derive_key("test passphrase", &crypto::generate_salt()).unwrap()
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

    #[test]
    fn create_then_get_decrypts_secret_and_notes() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let key = test_key();
        let created = create(
            &conn,
            NewVaultEntry {
                customer_id,
                system_id: None,
                label: "Router Admin".to_string(),
                username: "admin".to_string(),
                secret: Some("hunter2".to_string()),
                url: "https://192.168.1.1".to_string(),
                notes: Some("Seriennummer auf der Rückseite".to_string()),
            },
            &key,
            &berlin(),
        )
        .unwrap();
        assert_eq!(created.secret, Some("hunter2".to_string()));
        assert_eq!(
            created.notes,
            Some("Seriennummer auf der Rückseite".to_string())
        );

        let fetched = get(&conn, created.id, &key).unwrap();
        assert_eq!(fetched, created);
    }

    #[test]
    fn create_with_no_secret_or_notes_leaves_both_none() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let key = test_key();
        let created = create(
            &conn,
            NewVaultEntry {
                customer_id,
                system_id: None,
                label: "WLAN Gast".to_string(),
                username: "".to_string(),
                secret: None,
                url: "".to_string(),
                notes: None,
            },
            &key,
            &berlin(),
        )
        .unwrap();
        assert_eq!(created.secret, None);
        assert_eq!(created.notes, None);
    }

    #[test]
    fn get_with_wrong_key_fails_to_decrypt() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let key = test_key();
        let wrong_key =
            crypto::derive_key("different passphrase", &crypto::generate_salt()).unwrap();
        let created = create(
            &conn,
            NewVaultEntry {
                customer_id,
                system_id: None,
                label: "Test".to_string(),
                username: "".to_string(),
                secret: Some("hunter2".to_string()),
                url: "".to_string(),
                notes: None,
            },
            &key,
            &berlin(),
        )
        .unwrap();
        assert!(get(&conn, created.id, &wrong_key).is_err());
    }

    #[test]
    fn create_rejects_blank_label() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let err = create(
            &conn,
            NewVaultEntry {
                customer_id,
                system_id: None,
                label: "   ".to_string(),
                username: "".to_string(),
                secret: None,
                url: "".to_string(),
                notes: None,
            },
            &test_key(),
            &berlin(),
        )
        .unwrap_err();
        assert_eq!(err.code(), "validation");
    }

    #[test]
    fn list_for_customer_orders_by_label_and_excludes_other_customers() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let other_customer_id = customers::create(
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
        let key = test_key();
        for label in ["Zabbix", "Admin-Login"] {
            create(
                &conn,
                NewVaultEntry {
                    customer_id,
                    system_id: None,
                    label: label.to_string(),
                    username: "".to_string(),
                    secret: None,
                    url: "".to_string(),
                    notes: None,
                },
                &key,
                &berlin(),
            )
            .unwrap();
        }
        create(
            &conn,
            NewVaultEntry {
                customer_id: other_customer_id,
                system_id: None,
                label: "Anderer Kunde".to_string(),
                username: "".to_string(),
                secret: None,
                url: "".to_string(),
                notes: None,
            },
            &key,
            &berlin(),
        )
        .unwrap();

        let entries = list_for_customer(&conn, customer_id, &key).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].label, "Admin-Login");
        assert_eq!(entries[1].label, "Zabbix");
    }

    #[test]
    fn update_changes_fields_and_reencrypts() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let key = test_key();
        let created = create(
            &conn,
            NewVaultEntry {
                customer_id,
                system_id: None,
                label: "Alt".to_string(),
                username: "old".to_string(),
                secret: Some("old-secret".to_string()),
                url: "".to_string(),
                notes: None,
            },
            &key,
            &berlin(),
        )
        .unwrap();

        let updated = update(
            &conn,
            created.id,
            UpdateVaultEntry {
                system_id: None,
                label: "Neu".to_string(),
                username: "new".to_string(),
                secret: Some("new-secret".to_string()),
                url: "".to_string(),
                notes: None,
            },
            &key,
            &berlin(),
        )
        .unwrap();
        assert_eq!(updated.label, "Neu");
        assert_eq!(updated.secret, Some("new-secret".to_string()));
    }

    #[test]
    fn update_missing_entry_returns_not_found() {
        let conn = migrated_connection();
        let err = update(
            &conn,
            999,
            UpdateVaultEntry {
                system_id: None,
                label: "x".to_string(),
                username: "".to_string(),
                secret: None,
                url: "".to_string(),
                notes: None,
            },
            &test_key(),
            &berlin(),
        )
        .unwrap_err();
        assert_eq!(err.code(), "not_found");
    }

    #[test]
    fn delete_removes_entry() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let key = test_key();
        let created = create(
            &conn,
            NewVaultEntry {
                customer_id,
                system_id: None,
                label: "Test".to_string(),
                username: "".to_string(),
                secret: None,
                url: "".to_string(),
                notes: None,
            },
            &key,
            &berlin(),
        )
        .unwrap();
        delete(&conn, created.id).unwrap();
        assert!(get(&conn, created.id, &key).is_err());
    }

    #[test]
    fn delete_missing_entry_returns_not_found() {
        let conn = migrated_connection();
        let err = delete(&conn, 999).unwrap_err();
        assert_eq!(err.code(), "not_found");
    }
}
