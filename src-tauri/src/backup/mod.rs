pub mod crypto;

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

use crate::config::AutoBackupFrequency;
use crate::error::AppError;

/// Name of the SQLite snapshot entry inside a backup zip, and of the live
/// database file under `data_dir`.
pub const DB_FILE_NAME: &str = "wartungsdoku.db";

/// Name of the app config file, both under `data_dir` (live) and as a zip
/// entry inside a backup. Holds app settings plus non-secret RMM plugin
/// connection metadata (`Config::ninja_connections`/`ninja_org_mappings`) --
/// see `crate::config::Config`. Plugin *credentials* are never in here, see
/// the data_dir-preservation comment in `apply_pending_restore_if_present`
/// for why this file needs special handling on restore that
/// `wartungsdoku.db`/`attachments/` don't.
const CONFIG_FILE_NAME: &str = "config.toml";

/// Directory (relative to `data_dir`) holding cached last-synced device
/// lists per RMM plugin connection, e.g.
/// `plugin-cache/ninja-<connection_id>.json` -- see
/// `commands::plugins::plugin_cache_dir`/`write_ninja_cache` for the writing
/// side of this same convention.
const PLUGIN_CACHE_DIR_NAME: &str = "plugin-cache";

/// Staging directory (relative to `data_dir`) a restore is extracted into
/// before it is applied at the next startup. Its presence (specifically
/// `<data_dir>/.pending_restore/wartungsdoku.db`) is itself the marker that a
/// restore is pending -- no separate flag file needed.
const PENDING_RESTORE_DIR_NAME: &str = ".pending_restore";

/// Scratch directory (relative to `data_dir`) for the temporary `VACUUM INTO`
/// snapshot produced while building a backup.
const SCRATCH_DIR_NAME: &str = ".tmp";

/// Whether an automatic backup is due, given the configured `frequency`, the
/// UTC timestamp of the last successful automatic backup (`None` if there
/// never was one), and the current time. Pure function so the scheduler's
/// actual "when do we wake up and check" logic can stay a thin, untested
/// wrapper around this -- all the interesting behavior lives here where it's
/// cheap to test.
///
/// A `last_run_utc` that fails to parse (e.g. hand-edited or from a future
/// format change) is treated the same as `None` -- due now -- rather than
/// silently never running again.
pub fn is_auto_backup_due(
    frequency: AutoBackupFrequency,
    last_run_utc: Option<&str>,
    now: DateTime<Utc>,
) -> bool {
    let Some(last_run_utc) = last_run_utc else {
        return true;
    };
    let Ok(last_run) = DateTime::parse_from_rfc3339(last_run_utc) else {
        return true;
    };
    let interval = match frequency {
        AutoBackupFrequency::Daily => chrono::Duration::days(1),
        AutoBackupFrequency::Weekly => chrono::Duration::days(7),
        AutoBackupFrequency::Monthly => chrono::Duration::days(30),
    };
    now.signed_duration_since(last_run) >= interval
}

/// Produces one consistent backup zip at `dest_path` containing a snapshot of
/// the live database, `data_dir/config.toml` (app settings and non-secret
/// plugin connection metadata -- never plugin credentials, those live only in
/// the OS keyring, see `docs/PLUGIN_ARCHITECTURE.md`), everything under
/// `data_dir/attachments/`, and everything under `data_dir/plugin-cache/`.
///
/// The live database is WAL-mode and reached through a pooled connection with
/// other connections potentially open at the same time, so this deliberately
/// does *not* copy `wartungsdoku.db`/`-wal`/`-shm` directly -- that could copy
/// a torn, inconsistent state, and the raw files may be locked on Windows
/// anyway. `VACUUM INTO` instead asks SQLite itself for a single, complete,
/// consistent snapshot file, which is then what gets zipped.
pub fn create_backup(conn: &Connection, data_dir: &Path, dest_path: &Path) -> Result<(), AppError> {
    let scratch_dir = data_dir.join(SCRATCH_DIR_NAME);
    std::fs::create_dir_all(&scratch_dir)?;
    let snapshot_path = scratch_dir.join(format!("backup-snapshot-{}.db", std::process::id()));
    // VACUUM INTO refuses to overwrite an existing file -- clear out any
    // leftover snapshot from a previous run that crashed before cleanup.
    let _ = std::fs::remove_file(&snapshot_path);

    let snapshot_path_str = snapshot_path.to_str().ok_or_else(|| {
        AppError::Backup("Datenverzeichnis-Pfad enthält ungültige Zeichen".to_string())
    })?;

    let result = conn
        .execute("VACUUM INTO ?1", [snapshot_path_str])
        .map_err(AppError::from)
        .and_then(|_| write_backup_zip(&snapshot_path, data_dir, dest_path));

    let _ = std::fs::remove_file(&snapshot_path);
    result
}

/// Like `create_backup`, but the finished zip is encrypted with `passphrase`
/// (see `crypto::encrypt_file`) before landing at `dest_path`. The
/// unencrypted zip only ever exists transiently under `data_dir/.tmp/`.
pub fn create_backup_encrypted(
    conn: &Connection,
    data_dir: &Path,
    dest_path: &Path,
    passphrase: &str,
) -> Result<(), AppError> {
    let scratch_dir = data_dir.join(SCRATCH_DIR_NAME);
    std::fs::create_dir_all(&scratch_dir)?;
    let tmp_zip_path = scratch_dir.join(format!("backup-plain-{}.zip", std::process::id()));
    let _ = std::fs::remove_file(&tmp_zip_path);

    let result = create_backup(conn, data_dir, &tmp_zip_path)
        .and_then(|_| crypto::encrypt_file(&tmp_zip_path, dest_path, passphrase));

    let _ = std::fs::remove_file(&tmp_zip_path);
    result
}

/// Like `stage_restore`, but `source_path` is an encrypted backup (see
/// `crypto::encrypt_file`) rather than a plain zip -- decrypted to a
/// transient temp file under `data_dir/.tmp/` first, then handled exactly
/// like a plain-zip restore.
pub fn stage_restore_encrypted(
    data_dir: &Path,
    source_path: &Path,
    passphrase: &str,
) -> Result<(), AppError> {
    let scratch_dir = data_dir.join(SCRATCH_DIR_NAME);
    std::fs::create_dir_all(&scratch_dir)?;
    let tmp_zip_path = scratch_dir.join(format!("restore-plain-{}.zip", std::process::id()));
    let _ = std::fs::remove_file(&tmp_zip_path);

    let result = crypto::decrypt_file(source_path, &tmp_zip_path, passphrase)
        .and_then(|_| stage_restore(data_dir, &tmp_zip_path));

    let _ = std::fs::remove_file(&tmp_zip_path);
    result
}

fn write_backup_zip(
    snapshot_path: &Path,
    data_dir: &Path,
    dest_path: &Path,
) -> Result<(), AppError> {
    let file = File::create(dest_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    zip.start_file(DB_FILE_NAME, options)?;
    copy_file_into(snapshot_path, &mut zip)?;

    // config.toml always exists after the very first run (see `lib.rs::run`,
    // which saves it unconditionally at startup), but a genuinely fresh
    // install without one yet is not a backup failure -- just nothing to add
    // here.
    let config_path = data_dir.join(CONFIG_FILE_NAME);
    if config_path.exists() {
        zip.start_file(CONFIG_FILE_NAME, options)?;
        copy_file_into(&config_path, &mut zip)?;
    }

    add_directory_to_zip(&mut zip, data_dir, "attachments", options)?;
    add_directory_to_zip(&mut zip, data_dir, PLUGIN_CACHE_DIR_NAME, options)?;

    zip.finish()?;
    Ok(())
}

/// Recursively adds every file under `data_dir/<dir_name>/` to `zip`,
/// preserving its path relative to `data_dir` (e.g. `attachments/ab/cd.png`,
/// `plugin-cache/ninja-acme-1.json`) as the zip entry name. A no-op if the
/// directory doesn't exist (e.g. no plugin has ever synced yet, or a fresh
/// install with no attachments).
fn add_directory_to_zip<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    data_dir: &Path,
    dir_name: &str,
    options: SimpleFileOptions,
) -> Result<(), AppError> {
    let dir = data_dir.join(dir_name);
    if !dir.exists() {
        return Ok(());
    }
    for entry_path in walk_files(&dir)? {
        let relative = entry_path.strip_prefix(data_dir).map_err(|_| {
            AppError::Backup(format!(
                "Pfad in '{dir_name}' liegt außerhalb des Datenverzeichnisses"
            ))
        })?;
        let zip_entry_name = relative.to_string_lossy().replace('\\', "/");
        zip.start_file(zip_entry_name, options)?;
        copy_file_into(&entry_path, zip)?;
    }
    Ok(())
}

fn copy_file_into<W: Write + std::io::Seek>(
    path: &Path,
    zip: &mut zip::ZipWriter<W>,
) -> Result<(), AppError> {
    let mut buf = Vec::new();
    File::open(path)?.read_to_end(&mut buf)?;
    zip.write_all(&buf)?;
    Ok(())
}

/// Recursively lists every file (not directory) under `root`.
fn walk_files(root: &Path) -> Result<Vec<PathBuf>, AppError> {
    let mut result = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                stack.push(path);
            } else {
                result.push(path);
            }
        }
    }
    Ok(result)
}

/// Extracts `source_path` (a backup zip) into `data_dir/.pending_restore/`,
/// validating along the way, without touching any of the live files. The
/// actual swap happens later, at the next startup, via
/// `apply_pending_restore_if_present` -- see that function's doc comment for
/// why.
///
/// `extract_zip` below extracts *every* entry in the zip generically (it
/// doesn't special-case file names), so a `config.toml` entry and any
/// `plugin-cache/**` entries are staged right alongside the db file and
/// `attachments/**` with no extra code needed here. An older backup made
/// before this function's caller started including those is simply a zip
/// without those entries -- staging silently ends up without them too, which
/// `apply_pending_restore_if_present` treats as "nothing to do for those
/// two", not an error.
pub fn stage_restore(data_dir: &Path, source_path: &Path) -> Result<(), AppError> {
    let staging_dir = data_dir.join(PENDING_RESTORE_DIR_NAME);
    // Start from a clean slate -- a previous aborted/failed restore attempt
    // shouldn't leave partial files around to confuse this one.
    if staging_dir.exists() {
        std::fs::remove_dir_all(&staging_dir)?;
    }
    std::fs::create_dir_all(&staging_dir)?;

    if let Err(e) = extract_zip(source_path, &staging_dir) {
        let _ = std::fs::remove_dir_all(&staging_dir);
        return Err(e);
    }

    let staged_db_path = staging_dir.join(DB_FILE_NAME);
    if !staged_db_path.exists() {
        let _ = std::fs::remove_dir_all(&staging_dir);
        return Err(AppError::Backup(format!(
            "Die Zip-Datei enthält keine '{DB_FILE_NAME}' -- falsche Datei ausgewählt?"
        )));
    }

    if let Err(e) = validate_sqlite_file(&staged_db_path) {
        let _ = std::fs::remove_dir_all(&staging_dir);
        return Err(e);
    }

    Ok(())
}

/// Rejects the staged file early (with a clear error) if it isn't actually a
/// usable SQLite database -- the main user-facing validation point for "picked
/// the wrong file" or a corrupted backup.
fn validate_sqlite_file(path: &Path) -> Result<(), AppError> {
    let conn = Connection::open(path)
        .map_err(|e| AppError::Backup(format!("Datenbank im Backup ist ungültig: {e}")))?;
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
        row.get::<_, i64>(0)
    })
    .map_err(|e| AppError::Backup(format!("Datenbank im Backup ist ungültig: {e}")))?;
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|e| {
            AppError::Backup(format!(
                "Integritätsprüfung der Backup-Datenbank fehlgeschlagen: {e}"
            ))
        })?;
    if integrity != "ok" {
        return Err(AppError::Backup(format!(
            "Datenbank im Backup ist beschädigt: {integrity}"
        )));
    }
    Ok(())
}

/// Extracts every entry of the zip at `source_path` into `target_dir`.
/// `enclosed_name()` (rather than `mangled_name()`/the raw stored name) is
/// what keeps a maliciously or accidentally crafted zip (e.g. `../../evil`)
/// from writing outside `target_dir`.
fn extract_zip(source_path: &Path, target_dir: &Path) -> Result<(), AppError> {
    let file = File::open(source_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        let out_path = target_dir.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out_file = File::create(&out_path)?;
            std::io::copy(&mut entry, &mut out_file)?;
        }
    }
    Ok(())
}

/// Appends `suffix` to `path`'s filename without going through a lossy
/// UTF-8 round-trip (important for sidecar names like `wartungsdoku.db-wal`
/// and `.bak-<timestamp>` suffixes on arbitrary user data-dir paths).
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut os_string = path.as_os_str().to_owned();
    os_string.push(suffix);
    PathBuf::from(os_string)
}

/// If a restore was staged by `stage_restore` (i.e.
/// `data_dir/.pending_restore/wartungsdoku.db` exists), applies it now by
/// moving the staged files into their live locations.
///
/// MUST be called before `build_pool` opens any connection to the live
/// database -- that's the entire point of the staged-restore design. Once no
/// pool exists yet, the live db/-wal/-shm files and the attachments tree are
/// free to be replaced outright, which is safe on Windows (no open-file
/// locks) and avoids ever touching a live, in-use SQLite file.
///
/// As a safety net the current live files are renamed aside to timestamped
/// `.bak-<timestamp>` siblings rather than deleted, in case the restored
/// backup turns out to be the wrong one. `config.toml` and `plugin-cache/`
/// get the exact same treatment, when the staged restore includes them (an
/// older backup made before this function's caller started including those
/// simply won't have them staged -- skipped, not an error).
///
/// `config.toml` gets one more step after being moved into place: it records
/// `data_dir` itself, frozen at backup time on whatever machine made the
/// backup. Restoring it verbatim would overwrite the live, correct data_dir
/// (the `data_dir` parameter this function already has in scope) with that
/// possibly-foreign path, corrupting the app's own sense of where its data
/// lives. So immediately after the staged file lands at its live location,
/// it's re-opened and only its `data_dir` field is overwritten with the
/// current machine's real, live `data_dir` -- every other restored setting
/// (autostart, hotkeys, plugin connections, ...) is left exactly as the
/// backup had it.
pub fn apply_pending_restore_if_present(data_dir: &Path) -> Result<(), AppError> {
    let staging_dir = data_dir.join(PENDING_RESTORE_DIR_NAME);
    let staged_db_path = staging_dir.join(DB_FILE_NAME);
    if !staged_db_path.exists() {
        return Ok(());
    }

    eprintln!(
        "Ausstehende Wiederherstellung gefunden, wende sie an: {}",
        staging_dir.display()
    );
    let timestamp = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string();
    let bak_suffix = format!(".bak-{timestamp}");

    let live_db_path = data_dir.join(DB_FILE_NAME);
    backup_aside(&live_db_path, &bak_suffix)?;
    backup_aside(&with_suffix(&live_db_path, "-wal"), &bak_suffix)?;
    backup_aside(&with_suffix(&live_db_path, "-shm"), &bak_suffix)?;

    let live_attachments_dir = data_dir.join("attachments");
    backup_aside(&live_attachments_dir, &bak_suffix)?;

    std::fs::rename(&staged_db_path, &live_db_path)?;
    let staged_attachments_dir = staging_dir.join("attachments");
    if staged_attachments_dir.exists() {
        std::fs::rename(&staged_attachments_dir, &live_attachments_dir)?;
    }

    // config.toml -- see this function's doc comment for the data_dir
    // preservation fixup below.
    let staged_config_path = staging_dir.join(CONFIG_FILE_NAME);
    if staged_config_path.exists() {
        let live_config_path = data_dir.join(CONFIG_FILE_NAME);
        backup_aside(&live_config_path, &bak_suffix)?;
        std::fs::rename(&staged_config_path, &live_config_path)?;

        // Fixed up as the very last step for this file, after it's already
        // sitting at its live location: every setting the backup recorded is
        // kept, except data_dir, which always reflects reality on the
        // machine doing the restore.
        let mut restored_config = crate::config::Config::load_or_default(&live_config_path)?;
        restored_config.data_dir = data_dir.to_path_buf();
        restored_config.save(&live_config_path)?;
    }

    // plugin-cache/ -- same rename-current-aside-then-move-staged-in pattern
    // as attachments/ above.
    let staged_plugin_cache_dir = staging_dir.join(PLUGIN_CACHE_DIR_NAME);
    if staged_plugin_cache_dir.exists() {
        let live_plugin_cache_dir = data_dir.join(PLUGIN_CACHE_DIR_NAME);
        backup_aside(&live_plugin_cache_dir, &bak_suffix)?;
        std::fs::rename(&staged_plugin_cache_dir, &live_plugin_cache_dir)?;
    }

    std::fs::remove_dir_all(&staging_dir)?;
    eprintln!("Wiederherstellung angewendet. Vorherige Daten gesichert mit Suffix '{bak_suffix}'.");
    Ok(())
}

/// Renames `path` aside to `<path><bak_suffix>` if it exists; a no-op
/// otherwise (e.g. a fresh install has no `-wal`/`-shm` sidecars yet).
fn backup_aside(path: &Path, bak_suffix: &str) -> Result<(), AppError> {
    if !path.exists() {
        return Ok(());
    }
    std::fs::rename(path, with_suffix(path, bak_suffix))?;
    Ok(())
}

#[cfg(test)]
// Test fixtures mutate a couple of fields on a `Config::default()` binding;
// that reads clearer here than a full struct literal with
// `..Default::default()` and would only get more brittle as fields are added.
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn seed_data_dir(data_dir: &Path) -> Connection {
        let conn = crate::db::test_support::migrated_connection();
        let tz: chrono_tz::Tz = "Europe/Berlin".parse().unwrap();
        let customer_id = crate::db::customers::create(
            &conn,
            crate::db::customers::NewCustomer {
                name: "ACME".into(),
                short_code: "ACME".into(),
                notes: "".into(),
            },
            &tz,
        )
        .unwrap()
        .id;
        crate::db::entries::create(
            &conn,
            data_dir,
            crate::db::entries::NewEntry {
                customer_id,
                system_id: None,
                title: "Titel".into(),
                body_md: "".into(),
                category: crate::db::entries::Category::Wartung,
                performed_at_utc: "2026-09-07T12:00:00.000Z".into(),
                performed_at_tz: "Europe/Berlin".into(),
                tag_names: vec![],
                pending_attachments: vec![],
            },
            &tz,
        )
        .unwrap();

        // A real attachment file under data_dir/attachments/.. , mirroring
        // attachments::store's content-addressed layout.
        let saved = crate::attachments::store::save_content_addressed(
            data_dir,
            b"attachment bytes",
            "shot.png",
        )
        .unwrap();
        assert!(data_dir.join(&saved.relative_path).exists());

        conn
    }

    /// Writes a `config.toml` at `data_dir/config.toml` whose `data_dir`
    /// field is `recorded_data_dir` -- deliberately a separate parameter from
    /// `data_dir` so tests can simulate a backup made on a different machine
    /// (or the data dir having moved since), without needing to touch
    /// `src-tauri/src/config.rs` itself.
    fn seed_config_toml(data_dir: &Path, recorded_data_dir: &Path, autostart_enabled: bool) {
        let mut config = crate::config::Config::default();
        config.data_dir = recorded_data_dir.to_path_buf();
        config.autostart_enabled = autostart_enabled;
        config.save(&data_dir.join(CONFIG_FILE_NAME)).unwrap();
    }

    /// Seeds `data_dir/plugin-cache/` with one fixture cache file, mirroring
    /// the real `ninja-<connection_id>.json` naming convention from
    /// `commands::plugins::write_ninja_cache`.
    fn seed_plugin_cache(data_dir: &Path) {
        let dir = data_dir.join(PLUGIN_CACHE_DIR_NAME);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("ninja-acme-1.json"),
            br#"{"synced_at_utc":"2026-09-07T12:00:00.000Z","groups":[]}"#,
        )
        .unwrap();
    }

    fn read_zip_entry_names(zip_path: &Path) -> Vec<String> {
        let file = File::open(zip_path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect()
    }

    #[test]
    fn create_backup_zip_contains_db_and_attachments() {
        let dir = tempdir().unwrap();
        let conn = seed_data_dir(dir.path());
        let dest_zip = dir.path().join("backup.zip");

        create_backup(&conn, dir.path(), &dest_zip).unwrap();

        assert!(dest_zip.exists());
        let names = read_zip_entry_names(&dest_zip);
        assert!(names.contains(&DB_FILE_NAME.to_string()));
        assert!(names
            .iter()
            .any(|n| n.starts_with("attachments/") && n.ends_with(".png")));

        // Scratch snapshot cleaned up, no leftovers.
        let scratch_dir = dir.path().join(SCRATCH_DIR_NAME);
        if scratch_dir.exists() {
            assert_eq!(std::fs::read_dir(&scratch_dir).unwrap().count(), 0);
        }
    }

    #[test]
    fn create_backup_zip_contains_config_toml_and_plugin_cache() {
        let dir = tempdir().unwrap();
        let conn = seed_data_dir(dir.path());
        seed_config_toml(dir.path(), dir.path(), true);
        seed_plugin_cache(dir.path());
        let dest_zip = dir.path().join("backup.zip");

        create_backup(&conn, dir.path(), &dest_zip).unwrap();

        let names = read_zip_entry_names(&dest_zip);
        assert!(names.contains(&CONFIG_FILE_NAME.to_string()));
        assert!(names
            .iter()
            .any(|n| n.starts_with("plugin-cache/") && n.ends_with(".json")));
    }

    #[test]
    fn create_backup_succeeds_without_config_toml_present() {
        let dir = tempdir().unwrap();
        let conn = seed_data_dir(dir.path());
        // No config.toml written -- simulates a fresh install before any
        // setting has ever triggered a save. This must not fail the backup.
        let dest_zip = dir.path().join("backup.zip");

        create_backup(&conn, dir.path(), &dest_zip).unwrap();

        let names = read_zip_entry_names(&dest_zip);
        assert!(!names.contains(&CONFIG_FILE_NAME.to_string()));
    }

    #[test]
    fn create_backup_zip_db_entry_is_a_valid_database() {
        let dir = tempdir().unwrap();
        let conn = seed_data_dir(dir.path());
        let dest_zip = dir.path().join("backup.zip");
        create_backup(&conn, dir.path(), &dest_zip).unwrap();

        let extract_dir = tempdir().unwrap();
        extract_zip(&dest_zip, extract_dir.path()).unwrap();
        let restored_conn = Connection::open(extract_dir.path().join(DB_FILE_NAME)).unwrap();
        let count: i64 = restored_conn
            .query_row("SELECT count(*) FROM entries", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn stage_restore_rejects_zip_without_db_entry() {
        let dir = tempdir().unwrap();
        let zip_path = dir.path().join("bad.zip");
        let file = File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("not-a-db.txt", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"oops").unwrap();
        zip.finish().unwrap();

        let result = stage_restore(dir.path(), &zip_path);
        assert!(matches!(result, Err(AppError::Backup(_))));
        assert!(!dir.path().join(PENDING_RESTORE_DIR_NAME).exists());
    }

    #[test]
    fn stage_restore_rejects_zip_with_corrupt_db_entry() {
        let dir = tempdir().unwrap();
        let zip_path = dir.path().join("corrupt.zip");
        let file = File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(DB_FILE_NAME, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"this is not a sqlite file").unwrap();
        zip.finish().unwrap();

        let result = stage_restore(dir.path(), &zip_path);
        assert!(matches!(result, Err(AppError::Backup(_))));
        assert!(!dir.path().join(PENDING_RESTORE_DIR_NAME).exists());
    }

    #[test]
    fn stage_restore_stages_config_toml_and_plugin_cache_when_present() {
        let dir = tempdir().unwrap();
        let conn = seed_data_dir(dir.path());
        seed_config_toml(dir.path(), dir.path(), true);
        seed_plugin_cache(dir.path());
        let backup_zip = dir.path().join("backup.zip");
        create_backup(&conn, dir.path(), &backup_zip).unwrap();

        stage_restore(dir.path(), &backup_zip).unwrap();

        let staging_dir = dir.path().join(PENDING_RESTORE_DIR_NAME);
        assert!(staging_dir.join(CONFIG_FILE_NAME).exists());
        assert!(staging_dir
            .join(PLUGIN_CACHE_DIR_NAME)
            .join("ninja-acme-1.json")
            .exists());
    }

    #[test]
    fn stage_restore_skips_missing_config_toml_and_plugin_cache_gracefully() {
        let dir = tempdir().unwrap();
        let conn = seed_data_dir(dir.path());
        // No config.toml / plugin-cache seeded -- simulates an older-format
        // backup made before this feature existed. Staging (and later
        // applying) must still succeed for the db+attachments it does have.
        let backup_zip = dir.path().join("backup.zip");
        create_backup(&conn, dir.path(), &backup_zip).unwrap();

        stage_restore(dir.path(), &backup_zip).unwrap();

        let staging_dir = dir.path().join(PENDING_RESTORE_DIR_NAME);
        assert!(staging_dir.join(DB_FILE_NAME).exists());
        assert!(!staging_dir.join(CONFIG_FILE_NAME).exists());
        assert!(!staging_dir.join(PLUGIN_CACHE_DIR_NAME).exists());
    }

    #[test]
    fn stage_restore_then_apply_replaces_live_files_and_backs_up_previous_ones() {
        let dir = tempdir().unwrap();
        let conn = seed_data_dir(dir.path());
        let backup_zip = dir.path().join("backup.zip");
        create_backup(&conn, dir.path(), &backup_zip).unwrap();

        // Simulate an actual live install: a db file at the live path plus its
        // WAL sidecar, distinct content from what's inside the backup zip.
        let live_db_path = dir.path().join(DB_FILE_NAME);
        std::fs::write(&live_db_path, b"old live db bytes").unwrap();
        std::fs::write(with_suffix(&live_db_path, "-wal"), b"old wal bytes").unwrap();

        stage_restore(dir.path(), &backup_zip).unwrap();
        assert!(dir
            .path()
            .join(PENDING_RESTORE_DIR_NAME)
            .join(DB_FILE_NAME)
            .exists());

        apply_pending_restore_if_present(dir.path()).unwrap();

        // Staging directory gone, live db replaced with the restored one.
        assert!(!dir.path().join(PENDING_RESTORE_DIR_NAME).exists());
        assert!(live_db_path.exists());
        let restored_conn = Connection::open(&live_db_path).unwrap();
        let count: i64 = restored_conn
            .query_row("SELECT count(*) FROM entries", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);

        // Previous live db and its WAL sidecar were backed up aside, not deleted.
        let bak_db = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("wartungsdoku.db.bak-")
            });
        assert!(
            bak_db.is_some(),
            "vorherige Live-DB hätte als .bak-<timestamp> gesichert werden müssen"
        );

        // Restored attachment present at the live attachments path.
        let restored_attachment_present = walk_files(&dir.path().join("attachments"))
            .unwrap()
            .iter()
            .any(|p| p.extension().and_then(|e| e.to_str()) == Some("png"));
        assert!(restored_attachment_present);
    }

    #[test]
    fn apply_pending_restore_is_a_noop_when_nothing_staged() {
        let dir = tempdir().unwrap();
        apply_pending_restore_if_present(dir.path()).unwrap();
        assert!(!dir.path().join(PENDING_RESTORE_DIR_NAME).exists());
    }

    #[test]
    fn apply_pending_restore_preserves_live_data_dir_not_the_backed_up_one() {
        let dir = tempdir().unwrap();
        let conn = seed_data_dir(dir.path());
        // The config.toml inside the backup records a bogus data_dir, as if
        // this backup was made on a different machine (or the data dir moved
        // since). autostart_enabled is also flipped from the default, to
        // prove other settings DO come back from the backup, in contrast to
        // data_dir which must not.
        let bogus_data_dir = PathBuf::from("C:/some/other/machine/wartungsdoku-data");
        seed_config_toml(dir.path(), &bogus_data_dir, false);
        let backup_zip = dir.path().join("backup.zip");
        create_backup(&conn, dir.path(), &backup_zip).unwrap();

        stage_restore(dir.path(), &backup_zip).unwrap();
        apply_pending_restore_if_present(dir.path()).unwrap();

        let restored_config =
            crate::config::Config::load_or_default(&dir.path().join(CONFIG_FILE_NAME)).unwrap();
        // data_dir must reflect THIS machine's real, live data dir (the
        // tempdir this test runs in), never the bogus one recorded inside
        // the backup.
        assert_eq!(restored_config.data_dir, dir.path().to_path_buf());
        assert_ne!(restored_config.data_dir, bogus_data_dir);
        // Every other restored setting, though, legitimately comes from the backup.
        assert!(!restored_config.autostart_enabled);
    }

    #[test]
    fn apply_pending_restore_backs_up_previous_config_toml_aside() {
        let dir = tempdir().unwrap();
        let conn = seed_data_dir(dir.path());
        seed_config_toml(dir.path(), dir.path(), true);
        let backup_zip = dir.path().join("backup.zip");
        create_backup(&conn, dir.path(), &backup_zip).unwrap();

        // Different live config.toml content since the backup was made.
        seed_config_toml(dir.path(), dir.path(), false);

        stage_restore(dir.path(), &backup_zip).unwrap();
        apply_pending_restore_if_present(dir.path()).unwrap();

        let bak_config = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("config.toml.bak-")
            });
        assert!(
            bak_config.is_some(),
            "vorheriges Live-config.toml hätte als .bak-<timestamp> gesichert werden müssen"
        );
    }

    #[test]
    fn apply_pending_restore_replaces_plugin_cache_and_backs_up_previous_one() {
        let dir = tempdir().unwrap();
        let conn = seed_data_dir(dir.path());
        seed_plugin_cache(dir.path());
        let backup_zip = dir.path().join("backup.zip");
        create_backup(&conn, dir.path(), &backup_zip).unwrap();

        // Simulate different live plugin-cache content since the backup was
        // made (e.g. a plugin synced again).
        std::fs::remove_dir_all(dir.path().join(PLUGIN_CACHE_DIR_NAME)).unwrap();
        std::fs::create_dir_all(dir.path().join(PLUGIN_CACHE_DIR_NAME)).unwrap();
        std::fs::write(
            dir.path()
                .join(PLUGIN_CACHE_DIR_NAME)
                .join("ninja-other.json"),
            b"{}",
        )
        .unwrap();

        stage_restore(dir.path(), &backup_zip).unwrap();
        apply_pending_restore_if_present(dir.path()).unwrap();

        // Restored plugin-cache file present at the live path.
        assert!(dir
            .path()
            .join(PLUGIN_CACHE_DIR_NAME)
            .join("ninja-acme-1.json")
            .exists());

        // Previous live plugin-cache dir was backed up aside, not deleted.
        let bak_plugin_cache = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("plugin-cache.bak-")
            });
        assert!(
            bak_plugin_cache.is_some(),
            "vorheriges Live-plugin-cache hätte als .bak-<timestamp> gesichert werden müssen"
        );
    }

    #[test]
    fn create_backup_encrypted_then_stage_restore_encrypted_roundtrips() {
        let dir = tempdir().unwrap();
        let conn = seed_data_dir(dir.path());
        let dest = dir.path().join("backup.wdbk");

        create_backup_encrypted(&conn, dir.path(), &dest, "hunter2").unwrap();
        assert!(crypto::is_encrypted_file(&dest).unwrap());

        stage_restore_encrypted(dir.path(), &dest, "hunter2").unwrap();

        let staging_dir = dir.path().join(PENDING_RESTORE_DIR_NAME);
        assert!(staging_dir.join(DB_FILE_NAME).exists());
    }

    #[test]
    fn stage_restore_encrypted_with_wrong_passphrase_fails_and_leaves_no_staging_dir() {
        let dir = tempdir().unwrap();
        let conn = seed_data_dir(dir.path());
        let dest = dir.path().join("backup.wdbk");
        create_backup_encrypted(&conn, dir.path(), &dest, "correct").unwrap();

        let result = stage_restore_encrypted(dir.path(), &dest, "wrong");

        assert!(matches!(result, Err(AppError::Backup(_))));
        assert!(!dir.path().join(PENDING_RESTORE_DIR_NAME).exists());
    }

    #[test]
    fn is_auto_backup_due_when_never_run_before() {
        assert!(is_auto_backup_due(
            AutoBackupFrequency::Daily,
            None,
            Utc::now()
        ));
    }

    #[test]
    fn is_auto_backup_due_when_last_run_timestamp_is_unparseable() {
        assert!(is_auto_backup_due(
            AutoBackupFrequency::Daily,
            Some("not a timestamp"),
            Utc::now()
        ));
    }

    #[test]
    fn is_auto_backup_not_due_before_daily_interval_elapsed() {
        let now = Utc::now();
        let last_run = (now - chrono::Duration::hours(2)).to_rfc3339();
        assert!(!is_auto_backup_due(
            AutoBackupFrequency::Daily,
            Some(&last_run),
            now
        ));
    }

    #[test]
    fn is_auto_backup_due_after_daily_interval_elapsed() {
        let now = Utc::now();
        let last_run = (now - chrono::Duration::hours(25)).to_rfc3339();
        assert!(is_auto_backup_due(
            AutoBackupFrequency::Daily,
            Some(&last_run),
            now
        ));
    }

    #[test]
    fn is_auto_backup_due_respects_weekly_and_monthly_frequency() {
        let now = Utc::now();
        let three_days_ago = (now - chrono::Duration::days(3)).to_rfc3339();
        assert!(!is_auto_backup_due(
            AutoBackupFrequency::Weekly,
            Some(&three_days_ago),
            now
        ));
        assert!(is_auto_backup_due(
            AutoBackupFrequency::Daily,
            Some(&three_days_ago),
            now
        ));

        let forty_days_ago = (now - chrono::Duration::days(40)).to_rfc3339();
        assert!(is_auto_backup_due(
            AutoBackupFrequency::Monthly,
            Some(&forty_days_ago),
            now
        ));
    }

    #[test]
    fn apply_pending_restore_skips_config_and_plugin_cache_gracefully_when_not_staged() {
        let dir = tempdir().unwrap();
        let conn = seed_data_dir(dir.path());
        // Old-format backup: no config.toml, no plugin-cache. The db+attachments
        // restore must still fully succeed, with no error from the two missing
        // pieces.
        let backup_zip = dir.path().join("backup.zip");
        create_backup(&conn, dir.path(), &backup_zip).unwrap();

        stage_restore(dir.path(), &backup_zip).unwrap();
        apply_pending_restore_if_present(dir.path()).unwrap();

        assert!(dir.path().join(DB_FILE_NAME).exists());
        assert!(!dir.path().join(PENDING_RESTORE_DIR_NAME).exists());
    }
}
