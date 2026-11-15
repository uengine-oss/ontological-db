//! `ext_b`: stands in for the v2 extension `ontological_age`. Same shape as ext_a; its control
//! file sets `schema = 'ext_b_s'`, so everything unqualified lands there.
use pgrx::prelude::*;

#[cfg(feature = "variant-extern-only")]
extern crate og_shared as _;

::pgrx::pg_module_magic!(name, version);

extension_sql!(
    r#"
CREATE TABLE k0_edition_boot (note text);
INSERT INTO k0_edition_boot VALUES ('ext_b bootstrap');
"#,
    name = "k0_edition_boot",
    bootstrap
);

#[pg_guard]
pub extern "C-unwind" fn _PG_init() {
    #[cfg(not(any(feature = "variant-unlinked", feature = "variant-extern-only")))]
    og_shared::set_edition("ext_b");
}

/// Only in ext_b.
#[pg_extern]
fn ext_b_only() -> &'static str {
    "ext_b_only"
}

/// Same SQL name as ext_a's, different code.
#[pg_extern]
fn k0_edition_local() -> &'static str {
    "ext_b's own k0_edition_local"
}

extension_sql!(
    r#"CREATE VIEW k0_finalize AS SELECT 'ext_b finalize'::text AS note;"#,
    name = "k0_finalize",
    finalize
);
