//! Write-time security context — spec 014.
//!
//! Three questions, answered by three mechanisms, none of which depends on the
//! engine parsing a query correctly:
//!
//!   * **What may this role write?** Ordinary PostgreSQL table and column
//!     grants on the per-type storage tables, issued by `og_grant_write`. A type
//!     is a table and a property is a column, so Neo4j's `CREATE ON … NODES L`
//!     and `SET PROPERTY {p}` map onto `INSERT ON n_L` and `UPDATE (p_p) ON n_L`
//!     one for one — and hold for raw SQL exactly as they hold for Cypher. The
//!     shared registries are covered by row-level security in `access.sql`.
//!
//!   * **Who wrote it, from where?** System-managed columns stamped by a
//!     trigger from the session (`og_enable_stamping`), and the `og.actor` /
//!     `og.source` / `og.request_id` settings a gateway sets per transaction.
//!     Nothing a caller writes into a row can forge them.
//!
//!   * **Which writes are allowed at all?** Actions (`action`): a named,
//!     parameterised write backed by a SECURITY DEFINER function, so an agent
//!     that holds only read access plus EXECUTE on an action can do that one
//!     thing and nothing else.
//!
//! The checks in this module that run before a write (`check_write`) exist for
//! the error message. PostgreSQL enforces the same rule a moment later either
//! way.

pub mod action;

use crate::catalog::{labeling, types};
use crate::spiu;
use pgrx::prelude::*;
use pgrx::PgSqlErrorCode;
use serde_json::Value;

/// Keys no caller may write as a property. `_id`, `_type`, `_src`, `_dst` are
/// what the engine puts into an element's JSON form — a user value under one of
/// those names would override the synthetic one on the way out — and anything
/// starting with `__` is a system column or the overflow payload itself.
pub fn is_reserved_key(k: &str) -> bool {
    matches!(k, "_id" | "_type" | "_src" | "_dst") || k.starts_with("__")
}

/// Refuse a property payload that names a reserved key.
pub fn check_reserved_keys(props: &Value) {
    if let Some(obj) = props.as_object() {
        if let Some(k) = obj.keys().find(|k| is_reserved_key(k)) {
            ereport!(
                ERROR,
                PgSqlErrorCode::ERRCODE_INSUFFICIENT_PRIVILEGE,
                format!(
                    "OG_RESERVED_PROPERTY: '{k}' is system-managed and cannot be written. \
                     Names starting with '__' and the keys _id, _type, _src, _dst are reserved."
                )
            );
        }
    }
}

/// What a write is about to do, for `check_write`.
pub enum WriteOp {
    Create,
    Set,
    Delete,
}

/// Fail with a readable message when the current role could not perform this
/// write. `cols` are the physical columns the statement will name.
pub fn check_write(tid: i32, table: &str, op: WriteOp, cols: &[String]) {
    let (priv_name, verb) = match op {
        WriteOp::Create => ("INSERT", "create"),
        WriteOp::Set => ("UPDATE", "set properties on"),
        WriteOp::Delete => ("DELETE", "delete"),
    };
    let denied: Option<String> = match op {
        WriteOp::Delete => {
            let ok = spiu::one::<bool>(
                "SELECT has_table_privilege($1, 'DELETE')",
                &[table.into()],
            )
            .ok()
            .flatten()
            .unwrap_or(false);
            if ok { None } else { Some(String::new()) }
        }
        _ => spiu::one::<String>(
            "SELECT c FROM unnest($2::text[]) AS c
              WHERE NOT has_column_privilege($1, c, $3) LIMIT 1",
            &[table.into(), cols.to_vec().into(), priv_name.into()],
        )
        .ok()
        .flatten(),
    };
    let Some(col) = denied else { return };
    let tname = types::try_type_name(tid).unwrap_or_else(|| format!("type#{tid}"));
    let role = spiu::one::<String>("SELECT current_user::text", &[]).ok().flatten().unwrap_or_default();
    let what = match col.as_str() {
        "" => String::new(),
        "__ext" => " (undeclared properties)".to_string(),
        "id" | "src" | "dst" => String::new(),
        c => format!(" (property '{}')", property_name_for(tid, c)),
    };
    ereport!(
        ERROR,
        PgSqlErrorCode::ERRCODE_INSUFFICIENT_PRIVILEGE,
        format!("OG_WRITE_DENIED: role '{role}' may not {verb} '{tname}'{what}"),
        "grant it with og_grant_write(role, graph, type, ops, props)"
    );
}

fn property_name_for(tid: i32, col: &str) -> String {
    spiu::one::<String>(
        "SELECT name FROM og_catalog.property WHERE type_id = $1 AND column_name = $2 LIMIT 1",
        &[tid.into(), col.into()],
    )
    .ok()
    .flatten()
    .unwrap_or_else(|| col.trim_start_matches("p_").to_string())
}

/// May the current role promote an undeclared property to a column?
///
/// Promotion is `og_add_property`, which is schema change. A writer without it
/// keeps the value in `__ext` instead of failing the write — which is what the
/// privilege model always said happened, and until now did not.
pub fn can_change_schema() -> bool {
    spiu::one::<bool>(
        "SELECT has_function_privilege(
                    'og_add_property(text,text,text,text,boolean,boolean)', 'EXECUTE')
            AND has_table_privilege('og_catalog.property', 'INSERT')",
        &[],
    )
    .ok()
    .flatten()
    .unwrap_or(false)
}

// --------------------------------------------------------------------------
// Fine-grained write grants
// --------------------------------------------------------------------------

/// Grant a role write access to one type (and its subtypes) — spec 014 FR-001.
///
/// * `ops` — any of `create`, `set`, `delete`
/// * `props` — the properties `create` and `set` may write; NULL means every
///   property, including undeclared ones kept in `__ext`
///
/// Read access is granted too if the role has none: a writer that cannot see
/// what it is writing to cannot run a MATCH.
#[pg_extern]
fn og_grant_write(
    role: &str,
    graph: &str,
    type_name: &str,
    ops: Vec<String>,
    props: default!(Option<Vec<String>>, "NULL"),
) {
    let gid = types::graph_id(graph);
    let tid = types::type_id(gid, type_name);
    if spiu::one::<i32>("SELECT 1 FROM pg_roles WHERE rolname = $1", &[role.into()])
        .ok()
        .flatten()
        .is_none()
    {
        error!("role '{role}' does not exist — create it first, then call og_grant_write");
    }
    for op in &ops {
        if !matches!(op.as_str(), "create" | "set" | "delete") {
            error!("unknown write operation '{op}' (create | set | delete)");
        }
    }
    if let Some(ps) = &props {
        if let Some(bad) = ps.iter().find(|p| is_reserved_key(p)) {
            error!("'{bad}' is system-managed and cannot be granted");
        }
    }

    ensure_writer_base(role);

    for op in &ops {
        Spi::run_with_args(
            "INSERT INTO og_catalog.write_grant (role, type_id, op, props) VALUES ($1, $2, $3, $4)
             ON CONFLICT (role, type_id, op) DO UPDATE SET props = EXCLUDED.props",
            &[role.into(), tid.into(), op.as_str().into(), props.clone().into()],
        )
        .unwrap_or_else(|e| error!("could not record the grant: {e}"));
    }
    for sub in labeling::og_subtypes(tid) {
        reapply_for_role(role, sub);
    }
}

/// Take back what `og_grant_write` gave on a type. `ops` NULL means all three.
#[pg_extern]
fn og_revoke_write(
    role: &str,
    graph: &str,
    type_name: &str,
    ops: default!(Option<Vec<String>>, "NULL"),
) {
    let gid = types::graph_id(graph);
    let tid = types::type_id(gid, type_name);
    let ops = ops.unwrap_or_else(|| vec!["create".into(), "set".into(), "delete".into()]);
    Spi::run_with_args(
        "DELETE FROM og_catalog.write_grant WHERE role = $1 AND type_id = $2 AND op = ANY($3)",
        &[role.into(), tid.into(), ops.into()],
    )
    .unwrap_or_else(|e| error!("could not clear the grant record: {e}"));
    for sub in labeling::og_subtypes(tid) {
        reapply_for_role(role, sub);
    }
}

/// The registry privileges every writer needs, and read access.
///
/// The registries are protected by row-level security (see `access.sql`), so
/// holding DML on them grants nothing a type grant does not.
fn ensure_writer_base(role: &str) {
    let has_level = spiu::one::<String>(
        "SELECT level FROM og_catalog.grantee WHERE role = $1",
        &[role.into()],
    )
    .ok()
    .flatten();
    if has_level.is_none() {
        Spi::run_with_args("SELECT og_grant($1, 'read')", &[role.into()])
            .unwrap_or_else(|e| error!("could not grant read access: {e}"));
    }
    let r = spiu::ident(role);
    Spi::run(&format!(
        "GRANT INSERT, UPDATE, DELETE ON og_data.og_node, og_data.og_edge, og_data.og_adj,
               og_data.og_role_player, og_data.og_id_alloc TO {r}"
    ))
    .unwrap_or_else(|e| error!("registry grant failed: {e}"));
    crate::catalog::privileges::grant_write_functions(&r);
}

/// Recompute one role's privileges on one concrete type's table from the
/// recorded grants on that type and its ancestors.
fn reapply_for_role(role: &str, sub: i32) {
    let Some(table) = types::storage_table(sub) else { return };
    let r = spiu::ident(role);
    let t = spiu::qname(&table);
    // Table-level REVOKE takes the column-level grants with it.
    Spi::run(&format!("REVOKE INSERT, UPDATE, DELETE ON {t} FROM {r}"))
        .unwrap_or_else(|e| error!("revoke on {table} failed: {e}"));

    let grants: Vec<(String, Option<Vec<String>>)> = Spi::connect(|client| {
        client
            .select(
                "SELECT op, props FROM og_catalog.write_grant
                  WHERE role = $1 AND type_id = ANY($2)",
                None,
                &[role.into(), labeling::og_supertypes(sub).into()],
            )
            .map(|rows| {
                rows.filter_map(|row| {
                    Some((row.get::<String>(1).ok()??, row.get::<Vec<String>>(2).ok()?))
                })
                .collect()
            })
            .unwrap_or_default()
    });

    let is_edge = table.starts_with("og_data.e_");
    // Several grants can reach one table (on the type and on an ancestor);
    // the union of what they allow is what the role gets.
    let mut all = std::collections::BTreeMap::<&str, bool>::new();
    let mut cols = std::collections::BTreeMap::<&str, Vec<String>>::new();
    for (op, props) in &grants {
        let op = op.as_str();
        match props {
            None => {
                all.insert(op, true);
            }
            Some(ps) => {
                let entry = cols.entry(op).or_default();
                for p in ps {
                    if let Some(c) = column_of(sub, p) {
                        entry.push(c);
                    }
                }
            }
        }
    }
    for op in ["create", "set", "delete"] {
        let privname = match op {
            "create" => "INSERT",
            "set" => "UPDATE",
            _ => "DELETE",
        };
        if all.contains_key(op) {
            Spi::run(&format!("GRANT {privname} ON {t} TO {r}"))
                .unwrap_or_else(|e| error!("grant on {table} failed: {e}"));
            continue;
        }
        let Some(list) = cols.get(op) else { continue };
        if op == "delete" {
            continue;
        }
        let mut list: Vec<String> = list.clone();
        if op == "create" {
            list.push("id".into());
            if is_edge {
                list.push("src".into());
                list.push("dst".into());
            }
        }
        list.sort();
        list.dedup();
        if list.is_empty() {
            continue;
        }
        let quoted = list.iter().map(|c| spiu::ident(c)).collect::<Vec<_>>().join(", ");
        Spi::run(&format!("GRANT {privname} ({quoted}) ON {t} TO {r}"))
            .unwrap_or_else(|e| error!("column grant on {table} failed: {e}"));
    }
}

fn column_of(tid: i32, prop: &str) -> Option<String> {
    spiu::one::<String>(
        "SELECT column_name FROM og_catalog.property WHERE type_id = $1 AND name = $2",
        &[tid.into(), prop.into()],
    )
    .ok()
    .flatten()
}

/// Replay every recorded write grant onto one concrete type's table. Called
/// when the table is created and whenever it gains a column, so a grant issued
/// on a supertype — or on "all properties" — reaches storage that did not exist
/// when it was issued.
pub fn replay_write_grants(sub: i32) {
    let roles: Vec<String> = Spi::connect(|client| {
        client
            .select(
                "SELECT DISTINCT role FROM og_catalog.write_grant WHERE type_id = ANY($1)",
                None,
                &[labeling::og_supertypes(sub).into()],
            )
            .map(|rows| rows.filter_map(|r| r.get::<String>(1).ok().flatten()).collect())
            .unwrap_or_default()
    });
    for role in roles {
        reapply_for_role(&role, sub);
    }
}

// --------------------------------------------------------------------------
// System-managed stamps
// --------------------------------------------------------------------------

/// The stamp columns and their types. Also the list `og__stamp()` writes.
pub const STAMP_COLUMNS: &[(&str, &str)] = &[
    ("__created_at", "timestamptz"),
    ("__created_by", "text"),
    ("__updated_at", "timestamptz"),
    ("__updated_by", "text"),
    ("__actor", "text"),
    ("__source", "text"),
];

/// Stamp every write to a type with who, when and from where — spec 014 FR-007.
///
/// `created_by` / `updated_by` are the database role that ran the statement;
/// `actor` and `source` are what the application asserted through `og.actor`
/// and `og.source`. Read them back with `og_meta(id)`.
#[pg_extern]
fn og_enable_stamping(graph: &str, type_name: &str) {
    let gid = types::graph_id(graph);
    let tid = types::type_id(gid, type_name);
    Spi::run_with_args(
        "INSERT INTO og_catalog.stamped (type_id) VALUES ($1) ON CONFLICT DO NOTHING",
        &[tid.into()],
    )
    .expect("stamping record failed");
    for sub in labeling::og_subtypes(tid) {
        if let Some(table) = types::storage_table(sub) {
            stamp_table(&table);
        }
    }
    labeling::bump_schema_version(gid, &format!("enable stamping on {type_name}"));
}

pub fn stamp_table(table: &str) {
    for (col, ty) in STAMP_COLUMNS {
        Spi::run(&format!("ALTER TABLE {table} ADD COLUMN IF NOT EXISTS {col} {ty}"))
            .unwrap_or_else(|e| error!("could not add {col} to {table}: {e}"));
    }
    Spi::run(&format!(
        "CREATE OR REPLACE TRIGGER og_a_stamp BEFORE INSERT OR UPDATE ON {table}
           FOR EACH ROW EXECUTE FUNCTION og__stamp()"
    ))
    .unwrap_or_else(|e| error!("could not attach the stamp trigger to {table}: {e}"));
}

/// Is this type — or any ancestor — stamped? New subtypes inherit it.
pub fn inherits_stamping(tid: i32) -> bool {
    spiu::one::<bool>(
        "SELECT true FROM og_catalog.stamped WHERE type_id = ANY($1) LIMIT 1",
        &[labeling::og_supertypes(tid).into()],
    )
    .ok()
    .flatten()
    .unwrap_or(false)
}

// --------------------------------------------------------------------------
// Audit
// --------------------------------------------------------------------------

thread_local! {
    /// The Action a write is running under, for the audit row.
    pub static CURRENT_ACTION: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// Write one audit row through the SECURITY DEFINER writer. `current_user` is
/// evaluated here, in the caller's context, and checked on the other side.
pub fn audit(lang: &str, graph: &str, query: &str, rows: i64, started: std::time::Instant, err: Option<&str>) {
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    let action = CURRENT_ACTION.with(|a| a.borrow().clone());
    Spi::run_with_args(
        "SELECT og__audit($1, $2, $3, $4, $5, current_user::text, $6)",
        &[
            format!("[{graph}] {query}").into(),
            lang.into(),
            rows.into(),
            ms.into(),
            err.map(|e| e.chars().take(200).collect::<String>()).into(),
            action.into(),
        ],
    )
    .ok();
}
