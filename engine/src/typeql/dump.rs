//! Schema → TypeQL — spec 010 T047 (FR-013).
//!
//! Round-tripping is the honest test of a schema mapping: if `define` throws
//! information away, the dump cannot reproduce the input, and SC-006 fails.

use crate::catalog::types;
use pgrx::prelude::*;

/// Render the graph's schema as a TypeQL `define` block.
#[pg_extern(stable)]
fn og_typeql_schema(graph: &str) -> String {
    let gid = types::graph_id(graph);
    let mut out = String::from("define\n");

    for kind in ['e', 'r', 'a'] {
        let rows: Vec<(i32, String, bool)> = Spi::connect(|client| {
            client
                .select(
                    "SELECT type_id, name, is_abstract FROM og_catalog.type
                      WHERE graph_id = $1 AND kind = $2::text::\"char\" AND name NOT LIKE '$%'
                      ORDER BY name",
                    None,
                    &[gid.into(), kind.to_string().into()],
                )
                .expect("type listing failed")
                .filter_map(|r| {
                    Some((r.get::<i32>(1).ok()??, r.get::<String>(2).ok()??, r.get::<bool>(3).ok()??))
                })
                .collect()
        });

        for (tid, name, is_abstract) in rows {
            let word = match kind {
                'e' => "entity",
                'r' => "relation",
                _ => "attribute",
            };
            let mut line = format!("{word} {name}");
            if let Some(parent) = parent_name(tid) {
                line.push_str(&format!(" sub {parent}"));
            }
            if is_abstract {
                line.push_str(" @abstract");
            }
            let mut clauses: Vec<String> = Vec::new();

            if kind == 'a' {
                if let Some(v) = super::schema::value_type_of(tid) {
                    if parent_name(tid).is_none() {
                        clauses.push(format!("value {v}"));
                    }
                }
            }
            for (role, parent_role) in roles_of(tid) {
                clauses.push(match parent_role {
                    Some(p) => format!("relates {role} as {p}"),
                    None => format!("relates {role}"),
                });
            }
            for target in constraints_of(tid, "owns") {
                clauses.push(format!("owns {target}"));
            }
            for target in constraints_of(tid, "plays") {
                clauses.push(format!("plays {target}"));
            }
            if clauses.is_empty() {
                out.push_str(&format!("{line};\n"));
            } else {
                out.push_str(&format!("{line},\n    {};\n", clauses.join(",\n    ")));
            }
        }
        out.push('\n');
    }

    let funs: Vec<String> = Spi::connect(|client| {
        client
            .select(
                "SELECT body FROM og_catalog.typeql_function WHERE graph_id = $1 ORDER BY name",
                None,
                &[gid.into()],
            )
            .expect("function listing failed")
            .filter_map(|r| r.get::<String>(1).ok().flatten())
            .collect()
    });
    for f in funs {
        out.push_str(&f);
        out.push_str("\n\n");
    }
    out
}

fn parent_name(tid: i32) -> Option<String> {
    crate::spiu::one::<String>(
        "SELECT p.name FROM og_catalog.type_parent tp
           JOIN og_catalog.type p ON p.type_id = tp.parent_id
          WHERE tp.type_id = $1 LIMIT 1",
        &[tid.into()],
    )
    .ok()
    .flatten()
}

fn roles_of(tid: i32) -> Vec<(String, Option<String>)> {
    Spi::connect(|client| {
        client
            .select(
                "SELECT r.name, p.name FROM og_catalog.role r
                   LEFT JOIN og_catalog.role p ON p.role_id = r.parent_role_id
                  WHERE r.rel_type_id = $1 ORDER BY r.ordinal",
                None,
                &[tid.into()],
            )
            .expect("role listing failed")
            .filter_map(|r| Some((r.get::<String>(1).ok()??, r.get::<String>(2).ok().flatten())))
            .collect()
    })
}

fn constraints_of(tid: i32, kind: &str) -> Vec<String> {
    Spi::connect(|client| {
        client
            .select(
                "SELECT target FROM og_catalog.og_constraint
                  WHERE type_id = $1 AND kind = $2 AND target IS NOT NULL ORDER BY target",
                None,
                &[tid.into(), kind.into()],
            )
            .expect("constraint listing failed")
            .filter_map(|r| r.get::<String>(1).ok().flatten())
            .collect()
    })
}
