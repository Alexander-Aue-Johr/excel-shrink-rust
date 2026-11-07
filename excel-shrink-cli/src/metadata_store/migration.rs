//! Copy legacy metadata into a verified database without changing its source.
use super::{read_snapshot, validate_key, Store};
use anyhow::{ensure, Context, Result};
use clap::{Parser, Subcommand};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(name = "excel-shrink metadata-store")]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Copy JSON metadata into a new database and verify every retained byte.
    Import { source: PathBuf, database: PathBuf },
    /// Verify that the database still matches all metadata in the source tree.
    Verify { source: PathBuf, database: PathBuf },
    /// Restore retained JSON metadata into a new, empty directory.
    Export {
        database: PathBuf,
        destination: PathBuf,
    },
    /// Read a JSON snapshot, resolving its SQLite reference when present.
    Read { path: PathBuf },
}

/// Expose migration separately from activation so old runners remain recoverable.
pub(crate) fn run(arguments: Vec<OsString>) -> Result<()> {
    let arguments =
        Arguments::parse_from(std::iter::once(OsString::from("metadata-store")).chain(arguments));
    match arguments.command {
        Command::Import { source, database } => {
            import(&source, &database)?;
            println!("Metadata imported and verified; original files are unchanged.");
        }
        Command::Verify { source, database } => {
            let store = Store::open_readonly(&database)?;
            let count = verify(&source, &store)?;
            println!("Verified {count} metadata documents against their source bytes.");
        }
        Command::Export {
            database,
            destination,
        } => {
            export(&Store::open_readonly(&database)?, &destination)?;
            println!("Metadata exported into {}.", destination.display());
        }
        Command::Read { path } => {
            use std::io::Write;
            std::io::stdout().write_all(&read_snapshot(&path, 32 * 1024 * 1024)?)?;
        }
    }
    Ok(())
}

fn metadata_files(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    ensure!(root.is_dir(), "Metadata source must be a directory");
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        if !matches!(
            entry.path().extension().and_then(|ext| ext.to_str()),
            Some("json" | "jsonl")
        ) {
            continue;
        }
        let key = entry
            .path()
            .strip_prefix(root)?
            .to_str()
            .context("Metadata path is not UTF-8")?
            .replace('\\', "/");
        validate_key(&key)?;
        files.push((key, entry.into_path()));
    }
    files.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

/// Publish a new database only after a second source pass proves byte equality.
fn import(source: &Path, database: &Path) -> Result<()> {
    ensure!(!database.exists(), "Migration destination already exists");
    let parent = database
        .parent()
        .context("Database has no parent directory")?;
    fs::create_dir_all(parent)?;
    let pending = parent.join(format!(".metadata-import-{}.sqlite", uuid::Uuid::new_v4()));
    let store = Store::open(&pending)?;
    store.connection.execute_batch("BEGIN IMMEDIATE")?;
    for (index, (key, path)) in metadata_files(source)?.into_iter().enumerate() {
        let bytes = fs::read(&path).with_context(|| format!("Read {}", path.display()))?;
        if bytes.len() <= 4096 {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                ensure!(
                    value.get("_metadata_store").is_none(),
                    "Export live SQLite snapshots before importing metadata: {key}"
                );
            }
        }
        store.put(&key, &bytes)?;
        if index % 100 == 99 {
            store.connection.execute_batch("COMMIT; BEGIN IMMEDIATE")?;
        }
    }
    store.connection.execute_batch("COMMIT")?;
    let count = verify(source, &store)?;
    store.connection.execute(
        "INSERT INTO metadata(key,value) VALUES ('verified_document_count',?1)",
        [count.to_string()],
    )?;
    let check: String = store
        .connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    ensure!(check == "ok", "SQLite integrity check failed: {check}");
    store
        .connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    drop(store);
    // A hard link publishes without replacing an independently created destination.
    fs::hard_link(&pending, database).context("Publish verified metadata database")?;
    fs::remove_file(pending)?;
    Ok(())
}

fn verify(source: &Path, store: &Store) -> Result<usize> {
    let files = metadata_files(source)?;
    let stored: i64 = store
        .connection
        .query_row("SELECT count(*) FROM documents", [], |row| row.get(0))?;
    ensure!(
        stored == files.len() as i64,
        "Source metadata inventory differs from database"
    );
    for (key, path) in &files {
        let original = fs::read(path)?;
        ensure!(
            store.get(key)?.as_deref() == Some(original.as_slice()),
            "Source metadata changed or is missing: {key}"
        );
    }
    Ok(files.len())
}

/// Export to a new directory, refusing overwrites and preserving exact JSON bytes.
fn export(store: &Store, destination: &Path) -> Result<()> {
    fs::create_dir(destination).context("Export destination must not exist")?;
    let mut statement = store
        .connection
        .prepare("SELECT path FROM documents ORDER BY path")?;
    let keys = statement.query_map([], |row| row.get::<_, String>(0))?;
    for key in keys {
        let key = key?;
        validate_key(&key)?;
        let path = destination.join(&key);
        fs::create_dir_all(path.parent().context("Export path has no parent")?)?;
        let bytes = store
            .get(&key)?
            .context("Metadata disappeared during export")?;
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn roundtrip_preserves_unknown_fields_and_jsonl_bytes() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("metadata-store-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("source/records"))?;
        let source = root.join("source");
        fs::write(
            source.join("records/a.json"),
            b"{ \"future\": [1,2], \"status\": \"failed\" }\n",
        )?;
        fs::write(source.join("events.jsonl"), b"{\"id\":1}\n{\"id\":2}\n")?;
        fs::write(source.join("image.png"), b"binary evidence stays external")?;
        let database = root.join("run.sqlite");
        import(&source, &database)?;
        let store = Store::open_readonly(&database)?;
        assert_eq!(verify(&source, &store)?, 2);
        export(&store, &root.join("export"))?;
        assert_eq!(
            fs::read(source.join("events.jsonl"))?,
            fs::read(root.join("export/events.jsonl"))?
        );
        assert!(!root.join("export/image.png").exists());
        fs::write(source.join("records/a.json"), b"changed")?;
        assert!(verify(&source, &store).is_err());
        assert!(import(&source, &database).is_err());
        drop(store);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn keys_cannot_escape_the_export_directory() {
        for key in ["", "../a.json", "/a.json", "C:/a.json", "a\\b.json"] {
            assert!(validate_key(key).is_err(), "{key}");
        }
        assert!(validate_key("records/a.json").is_ok());
    }
}
