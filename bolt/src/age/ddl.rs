//! Neo4j's schema commands and the few procedures a client asks of a graph,
//! answered on AGE's label tables.
//!
//! AGE has no `CREATE CONSTRAINT` / `CREATE INDEX` (apache/age #1635). A
//! label here is a table (`"<graph>"."<label>"`, its properties one `agtype`
//! column), so a property index is an expression index on that table, and a
//! uniqueness constraint a unique one. AGE's executor inserts through the
//! table's indexes, so a unique index holds in Cypher writes too.
//!
//! | Neo4j | here |
//! |---|---|
//! | `… REQUIRE n.p IS UNIQUE`, `IS NODE KEY` | `CREATE UNIQUE INDEX` on `properties -> 'p'` (each property) |
//! | `CREATE [RANGE|TEXT|POINT] INDEX … ON (n.p, …)` | `CREATE INDEX` on the same expressions |
//! | `IS NOT NULL`, `FULLTEXT`, `VECTOR`, `LOOKUP` | accepted and counted, nothing built — AGE has nothing that enforces or serves them |
//! | `DROP INDEX/CONSTRAINT name` | `DROP INDEX` |
//!
//! The label's table is made if it is not there yet (`create_vlabel` /
//! `create_elabel`), because a schema script runs before any data does — and
//! unlike Ontological, a schema-less store can index a property nothing has yet.

use super::{sql_ident, sql_lit, Stats};
use cypher_syntax::ast::*;
use postgres::Client;

pub struct Failed {
    pub code: &'static str,
    pub message: String,
}

fn pg_failed(e: &postgres::Error) -> Failed {
    let message = e.as_db_error().map(|d| d.message().to_string()).unwrap_or_else(|| e.to_string());
    let code = match e.code().map(|c| c.code()) {
        Some("23505") => "Neo.ClientError.Schema.ConstraintCreationFailed",
        Some("42501") => "Neo.ClientError.Security.Forbidden",
        _ => "Neo.DatabaseError.Schema.IndexCreationFailed",
    };
    Failed { code, message }
}

/// The index's name in the graph's schema: the one given, or Neo4j's style of
/// a generated one. PostgreSQL keeps 63 bytes of a name.
fn index_name(given: &Option<String>, kind: &str, label: &str, props: &[String]) -> String {
    let n = given.clone().unwrap_or_else(|| format!("{kind}_{label}_{}", props.join("_")));
    if n.len() <= 63 {
        return n;
    }
    // keep the start, make the end unique
    let mut h: u64 = 0xcbf29ce484222325;
    for b in n.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    let mut cut = 50;
    while !n.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}_{:012x}", &n[..cut], h & 0xffff_ffff_ffff)
}

/// `properties -> 'p'`, as AGE's own property access compiles it.
fn prop_expr(p: &str) -> String {
    let key = serde_json::Value::String(p.to_string()).to_string();
    format!("ag_catalog.agtype_access_operator(VARIADIC ARRAY[properties, {}::ag_catalog.agtype])", sql_lit(&key))
}

fn label_exists(pg: &mut Client, graph: &str, label: &str, kind: &str) -> Result<Option<String>, Failed> {
    let row = pg
        .query_opt(
            "SELECT l.kind::text FROM ag_catalog.ag_label l JOIN ag_catalog.ag_graph g ON g.graphid = l.graph \
             WHERE g.name = $1::text::name AND l.name = $2::text::name",
            &[&graph, &label],
        )
        .map_err(|e| pg_failed(&e))?;
    let _ = kind;
    Ok(row.map(|r| r.get::<_, String>(0)))
}

/// Make the label's table if it is not there.
pub fn ensure_label(pg: &mut Client, graph: &str, label: &str, edge: bool) -> Result<(), Failed> {
    let want = if edge { "e" } else { "v" };
    match label_exists(pg, graph, label, want)? {
        Some(k) if k == want => Ok(()),
        Some(_) => Err(Failed {
            code: "Neo.ClientError.Schema.ConstraintCreationFailed",
            message: format!("'{label}' is already a {} label in graph '{graph}'", if edge { "vertex" } else { "relationship" }),
        }),
        None => {
            let f = if edge { "create_elabel" } else { "create_vlabel" };
            pg.batch_execute(&format!("SELECT ag_catalog.{f}({}, {})", sql_lit(graph), sql_lit(label)))
                .map_err(|e| pg_failed(&e))
        }
    }
}

fn index_exists(pg: &mut Client, graph: &str, name: &str) -> Result<bool, Failed> {
    let row = pg
        .query_opt(
            "SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1::text::name AND c.relname = $2::text::name AND c.relkind = 'i'",
            &[&graph, &name],
        )
        .map_err(|e| pg_failed(&e))?;
    Ok(row.is_some())
}

fn exists_already(what: &str, name: &str) -> Failed {
    Failed {
        code: "Neo.ClientError.Schema.EquivalentSchemaRuleAlreadyExists",
        message: format!("an {what} named '{name}' already exists"),
    }
}

/// Run one schema command. Returns what it changed.
pub fn run(pg: &mut Client, graph: &str, d: &Ddl) -> Result<Stats, Failed> {
    let mut st = Stats::default();
    match d {
        Ddl::CreateConstraint { name, label, props, kind, if_not_exists } => {
            let n = index_name(name, "constraint", label, props);
            if index_exists(pg, graph, &n)? {
                if *if_not_exists {
                    return Ok(st);
                }
                return Err(exists_already("constraint", &n));
            }
            match kind {
                ConstraintKind::Unique | ConstraintKind::NodeKey => {
                    ensure_label(pg, graph, label, false)?;
                    let cols = props.iter().map(|p| prop_expr(p)).collect::<Vec<_>>().join(", ");
                    pg.batch_execute(&format!(
                        "CREATE UNIQUE INDEX {} ON {}.{} ({cols})",
                        sql_ident(&n),
                        sql_ident(graph),
                        sql_ident(label)
                    ))
                    .map_err(|e| pg_failed(&e))?;
                }
                // Not enforced: AGE writes properties as one value, and a CHECK
                // on it would also refuse the vertex a client creates first and
                // fills in next.
                ConstraintKind::NotNull => {}
            }
            st.constraints_added = 1;
        }
        Ddl::CreateIndex { name, kind, on_relationship, label, props, if_not_exists, .. } => {
            let n = index_name(name, "index", label, props);
            if index_exists(pg, graph, &n)? {
                if *if_not_exists {
                    return Ok(st);
                }
                return Err(exists_already("index", &n));
            }
            if *kind == IndexKind::Btree {
                ensure_label(pg, graph, label, *on_relationship)?;
                let cols = props.iter().map(|p| prop_expr(p)).collect::<Vec<_>>().join(", ");
                pg.batch_execute(&format!("CREATE INDEX {} ON {}.{} ({cols})", sql_ident(&n), sql_ident(graph), sql_ident(label)))
                    .map_err(|e| pg_failed(&e))?;
            }
            st.indexes_added = 1;
        }
        Ddl::Drop { name, if_exists, constraint } => {
            if !index_exists(pg, graph, name)? {
                if *if_exists {
                    return Ok(st);
                }
                return Err(Failed {
                    code: "Neo.ClientError.Schema.ConstraintDropFailed",
                    message: format!("no {} named '{name}'", if *constraint { "constraint" } else { "index" }),
                });
            }
            pg.batch_execute(&format!("DROP INDEX {}.{}", sql_ident(graph), sql_ident(name))).map_err(|e| pg_failed(&e))?;
            if *constraint {
                st.constraints_removed = 1;
            } else {
                st.indexes_removed = 1;
            }
        }
    }
    Ok(st)
}

// ------------------------------------------------------------- procedures

/// `CALL db.labels()`, `db.relationshipTypes()`, `db.propertyKeys()` —
/// alone, or with `YIELD` and a `RETURN` of what they yield.
pub struct Procedure {
    pub which: Which,
    /// (yielded column, the name it is returned as)
    pub out: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Which {
    Labels,
    RelationshipTypes,
    PropertyKeys,
}

impl Procedure {
    pub fn recognise(q: &Query) -> Option<Procedure> {
        if q.union.is_some() {
            return None;
        }
        let (name, yields) = match q.clauses.first()? {
            Clause::Call { name, args, yields } if args.is_empty() => (name, yields),
            _ => return None,
        };
        let (which, col) = match name.to_ascii_lowercase().as_str() {
            "db.labels" => (Which::Labels, "label"),
            "db.relationshiptypes" => (Which::RelationshipTypes, "relationshipType"),
            "db.propertykeys" => (Which::PropertyKeys, "propertyKey"),
            _ => return None,
        };
        let alias = yields.iter().find(|(c, _)| c == col).map(|(_, a)| a.clone()).unwrap_or_else(|| col.to_string());
        let out = match &q.clauses[1..] {
            [] => vec![alias],
            [Clause::Return(p)] if p.order.is_empty() && p.skip.is_none() && p.limit.is_none() => {
                let mut out = Vec::new();
                for it in &p.items {
                    match &it.expr {
                        Expr::Var(v) if *v == alias => out.push(it.alias.clone().unwrap_or_else(|| v.clone())),
                        _ => return None,
                    }
                }
                out
            }
            _ => return None,
        };
        Some(Procedure { which, out })
    }

    pub fn columns(&self) -> Vec<String> {
        self.out.clone()
    }

    /// The SQL whose rows (one text column) are the answer.
    pub fn sql(&self, graph: &str) -> String {
        match self.which {
            Which::Labels | Which::RelationshipTypes => format!(
                "SELECT l.name::text FROM ag_catalog.ag_label l JOIN ag_catalog.ag_graph g ON g.graphid = l.graph \
                 WHERE g.name = {} AND l.kind = '{}' AND l.name NOT IN ('_ag_label_vertex', '_ag_label_edge') ORDER BY 1",
                sql_lit(graph),
                if self.which == Which::Labels { "v" } else { "e" }
            ),
            Which::PropertyKeys => format!(
                "SELECT DISTINCT k::text FROM (SELECT properties::jsonb AS p FROM {g}.\"_ag_label_vertex\" \
                 UNION ALL SELECT properties::jsonb FROM {g}.\"_ag_label_edge\") s, \
                 LATERAL jsonb_object_keys(s.p) AS k ORDER BY 1",
                g = sql_ident(graph)
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_fit_postgres() {
        assert_eq!(index_name(&None, "constraint", "Requirement", &["id".into()]), "constraint_Requirement_id");
        assert_eq!(index_name(&Some("c1".into()), "constraint", "L", &[]), "c1");
        let long = index_name(&None, "index", &"L".repeat(80), &["p".into()]);
        assert!(long.len() <= 63, "{long}");
        assert_ne!(long, index_name(&None, "index", &"L".repeat(81), &["p".into()]));
    }

    #[test]
    fn a_property_is_reached_as_age_reaches_it() {
        assert_eq!(prop_expr("id"), "ag_catalog.agtype_access_operator(VARIADIC ARRAY[properties, '\"id\"'::ag_catalog.agtype])");
        assert_eq!(prop_expr("it's"), "ag_catalog.agtype_access_operator(VARIADIC ARRAY[properties, '\"it''s\"'::ag_catalog.agtype])");
    }

    #[test]
    fn procedures_alone_or_returned() {
        let p = Procedure::recognise(&cypher_syntax::parse("CALL db.labels()").unwrap()).unwrap();
        assert_eq!((p.which, p.columns()), (Which::Labels, vec!["label".to_string()]));
        let p = Procedure::recognise(&cypher_syntax::parse("CALL db.propertyKeys() YIELD propertyKey AS k RETURN k AS key").unwrap()).unwrap();
        assert_eq!((p.which, p.columns()), (Which::PropertyKeys, vec!["key".to_string()]));
        assert!(Procedure::recognise(&cypher_syntax::parse("CALL db.labels() YIELD label RETURN count(label) AS n").unwrap()).is_none());
        assert!(Procedure::recognise(&cypher_syntax::parse("MATCH (n) RETURN n").unwrap()).is_none());
    }
}
