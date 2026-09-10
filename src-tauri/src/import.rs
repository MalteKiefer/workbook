//! Pure CSV parsing for the customer/system bulk-import commands
//! (`commands::customers::import_customers_from_csv`,
//! `commands::systems::import_systems_from_csv`). Header-based column
//! matching (case-insensitive, with common German/English aliases) so a
//! user's existing spreadsheet export doesn't need to be renamed to match
//! this app's exact field names first, and per-row `Result` collection so
//! one malformed row doesn't abort the rest of the import.
//!
//! Every parsed row -- `Ok` or `Err` -- is tagged with its true 1-based CSV
//! line number (header is row 1, so data starts at row 2), NOT its
//! position in the result `Vec`. Blank rows are dropped entirely rather
//! than tagged, so if row numbers were derived from the `Vec`'s own index
//! instead, every row number after the first blank line would silently be
//! wrong; the caller (`commands::customers`/`commands::systems`) reports
//! `row` straight from here for exactly that reason.

use crate::db::customers::NewCustomer;
use crate::db::systems::NewSystem;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ImportRowError {
    pub row: usize,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ImportSummary {
    pub imported: usize,
    pub errors: Vec<ImportRowError>,
}

/// One parsed row per successfully read CSV record, each tagged with its
/// true 1-based line number (see the module doc above) and either the
/// value built from it or why it was rejected.
type ParsedRows<T> = Result<Vec<(usize, Result<T, String>)>, String>;

fn find_column(headers: &[String], aliases: &[&str]) -> Option<usize> {
    headers
        .iter()
        .position(|h| aliases.contains(&h.trim().to_lowercase().as_str()))
}

fn read_headers(content: &str) -> Result<(csv::Reader<&[u8]>, Vec<String>), String> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(content.as_bytes());
    let headers = reader
        .headers()
        .map_err(|e| format!("CSV-Kopfzeile konnte nicht gelesen werden: {e}"))?
        .iter()
        .map(|s| s.to_string())
        .collect();
    Ok((reader, headers))
}

fn field(record: &csv::StringRecord, idx: Option<usize>) -> String {
    idx.and_then(|i| record.get(i))
        .unwrap_or("")
        .trim()
        .to_string()
}

pub fn parse_customers_csv(content: &str) -> ParsedRows<NewCustomer> {
    let (mut reader, headers) = read_headers(content)?;

    let name_idx = find_column(&headers, &["name", "kunde", "firma"])
        .ok_or_else(|| "Spalte \"name\" fehlt im CSV-Kopf.".to_string())?;
    let short_code_idx = find_column(
        &headers,
        &["short_code", "shortcode", "kürzel", "kuerzel", "code"],
    )
    .ok_or_else(|| "Spalte \"short_code\" (oder \"Kürzel\") fehlt im CSV-Kopf.".to_string())?;
    let notes_idx = find_column(&headers, &["notizen", "notes", "notiz"]);

    let mut results = Vec::new();
    for (i, record) in reader.records().enumerate() {
        let row = i + 2;
        let record = match record {
            Ok(r) => r,
            Err(e) => {
                results.push((row, Err(format!("Zeile konnte nicht gelesen werden: {e}"))));
                continue;
            }
        };
        if record.iter().all(|f| f.trim().is_empty()) {
            continue;
        }

        let name = field(&record, Some(name_idx));
        let short_code = field(&record, Some(short_code_idx));
        let notes = field(&record, notes_idx);

        if name.is_empty() {
            results.push((row, Err("Name fehlt.".to_string())));
            continue;
        }
        if short_code.is_empty() {
            results.push((row, Err("Kürzel (short_code) fehlt.".to_string())));
            continue;
        }

        results.push((
            row,
            Ok(NewCustomer {
                name,
                short_code,
                notes,
            }),
        ));
    }
    Ok(results)
}

pub fn parse_systems_csv(content: &str, customer_id: i64) -> ParsedRows<NewSystem> {
    let (mut reader, headers) = read_headers(content)?;

    let name_idx = find_column(
        &headers,
        &["name", "gerät", "geraet", "gerätename", "geraetename"],
    )
    .ok_or_else(|| "Spalte \"name\" fehlt im CSV-Kopf.".to_string())?;
    let ip_idx = find_column(&headers, &["ip", "ip-adresse", "ip_address", "ipaddress"]);
    let hostname_idx = find_column(&headers, &["hostname", "host"]);
    let type_idx = find_column(&headers, &["typ", "type", "system_type", "art"]);
    let notes_idx = find_column(&headers, &["notizen", "notes", "notiz"]);

    let mut results = Vec::new();
    for (i, record) in reader.records().enumerate() {
        let row = i + 2;
        let record = match record {
            Ok(r) => r,
            Err(e) => {
                results.push((row, Err(format!("Zeile konnte nicht gelesen werden: {e}"))));
                continue;
            }
        };
        if record.iter().all(|f| f.trim().is_empty()) {
            continue;
        }

        let name = field(&record, Some(name_idx));
        if name.is_empty() {
            results.push((row, Err("Name fehlt.".to_string())));
            continue;
        }

        results.push((
            row,
            Ok(NewSystem {
                customer_id,
                name,
                system_type: field(&record, type_idx),
                hostname: field(&record, hostname_idx),
                ip_address: field(&record, ip_idx),
                notes: field(&record, notes_idx),
            }),
        ));
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_customers_with_exact_header_names() {
        let csv = "name,short_code,notizen\nACME GmbH,ACME,Hauptkunde\n";
        let results = parse_customers_csv(csv).unwrap();
        assert_eq!(results.len(), 1);
        let (row, customer) = &results[0];
        assert_eq!(*row, 2);
        let customer = customer.as_ref().unwrap();
        assert_eq!(customer.name, "ACME GmbH");
        assert_eq!(customer.short_code, "ACME");
        assert_eq!(customer.notes, "Hauptkunde");
    }

    #[test]
    fn parses_customers_with_german_aliased_header_names_case_insensitively() {
        let csv = "Name,Kürzel,Notes\nBeispiel AG,BSP,\n";
        let results = parse_customers_csv(csv).unwrap();
        assert_eq!(results.len(), 1);
        let customer = results[0].1.as_ref().unwrap();
        assert_eq!(customer.name, "Beispiel AG");
        assert_eq!(customer.short_code, "BSP");
        assert_eq!(customer.notes, "");
    }

    #[test]
    fn missing_name_column_in_header_fails_the_whole_parse() {
        let csv = "short_code,notizen\nACME,x\n";
        let err = parse_customers_csv(csv).unwrap_err();
        assert!(err.contains("name"));
    }

    #[test]
    fn missing_short_code_column_in_header_fails_the_whole_parse() {
        let csv = "name,notizen\nACME GmbH,x\n";
        let err = parse_customers_csv(csv).unwrap_err();
        assert!(err.contains("short_code"));
    }

    #[test]
    fn customer_row_missing_name_is_a_row_error_not_an_abort() {
        let csv = "name,short_code\n,ACME\nBeispiel AG,BSP\n";
        let results = parse_customers_csv(csv).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, 2);
        assert!(results[0].1.is_err());
        assert!(results[1].1.is_ok());
    }

    #[test]
    fn customer_row_missing_short_code_is_a_row_error() {
        let csv = "name,short_code\nACME GmbH,\n";
        let results = parse_customers_csv(csv).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].1.as_ref().unwrap_err().contains("Kürzel"));
    }

    #[test]
    fn blank_customer_rows_are_silently_skipped() {
        let csv = "name,short_code\nACME GmbH,ACME\n,\n\n";
        let results = parse_customers_csv(csv).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn customer_notes_column_is_optional() {
        let csv = "name,short_code\nACME GmbH,ACME\n";
        let results = parse_customers_csv(csv).unwrap();
        assert_eq!(results[0].1.as_ref().unwrap().notes, "");
    }

    #[test]
    fn row_numbers_after_a_blank_line_still_match_the_true_csv_line() {
        // A blank line at CSV row 2 is skipped without being tagged; the
        // next data row is CSV row 3, not "row 2" as it would be if row
        // numbers were derived from the result Vec's own index instead.
        let csv = "name,short_code\n,\nACME GmbH,ACME\n";
        let results = parse_customers_csv(csv).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 3);
    }

    #[test]
    fn parses_systems_with_exact_header_names() {
        let csv = "name,ip,notizen\nweb-01,10.0.0.5,Produktionsserver\n";
        let results = parse_systems_csv(csv, 7).unwrap();
        assert_eq!(results.len(), 1);
        let system = results[0].1.as_ref().unwrap();
        assert_eq!(system.customer_id, 7);
        assert_eq!(system.name, "web-01");
        assert_eq!(system.ip_address, "10.0.0.5");
        assert_eq!(system.notes, "Produktionsserver");
    }

    #[test]
    fn parses_systems_with_all_optional_columns() {
        let csv = "name,ip,hostname,typ,notizen\nweb-01,10.0.0.5,web01.local,Server,x\n";
        let results = parse_systems_csv(csv, 7).unwrap();
        let system = results[0].1.as_ref().unwrap();
        assert_eq!(system.hostname, "web01.local");
        assert_eq!(system.system_type, "Server");
    }

    #[test]
    fn parses_systems_with_only_the_required_name_column() {
        let csv = "name\nweb-01\n";
        let results = parse_systems_csv(csv, 7).unwrap();
        assert_eq!(results.len(), 1);
        let system = results[0].1.as_ref().unwrap();
        assert_eq!(system.ip_address, "");
        assert_eq!(system.hostname, "");
        assert_eq!(system.system_type, "");
        assert_eq!(system.notes, "");
    }

    #[test]
    fn missing_name_column_in_systems_header_fails_the_whole_parse() {
        let csv = "ip,notizen\n10.0.0.5,x\n";
        let err = parse_systems_csv(csv, 7).unwrap_err();
        assert!(err.contains("name"));
    }

    #[test]
    fn system_row_missing_name_is_a_row_error_not_an_abort() {
        let csv = "name,ip\n,10.0.0.5\nweb-02,10.0.0.6\n";
        let results = parse_systems_csv(csv, 7).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, 2);
        assert!(results[0].1.is_err());
        assert!(results[1].1.is_ok());
    }

    #[test]
    fn blank_system_rows_are_silently_skipped() {
        let csv = "name,ip\nweb-01,10.0.0.5\n,\n";
        let results = parse_systems_csv(csv, 7).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn fields_are_trimmed_of_surrounding_whitespace() {
        let csv = "name,ip\n  web-01  ,  10.0.0.5  \n";
        let results = parse_systems_csv(csv, 7).unwrap();
        let system = results[0].1.as_ref().unwrap();
        assert_eq!(system.name, "web-01");
        assert_eq!(system.ip_address, "10.0.0.5");
    }

    #[test]
    fn quoted_field_with_embedded_comma_is_parsed_as_one_field() {
        let csv = "name,ip,notizen\nweb-01,10.0.0.5,\"Raum 3, Schrank 2\"\n";
        let results = parse_systems_csv(csv, 7).unwrap();
        assert_eq!(results[0].1.as_ref().unwrap().notes, "Raum 3, Schrank 2");
    }
}
