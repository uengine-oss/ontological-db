//! Who may do what — the privilege model.
//!
//! Until this module existed there was no answer to that question. Neither SQL
//! file contained a GRANT or a REVOKE, which meant two things at once: every
//! function carried PostgreSQL's default `EXECUTE` grant to PUBLIC, and no
//! table carried any grant at all. So the extension was simultaneously
//! wide open at the function boundary and unusable by anyone but its owner —
//! a non-owner could call `og_set_setting` or `og_drop_graph`, and could not
//! run a query.
//!
//! The shape here is deliberate:
//!
//!   * **Functions default to denied.** `access.sql` revokes `EXECUTE` from
//!     PUBLIC on every `og_*` function in the extension's schema. A function
//!     added later is denied until it is named in one of the lists below, which
//!     is the direction a mistake should fail in.
//!
//!   * **Roles are not created here.** Roles are cluster-wide and outlive
//!     `DROP EXTENSION`, and `CREATE ROLE` needs privileges the installer may
//!     not have. A DBA creates the role and calls `og_grant`.
//!
//!   * **Grants are remembered.** Storage tables are created at runtime, one
//!     per concrete type, so a grant issued today has to reach a table that
//!     does not exist yet. `og_catalog.grantee` records the intent and
//!     `apply_to_table` replays it at creation time.
//!
//! Table privileges, not `EXECUTE`, are what stop a reader from writing.
//! `og_cypher` is on the read list even though Cypher can `CREATE` and
//! `DELETE`, because a compiled query reads and writes ordinary tables: a role
//! holding only `SELECT` on `og_data` gets a permission error from the
//! statement itself. That is the same boundary PostgreSQL uses everywhere else,
//! and it does not depend on us parsing the query correctly to be safe.

use crate::spiu;
use pgrx::prelude::*;

/// Introspection, query and statistics. Read privileges on the tables are what
/// make these safe; see the module note on `og_cypher`.
const READ: &[&str] = &[
    "og_apoc_meta_schema", "og_as_of", "og_build_view", "og_check_integrity",
    "og_csr_hops",
    "og_csr_reach", "og_csr_stats", "og_cypher", "og_cypher_check",
    "og_cypher_columns", "og_cypher_explain", "og_cypher_json", "og_cypher_sql",
    "og_cypher_stats", "og_degree", "og_degree_all", "og_degree_distribution",
    "og_diagnose_empty", "og_dump_rdf", "og_edge_json", "og_edges",
    "og_embedding_stats", "og_estimate", "og_expand", "og_expand_batch",
    "og_explain_error", "og_graph_stats", "og_history", "og_hybrid_search",
    "og_id_local", "og_id_shard", "og_id_type", "og_interop_report",
    "og_is_subtype", "og_make_id", "og_mapping_report", "og_node_json",
    "og_nodes", "og_prop", "og_reach", "og_reach_sql", "og_schema",
    "og_schema_for", "og_similar", "og_stale_embeddings", "og_subtype_ids",
    "og_subtypes", "og_supertypes", "og_type_id", "og_type_name", "og_typeql",
    "og_typeql_check", "og_typeql_schema", "og_typeql_sql", "og_vector_search",
    "og_vector_search_exact", "og_vlp",
];

/// Data manipulation. `og_genai_encode` is here rather than under `admin`
/// because the embedding pipeline is a writer's job — the endpoint it talks to
/// is chosen by `og_set_setting`, which is not.
const WRITE: &[&str] = &[
    "og_add_role_player", "og_apply_role", "og_capture_history", "og_create_edge",
    "og_create_node", "og_csr_build", "og_csr_drop", "og_delete_edge",
    "og_delete_node", "og_genai_encode", "og_load_rdf", "og_mark_embedded",
    "og_set_node_props",
];

/// Everything else: schema change, catalog mutation, settings, mapping,
/// anything that rewrites or drops storage. Not enumerated — a function absent
/// from `READ` and `WRITE` lands here, so a new one is admin-only until someone
/// decides otherwise.
/// The graph name that means "every graph" — what `og_grant` did before it
/// could be scoped, and what an unscoped grant still records.
pub const ALL_GRAPHS: &str = "*";

/// Every storage table and view backing one graph's types, schema-qualified and
/// ready to interpolate.
///
/// Both shapes are here on purpose. The storage table holds the rows; the views
/// (`v_<tid>`, and the alias view named after the type) are what a compiled
/// query actually reads, and `security_invoker` means the caller needs SELECT on
/// each of them in its own right.
fn graph_storage(graph: &str) -> Vec<String> {
    Spi::connect(|client| {
        client
            .select(
                "SELECT 'og_data.' || quote_ident(c.relname)
                   FROM og_catalog.type t
                   JOIN og_catalog.graph g ON g.graph_id = t.graph_id
                   JOIN pg_class c
                     ON c.relname IN (split_part(t.storage_table, '.', 2),
                                      'v_'  || t.type_id,
                                      've_' || t.type_id,
                                      t.name)
                   JOIN pg_namespace n
                     ON n.oid = c.relnamespace AND n.nspname = 'og_data'
                  WHERE g.name = $1 AND c.relkind IN ('r', 'v')",
                None,
                &[graph.into()],
            )
            .map(|rows| rows.filter_map(|r| r.get::<String>(1).ok().flatten()).collect())
            .unwrap_or_default()
    })
}

fn level_rank(level: &str) -> i32 {
    match level {
        "read" => 1,
        "write" => 2,
        "admin" => 3,
        _ => error!("level must be one of 'read', 'write', 'admin' — got '{level}'"),
    }
}

/// Grant a role standing privileges on this extension's schemas.
///
/// The role must already exist; this deliberately does not create it. Levels
/// nest: `write` includes `read`, `admin` includes both.
#[pg_extern]
fn og_grant(
    role: &str,
    level: default!(&str, "'read'"),
    graph: default!(&str, "'*'"),
) {
    let rank = level_rank(level);
    // '*' is every graph — the behaviour this function had before it could be
    // scoped. Any other value must name a graph that exists, because a typo
    // would otherwise record a grant that silently covers nothing.
    if graph != ALL_GRAPHS
        && spiu::one::<i32>(
            "SELECT 1 FROM og_catalog.graph WHERE name = $1",
            &[graph.into()],
        )
        .ok()
        .flatten()
        .is_none()
    {
        error!("graph '{graph}' does not exist");
    }
    if spiu::one::<i32>("SELECT 1 FROM pg_roles WHERE rolname = $1", &[role.into()])
        .ok()
        .flatten()
        .is_none()
    {
        error!("role '{role}' does not exist — create it first, then call og_grant");
    }
    let r = spiu::ident(role);

    Spi::run(&format!("GRANT USAGE ON SCHEMA og_catalog, og_data TO {r}"))
        .unwrap_or_else(|e| error!("schema usage grant failed: {e}"));
    // The catalog is handed over whole: the compiler reads it to resolve every
    // label and property, so a scoped role still needs all of it. That means a
    // role scoped to one graph can enumerate another graph's label names —
    // metadata, never rows. Closing that gap is an RLS policy on og_catalog,
    // not a GRANT.
    Spi::run(&format!("GRANT SELECT ON ALL TABLES IN SCHEMA og_catalog TO {r}"))
        .unwrap_or_else(|e| error!("catalog select grant failed: {e}"));
    if graph == ALL_GRAPHS {
        Spi::run(&format!("GRANT SELECT ON ALL TABLES IN SCHEMA og_data TO {r}"))
            .unwrap_or_else(|e| error!("select grant failed: {e}"));
    } else {
        // Shared spines. og_node carries (id, type_id) and og_adj the edge
        // arrays, neither of which is per-graph, so both are handed over whole;
        // the rows themselves live in the per-type tables below and stay shut.
        Spi::run(&format!(
            "GRANT SELECT ON og_data.og_node, og_data.og_adj, og_data.og_edge TO {r}"
        ))
        .unwrap_or_else(|e| error!("spine select grant failed: {e}"));
        for t in graph_storage(graph) {
            Spi::run(&format!("GRANT SELECT ON {t} TO {r}"))
                .unwrap_or_else(|e| error!("select grant on {t} failed: {e}"));
        }
    }
    // SELECT, not USAGE: reading a sequence's `last_value` is not permission to
    // draw from it. Every Cypher read needs this one — the plan cache is keyed
    // on the schema counter, and that counter is a sequence.
    Spi::run(&format!(
        "GRANT SELECT ON ALL SEQUENCES IN SCHEMA og_catalog TO {r}"
    ))
    .unwrap_or_else(|e| error!("catalog sequence read grant failed: {e}"));
    // Every query — a read included — appends a row to og_audit, and the insert
    // runs inside the caller's transaction. Without this a `read` role cannot
    // run one statement: the audit write aborts first and the query never gets
    // to report its own result. Auditing is the extension's own bookkeeping,
    // not the caller's data, so it belongs at the lowest level rather than
    // behind `write`.
    Spi::run(&format!(
        "GRANT INSERT ON og_data.og_audit TO {r}"
    ))
    .unwrap_or_else(|e| error!("audit insert grant failed: {e}"));
    Spi::run(&format!(
        "GRANT USAGE, SELECT ON SEQUENCE og_data.og_audit_audit_id_seq TO {r}"
    ))
    .unwrap_or_else(|e| error!("audit sequence grant failed: {e}"));

    if rank >= 2 {
        // Writers touch og_data only. Promotion of an undeclared property is a
        // schema change and stays on the admin side of the line; the write path
        // already falls back to the `__ext` column when it cannot promote.
        if graph == ALL_GRAPHS {
            Spi::run(&format!(
                "GRANT INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA og_data TO {r}"
            ))
            .unwrap_or_else(|e| error!("dml grant failed: {e}"));
        } else {
            Spi::run(&format!(
                "GRANT INSERT, UPDATE, DELETE ON og_data.og_node, og_data.og_adj, \
                 og_data.og_edge TO {r}"
            ))
            .unwrap_or_else(|e| error!("spine dml grant failed: {e}"));
            for t in graph_storage(graph) {
                Spi::run(&format!("GRANT INSERT, UPDATE, DELETE ON {t} TO {r}"))
                    .unwrap_or_else(|e| error!("dml grant on {t} failed: {e}"));
            }
        }
        Spi::run(&format!(
            "GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA og_data TO {r}"
        ))
        .unwrap_or_else(|e| error!("sequence grant failed: {e}"));
    }
    if rank >= 3 {
        Spi::run(&format!(
            "GRANT INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA og_catalog TO {r}"
        ))
        .unwrap_or_else(|e| error!("catalog dml grant failed: {e}"));
        Spi::run(&format!(
            "GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA og_catalog TO {r}"
        ))
        .unwrap_or_else(|e| error!("catalog sequence grant failed: {e}"));
    }

    grant_functions(&r, rank);

    Spi::run_with_args(
        "INSERT INTO og_catalog.grantee (role, level, graph) VALUES ($1, $2, $3)
         ON CONFLICT (role, graph) DO UPDATE SET level = EXCLUDED.level",
        &[role.into(), level.into(), graph.into()],
    )
    .unwrap_or_else(|e| error!("could not record the grant: {e}"));
}

/// Take back everything `og_grant` handed out.
///
/// The role itself is left alone, for the same reason `og_grant` does not
/// create it: its lifetime is the DBA's business, not ours.
#[pg_extern]
fn og_revoke(role: &str, graph: default!(&str, "'*'")) {
    let r = spiu::ident(role);
    // Scoped revoke takes back one project and leaves the rest of the role's
    // access — and the function grants — alone. Only '*' clears the role out.
    if graph != ALL_GRAPHS {
        for t in graph_storage(graph) {
            Spi::run(&format!("REVOKE ALL ON {t} FROM {r}"))
                .unwrap_or_else(|e| error!("revoke on {t} failed: {e}"));
        }
        Spi::run_with_args(
            "DELETE FROM og_catalog.grantee WHERE role = $1 AND graph = $2",
            &[role.into(), graph.into()],
        )
        .unwrap_or_else(|e| error!("could not clear the grant record: {e}"));
        return;
    }
    for stmt in [
        format!("REVOKE ALL ON ALL TABLES IN SCHEMA og_catalog, og_data FROM {r}"),
        format!("REVOKE ALL ON ALL SEQUENCES IN SCHEMA og_catalog, og_data FROM {r}"),
        format!(
            "REVOKE ALL ON ALL FUNCTIONS IN SCHEMA {} FROM {r}",
            spiu::ident(&ext_schema())
        ),
        format!("REVOKE USAGE ON SCHEMA og_catalog, og_data FROM {r}"),
    ] {
        Spi::run(&stmt).unwrap_or_else(|e| error!("revoke failed: {e}"));
    }
    Spi::run_with_args("DELETE FROM og_catalog.grantee WHERE role = $1", &[role.into()])
        .unwrap_or_else(|e| error!("could not clear the grant record: {e}"));
}

/// The schema the extension was installed into, unquoted.
///
/// Returned raw because it is used two ways — compared against `nspname` as a
/// value, and written into DDL as an identifier — and quoting it here would mean
/// un-quoting it there. Round-tripping through `ident` and back is exactly where
/// a name containing a quote character stops surviving. `relocatable = false`
/// fixes the schema at install time, so this is a lookup rather than a guess.
fn ext_schema() -> String {
    spiu::one::<String>(
        "SELECT n.nspname FROM pg_extension e
           JOIN pg_namespace n ON n.oid = e.extnamespace
          WHERE e.extname = 'ontological'",
        &[],
    )
    .ok()
    .flatten()
    .unwrap_or_else(|| "public".into())
}

/// Grant `EXECUTE` on every overload of every function at or below `rank`.
///
/// Overloads matter: several of these are exposed at more than one arity, and
/// naming the function without its argument types would only reach one of them.
fn grant_functions(quoted_role: &str, rank: i32) {
    let mut names: Vec<&str> = READ.to_vec();
    if rank >= 2 {
        names.extend_from_slice(WRITE);
    }
    let schema = ext_schema();

    let sigs: Vec<String> = if rank >= 3 {
        // Admin gets the whole surface, including everything not enumerated.
        Spi::connect(|client| {
            client
                .select(
                    "SELECT p.oid::regprocedure::text FROM pg_proc p
                       JOIN pg_namespace n ON n.oid = p.pronamespace
                      WHERE n.nspname = $1 AND p.proname LIKE 'og\\_%'",
                    None,
                    &[schema.as_str().into()],
                )
                .map(|rows| rows.filter_map(|r| r.get::<String>(1).ok().flatten()).collect())
                .unwrap_or_default()
        })
    } else {
        let list = names.join(",");
        Spi::connect(|client| {
            client
                .select(
                    "SELECT p.oid::regprocedure::text FROM pg_proc p
                       JOIN pg_namespace n ON n.oid = p.pronamespace
                      WHERE n.nspname = $1
                        AND p.proname = ANY (string_to_array($2, ','))",
                    None,
                    &[schema.as_str().into(), list.as_str().into()],
                )
                .map(|rows| rows.filter_map(|r| r.get::<String>(1).ok().flatten()).collect())
                .unwrap_or_default()
        })
    };

    for sig in sigs {
        Spi::run(&format!("GRANT EXECUTE ON FUNCTION {sig} TO {quoted_role}"))
            .unwrap_or_else(|e| error!("execute grant on {sig} failed: {e}"));
    }
}

/// Re-issue the recorded grants against a table or view created just now.
///
/// Called from every site that creates storage at runtime. Failures are logged
/// rather than raised: a missing grant must not be the reason a type cannot be
/// created, and `og_grant` re-run repairs it.
pub fn apply_to_table(tid: i32, table: &str) {
    for (role, level) in reaching(tid) {
        let r = spiu::ident(&role);
        // Interpolated as given, not re-quoted. Every caller builds its own DDL
        // from this same string, and one of them — the alias view named after
        // the type — hands over a name that is already quoted. Running it
        // through `qname` a second time produced `"og_data"."""Name"""`,
        // and because the failed GRANT aborts the surrounding transaction, that
        // turned "a grant is recorded" into "no new label can be created".
        let t = table;
        if let Err(e) = Spi::run(&format!("GRANT SELECT ON {t} TO {r}")) {
            pgrx::log!("could not grant SELECT on {table} to {role}: {e}");
        }
        if level_rank(&level) >= 2 {
            if let Err(e) = Spi::run(&format!("GRANT INSERT, UPDATE, DELETE ON {t} TO {r}")) {
                pgrx::log!("could not grant DML on {table} to {role}: {e}");
            }
        }
    }
}

/// A view is read-only from the caller's side, so only `SELECT` is replayed.
pub fn apply_to_view(tid: i32, view: &str) {
    for (role, _) in reaching(tid) {
        let r = spiu::ident(&role);
        if let Err(e) = Spi::run(&format!("GRANT SELECT ON {view} TO {r}")) {
            pgrx::log!("could not grant SELECT on {view} to {role}: {e}");
        }
    }
}

/// The recorded grants that reach a newly created type: the unscoped ones, plus
/// the ones naming that type's own graph.
///
/// Replaying every recorded grant onto every new table — which is what this used
/// to do — is what made graph scoping decay. A role confined to one project
/// stayed confined only until the next label was created anywhere.
fn reaching(tid: i32) -> Vec<(String, String)> {
    Spi::connect(|client| {
        client
            .select(
                "SELECT gr.role, gr.level
                   FROM og_catalog.grantee gr
                  WHERE gr.graph = $1
                     OR gr.graph = (SELECT g.name
                                      FROM og_catalog.type t
                                      JOIN og_catalog.graph g ON g.graph_id = t.graph_id
                                     WHERE t.type_id = $2)",
                None,
                &[ALL_GRAPHS.into(), tid.into()],
            )
            .map(|rows| {
                rows.filter_map(|r| Some((r.get::<String>(1).ok()??, r.get::<String>(2).ok()??)))
                    .collect()
            })
            .unwrap_or_default()
    })
}

fn recorded() -> Vec<(String, String)> {
    Spi::connect(|client| {
        client
            .select("SELECT role, level FROM og_catalog.grantee ORDER BY role, graph", None, &[])
            .map(|rows| {
                rows.filter_map(|r| {
                    Some((r.get::<String>(1).ok()??, r.get::<String>(2).ok()??))
                })
                .collect()
            })
            .unwrap_or_default()
    })
}
