//! `ext_a`: stands in for the v1 extension `ontological`. A thin shell over `og_shared`.
use pgrx::prelude::*;

// Pg_magic_func + the schema section's sentinel. `name, version` read CARGO_PKG_NAME /
// CARGO_PKG_VERSION where the macro is expanded, so it belongs in the extension crate.
::pgrx::pg_module_magic!(name, version);

// The edition's own first SQL. og_shared's extension_sql! entries require it by name.
extension_sql!(
    r#"
CREATE TABLE k0_edition_boot (note text);
INSERT INTO k0_edition_boot VALUES ('ext_a bootstrap');
"#,
    name = "k0_edition_boot",
    bootstrap
);

#[pg_guard]
pub extern "C-unwind" fn _PG_init() {
    og_shared::set_edition("ext_a");
}

/// Only in ext_a.
#[pg_extern]
fn ext_a_only() -> String {
    format!("ext_a_only ({})", og_shared::so_name())
}

/// Same SQL name in both extensions, but each defines its own (edition-specific code).
#[pg_extern]
fn k0_edition_local() -> &'static str {
    "ext_a's own k0_edition_local"
}

extension_sql!(
    r#"CREATE VIEW k0_finalize AS SELECT 'ext_a finalize'::text AS note;"#,
    name = "k0_finalize",
    finalize
);
