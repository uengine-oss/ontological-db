//! Source profiling — what a relationship-inference pass needs to know about a
//! datasource's tables, read through the connector without copying anything.
//!
//! ```text
//! SHOW COLUMN STATS FROM erp.purchase_order [SAMPLE 1000]
//!     per column, over the first N rows: nulls, distinct values, whether the
//!     non-null values are unique, the five most common, a few short samples
//! SHOW VALUE OVERLAP FROM erp.purchase_order(supplier_id) IN erp.supplier(supplier_id) [SAMPLE 200]
//!     how many of up to N distinct child values the parent column holds, and
//!     whether the parent's values are unique in a sample of its own
//! ```
//!
//! These are measurements. Which pair is a relationship is the caller's call
//! (Ontological Studio's relationship suggestions), the way `ANALYZE` measures
//! and the planner decides.

use crate::spiu::{ident, one};
use pgrx::prelude::*;
use pgrx::JsonB;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

const MAX_ROWS: i64 = 10_000;
const MAX_VALUES: i64 = 1_000;

/// The relation a datasource table is read through (`"erp"."item"`), quoted.
fn relation(ds: &str, table: &str) -> String {
    one::<String>(
        "SELECT format('%I.%I', n.nspname, c.relname)
           FROM pg_foreign_table ft JOIN pg_foreign_server s ON s.oid = ft.ftserver
           JOIN og_catalog.datasource d ON d.server_name = s.srvname
           JOIN pg_class c ON c.oid = ft.ftrelid JOIN pg_namespace n ON n.oid = c.relnamespace
          WHERE d.name = $1 AND c.relname = $2
          ORDER BY (n.nspname = $1) DESC LIMIT 1",
        &[ds.into(), table.into()],
    )
    .ok()
    .flatten()
    .unwrap_or_else(|| error!("no table '{table}' in datasource '{ds}'"))
}

struct Col {
    name: String,
    ty: String,
    typname: String,
    nullable: bool,
    /// The column a virtual node over this table uses as its id.
    key: bool,
}

fn columns(rel: &str) -> Vec<Col> {
    Spi::connect(|c| {
        c.select(
            "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), t.typname::text, NOT a.attnotnull,
                    EXISTS (SELECT 1 FROM og_catalog.mapping m
                             WHERE to_regclass(m.source_table) = a.attrelid AND m.id_column = a.attname)
               FROM pg_attribute a JOIN pg_type t ON t.oid = a.atttypid
              WHERE a.attrelid = to_regclass($1) AND a.attnum > 0 AND NOT a.attisdropped
              ORDER BY a.attnum",
            None,
            &[rel.into()],
        )
        .expect("column lookup failed")
        .filter_map(|r| {
            Some(Col {
                name: r.get::<String>(1).ok()??,
                ty: r.get::<String>(2).ok()??,
                typname: r.get::<String>(3).ok()??,
                nullable: r.get::<bool>(4).ok()??,
                key: r.get::<bool>(5).ok()??,
            })
        })
        .collect()
    })
}

/// The columns of the table's primary key at its source, `None` where the datasource
/// does not know (see `probe_source_catalog`).
fn source_primary_key(ds: &str, table: &str) -> Option<Vec<String>> {
    let sql = format!(
        "SELECT {entry}->'pk'
           FROM pg_foreign_table ft JOIN pg_foreign_server s ON s.oid = ft.ftserver
           JOIN og_catalog.datasource d ON d.server_name = s.srvname
           JOIN pg_class c ON c.oid = ft.ftrelid JOIN pg_namespace n ON n.oid = c.relnamespace
          WHERE d.name = $1 AND c.relname = $2
          ORDER BY (n.nspname = $1) DESC LIMIT 1",
        entry = super::SOURCE_ENTRY_SQL
    );
    match one::<JsonB>(&sql, &[ds.into(), table.into()]).ok().flatten()?.0 {
        Value::Array(a) => Some(a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()),
        _ => None,
    }
}

fn column<'a>(cols: &'a [Col], name: &str, ds: &str, table: &str) -> &'a Col {
    cols.iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| error!("no column '{name}' in {ds}.{table}"))
}

/// A value as text, the way a sample is shown and compared; NULL is none.
fn text(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// Per-column statistics over the first `sample` rows of a datasource table.
#[pg_extern]
fn og_fabric_column_stats(datasource: &str, table: &str, sample: default!(i64, 1000)) -> JsonB {
    let n = sample.clamp(1, MAX_ROWS);
    let rel = relation(datasource, table);
    let cols = columns(&rel);
    let pk = source_primary_key(datasource, table);
    let rows: Vec<Map<String, Value>> = Spi::connect(|c| {
        c.select(&format!("SELECT to_jsonb(s) FROM (SELECT * FROM {rel} LIMIT {n}) s"), None, &[])
            .unwrap_or_else(|e| error!("could not read {datasource}.{table}: {e}"))
            .filter_map(|r| match r.get::<JsonB>(1).ok().flatten() {
                Some(JsonB(Value::Object(m))) => Some(m),
                _ => None,
            })
            .collect()
    });
    let out: Vec<Value> = cols
        .iter()
        .map(|col| {
            let mut counts: HashMap<String, i64> = HashMap::new();
            let mut first_seen: Vec<String> = Vec::new();
            let (mut nulls, mut longest) = (0_i64, 0_usize);
            for r in &rows {
                match r.get(&col.name).and_then(text) {
                    None => nulls += 1,
                    Some(v) => {
                        longest = longest.max(v.chars().count());
                        let e = counts.entry(v.clone()).or_insert(0);
                        if *e == 0 {
                            first_seen.push(v);
                        }
                        *e += 1;
                    }
                }
            }
            let non_null = rows.len() as i64 - nulls;
            let distinct = counts.len() as i64;
            let mut top: Vec<(&String, &i64)> = counts.iter().collect();
            top.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let top_values: Vec<Value> = top.iter().take(5).map(|(v, c)| json!([v, c])).collect();
            let samples: Vec<Value> =
                first_seen.iter().filter(|v| v.chars().count() <= 40).take(10).map(|v| json!(v)).collect();
            json!({
                "column": col.name, "type": col.ty, "nullable": col.nullable, "key": col.key,
                "primary_key": pk.as_ref().map(|k| k.contains(&col.name)),
                "sampled": rows.len(), "nulls": nulls, "distinct": distinct,
                "unique": non_null > 0 && distinct == non_null,
                "max_length": longest, "top_values": top_values, "samples": samples,
            })
        })
        .collect();
    JsonB(Value::Array(out))
}

fn integer_family(t: &str) -> bool {
    matches!(t, "int2" | "int4" | "int8")
}

/// How much of a child column's values a parent column holds.
///
/// Up to `sample` distinct child values, taken from the first 5,000 non-null
/// rows in an order that is fixed but not the source's (so not just its lowest
/// keys), are looked up in the parent column — compared in the parent's type
/// when both columns share one, so the connector can push the lookup down, and
/// as text otherwise. The parent's uniqueness is judged on its first 1,000
/// non-null values, as `SHOW COLUMN STATS` would.
#[pg_extern]
fn og_fabric_value_overlap(
    child_datasource: &str,
    child_table: &str,
    child_column: &str,
    parent_datasource: &str,
    parent_table: &str,
    parent_column: &str,
    sample: default!(i64, 200),
) -> JsonB {
    let n = sample.clamp(1, MAX_VALUES);
    let crel = relation(child_datasource, child_table);
    let prel = relation(parent_datasource, parent_table);
    let ccols = columns(&crel);
    let pcols = columns(&prel);
    let cc = column(&ccols, child_column, child_datasource, child_table);
    let pc = column(&pcols, parent_column, parent_datasource, parent_table);
    let (c, p) = (ident(&cc.name), ident(&pc.name));

    let values: Vec<String> = Spi::connect(|cl| {
        cl.select(
            &format!(
                "SELECT v FROM (SELECT DISTINCT {c}::text AS v
                                  FROM (SELECT {c} FROM {crel} WHERE {c} IS NOT NULL LIMIT 5000) a) d
                  ORDER BY md5(v) LIMIT {n}"
            ),
            None,
            &[],
        )
        .unwrap_or_else(|e| error!("could not read {child_datasource}.{child_table}: {e}"))
        .filter_map(|r| r.get::<String>(1).ok().flatten())
        .collect()
    });
    let typed = cc.typname == pc.typname || (integer_family(&cc.typname) && integer_family(&pc.typname));
    let matched = if values.is_empty() {
        0
    } else {
        let q = if typed {
            format!("SELECT count(DISTINCT {p})::bigint FROM {prel} WHERE {p} = ANY($1::text[]::{}[])", pc.ty)
        } else {
            format!("SELECT count(DISTINCT {p}::text)::bigint FROM {prel} WHERE {p}::text = ANY($1::text[])")
        };
        one::<i64>(&q, &[values.clone().into()])
            .unwrap_or_else(|e| error!("could not read {parent_datasource}.{parent_table}: {e}"))
            .unwrap_or(0)
    };
    let (ps, pd) = Spi::connect(|cl| {
        cl.select(
            &format!(
                "SELECT count(*)::bigint, count(DISTINCT {p})::bigint
                   FROM (SELECT {p} FROM {prel} WHERE {p} IS NOT NULL LIMIT 1000) s"
            ),
            Some(1),
            &[],
        )
        .ok()
        .and_then(|mut t| t.next())
        .and_then(|r| Some((r.get::<i64>(1).ok()??, r.get::<i64>(2).ok()??)))
    })
    .unwrap_or((0, 0));
    let sampled = values.len() as i64;
    JsonB(json!({
        "sampled": sampled, "matched": matched,
        "ratio": if sampled > 0 { matched as f64 / sampled as f64 } else { 0.0 },
        "parent_sampled": ps, "parent_distinct": pd, "parent_unique": ps > 0 && ps == pd,
        "parent_key": pc.key, "compare": if typed { "typed" } else { "text" },
    }))
}
