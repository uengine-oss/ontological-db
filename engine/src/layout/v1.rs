//! `LayoutV1` — the storage layout `ontological` has always had: a registry
//! (`og_data.og_node` / `og_edge`), a table per type (`og_data.n_<tid>`,
//! `e_<tid>`, `a_<tid>`, history `h_*`) with a column per declared property
//! (`p_*`) and the rest in `__ext`, adjacency segments (`og_data.og_adj`), union
//! views per type (`v_` / `vp_` / `ve_` / `vep_`).
//!
//! **An empty skeleton (K1).** Every method is a `todo!` naming the card that
//! moves the v1 code here: K2 the Cypher read compiler (`cypher/compile.rs`,
//! `cypher/views.rs`), K3 the write path (`cypher/mod.rs`, `storage/`), K4
//! TypeQL (`typeql/compile.rs`, `write.rs`, `schema.rs`), K5 traversal
//! (`storage/traverse.rs`). What each must produce is on the trait
//! ([`super::Layout`]): v1's text today, byte for byte. Nothing calls this yet,
//! so none of these panics can be reached.

use super::{Aliases, Dir, ElementId, GraphId, Kind, LabelSet, Layout, PathSem, PropRead, SqlExpr, SqlFrag, TimeScope, TypeId};
use super::OwnershipJoin;
use serde_json::Value;

/// The v1 layout. A unit struct: everything it needs is in the catalog.
#[derive(Clone, Copy, Debug, Default)]
pub struct LayoutV1;

/// The TypeQL methods (K4) spell what `typeql/compile.rs` wrote before, byte for byte.
#[cfg(test)]
mod k4_typeql {
    use super::super::{Kind, Layout, SqlExpr, SqlFrag};
    use super::LayoutV1;

    #[test]
    fn registry_and_role_players() {
        assert_eq!(LayoutV1.registry(Kind::Node).as_str(), "og_data.og_node");
        assert_eq!(LayoutV1.registry(Kind::Rel).as_str(), "og_data.og_edge");
        assert_eq!(LayoutV1.role_players().as_str(), "og_data.og_role_player");
    }

    #[test]
    fn the_type_of_an_id_is_a_shift_and_mask() {
        assert_eq!(
            LayoutV1.label_of_inline(&SqlExpr::new("n1.id")).as_str(),
            "(((n1.id) >> 36) & 262143)::int4"
        );
    }

    #[test]
    fn the_has_join_is_the_adjacency_segments() {
        let j = LayoutV1.ownership_join(
            &SqlExpr::new("n1.id"),
            7,
            &[12, 13],
            &SqlFrag::new("(SELECT id, val FROM og_data.a_12)"),
            "h2",
        );
        assert_eq!(
            j.from.as_str(),
            "og_data.og_adj h2a CROSS JOIN LATERAL unnest(h2a.nbr) AS h2u(nbr) \
             JOIN (SELECT id, val FROM og_data.a_12) h2v ON h2v.id = h2u.nbr"
        );
        let conds: Vec<&str> = j.conds.iter().map(|c| c.as_str()).collect();
        assert_eq!(
            conds,
            [
                "h2a.src = n1.id",
                "h2a.dir = 'o'::\"char\"",
                "h2a.etype = 7",
                "(((h2u.nbr) >> 36) & 262143)::int4 = ANY(ARRAY[12,13]::int4[])",
            ]
        );
    }
}

impl Layout for LayoutV1 {
    fn node_scan(&self, g: GraphId, labels: &LabelSet, _at: TimeScope) -> SqlFrag {
        match labels {
            // The union view reads a temporal type's history at the session's
            // instant itself, so the instant does not change the name.
            LabelSet::Under(t) => SqlFrag::new(crate::cypher::views::ensure_view(*t, false)),
            // The registry is shared by every graph — a node's graph is its
            // type's — so it is narrowed to `g`'s types, looked up when the
            // statement runs (a type created meanwhile must not be missed by a
            // cached plan). It holds only the present; the compiler guards an
            // instant (`og_require_current`).
            LabelSet::Any => SqlFrag::new(graph_registry(g)),
            LabelSet::Exactly(_) => {
                unimplemented!("LayoutV1::node_scan: v1 compiles no scan of an exact type set (TypeQL filters the registry)")
            }
        }
    }

    fn edge_scan(&self, _g: GraphId, types: &LabelSet, _at: TimeScope) -> SqlFrag {
        match types {
            LabelSet::Under(t) => SqlFrag::new(crate::cypher::views::ensure_view(*t, true)),
            _ => unimplemented!("LayoutV1::edge_scan: v1 scans relationships of one type only (others are read by id)"),
        }
    }

    fn registry(&self, kind: Kind) -> SqlFrag {
        SqlFrag::new(match kind {
            Kind::Node => "og_data.og_node",
            Kind::Rel => "og_data.og_edge",
        })
    }

    fn role_players(&self) -> SqlFrag {
        SqlFrag::new("og_data.og_role_player")
    }

    fn attr_values(&self, t: TypeId) -> Option<SqlFrag> {
        crate::catalog::types::storage_table(t).map(|tbl| SqlFrag::new(format!("SELECT id, val FROM {tbl}")))
    }

    fn ownership_join(
        &self,
        owner: &SqlExpr,
        has_type: TypeId,
        attr_types: &[TypeId],
        values: &SqlFrag,
        h: &str,
    ) -> OwnershipJoin {
        let ids = attr_types.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let nbr = SqlExpr::new(format!("{h}u.nbr"));
        OwnershipJoin {
            from: SqlFrag::new(format!(
                "og_data.og_adj {h}a CROSS JOIN LATERAL unnest({h}a.nbr) AS {h}u(nbr) \
                 JOIN {values} {h}v ON {h}v.id = {h}u.nbr"
            )),
            conds: vec![
                SqlExpr::new(format!("{h}a.src = {owner}")),
                SqlExpr::new(format!("{h}a.dir = 'o'::\"char\"")),
                SqlExpr::new(format!("{h}a.etype = {has_type}")),
                SqlExpr::new(format!("{} = ANY(ARRAY[{ids}]::int4[])", self.label_of_inline(&nbr))),
            ],
        }
    }

    fn label_of_inline(&self, id: &SqlExpr) -> SqlExpr {
        SqlExpr::new(format!("((({id}) >> {}) & {})::int4", crate::id::TYPE_SHIFT, crate::id::TYPE_MASK))
    }

    fn expand(
        &self,
        start: &SqlExpr,
        types: Option<&[TypeId]>,
        dir: Dir,
        temporal: &[TypeId],
        inner: &str,
        names: &mut dyn Aliases,
    ) -> SqlFrag {
        let type_pred = match types {
            Some(ids) => format!(" AND {inner}.etype = ANY({})", int4_array(ids)),
            None => String::new(),
        };
        SqlFrag::new(stored_hop_rows(start, inner, dir, &type_pred, temporal, names))
    }

    fn expand_except(
        &self,
        start: &SqlExpr,
        except: &[TypeId],
        dir: Dir,
        temporal: &[TypeId],
        inner: &str,
        names: &mut dyn Aliases,
    ) -> SqlFrag {
        let type_pred = format!(" AND {inner}.etype <> ALL({})", int4_array(except));
        SqlFrag::new(stored_hop_rows(start, inner, dir, &type_pred, temporal, names))
    }

    fn reach(
        &self,
        start: &SqlExpr,
        types: Option<&[TypeId]>,
        dir: Dir,
        min: u32,
        max: u32,
        sem: PathSem,
    ) -> SqlFrag {
        let f = match sem {
            PathSem::Trails => "og_vlp",
            PathSem::Reach => "og_reach",
        };
        let types = match types {
            Some(ids) => int4_array(ids),
            None => "NULL::int4[]".to_string(),
        };
        let d = match dir {
            Dir::Out => "'o'",
            Dir::In => "'i'",
            Dir::Both => "'b'",
        };
        SqlFrag::new(format!("{f}({start}, {types}, {d}::\"char\", {min}, {max})"))
    }

    fn walk_rels(&self, walk: &str) -> SqlExpr {
        // `og_vlp`'s `path` is the edge ids in order.
        SqlExpr::new(format!(
            "(CASE WHEN {walk}.path IS NULL THEN NULL ELSE (SELECT coalesce(jsonb_agg(og_edge_json(vx.e) \
             ORDER BY vx.o), '[]'::jsonb) FROM unnest({walk}.path) WITH ORDINALITY AS vx(e, o)) END)"
        ))
    }

    fn walk_nodes(&self, start: &SqlExpr, walk: &str) -> SqlExpr {
        // Each edge's other end, walked from the start — the only reading that
        // is right for undirected hops too.
        SqlExpr::new(format!(
            "(CASE WHEN {walk}.path IS NULL THEN NULL ELSE (WITH RECURSIVE pn(i, node) AS (\
             SELECT 0, {start} UNION ALL \
             SELECT pn.i + 1, CASE WHEN pe.src = pn.node THEN pe.dst ELSE pe.src END \
               FROM pn JOIN og_data.og_edge pe ON pe.id = ({walk}.path)[pn.i + 1] \
              WHERE pn.i < cardinality({walk}.path)) \
             SELECT coalesce(jsonb_agg(og_node_json(pn.node) ORDER BY pn.i), '[]'::jsonb) \
               FROM pn WHERE pn.i > 0) END)"
        ))
    }

    // ---- K5: traversal and storage upkeep. `storage/traverse.rs` and
    // `storage/stats.rs` keep the walks and the SQL functions; the SQL they ran
    // is here, as it was there. Multi-line SQL below keeps its old columns on
    // the continuation lines: the text is compared byte for byte (the mold's
    // trace layer), so it is not re-indented with the code around it.

    fn frontier_step(&self, dir: Dir, frontier: &str, types: &str) -> String {
        // The type ids go in as a bound parameter rather than a literal list:
        // a thousand-element list rewritten per level would be its own cost.
        let dir = match dir {
            Dir::Out => "a.dir = 'o'::\"char\"",
            Dir::In => "a.dir = 'i'::\"char\"",
            Dir::Both => "a.dir IN ('o','i')",
        };
        format!(
            "SELECT a.nbr FROM og_data.og_adj a WHERE a.src = ANY({frontier}::int8[]) \
             AND {dir} AND ({types}::int4[] IS NULL OR a.etype = ANY({types}::int4[]))"
        )
    }

    fn topology_scan(&self, types: &str) -> String {
        // One scan covers both directions: og_adj stores an 'i' segment for
        // every 'o' segment, so the reverse CSR costs no extra I/O.
        format!(
            "SELECT a.src, a.dir::text, a.nbr FROM og_data.og_adj a \
             WHERE a.dir IN ('o','i') AND ({types}::int4[] IS NULL OR a.etype = ANY({types}::int4[]))"
        )
    }

    fn graph_stats(&self, gid: GraphId, graph: &str) -> Value {
        let (nodes, edges) = crate::spiu::two::<i64, i64>(
            "SELECT (SELECT count(*) FROM og_data.og_node n
                   JOIN og_catalog.type t ON t.type_id = n.type_id WHERE t.graph_id = $1),
                (SELECT count(*) FROM og_data.og_edge e
                   JOIN og_catalog.type t ON t.type_id = e.type_id WHERE t.graph_id = $1)",
            &[gid.into()],
        )
        .expect("count failed");

        let types: Vec<Value> = pgrx::spi::Spi::connect(|client| {
            client
                .select(
                    "SELECT t.name, t.kind::text, t.is_abstract,
                        COALESCE((SELECT count(*) FROM og_data.og_node n WHERE n.type_id = t.type_id), 0)
                      + COALESCE((SELECT count(*) FROM og_data.og_edge e WHERE e.type_id = t.type_id), 0)
                   FROM og_catalog.type t WHERE t.graph_id = $1 ORDER BY t.name",
                    None,
                    &[gid.into()],
                )
                .expect("type stats failed")
                .map(|r| {
                    serde_json::json!({
                        "name": r.get::<String>(1).unwrap(),
                        "kind": r.get::<String>(2).unwrap(),
                        "abstract": r.get::<bool>(3).unwrap().unwrap_or(false),
                        "instances": r.get::<i64>(4).unwrap().unwrap_or(0),
                    })
                })
                .collect()
        });

        let (segments, fill, supernodes) = pgrx::spi::Spi::connect(|client| {
            client
                .select(
                    "SELECT count(*)::int8,
                        COALESCE(avg(n)::float8, 0),
                        count(*) FILTER (WHERE seq > 0)::int8
                   FROM og_data.og_adj",
                    None,
                    &[],
                )
                .expect("segment stats failed")
                .next()
                .map(|r| {
                    (
                        r.get::<i64>(1).unwrap().unwrap_or(0),
                        r.get::<f64>(2).unwrap().unwrap_or(0.0),
                        r.get::<i64>(3).unwrap().unwrap_or(0),
                    )
                })
                .unwrap_or((0, 0.0, 0))
        });

        let chunk = crate::storage::adjacency::CHUNK as f64;
        serde_json::json!({
            "graph": graph,
            "nodes": nodes.unwrap_or(0),
            "edges": edges.unwrap_or(0),
            "types": types,
            "adjacency": {
                "segments": segments,
                "avg_fill": fill,
                "chunk_size": chunk,
                // 1.0 = perfectly packed; lower means reorganisation will help.
                "packing_ratio": if segments > 0 { fill / chunk } else { 1.0 },
                "chunked_supernodes": supernodes,
            }
        })
    }

    fn degree_distribution(&self, gid: GraphId) -> Vec<Value> {
        pgrx::spi::Spi::connect(|client| {
            client
                .select(
                    "WITH d AS (
                     SELECT a.src, sum(a.n) AS deg
                       FROM og_data.og_adj a
                       JOIN og_data.og_node nd ON nd.id = a.src
                       JOIN og_catalog.type t ON t.type_id = nd.type_id AND t.graph_id = $1
                      WHERE a.dir = 'o'
                      GROUP BY a.src)
                 SELECT width_bucket(deg, 0, 1024, 8) AS b, count(*)::int8, max(deg)::int8
                   FROM d GROUP BY b ORDER BY b",
                    None,
                    &[gid.into()],
                )
                .expect("degree distribution failed")
                .map(|r| {
                    serde_json::json!({
                        "bucket": r.get::<i32>(1).unwrap(),
                        "nodes": r.get::<i64>(2).unwrap().unwrap_or(0),
                        "max_degree": r.get::<i64>(3).unwrap().unwrap_or(0),
                    })
                })
                .collect()
        })
    }

    fn reorganize(&self, gid: GraphId) -> i64 {
        let targets: Vec<(i64, i32, String)> = pgrx::spi::Spi::connect(|client| {
            client
                .select(
                    "SELECT a.src, a.etype, a.dir::text
                   FROM og_data.og_adj a
                   JOIN og_data.og_node nd ON nd.id = a.src
                   JOIN og_catalog.type t ON t.type_id = nd.type_id AND t.graph_id = $1
                  GROUP BY a.src, a.etype, a.dir
                 HAVING count(*) > 1 AND sum(a.n) <= count(*) * $2 - $2",
                    None,
                    &[gid.into(), crate::storage::adjacency::CHUNK.into()],
                )
                .expect("reorg scan failed")
                .filter_map(|r| {
                    Some((r.get::<i64>(1).ok()??, r.get::<i32>(2).ok()??, r.get::<String>(3).ok()??))
                })
                .collect()
        });

        let mut repacked = 0i64;
        for (src, etype, dir) in targets {
            pgrx::spi::Spi::run_with_args(
                "WITH flat AS (
                 SELECT u.nbr, u.eid,
                        ((row_number() OVER (ORDER BY a.seq)) - 1) / $4 AS newseq
                   FROM og_data.og_adj a, LATERAL unnest(a.nbr, a.eid) AS u(nbr, eid)
                  WHERE a.src = $1 AND a.etype = $2 AND a.dir = $3::text::\"char\"),
             packed AS (
                 SELECT newseq, count(*)::int4 AS n,
                        array_agg(nbr) AS nbr, array_agg(eid) AS eid
                   FROM flat GROUP BY newseq),
             wipe AS (
                 DELETE FROM og_data.og_adj
                  WHERE src = $1 AND etype = $2 AND dir = $3::text::\"char\")
             INSERT INTO og_data.og_adj (src, etype, dir, seq, n, nbr, eid)
             SELECT $1, $2, $3::text::\"char\", newseq::int4, n, nbr, eid FROM packed",
                &[src.into(), etype.into(), dir.into(), (crate::storage::adjacency::CHUNK as i64).into()],
            )
            .expect("repack failed");
            repacked += 1;
        }
        repacked
    }

    fn check_integrity(&self) -> Vec<(String, Option<ElementId>, String)> {
        let mut findings: Vec<(String, Option<i64>, String)> = Vec::new();

        pgrx::spi::Spi::connect(|client| {
            // 1. adjacency entries pointing at edges that no longer exist
            let rows = client
                .select(
                    "SELECT DISTINCT e FROM og_data.og_adj a, LATERAL unnest(a.eid) AS e
                  WHERE NOT EXISTS (SELECT 1 FROM og_data.og_edge x WHERE x.id = e) LIMIT 100",
                    None,
                    &[],
                )
                .expect("integrity scan 1 failed");
            for r in rows {
                findings.push((
                    "dangling_adjacency".into(),
                    r.get::<i64>(1).ok().flatten(),
                    "adjacency references a non-existent edge".into(),
                ));
            }

            // 2. edges whose adjacency entry is missing in either direction
            let rows = client
                .select(
                    "SELECT e.id FROM og_data.og_edge e
                  WHERE NOT EXISTS (
                        SELECT 1 FROM og_data.og_adj a
                         WHERE a.src = e.src AND a.dir = 'o' AND e.id = ANY(a.eid))
                     OR NOT EXISTS (
                        SELECT 1 FROM og_data.og_adj a
                         WHERE a.src = e.dst AND a.dir = 'i' AND e.id = ANY(a.eid))
                  LIMIT 100",
                    None,
                    &[],
                )
                .expect("integrity scan 2 failed");
            for r in rows {
                findings.push((
                    "missing_adjacency".into(),
                    r.get::<i64>(1).ok().flatten(),
                    "edge is not reachable from one of its endpoints".into(),
                ));
            }

            // 3. array length vs n
            let rows = client
                .select(
                    "SELECT src FROM og_data.og_adj
                  WHERE n <> COALESCE(array_length(nbr,1),0)
                     OR COALESCE(array_length(nbr,1),0) <> COALESCE(array_length(eid,1),0)
                  LIMIT 100",
                    None,
                    &[],
                )
                .expect("integrity scan 3 failed");
            for r in rows {
                findings.push((
                    "segment_length_mismatch".into(),
                    r.get::<i64>(1).ok().flatten(),
                    "segment count does not match array length".into(),
                ));
            }

            // 4. registry vs typed table
            let rows = client
                .select(
                    "SELECT n.id FROM og_data.og_node n
                  WHERE NOT EXISTS (SELECT 1 FROM og_catalog.type t WHERE t.type_id = n.type_id)
                  LIMIT 100",
                    None,
                    &[],
                )
                .expect("integrity scan 4 failed");
            for r in rows {
                findings.push((
                    "orphan_node".into(),
                    r.get::<i64>(1).ok().flatten(),
                    "node references an unknown type".into(),
                ));
            }
        });

        findings
    }

    fn prop(&self, alias: &str, scanned: Option<TypeId>, key: &str) -> PropRead {
        use crate::cypher::compile::sql_str;
        if key == "_id" {
            return PropRead { sql: SqlExpr::new(format!("{alias}.id")), sql_type: Some("int8".into()) };
        }
        if let Some(t) = scanned {
            // A column of the view, or — over a wide view — a typed read of
            // the packed value (views::Layout).
            if let Some((sql, dt)) = crate::cypher::views::view_layout(t).read(alias, key) {
                return PropRead { sql: SqlExpr::new(sql), sql_type: Some(dt) };
            }
        }
        let sql = match scanned {
            // Typed view: the property is not declared, so it can only live in
            // the extension payload.
            Some(_) => format!("({alias}.__ext->>{})", sql_str(key)),
            // A registry row: resolved through the catalog at run time.
            None => format!("(og_node_json({alias}.id)->>{})", sql_str(key)),
        };
        PropRead { sql: SqlExpr::new(sql), sql_type: None }
    }

    fn prop_json(&self, alias: &str, scanned: Option<TypeId>, key: &str) -> SqlExpr {
        use crate::cypher::compile::sql_str;
        if key == "_id" {
            return SqlExpr::new(format!("to_jsonb({alias}.id)"));
        }
        if let Some(t) = scanned {
            if let Some(sql) = crate::cypher::views::view_layout(t).read_json(alias, key) {
                return SqlExpr::new(sql);
            }
            return SqlExpr::new(format!("({alias}.__ext -> {})", sql_str(key)));
        }
        SqlExpr::new(format!("(og_node_json({alias}.id) -> {})", sql_str(key)))
    }

    fn prop_type(&self, t: TypeId, key: &str) -> Option<String> {
        crate::cypher::views::view_properties(t).get(key).map(|(_, dt)| dt.clone())
    }

    fn element_json(&self, alias: &str, scanned: Option<TypeId>, kind: Kind) -> SqlExpr {
        use crate::cypher::compile::{json_object, sql_str};
        let Some(t) = scanned else {
            return self.element_json_of_id(&self.element_id(alias), kind);
        };
        // Packed properties of a wide view are already in `__ext`.
        let props = crate::cypher::views::view_layout(t).columns;
        let mut pairs = vec![format!("'_id', {alias}.id"), format!("'_type', og_type_name({alias}.type_id)")];
        if kind == Kind::Rel {
            pairs.push(format!("'_src', {alias}.src"));
            pairs.push(format!("'_dst', {alias}.dst"));
        }
        for (name, (col, _)) in &props {
            pairs.push(format!("{}, to_jsonb({alias}.{col})", sql_str(name)));
        }
        // An OPTIONAL MATCH that found nothing leaves the alias all NULL, and
        // the element is then null — not an empty object, which `count(n)`
        // would count and a client would take for a node.
        SqlExpr::new(format!(
            "(CASE WHEN {alias}.id IS NULL THEN NULL ELSE \
             jsonb_strip_nulls({}) || COALESCE({alias}.__ext,'{{}}'::jsonb) END)",
            json_object(&pairs)
        ))
    }

    fn element_json_of_id(&self, id: &SqlExpr, kind: Kind) -> SqlExpr {
        SqlExpr::new(match kind {
            Kind::Node => format!("og_node_json({id})"),
            Kind::Rel => format!("og_edge_json({id})"),
        })
    }

    fn element_id(&self, alias: &str) -> SqlExpr {
        SqlExpr::new(format!("{alias}.id"))
    }

    fn element_type(&self, alias: &str) -> SqlExpr {
        SqlExpr::new(format!("{alias}.type_id"))
    }

    fn label_of(&self, id: &SqlExpr) -> SqlExpr {
        SqlExpr::new(format!("og_id_type({id})"))
    }

    fn type_of_element(&self, id: ElementId) -> TypeId {
        crate::id::id_type(id)
    }

    fn synthetic_rel_id(&self, rel_type: TypeId, src: &SqlExpr, dst: &SqlExpr) -> SqlExpr {
        // Shard 0, the relationship's type, and the pair hashed into the
        // 36-bit local part (`id.rs`).
        SqlExpr::new(format!(
            "og_make_id(0, {rel_type}, hashtextextended({src}::text || ':' || {dst}::text, 0) & {mask})",
            mask = (1_i64 << 36) - 1,
        ))
    }

    fn temporal_types(&self, g: GraphId, kind: Kind, among: Option<&[TypeId]>) -> Vec<TypeId> {
        use crate::catalog::types;
        match (kind, among) {
            // A relationship type is read from adjacency segments only when it
            // is stored as an edge table; of those, the temporal ones.
            (Kind::Rel, Some(ids)) => ids
                .iter()
                .copied()
                .filter(|t| {
                    types::storage_table(*t).is_some_and(|s| s.starts_with("og_data.e_"))
                        && crate::temporal::is_temporal(*t)
                })
                .collect(),
            (Kind::Rel, None) => crate::temporal::temporal_tables(g, 'e'),
            (Kind::Node, None) => crate::temporal::temporal_tables(g, 'n'),
            // Not asked by the compilers.
            (Kind::Node, Some(ids)) => ids.iter().copied().filter(|t| crate::temporal::is_temporal(*t)).collect(),
        }
    }

    fn size_estimate(&self, _g: GraphId) -> Option<(f64, f64)> {
        // The whole database, not one graph: the registries are shared. The
        // statement's text, line break and indentation included, is what it
        // has always been (the generated-SQL mold traces it).
        let est = crate::spiu::two::<f32, f32>(
            concat!(
                "SELECT (SELECT reltuples FROM pg_class WHERE oid = 'og_data.og_node'::regclass),\n",
                "                (SELECT reltuples FROM pg_class WHERE oid = 'og_data.og_edge'::regclass)",
            ),
            &[],
        );
        match est {
            Ok((Some(n), Some(e))) if n > 0.0 && e > 0.0 => Some((n as f64, e as f64)),
            _ => None,
        }
    }

    fn take_uncacheable(&self) -> bool {
        crate::cypher::views::take_created_view()
    }

    fn create_node(&self, g: GraphId, t: TypeId, type_name: &str, props: Value) -> ElementId {
        crate::storage::create_node_inner(g, t, type_name, props)
    }

    fn create_edge(
        &self,
        g: GraphId,
        t: TypeId,
        type_name: &str,
        src: ElementId,
        dst: ElementId,
        props: Value,
    ) -> ElementId {
        crate::storage::create_edge_inner(g, t, type_name, src, dst, props)
    }

    fn set_node_props(&self, id: ElementId, props: Value) {
        crate::storage::set_node_props_inner(id, props)
    }

    fn set_edge_props(&self, id: ElementId, props: Value) {
        crate::storage::set_edge_props_inner(id, props)
    }

    fn delete_node(&self, id: ElementId) -> i64 {
        crate::storage::delete_node_inner(id)
    }

    fn delete_edge(&self, id: ElementId) -> i64 {
        crate::storage::delete_edge_inner(id)
    }

    fn degree(&self, id: ElementId) -> i64 {
        crate::storage::adjacency::total_degree(id)
    }

    fn refuse_virtual_write(&self, t: TypeId) {
        crate::storage::refuse_virtual_write(t)
    }

    fn type_renamed(&self, t: TypeId, old: &str, new: &str) {
        crate::storage::type_renamed(t, old, new)
    }

    // ---------------------------------------------------------------- TypeQL (K3 → K4)

    fn type_store(&self, t: TypeId) -> Option<super::TypeStore> {
        crate::catalog::types::storage_table(t).map(super::TypeStore)
    }

    fn create_instance(&self, t: TypeId, store: &super::TypeStore) -> Result<ElementId, super::WriteError> {
        crate::storage::create_instance(t, &store.0)
    }

    fn find_attribute(&self, store: &super::TypeStore, value: &SqlExpr) -> Result<Option<ElementId>, String> {
        crate::storage::find_attribute(&store.0, value.as_str())
    }

    fn create_attribute(&self, t: TypeId, store: &super::TypeStore, value: &SqlExpr) -> Result<ElementId, super::WriteError> {
        crate::storage::create_attribute(t, &store.0, value.as_str())
    }

    fn other_owner(
        &self,
        has: TypeId,
        store: &super::TypeStore,
        value: &SqlExpr,
        except: ElementId,
        owner_types: &[TypeId],
    ) -> Result<Option<ElementId>, String> {
        crate::storage::other_owner(has, &store.0, value.as_str(), except, owner_types)
    }

    fn count_owned(&self, has: TypeId, owner: ElementId, attr_types: &[TypeId]) -> Result<i64, String> {
        crate::storage::count_owned(has, owner, attr_types)
    }

    fn owned_attributes(&self, has: TypeId, owner: ElementId, attr_types: &[TypeId]) -> Vec<ElementId> {
        crate::storage::owned_attributes(has, owner, attr_types)
    }

    fn ownerships_of(&self, has: TypeId, id: ElementId) -> Vec<(ElementId, ElementId, ElementId)> {
        crate::storage::ownerships_of(has, id)
    }

    fn link_has(&self, has: TypeId, owner: ElementId, attr: ElementId) -> Result<(), super::WriteError> {
        crate::storage::link_has(has, owner, attr)
    }

    fn unlink_has(&self, has: TypeId, owner: ElementId, attr: ElementId) {
        crate::storage::unlink_has(has, owner, attr)
    }

    fn role_filled(&self, rel: ElementId, role: super::RoleId) -> bool {
        crate::storage::role_filled(rel, role)
    }

    fn add_role_player(&self, rel: ElementId, role: super::RoleId, player: ElementId) -> Result<(), String> {
        crate::storage::add_role_player(rel, role, player)
    }

    fn remove_role_player(&self, rel: ElementId, player: ElementId, roles: &[super::RoleId]) -> Result<(), String> {
        crate::storage::remove_role_player(rel, player, roles)
    }

    fn relations_played(&self, player: ElementId) -> Vec<ElementId> {
        crate::storage::relations_played(player)
    }

    fn remove_instance(&self, id: ElementId) {
        crate::storage::remove_instance(id)
    }
}

// ---------------------------------------------------------------------- K2: hop helpers

/// The node registry narrowed to graph `g`'s types (`node_scan(Any)`).
fn graph_registry(g: GraphId) -> String {
    format!(
        "(SELECT gn.* FROM og_data.og_node gn WHERE gn.type_id IN \
         (SELECT gt.type_id FROM og_catalog.type gt WHERE gt.graph_id = {g}))"
    )
}

/// The adjacency segments of one hop, unnested to rows `(nbr, eid)`.
fn adjacency_segments(start: &SqlExpr, a: &str, dir: Dir, type_pred: &str) -> String {
    let dir_pred = match dir {
        Dir::Both => format!("{a}.dir IN ('o','i')"),
        Dir::Out => format!("{a}.dir = 'o'::\"char\""),
        Dir::In => format!("{a}.dir = 'i'::\"char\""),
    };
    format!(
        "SELECT u.nbr, u.eid FROM og_data.og_adj {a}, \
         LATERAL unnest({a}.nbr, {a}.eid) AS u(nbr, eid) \
         WHERE {a}.src = {start} AND {dir_pred}{type_pred}"
    )
}

/// `ARRAY[<ids>]::int4[]`.
fn int4_array(ids: &[TypeId]) -> String {
    format!("ARRAY[{}]::int4[]", ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(","))
}

/// The stored edges of one hop out of the node whose id is `start`, as rows
/// `(nbr, eid)`: the adjacency segments aliased `a`, narrowed by `type_pred`
/// (which names `a`), and the edge relation of each temporal relationship type
/// in `temporal`, read in place of its segments (they know only the present).
/// The `te` aliases come from the compiler's counter, in this order.
fn stored_hop_rows(
    start: &SqlExpr,
    a: &str,
    dir: Dir,
    type_pred: &str,
    temporal: &[TypeId],
    names: &mut dyn Aliases,
) -> String {
    let segments = adjacency_segments(start, a, dir, type_pred);
    if temporal.is_empty() {
        return segments;
    }
    let mut parts = vec![format!("{segments} AND {a}.etype <> ALL({})", int4_array(temporal))];
    for t in temporal {
        let Some(erel) = crate::temporal::edge_relation(*t) else { continue };
        let te = names.fresh("te");
        if matches!(dir, Dir::Out | Dir::Both) {
            parts.push(format!("SELECT {te}.dst, {te}.id FROM {erel} {te} WHERE {te}.src = {start}"));
        }
        if matches!(dir, Dir::In | Dir::Both) {
            let te2 = names.fresh("te");
            parts.push(format!("SELECT {te2}.src, {te2}.id FROM {erel} {te2} WHERE {te2}.dst = {start}"));
        }
    }
    parts.join(" UNION ALL ")
}

#[cfg(test)]
mod k2_tests {
    //! The text v1 builds without the catalog, as `cypher/compile.rs` wrote it
    //! before K2 (the generated-SQL mold checks the rest against a database).
    //! Only what reaches no SPI is called: this test binary has no PostgreSQL
    //! to resolve SPI's symbols against, and macOS refuses to load a binary
    //! that links them.
    use super::*;

    #[test]
    fn a_hop_reads_the_adjacency_segments() {
        let start = LayoutV1.element_id("n1");
        let types = format!(" AND adj2.etype = ANY({})", int4_array(&[3, 5]));
        assert_eq!(
            adjacency_segments(&start, "adj2", Dir::Out, &types),
            "SELECT u.nbr, u.eid FROM og_data.og_adj adj2, LATERAL unnest(adj2.nbr, adj2.eid) AS u(nbr, eid) \
             WHERE adj2.src = n1.id AND adj2.dir = 'o'::\"char\" AND adj2.etype = ANY(ARRAY[3,5]::int4[])"
        );
        assert!(adjacency_segments(&start, "adj2", Dir::Both, "").ends_with("WHERE adj2.src = n1.id AND adj2.dir IN ('o','i')"));
        assert!(adjacency_segments(&start, "adj2", Dir::In, "").ends_with("adj2.dir = 'i'::\"char\""));
        assert_eq!(int4_array(&[]), "ARRAY[]::int4[]");
    }

    #[test]
    fn a_variable_length_hop_is_a_function_call() {
        let l = LayoutV1;
        let start = l.element_id("n1");
        assert_eq!(
            l.reach(&start, Some(&[4]), Dir::Both, 1, 3, PathSem::Trails).as_str(),
            "og_vlp(n1.id, ARRAY[4]::int4[], 'b'::\"char\", 1, 3)"
        );
        assert_eq!(
            l.reach(&start, None, Dir::In, 1, 30, PathSem::Reach).as_str(),
            "og_reach(n1.id, NULL::int4[], 'i'::\"char\", 1, 30)"
        );
        assert!(l.walk_rels("vl4").as_str().starts_with("(CASE WHEN vl4.path IS NULL THEN NULL ELSE (SELECT"));
        assert!(l.walk_nodes(&start, "vl4").as_str().contains("SELECT 0, n1.id UNION ALL"));
    }

    #[test]
    fn ids_types_and_virtual_relationships() {
        let l = LayoutV1;
        assert_eq!(l.element_type("e3").as_str(), "e3.type_id");
        assert_eq!(l.label_of(&SqlExpr::from("u2.eid")).as_str(), "og_id_type(u2.eid)");
        assert_eq!(l.element_json_of_id(&SqlExpr::from("x"), Kind::Node).as_str(), "og_node_json(x)");
        assert_eq!(l.element_json_of_id(&SqlExpr::from("x"), Kind::Rel).as_str(), "og_edge_json(x)");
        assert_eq!(
            l.synthetic_rel_id(12, &l.element_id("n1"), &l.element_id("n2")).as_str(),
            "og_make_id(0, 12, hashtextextended(n1.id::text || ':' || n2.id::text, 0) & 68719476735)"
        );
        assert_eq!(
            graph_registry(5),
            "(SELECT gn.* FROM og_data.og_node gn WHERE gn.type_id IN \
             (SELECT gt.type_id FROM og_catalog.type gt WHERE gt.graph_id = 5))"
        );
    }
}
