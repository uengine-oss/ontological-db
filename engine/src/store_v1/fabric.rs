//! v1's shape of a virtual node type (`crate::fabric`, ADR-029): the type's
//! table replaced by a view over the source relation that looks like a v1 type
//! table — `id` from the key, a `p_*` column per property, an empty `__ext`
//! (P1 K6 moved it out of `fabric::map_relation`, which decides what to map).

use crate::catalog::types;
use crate::spiu::{ident, one};
use pgrx::prelude::*;

/// 36 bits: the local part of a node identifier (see `id.rs`).
const LOCAL_MASK: i64 = (1_i64 << 36) - 1;

/// Replace `tid`'s storage with a view over `lschema.rtable`: `id` made from
/// `key_column` (of `key_type`), and for each `(property, column, _)` of `picks`
/// the column cast to the property's declared type. Returns the source relation
/// as the view reads it (`"schema"."table"`).
pub fn virtual_view(
    tid: i32,
    type_name: &str,
    lschema: &str,
    rtable: &str,
    key_column: &str,
    key_type: &str,
    picks: &[(String, String, String)],
) -> String {
    let ftable = format!("{}.{}", ident(lschema), ident(rtable));
    let key = ident(key_column);
    let id_expr = match key_type {
        "smallint" | "integer" | "bigint" => format!("og_make_id(0, {tid}, ({key})::int8)"),
        _ => format!("og_make_id(0, {tid}, hashtextextended(({key})::text, 0) & {LOCAL_MASK})"),
    };
    let mut select = vec![format!("{id_expr} AS id")];
    for (prop, col, _) in picks {
        let dtype = one::<String>(
            "SELECT data_type FROM og_catalog.property WHERE type_id = $1 AND name = $2",
            &[tid.into(), prop.as_str().into()],
        )
        .ok()
        .flatten()
        .unwrap_or_else(|| "text".into());
        select.push(format!("({})::{dtype} AS {}", ident(col), super::type_tables::column_name(prop)));
    }
    select.push("NULL::jsonb AS __ext".into());

    let table = types::storage_table(tid)
        .unwrap_or_else(|| error!("'{type_name}' is abstract and cannot be virtual"));
    // The type's storage may already be a view from an earlier mapping.
    let is_view = one::<bool>("SELECT relkind = 'v' FROM pg_class WHERE oid = $1::regclass", &[table.as_str().into()])
        .ok()
        .flatten()
        .unwrap_or(false);
    let kind = if is_view { "VIEW" } else { "TABLE" };
    Spi::run(&format!("DROP {kind} IF EXISTS {table} CASCADE"))
        .unwrap_or_else(|e| error!("failed to drop storage of '{type_name}': {e}"));
    Spi::run(&format!(
        "CREATE VIEW {table}{} AS SELECT {} FROM {ftable}",
        crate::spiu::VIEW_SECURITY,
        select.join(", ")
    ))
    .unwrap_or_else(|e| error!("failed to create virtual view for '{type_name}': {e}"));
    super::privileges::apply_to_view(&table);
    ftable
}
