//! Checks the migration files without a database.
//!
//! sqlx keys `_sqlx_migrations` by version alone. Two migrations sharing a version
//! (two branches merged with the same timestamp) compile, pass every in-memory test,
//! then fail on PostgreSQL with a duplicate key or a checksum mismatch.

use std::collections::HashMap;
use std::fs::read_dir;
use std::path::Path;

#[test]
fn every_migration_version_belongs_to_a_single_migration() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut names_by_version: HashMap<String, Vec<String>> = HashMap::new();

    for entry in read_dir(&dir).expect("read migrations directory") {
        let file_name = entry.expect("read migration entry").file_name();
        let file_name = file_name.to_string_lossy();
        let Some(stem) = file_name
            .strip_suffix(".up.sql")
            .or_else(|| file_name.strip_suffix(".down.sql"))
        else {
            continue;
        };
        let (version, name) = stem
            .split_once('_')
            .unwrap_or_else(|| panic!("migration without a version: {file_name}"));
        let names = names_by_version.entry(version.to_string()).or_default();
        if !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }

    assert!(
        !names_by_version.is_empty(),
        "no migration found in {dir:?}"
    );
    let mut duplicates: Vec<_> = names_by_version
        .into_iter()
        .filter(|(_, names)| names.len() > 1)
        .collect();
    duplicates.sort();
    assert!(
        duplicates.is_empty(),
        "migration versions shared by several migrations: {duplicates:?}"
    );
}
