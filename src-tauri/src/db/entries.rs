use chrono_tz::Tz;
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::db::tags;
use crate::error::AppError;
use crate::time::now_with_tz;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Wartung,
    Stoerung,
    Aenderung,
    Installation,
    Sonstiges,
}

impl Category {
    fn as_db_str(&self) -> &'static str {
        match self {
            Category::Wartung => "wartung",
            Category::Stoerung => "stoerung",
            Category::Aenderung => "aenderung",
            Category::Installation => "installation",
            Category::Sonstiges => "sonstiges",
        }
    }

    fn from_db_str(s: &str) -> Result<Self, AppError> {
        match s {
            "wartung" => Ok(Category::Wartung),
            "stoerung" => Ok(Category::Stoerung),
            "aenderung" => Ok(Category::Aenderung),
            "installation" => Ok(Category::Installation),
            "sonstiges" => Ok(Category::Sonstiges),
            other => Err(AppError::Database(format!("unbekannte category: {other}"))),
        }
    }
}

impl ToSql for Category {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.as_db_str()))
    }
}

impl FromSql for Category {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let s = value.as_str()?;
        Category::from_db_str(s).map_err(|_| FromSqlError::InvalidType)
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Entry {
    pub id: i64,
    pub customer_id: i64,
    pub system_id: Option<i64>,
    pub title: String,
    pub body_md: String,
    pub category: Category,
    pub performed_at_utc: String,
    pub performed_at_tz: String,
    pub created_at_utc: String,
    pub created_at_tz: String,
    pub updated_at_utc: String,
    pub updated_at_tz: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NewEntry {
    pub customer_id: i64,
    pub system_id: Option<i64>,
    pub title: String,
    pub body_md: String,
    pub category: Category,
    pub performed_at_utc: String,
    pub performed_at_tz: String,
    pub tag_names: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UpdateEntry {
    pub system_id: Option<i64>,
    pub title: String,
    pub body_md: String,
    pub category: Category,
    pub performed_at_utc: String,
    pub performed_at_tz: String,
    pub tag_names: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct EntryFilter {
    pub customer_id: Option<i64>,
    pub system_id: Option<i64>,
    pub category: Option<Category>,
    pub tag: Option<String>,
    pub from_utc: Option<String>,
    pub to_utc: Option<String>,
}

fn row_to_entry(row: &Row) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: row.get("id")?,
        customer_id: row.get("customer_id")?,
        system_id: row.get("system_id")?,
        title: row.get("title")?,
        body_md: row.get("body_md")?,
        category: row.get("category")?,
        performed_at_utc: row.get("performed_at_utc")?,
        performed_at_tz: row.get("performed_at_tz")?,
        created_at_utc: row.get("created_at_utc")?,
        created_at_tz: row.get("created_at_tz")?,
        updated_at_utc: row.get("updated_at_utc")?,
        updated_at_tz: row.get("updated_at_tz")?,
        tags: Vec::new(),
    })
}

pub fn create(conn: &Connection, input: NewEntry, tz: &Tz) -> Result<Entry, AppError> {
    let (now_utc, now_tz) = now_with_tz(tz);
    conn.execute(
        "INSERT INTO entries (customer_id, system_id, title, body_md, category, performed_at_utc, performed_at_tz, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?8, ?9)",
        params![
            input.customer_id,
            input.system_id,
            input.title,
            input.body_md,
            input.category,
            input.performed_at_utc,
            input.performed_at_tz,
            now_utc,
            now_tz,
        ],
    )?;
    let id = conn.last_insert_rowid();
    tags::set_tags_for_entry(conn, id, &input.tag_names)?;
    get(conn, id)
}

pub fn get(conn: &Connection, id: i64) -> Result<Entry, AppError> {
    let mut entry = conn
        .query_row("SELECT * FROM entries WHERE id = ?1", params![id], row_to_entry)
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("Eintrag {id} nicht gefunden")))?;
    entry.tags = tags::tags_for_entry(conn, id)?;
    Ok(entry)
}

pub fn update(conn: &Connection, id: i64, input: UpdateEntry, tz: &Tz) -> Result<Entry, AppError> {
    let (now_utc, now_tz) = now_with_tz(tz);
    let changed = conn.execute(
        "UPDATE entries SET system_id = ?1, title = ?2, body_md = ?3, category = ?4, performed_at_utc = ?5, performed_at_tz = ?6, updated_at_utc = ?7, updated_at_tz = ?8 WHERE id = ?9",
        params![
            input.system_id,
            input.title,
            input.body_md,
            input.category,
            input.performed_at_utc,
            input.performed_at_tz,
            now_utc,
            now_tz,
            id,
        ],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound(format!("Eintrag {id} nicht gefunden")));
    }
    tags::set_tags_for_entry(conn, id, &input.tag_names)?;
    get(conn, id)
}

pub fn list(conn: &Connection, filter: &EntryFilter) -> Result<Vec<Entry>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT * FROM entries e
         WHERE (?1 IS NULL OR customer_id = ?1)
           AND (?2 IS NULL OR system_id = ?2)
           AND (?3 IS NULL OR category = ?3)
           AND (?4 IS NULL OR performed_at_utc >= ?4)
           AND (?5 IS NULL OR performed_at_utc <= ?5)
           AND (?6 IS NULL OR EXISTS (
                 SELECT 1 FROM entry_tags et JOIN tags t ON t.id = et.tag_id
                 WHERE et.entry_id = e.id AND t.name = ?6
               ))
         ORDER BY performed_at_utc DESC",
    )?;
    let category_str = filter.category.map(|c| c.as_db_str());
    let rows = stmt.query_map(
        params![filter.customer_id, filter.system_id, category_str, filter.from_utc, filter.to_utc, filter.tag],
        row_to_entry,
    )?;
    let mut result = Vec::new();
    for row in rows {
        let mut entry = row?;
        entry.tags = tags::tags_for_entry(conn, entry.id)?;
        result.push(entry);
    }
    Ok(result)
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
        customers::create(conn, NewCustomer { name: "ACME".into(), short_code: "ACME".into(), notes: "".into() }, &berlin())
            .unwrap()
            .id
    }

    fn new_entry(customer_id: i64, title: &str, performed_at_utc: &str, category: Category) -> NewEntry {
        NewEntry {
            customer_id,
            system_id: None,
            title: title.into(),
            body_md: "Inhalt".into(),
            category,
            performed_at_utc: performed_at_utc.into(),
            performed_at_tz: "Europe/Berlin".into(),
            tag_names: vec!["exchange".into()],
        }
    }

    #[test]
    fn create_then_get_includes_tags() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(&conn, new_entry(customer_id, "Update", "2026-09-07T12:00:00.000Z", Category::Wartung), &berlin()).unwrap();
        assert_eq!(created.tags, vec!["exchange".to_string()]);
        assert_eq!(created.created_at_utc, created.updated_at_utc);
        assert_eq!(get(&conn, created.id).unwrap(), created);
    }

    #[test]
    fn update_replaces_tags_and_keeps_created_at() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(&conn, new_entry(customer_id, "Update", "2026-09-07T12:00:00.000Z", Category::Wartung), &berlin()).unwrap();

        let updated = update(
            &conn,
            created.id,
            UpdateEntry {
                system_id: None,
                title: "Update v2".into(),
                body_md: "Neuer Inhalt".into(),
                category: Category::Stoerung,
                performed_at_utc: "2026-09-06T09:00:00.000Z".into(),
                performed_at_tz: "Europe/Berlin".into(),
                tag_names: vec!["firewall".into()],
            },
            &berlin(),
        )
        .unwrap();

        assert_eq!(updated.title, "Update v2");
        assert_eq!(updated.category, Category::Stoerung);
        assert_eq!(updated.tags, vec!["firewall".to_string()]);
        assert_eq!(updated.created_at_utc, created.created_at_utc);
    }

    #[test]
    fn list_orders_by_performed_at_descending() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let older = create(&conn, new_entry(customer_id, "Älter", "2026-09-01T10:00:00.000Z", Category::Wartung), &berlin()).unwrap();
        let newer = create(&conn, new_entry(customer_id, "Neuer", "2026-09-07T10:00:00.000Z", Category::Wartung), &berlin()).unwrap();

        let result = list(&conn, &EntryFilter::default()).unwrap();
        assert_eq!(result.iter().map(|e| e.id).collect::<Vec<_>>(), vec![newer.id, older.id]);
    }

    #[test]
    fn list_filters_by_category_and_date_range() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        create(&conn, new_entry(customer_id, "Wartung", "2026-09-05T10:00:00.000Z", Category::Wartung), &berlin()).unwrap();
        let stoerung = create(&conn, new_entry(customer_id, "Störung", "2026-09-06T10:00:00.000Z", Category::Stoerung), &berlin()).unwrap();

        let by_category = list(&conn, &EntryFilter { category: Some(Category::Stoerung), ..Default::default() }).unwrap();
        assert_eq!(by_category.iter().map(|e| e.id).collect::<Vec<_>>(), vec![stoerung.id]);

        let by_range = list(
            &conn,
            &EntryFilter { from_utc: Some("2026-09-06T00:00:00.000Z".into()), to_utc: Some("2026-09-07T00:00:00.000Z".into()), ..Default::default() },
        )
        .unwrap();
        assert_eq!(by_range.iter().map(|e| e.id).collect::<Vec<_>>(), vec![stoerung.id]);
    }

    #[test]
    fn list_filters_by_tag() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let with_tag = create(&conn, new_entry(customer_id, "Mit Tag", "2026-09-05T10:00:00.000Z", Category::Wartung), &berlin()).unwrap();
        create(
            &conn,
            NewEntry { tag_names: vec!["anderes".into()], ..new_entry(customer_id, "Ohne passendes Tag", "2026-09-06T10:00:00.000Z", Category::Wartung) },
            &berlin(),
        )
        .unwrap();

        let result = list(&conn, &EntryFilter { tag: Some("exchange".into()), ..Default::default() }).unwrap();
        assert_eq!(result.iter().map(|e| e.id).collect::<Vec<_>>(), vec![with_tag.id]);
    }
}
