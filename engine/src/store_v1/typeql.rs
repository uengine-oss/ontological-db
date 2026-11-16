//! v1's storage for TypeQL types (P1 K4 gathered it in `typeql::schema`, K6 moved it here).
//!
//! What a TypeQL type needs in v1's storage (`ontological`): a table per type
//! in `og_data` — `n_<tid>` for entity and relation instances, `a_<tid>(id, val
//! UNIQUE)` for attribute instances, `e_<tid>(id, src, dst)` for the `$has`
//! ownerships — its name in `og_catalog.type.storage_table`, and an id space
//! in `og_data.og_id_alloc`. A layout that keeps types otherwise (v2: a label
//! per type, DESIGN.md 4.6) has its own counterpart; the `define` passes in
//! `crate::typeql::schema` only say *when* a type needs storage. The statements
//! are v1's, text and order.

use crate::catalog::types;
use crate::typeql::schema::{is_abstract, value_type_sql, SResult};
use pgrx::prelude::*;

fn attr_table(tid: i32) -> String {
    format!("og_data.a_{tid}")
}

fn set_storage_table(tid: i32, table: &str) {
    Spi::run_with_args(
        "UPDATE og_catalog.type SET storage_table = $2 WHERE type_id = $1",
        &[tid.into(), table.into()],
    )
    .expect("storage_table update failed");
}

/// The id space of a newly declared type (`declare_shell`).
pub fn reserve_ids(tid: i32) {
    Spi::run_with_args(
        "INSERT INTO og_data.og_id_alloc (type_id, next_id) VALUES ($1, 1)
         ON CONFLICT DO NOTHING",
        &[tid.into()],
    )
    .ok();
}

/// Entity and (reified) relation instances live in a skinny node table; their
/// attributes are separate instances, so there are no property columns here.
pub fn ensure_instance_table(tid: i32, is_abstract: bool) {
    if is_abstract || types::storage_table(tid).is_some() {
        return;
    }
    let table = crate::store_v1::type_tables::node_table(tid);
    // `__ext` is not used by TypeQL — attributes are instances, not columns —
    // but the Cypher view builder projects it, and both languages read the
    // same tables (FR-040).
    Spi::run(&format!(
        "CREATE TABLE IF NOT EXISTS {table} (id int8 PRIMARY KEY, __ext jsonb)"
    ))
    .expect("instance table creation failed");
    crate::store_v1::privileges::apply_to_table(&table);
    set_storage_table(tid, &table);
}

/// Attribute instances are deduplicated by value — that is what makes two books
/// share one `genre` instance (FR-016), so the UNIQUE index is load-bearing,
/// not merely defensive. `vt` is the TypeQL value type, already recorded.
pub fn ensure_attr_table(tid: i32, name: &str, vt: &str) -> SResult<()> {
    if is_abstract(tid) || types::storage_table(tid).is_some() {
        return Ok(());
    }
    let sql_ty = value_type_sql(vt)?;
    let table = attr_table(tid);
    // UNIQUE on the value is load-bearing, not defensive: it is what makes two
    // owners of "fiction" share one instance (FR-016).
    Spi::run(&format!(
        "CREATE TABLE IF NOT EXISTS {table} \
             (id int8 PRIMARY KEY, val {sql_ty} NOT NULL UNIQUE, __ext jsonb)"
    ))
    .map_err(|e| format!("attribute storage for '{name}' failed: {e}"))?;
    crate::store_v1::privileges::apply_to_table(&table);
    set_storage_table(tid, &table);
    // Declaring `val` as a property makes the attribute's value visible through
    // the Cypher surface without a second projection path.
    Spi::run_with_args(
        "INSERT INTO og_catalog.property
            (prop_id, type_id, name, data_type, column_name, required)
         VALUES (nextval('og_catalog.property_id_seq')::int4, $1, 'val', $2, 'val', true)
         ON CONFLICT (type_id, name) DO NOTHING",
        &[tid.into(), sql_ty.into()],
    )
    .ok();
    Ok(())
}

/// The `$has` relationship type's table, indexes and id space.
pub fn ensure_has_table(tid: i32) {
    let table = crate::store_v1::type_tables::edge_table(tid);
    Spi::run(&format!(
        "CREATE TABLE IF NOT EXISTS {table} (id int8 PRIMARY KEY, src int8 NOT NULL, dst int8 NOT NULL)"
    ))
    .expect("$has table creation failed");
    crate::store_v1::privileges::apply_to_table(&table);
    Spi::run(&format!("CREATE INDEX IF NOT EXISTS e_{tid}_src ON {table} (src)")).ok();
    Spi::run(&format!("CREATE INDEX IF NOT EXISTS e_{tid}_dst ON {table} (dst)")).ok();
    set_storage_table(tid, &table);
    Spi::run_with_args(
        "INSERT INTO og_data.og_id_alloc (type_id, next_id) VALUES ($1, 1) ON CONFLICT DO NOTHING",
        &[tid.into()],
    )
    .ok();
}
