//! Schema introspection operations.
//!
//! Each sub-module covers one aspect of PostgreSQL schema inspection:
//!
//! | Module | Operations |
//! |--------|-----------|
//! | [`databases`] | [`ListDatabases`](databases::ListDatabases) |
//! | [`schemas`] | [`ListSchemas`](schemas::ListSchemas) |
//! | [`tables`] | [`ListTables`](tables::ListTables), [`TableExists`](tables::TableExists) |
//! | [`columns`] | [`ListColumns`](columns::ListColumns) |
//! | [`indexes`] | [`ListIndexes`](indexes::ListIndexes) |
//! | [`constraints`] | [`ListConstraints`](constraints::ListConstraints) |

pub mod columns;
pub mod constraints;
pub mod databases;
pub mod indexes;
pub mod schemas;
pub mod tables;
