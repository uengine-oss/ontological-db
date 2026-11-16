//! v1's storage for types — the DDL half of `crate::catalog::types` (P1 K6).
//!
//! Declaring a type generates real DDL: a per-type table (`og_data.n_<tid>` /
//! `e_<tid>`) whose declared properties are **actual typed columns** (`p_*`),
//! the rest in `__ext`, and a view named after the type. Creating, altering and
//! dropping a type here does both halves in one place, catalog rows and
//! storage, in the order they always ran; the catalog lookups they use stay in
//! `crate::catalog::types`.

use crate::catalog::labeling;
use crate::catalog::types::{graph_id, storage_table, try_type_id, try_type_name, type_id, type_kind, map_data_type};
use pgrx::prelude::*;

/// Physical column name for a declared property. Deterministic and injection-safe.
///
/// Letters outside ASCII are kept rather than folded to `_`: a graph with
/// Korean property names would otherwise map `이름` and `용량` to the same
/// column and silently merge two properties. PostgreSQL accepts those
/// characters in an unquoted identifier under UTF-8, so the name stays usable
/// as written. ASCII mapping is unchanged, so existing columns keep their names.
pub fn column_name(prop: &str) -> String {
    let mut s = String::with_capacity(prop.len() + 2);
    s.push_str("p_");
    for c in prop.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            s.push(c.to_ascii_lowercase());
        } else if c.is_alphanumeric() {
            s.extend(c.to_lowercase());
        } else {
            s.push('_');
        }
    }
    s
}

pub fn node_table(type_id: i32) -> String {
    format!("og_data.n_{type_id}")
}

pub fn edge_table(type_id: i32) -> String {
    format!("og_data.e_{type_id}")
}

/// Create (or move) the human-readable view for a type.
///
/// Named exactly as the type, so `og_data."MeetingRoom"` works in psql and in
/// any BI tool pointed at this database.
pub fn ensure_alias_view(tid: i32, name: &str, table: &str) {
    let view = alias_view_name(name);
    // A type name is user input; quoting is what makes that safe here.
    let _ = Spi::run(&format!("DROP VIEW IF EXISTS {view}"));
    if let Err(e) = Spi::run(&format!(
        "CREATE VIEW {view}{} AS SELECT * FROM {table}",
        crate::spiu::VIEW_SECURITY
    )) {
        // Never fatal: the view is a convenience, and a name that collides with
        // something already in the schema must not stop a type from existing.
        pgrx::log!("could not create the alias view for type {tid} ({name}): {e}");
    } else {
        // `apply_to_view` quotes what it is given; `view` is quoted already,
        // and quoting it twice named a relation that does not exist — which
        // failed every og_add_property once any role had been granted access.
        crate::store_v1::privileges::apply_to_view(&format!("og_data.{name}"));
    }
}

pub fn drop_alias_view(name: &str) {
    let _ = Spi::run(&format!("DROP VIEW IF EXISTS {}", alias_view_name(name)));
}

fn alias_view_name(name: &str) -> String {
    format!("og_data.\"{}\"", name.replace('"', "\"\""))
}

/// Place an already-declared type under `parents`, if that is safe.
///
/// "Safe" is narrow on purpose:
///
///   * it is already a subtype of every parent — nothing to do;
///   * it has parents of its own — someone placed it deliberately, and moving
///     it would silently rewrite a hierarchy the author chose;
///   * it holds rows — a type's storage table is its own, but its inherited
///     columns come from its parents, so adopting a populated type would mean
///     backfilling. Refuse rather than half-do it.
///
/// What remains is the case this exists for: a root type with no instances,
/// which is what `CREATE CONSTRAINT` leaves behind.
pub fn adopt_if_unplaced(gid: i32, tid: i32, name: &str, parents: &[String]) {
    if parents.is_empty() {
        return;
    }
    let parent_ids: Vec<i32> = parents.iter().map(|p| type_id(gid, p)).collect();
    if parent_ids.iter().all(|p| *p == tid || labeling::og_is_subtype(tid, *p)) {
        return;
    }
    let has_parent = crate::spiu::one::<bool>(
        "SELECT true FROM og_catalog.type_parent WHERE type_id = $1 LIMIT 1",
        &[tid.into()],
    )
    .ok()
    .flatten()
    .unwrap_or(false);
    if has_parent {
        return;
    }

    let kind = type_kind(tid);
    let table = match kind {
        'e' => node_table(tid),
        'r' => edge_table(tid),
        // Attribute types have no storage and no instances to weigh.
        _ => return,
    };
    let occupied = crate::spiu::one::<bool>(
        &format!("SELECT true FROM {table} LIMIT 1"),
        &[],
    )
    .ok()
    .flatten()
    .unwrap_or(false);
    if occupied {
        return;
    }

    for pid in parent_ids {
        if type_kind(pid) != kind {
            return;
        }
        Spi::run_with_args(
            "INSERT INTO og_catalog.type_parent (type_id, parent_id) VALUES ($1, $2)
             ON CONFLICT DO NOTHING",
            &[tid.into(), pid.into()],
        )
        .expect("parent insert failed");
        copy_inherited_properties(pid, tid, &table);
    }
    // Subtype tests read the nested-set intervals in `og_catalog.type_label`,
    // not `type_parent` directly. Without this the edge exists and
    // `og_is_subtype` still says no — the adoption would look like it did
    // nothing at all.
    labeling::relabel_graph(gid);
    labeling::bump_schema_version(gid, &format!("adopt {name} under {}", parents.join(":")));
    log!("type '{name}' adopted under ({}) on first labelled write", parents.join(":"));
}

#[pg_extern]
fn og_drop_graph(name: &str) {
    let gid = graph_id(name);
    Spi::connect(|client| {
        let rows = client
            .select(
                "SELECT storage_table FROM og_catalog.type
                  WHERE graph_id = $1 AND storage_table IS NOT NULL",
                None,
                &[gid.into()],
            )
            .expect("type lookup failed");
        let tables: Vec<String> = rows.filter_map(|r| r.get::<String>(1).ok().flatten()).collect();
        for t in tables {
            Spi::run(&format!("DROP TABLE IF EXISTS {t} CASCADE")).expect("drop table failed");
        }
    });
    Spi::run_with_args("DELETE FROM og_catalog.graph WHERE graph_id = $1", &[gid.into()])
        .expect("graph delete failed");
}

/// Declare a type.
///
/// * `kind` — `entity` | `relation` | `attribute`
/// * `parents` — supertypes; multiple entries make this multiple inheritance
///
/// Creates the backing storage table and re-labels the hierarchy.
#[pg_extern]
fn og_create_type(
    graph: &str,
    name: &str,
    kind: &str,
    parents: default!(Vec<Option<String>>, "'{}'"),
    is_abstract: default!(bool, false),
) -> i32 {
    let parent_names: Vec<String> = parents.into_iter().flatten().collect();
    create_type_inner(graph, name, kind, &parent_names, is_abstract)
}

/// The body of `og_create_type`, callable from Rust. The Cypher write path
/// needs it: Neo4j creates a label the first time it is written, and a graph
/// that refuses to do the same cannot stand in for one.
pub fn create_type_inner(
    graph: &str,
    name: &str,
    kind: &str,
    parents: &[String],
    is_abstract: bool,
) -> i32 {
    let gid = graph_id(graph);
    if try_type_id(gid, name).is_some() {
        error!("type '{name}' already exists in graph '{graph}'");
    }
    let k = match kind.to_ascii_lowercase().as_str() {
        "entity" | "e" | "node" => 'e',
        "relation" | "r" | "rel" | "edge" => 'r',
        "attribute" | "a" | "attr" => 'a',
        other => error!("unknown type kind '{other}' (entity | relation | attribute)"),
    };

    let parent_ids: Vec<i32> = parents
        .iter()
        .map(|p| {
            let pid = type_id(gid, p);
            let pk = type_kind(pid);
            if pk != k {
                error!("type '{name}' ({kind}) cannot inherit from '{p}' of kind '{pk}'");
            }
            pid
        })
        .collect();

    let tid = crate::spiu::one_mut::<i32>(
        "INSERT INTO og_catalog.type (type_id, graph_id, name, kind, is_abstract)
         VALUES (nextval('og_catalog.type_id_seq')::int4, $1, $2, $3::text::\"char\", $4)
         RETURNING type_id",
        &[gid.into(), name.into(), k.to_string().into(), is_abstract.into()],
    )
    .expect("type insert failed")
    .unwrap();

    for pid in &parent_ids {
        Spi::run_with_args(
            "INSERT INTO og_catalog.type_parent (type_id, parent_id) VALUES ($1, $2)",
            &[tid.into(), (*pid).into()],
        )
        .expect("parent insert failed");
    }

    // Storage table. Abstract types get none — they are never instantiated.
    if !is_abstract && k != 'a' {
        let table = if k == 'e' { node_table(tid) } else { edge_table(tid) };
        let base = if k == 'e' {
            format!("CREATE TABLE {table} (id int8 PRIMARY KEY, __ext jsonb)")
        } else {
            format!(
                "CREATE TABLE {table} (id int8 PRIMARY KEY, src int8 NOT NULL, \
                 dst int8 NOT NULL, __ext jsonb)"
            )
        };
        Spi::run(&base).expect("type table creation failed");
        crate::store_v1::privileges::apply_to_table(&table);
        // A view named after the type. The physical table is `n_<type_id>`
        // because a type can be renamed in constant time and its identifier
        // must not move; the cost is that `\dt` shows `n_45` and nothing about
        // MeetingRoom. This view puts the name back where a person looking at
        // the database can find it. It is a view, so it costs nothing to keep.
        ensure_alias_view(tid, name, &table);
        if k == 'r' {
            Spi::run(&format!("CREATE INDEX ON {table} (src)")).expect("index failed");
            Spi::run(&format!("CREATE INDEX ON {table} (dst)")).expect("index failed");
        }
        Spi::run_with_args(
            "UPDATE og_catalog.type SET storage_table = $2 WHERE type_id = $1",
            &[tid.into(), table.clone().into()],
        )
        .expect("storage_table update failed");

        // Inherited properties become columns on the child table too.
        for pid in &parent_ids {
            copy_inherited_properties(*pid, tid, &table);
        }
    }

    Spi::run_with_args(
        "INSERT INTO og_data.og_id_alloc (type_id, next_id) VALUES ($1, 1)
         ON CONFLICT DO NOTHING",
        &[tid.into()],
    )
    .expect("id alloc init failed");

    labeling::relabel_graph(gid);

    // Settings declared on an ancestor reach a subtype created later: the
    // labels above are what make the ancestry visible, so this comes after.
    if !is_abstract && k != 'a' {
        if crate::store_v1::security::inherits_stamping(tid) {
            if let Some(table) = storage_table(tid) {
                crate::store_v1::security::stamp_table(&table);
            }
        }
        if crate::store_v1::temporal::is_temporal(tid) {
            crate::store_v1::temporal::enable_table(tid);
        }
        inherit_history_capture(graph, tid);
        crate::store_v1::security::replay_write_grants(tid);
    }
    tid
}

/// `og_enable_history` on an ancestor covers a subtype created afterwards.
fn inherit_history_capture(graph: &str, tid: i32) {
    let on = crate::spiu::one::<bool>(
        "SELECT true FROM og_catalog.setting s
           JOIN og_catalog.type t ON s.key = 'history.' || $1 || '.' || t.name
          WHERE t.type_id = ANY($2) AND s.value = 'on' LIMIT 1",
        &[graph.into(), labeling::og_supertypes(tid).into()],
    )
    .ok()
    .flatten()
    .unwrap_or(false);
    if let (true, Some(table)) = (on, storage_table(tid)) {
        Spi::run(&format!(
            "CREATE OR REPLACE TRIGGER og_hist_{tid}
               AFTER INSERT OR UPDATE OR DELETE ON {table}
               FOR EACH ROW EXECUTE FUNCTION og_capture_history()"
        ))
        .unwrap_or_else(|e| error!("failed to enable history on {table}: {e}"));
    }
}

pub fn copy_inherited_properties(parent: i32, child: i32, child_table: &str) {
    let props: Vec<(String, String, String, bool, bool)> = Spi::connect(|client| {
        client
            .select(
                "WITH RECURSIVE anc(type_id) AS (
                     SELECT $1
                     UNION
                     SELECT tp.parent_id FROM og_catalog.type_parent tp JOIN anc ON anc.type_id = tp.type_id
                 )
                 SELECT p.name, p.data_type, p.column_name, p.required, p.is_key
                   FROM og_catalog.property p JOIN anc ON anc.type_id = p.type_id",
                None,
                &[parent.into()],
            )
            .expect("inherited property lookup failed")
            .map(|r| {
                (
                    r.get::<String>(1).unwrap().unwrap(),
                    r.get::<String>(2).unwrap().unwrap(),
                    r.get::<String>(3).unwrap().unwrap(),
                    r.get::<bool>(4).unwrap().unwrap_or(false),
                    r.get::<bool>(5).unwrap().unwrap_or(false),
                )
            })
            .collect()
    });
    for (name, dtype, col, required, is_key) in props {
        Spi::run(&format!("ALTER TABLE {child_table} ADD COLUMN IF NOT EXISTS {col} {dtype}"))
            .expect("inherited column add failed");
        if required {
            Spi::run(&format!("ALTER TABLE {child_table} ALTER COLUMN {col} SET NOT NULL")).ok();
        }
        if is_key {
            Spi::run(&format!("CREATE UNIQUE INDEX ON {child_table} ({col})")).ok();
        }
        Spi::run_with_args(
            "INSERT INTO og_catalog.property
                (prop_id, type_id, name, data_type, column_name, required, is_key)
             VALUES (nextval('og_catalog.property_id_seq')::int4, $1, $2, $3, $4, $5, $6)
             ON CONFLICT (type_id, name) DO NOTHING",
            &[
                child.into(),
                name.into(),
                dtype.into(),
                col.into(),
                required.into(),
                is_key.into(),
            ],
        )
        .expect("inherited property record failed");
    }
}

/// Declare a property. Optional properties are added without rewriting existing
/// rows (PostgreSQL 11+ fast default) — spec 002 SC-004.
#[pg_extern]
fn og_add_property(
    graph: &str,
    type_name: &str,
    prop: &str,
    data_type: &str,
    required: default!(bool, false),
    is_key: default!(bool, false),
) {
    let gid = graph_id(graph);
    let tid = type_id(gid, type_name);
    let dtype = map_data_type(data_type);
    let col = column_name(prop);

    if required {
        // Relation instances live in og_edge, not og_node; counting only nodes
        // let a required property onto a relation type with edges in it.
        let n = crate::spiu::one::<i64>(
            "SELECT (SELECT count(*) FROM og_data.og_node WHERE type_id = ANY($1))
                  + (SELECT count(*) FROM og_data.og_edge WHERE type_id = ANY($1))",
            &[labeling::og_subtypes(tid).into()],
        )
        .expect("count failed")
        .unwrap_or(0);
        if n > 0 {
            error!(
                "cannot add required property '{prop}' to '{type_name}': {n} existing instance(s) \
                 would violate it. add it as optional, backfill, then tighten."
            );
        }
    }

    Spi::run_with_args(
        "INSERT INTO og_catalog.property
            (prop_id, type_id, name, data_type, column_name, required, is_key)
         VALUES (nextval('og_catalog.property_id_seq')::int4, $1, $2, $3, $4, $5, $6)",
        &[tid.into(), prop.into(), dtype.clone().into(), col.clone().into(), required.into(), is_key.into()],
    )
    .expect("property insert failed");

    // Apply to this type and every descendant (inheritance is by column copy).
    for sub in labeling::og_subtypes(tid) {
        if let Some(table) = storage_table(sub) {
            Spi::run(&format!("ALTER TABLE {table} ADD COLUMN IF NOT EXISTS {col} {dtype}"))
                .expect("column add failed");
            if required {
                Spi::run(&format!("ALTER TABLE {table} ALTER COLUMN {col} SET NOT NULL"))
                    .expect("not-null failed");
            }
            // Values written before the property was declared live in the
            // extension payload. Declaring the property has to bring them into
            // the column, or an index built on it would see nothing — which is
            // exactly what "write first, index later" produces, and that is the
            // ordinary order for an application coming from a schemaless graph.
            Spi::run(&format!(
                "UPDATE {table} SET {col} = (__ext ->> {}) ::{dtype}, __ext = __ext - {} \
                  WHERE __ext ? {}",
                crate::cypher::compile::sql_str(prop),
                crate::cypher::compile::sql_str(prop),
                crate::cypher::compile::sql_str(prop),
            ))
            .unwrap_or_else(|e| {
                error!("failed to move existing '{prop}' values into its column: {e}")
            });

            if is_key {
                Spi::run(&format!(
                    "CREATE UNIQUE INDEX IF NOT EXISTS uq_{sub}_{col} ON {table} ({col})"
                ))
                .expect("unique index failed");
            }
            crate::store_v1::temporal::sync_history(sub);
            if sub != tid {
                Spi::run_with_args(
                    "INSERT INTO og_catalog.property
                        (prop_id, type_id, name, data_type, column_name, required, is_key)
                     VALUES (nextval('og_catalog.property_id_seq')::int4, $1, $2, $3, $4, $5, $6)
                     ON CONFLICT (type_id, name) DO NOTHING",
                    &[sub.into(), prop.into(), dtype.clone().into(), col.clone().into(),
                      required.into(), is_key.into()],
                )
                .expect("descendant property record failed");
            }
        }
    }
    // A view freezes its column list when it is created, so the type's
    // human-readable view has to be rebuilt whenever the type gains a column —
    // otherwise `og_data."Room"` keeps showing the shape it had on day one.
    for sub in labeling::og_subtypes(tid) {
        if let (Some(table), Some(sub_name)) = (storage_table(sub), try_type_name(sub)) {
            ensure_alias_view(sub, &sub_name, &table);
        }
        // A grant on "every property" of a type covers this column too.
        crate::store_v1::security::replay_write_grants(sub);
    }
    labeling::bump_schema_version(gid, &format!("add property {type_name}.{prop}"));
}

/// Create a secondary index on a declared property (spec 001 FR-016).
#[pg_extern]
fn og_create_index(graph: &str, type_name: &str, prop: &str) {
    let gid = graph_id(graph);
    let tid = type_id(gid, type_name);
    let col = column_name(prop);
    for sub in labeling::og_subtypes(tid) {
        if let Some(table) = storage_table(sub) {
            Spi::run(&format!("CREATE INDEX IF NOT EXISTS ix_{sub}_{col} ON {table} ({col})"))
                .expect("index creation failed");
        }
    }
    labeling::bump_schema_version(gid, &format!("create index {type_name}.{prop}"));
}

/// Drop a type. Refuses when instances exist unless `cascade` — spec 002 FR-024.
#[pg_extern]
fn og_drop_type(graph: &str, name: &str, cascade: default!(bool, false)) {
    let gid = graph_id(graph);
    let tid = type_id(gid, name);
    let subs = labeling::og_subtypes(tid);
    let n = crate::spiu::one::<i64>(
        "SELECT (SELECT count(*) FROM og_data.og_node WHERE type_id = ANY($1))
              + (SELECT count(*) FROM og_data.og_edge WHERE type_id = ANY($1))",
        &[subs.clone().into()],
    )
    .expect("instance count failed")
    .unwrap_or(0);
    if n > 0 && !cascade {
        error!("type '{name}' has {n} instance(s) (including subtypes). pass cascade => true to remove them");
    }
    for sub in subs {
        if let Some(table) = storage_table(sub) {
            Spi::run(&format!("DROP TABLE IF EXISTS {table} CASCADE")).expect("drop failed");
        }
        Spi::run_with_args("DELETE FROM og_data.og_node WHERE type_id = $1", &[sub.into()]).ok();
        Spi::run_with_args("DELETE FROM og_data.og_edge WHERE type_id = $1", &[sub.into()]).ok();
        Spi::run_with_args("DELETE FROM og_data.og_adj WHERE etype = $1", &[sub.into()]).ok();
        Spi::run_with_args("DELETE FROM og_catalog.type WHERE type_id = $1", &[sub.into()])
            .expect("type delete failed");
    }
    labeling::relabel_graph(gid);
}
