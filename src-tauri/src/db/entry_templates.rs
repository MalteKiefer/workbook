use chrono_tz::Tz;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::db::entries::Category;
use crate::error::AppError;
use crate::time::now_with_tz;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct EntryTemplate {
    pub id: i64,
    pub name: String,
    pub system_type: String,
    pub title: String,
    pub body_md: String,
    pub category: Category,
    pub tags_csv: String,
    pub created_at_utc: String,
    pub created_at_tz: String,
    pub updated_at_utc: String,
    pub updated_at_tz: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NewEntryTemplate {
    pub name: String,
    #[serde(default)]
    pub system_type: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body_md: String,
    pub category: Category,
    #[serde(default)]
    pub tags_csv: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UpdateEntryTemplate {
    pub name: String,
    #[serde(default)]
    pub system_type: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body_md: String,
    pub category: Category,
    #[serde(default)]
    pub tags_csv: String,
}

fn row_to_template(row: &Row) -> rusqlite::Result<EntryTemplate> {
    Ok(EntryTemplate {
        id: row.get("id")?,
        name: row.get("name")?,
        system_type: row.get("system_type")?,
        title: row.get("title")?,
        body_md: row.get("body_md")?,
        category: row.get("category")?,
        tags_csv: row.get("tags_csv")?,
        created_at_utc: row.get("created_at_utc")?,
        created_at_tz: row.get("created_at_tz")?,
        updated_at_utc: row.get("updated_at_utc")?,
        updated_at_tz: row.get("updated_at_tz")?,
    })
}

fn validate_name(name: &str) -> Result<(), AppError> {
    if name.trim().is_empty() {
        return Err(AppError::Validation(
            "Vorlagenname darf nicht leer sein.".to_string(),
        ));
    }
    Ok(())
}

pub fn create(conn: &Connection, input: NewEntryTemplate, tz: &Tz) -> Result<EntryTemplate, AppError> {
    validate_name(&input.name)?;
    let (now_utc, now_tz) = now_with_tz(tz);
    conn.execute(
        "INSERT INTO entry_templates (name, system_type, title, body_md, category, tags_csv, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?7, ?8)",
        params![
            input.name,
            input.system_type,
            input.title,
            input.body_md,
            input.category,
            input.tags_csv,
            now_utc,
            now_tz
        ],
    )?;
    get(conn, conn.last_insert_rowid())
}

pub fn get(conn: &Connection, id: i64) -> Result<EntryTemplate, AppError> {
    conn.query_row(
        "SELECT * FROM entry_templates WHERE id = ?1",
        params![id],
        row_to_template,
    )
    .optional()?
    .ok_or_else(|| AppError::NotFound(format!("Vorlage {id} nicht gefunden")))
}

/// Ordered by `system_type` first (empty/general templates last is not
/// special-cased -- empty string sorts before any non-empty one, which is
/// fine here), then `name`, so templates for the same system type sit
/// together in the management list and the apply dropdown.
pub fn list_all(conn: &Connection) -> Result<Vec<EntryTemplate>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT * FROM entry_templates ORDER BY system_type COLLATE NOCASE, name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map([], row_to_template)?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn update(
    conn: &Connection,
    id: i64,
    input: UpdateEntryTemplate,
    tz: &Tz,
) -> Result<EntryTemplate, AppError> {
    validate_name(&input.name)?;
    let (now_utc, now_tz) = now_with_tz(tz);
    let changed = conn.execute(
        "UPDATE entry_templates SET name = ?1, system_type = ?2, title = ?3, body_md = ?4, category = ?5, tags_csv = ?6, updated_at_utc = ?7, updated_at_tz = ?8 WHERE id = ?9",
        params![
            input.name,
            input.system_type,
            input.title,
            input.body_md,
            input.category,
            input.tags_csv,
            now_utc,
            now_tz,
            id
        ],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound(format!("Vorlage {id} nicht gefunden")));
    }
    get(conn, id)
}

pub fn delete(conn: &Connection, id: i64) -> Result<(), AppError> {
    let changed = conn.execute("DELETE FROM entry_templates WHERE id = ?1", params![id])?;
    if changed == 0 {
        return Err(AppError::NotFound(format!("Vorlage {id} nicht gefunden")));
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

    fn sample() -> NewEntryTemplate {
        NewEntryTemplate {
            name: "Monatliches Exchange-Update".to_string(),
            system_type: "Exchange Server".to_string(),
            title: "Exchange kumulatives Update".to_string(),
            body_md: "- Update eingespielt\n- Dienste geprüft".to_string(),
            category: Category::Wartung,
            tags_csv: "exchange, patchday".to_string(),
        }
    }

    #[test]
    fn create_then_get_roundtrips() {
        let conn = migrated_connection();
        let created = create(&conn, sample(), &berlin()).unwrap();
        assert_eq!(created.name, "Monatliches Exchange-Update");
        assert_eq!(created.system_type, "Exchange Server");
        assert_eq!(created.tags_csv, "exchange, patchday");

        let fetched = get(&conn, created.id).unwrap();
        assert_eq!(fetched, created);
    }

    #[test]
    fn create_rejects_blank_name() {
        let conn = migrated_connection();
        let mut input = sample();
        input.name = "   ".to_string();
        let err = create(&conn, input, &berlin()).unwrap_err();
        assert_eq!(err.code(), "validation");
    }

    #[test]
    fn list_all_orders_by_system_type_then_name() {
        let conn = migrated_connection();
        let mut a = sample();
        a.name = "Zabbix-Check".to_string();
        a.system_type = "Monitoring".to_string();
        create(&conn, a, &berlin()).unwrap();
        let mut b = sample();
        b.name = "Backup-Kontrolle".to_string();
        b.system_type = "Fileserver".to_string();
        create(&conn, b, &berlin()).unwrap();

        let all = list_all(&conn).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].system_type, "Fileserver");
        assert_eq!(all[1].system_type, "Monitoring");
    }

    #[test]
    fn update_changes_fields_and_bumps_updated_at() {
        let conn = migrated_connection();
        let created = create(&conn, sample(), &berlin()).unwrap();
        let updated = update(
            &conn,
            created.id,
            UpdateEntryTemplate {
                name: "Exchange-Update (überarbeitet)".to_string(),
                system_type: created.system_type.clone(),
                title: created.title.clone(),
                body_md: created.body_md.clone(),
                category: Category::Wartung,
                tags_csv: created.tags_csv.clone(),
            },
            &berlin(),
        )
        .unwrap();
        assert_eq!(updated.name, "Exchange-Update (überarbeitet)");
        assert_eq!(updated.id, created.id);
    }

    #[test]
    fn update_missing_template_returns_not_found() {
        let conn = migrated_connection();
        let err = update(
            &conn,
            999,
            UpdateEntryTemplate {
                name: "x".to_string(),
                system_type: "".to_string(),
                title: "".to_string(),
                body_md: "".to_string(),
                category: Category::Wartung,
                tags_csv: "".to_string(),
            },
            &berlin(),
        )
        .unwrap_err();
        assert_eq!(err.code(), "not_found");
    }

    #[test]
    fn delete_removes_template() {
        let conn = migrated_connection();
        let created = create(&conn, sample(), &berlin()).unwrap();
        delete(&conn, created.id).unwrap();
        let err = get(&conn, created.id).unwrap_err();
        assert_eq!(err.code(), "not_found");
    }

    #[test]
    fn delete_missing_template_returns_not_found() {
        let conn = migrated_connection();
        let err = delete(&conn, 999).unwrap_err();
        assert_eq!(err.code(), "not_found");
    }
}
