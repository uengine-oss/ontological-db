//! The storage-layout seam — design `DESIGN.md` 5.1 (P1, card K1).
//!
//! The Cypher and TypeQL compilers turn a pattern into SQL, and the SQL has to
//! name where elements are stored: which relation holds the nodes of a label,
//! how one hop finds the neighbours of a node, how a property is read, what a
//! node looks like as JSON. Today those names are written straight into the
//! compilers (`og_data.v_<tid>`, `og_adj`, `p_<prop>`, `__ext`, `og_node_json`
//! …). [`Layout`] is the one place they are to come from, so that a second
//! storage layout — v2, Apache AGE's shape, `ontological_age` — can sit behind
//! the same compilers without a line of them changing.
//!
//! K1 defined it; K2 (Cypher read), K3 (Cypher write and the storage API), K4
//! (TypeQL) and K5 (traversal) moved the code that wrote physical names into
//! v1's implementation, one call site at a time, and the generated-SQL mold
//! (`tests/seam/`) checks that every statement comes out byte for byte as
//! before. K6 moved that implementation into the v1 store
//! ([`crate::store_v1::layout::LayoutV1`]). That is why the methods take what
//! the call sites had in hand — an alias, a type id, a caller-minted alias for
//! the layout's own scan — and why each method's documentation says exactly
//! what v1 writes: that text is what the moved code must keep producing.
//!
//! Rules for the cards that fill it in (`P1-CARDS.md`): add methods, do not
//! rename or remove them (several cards edit this file in parallel); keep v1's
//! output identical (`scripts/mold.sh check`); semantics — which types a label
//! means, what a pattern matches — stay in the compilers. A layout decides
//! *where* things are, never *what* is asked.
//!
//! Note the name clash: [`crate::store_v1::views::Layout`] is an older struct (the
//! columns of one union view). Inside `views.rs` refer to this trait by its path.

use serde_json::Value;
use std::fmt;

/// A graph, as `og_catalog.graph.graph_id` numbers it.
pub type GraphId = i32;
/// A type (label, relationship type, attribute type), as the catalog numbers it.
/// Type ids are the catalog's, shared by every layout; which relation holds a
/// type's rows is the layout's business.
pub type TypeId = i32;
/// A node's or relationship's identifier.
pub type ElementId = i64;

/// A relation the compiler can put in a FROM list and alias: a table, a view, a
/// parenthesised subquery, a set-returning function call.
///
/// A scan is handed back as SQL rather than as a table name so that a layout
/// whose label is several tables (v2: one table per label, scanned `ONLY` and
/// glued with `UNION ALL` for a label with sub-labels) can answer with a
/// subquery where v1 answers with a view name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SqlFrag(String);

/// A scalar SQL expression.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SqlExpr(String);

macro_rules! sql_text {
    ($t:ident) => {
        impl $t {
            pub fn new(sql: impl Into<String>) -> Self {
                $t(sql.into())
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
            pub fn into_string(self) -> String {
                self.0
            }
        }
        impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl From<String> for $t {
            fn from(s: String) -> Self {
                $t(s)
            }
        }
        impl From<&str> for $t {
            fn from(s: &str) -> Self {
                $t(s.to_string())
            }
        }
    };
}
sql_text!(SqlFrag);
sql_text!(SqlExpr);

/// Node or relationship.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Node,
    Rel,
}

/// Which way a hop follows relationships, seen from the node it starts at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dir {
    Out,
    In,
    Both,
}

impl From<&crate::cypher::ast::Dir> for Dir {
    fn from(d: &crate::cypher::ast::Dir) -> Self {
        match d {
            crate::cypher::ast::Dir::Out => Dir::Out,
            crate::cypher::ast::Dir::In => Dir::In,
            crate::cypher::ast::Dir::Both => Dir::Both,
        }
    }
}

/// The types a scan is to cover, as the compiler resolved them.
///
/// The compiler decides what a label means (inheritance, umbrella labels,
/// unknown labels); the layout only finds the rows.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LabelSet {
    /// No label: every element of the graph.
    Any,
    /// One type and every type under it — `(v:Vehicle)` also finds Cars.
    /// v1 keeps one union view per type for exactly this.
    Under(TypeId),
    /// These type ids and no others, subtypes already expanded by the caller
    /// (what `rel_type_ids` and TypeQL's type sets produce). Sorted, no repeats.
    Exactly(Vec<TypeId>),
}

/// Which instant a read answers for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimeScope {
    /// The present only. The registries and v1's adjacency segments know
    /// nothing else; TypeQL reads at this scope (`guard_current_only`).
    Current,
    /// The session's instant — `og.valid_at` / `og.system_at`, read when the
    /// statement *runs*, so that one compiled (and cached) statement serves
    /// every instant. Where no instant is set this is the present.
    Session,
}

/// What a variable-length hop has to produce.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PathSem {
    /// Every trail (no relationship twice) of length `min..=max`, with the
    /// relationships it walked: a path or a relationship variable can see them,
    /// and so can a count. v1: `og_vlp`.
    Trails,
    /// Each node reachable within `min..=max`, once, at its shortest distance:
    /// for patterns that cannot observe multiplicity (the compiler decides,
    /// ADR-012/013). v1: `og_reach`.
    Reach,
}

/// A property read, and the SQL type of the value when the layout knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PropRead {
    pub sql: SqlExpr,
    /// The column's SQL type (`int8`, `text`, `timestamptz` …). `None` when the
    /// value is read as text out of JSON and its type is not known.
    pub sql_type: Option<String>,
}

/// An attribute-ownership join ([`Layout::ownership_join`]): the text to put
/// after `FROM`, and the conditions, in order, that make it the join meant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnershipJoin {
    pub from: SqlFrag,
    pub conds: Vec<SqlExpr>,
}

/// Mints aliases. The compiler implements this with its own counter, so that
/// SQL moved behind the seam numbers its aliases exactly as it did before.
pub trait Aliases {
    /// A new alias: `prefix` followed by the compiler's next number (`te7`).
    fn fresh(&mut self, prefix: &str) -> String;
}

/// Where a storage layout keeps graphs, as the compilers and the write path see it.
///
/// Object-safe: the compilers can hold a `&dyn Layout` ([`active`]).
///
/// Each method says what v1 produces, with `<…>` for what the caller passes in.
/// Methods whose v1 answer creates something (a view) or reads statistics run
/// SPI; the rest only build text.
pub trait Layout {
    // ------------------------------------------------------------------ scans

    /// The nodes `labels` covers in graph `g`, as a relation to alias. Its rows
    /// have at least `id` and `type_id`, and a column per property the layout
    /// gives one (see [`Layout::prop`]).
    ///
    /// v1 (`cypher/compile.rs` `bind_node`, `graph_nodes`; `store_v1/views.rs`):
    /// - `Under(t)`: `og_data.v_<t>`, or `og_data.vp_<t>` when the view packs
    ///   properties into `__ext` (more than `WIDE_VIEW_COLUMNS` columns). The view
    ///   is the `UNION ALL` of `og_data.n_<sub>` (`og_data.a_<sub>` for TypeQL
    ///   relation-as-node types) over `t` and its subtypes; for a temporal type
    ///   it also reads `og_data.h_<sub>` filtered to the session's instant, so
    ///   `at` does not change the name. Created on first use (`ensure_view`:
    ///   `CREATE OR REPLACE VIEW … WITH (security_invoker = true)`, through
    ///   `og__ensure_view` for a role that cannot create in `og_data`); a
    ///   compile that created one is not cached ([`Layout::take_uncacheable`]).
    /// - `Any`: `(SELECT gn.* FROM og_data.og_node gn WHERE gn.type_id IN
    ///   (SELECT gt.type_id FROM og_catalog.type gt WHERE gt.graph_id = <g>))` —
    ///   the registry, which holds only the present.
    /// - `Exactly(..)`: not produced by v1 (TypeQL scans [`Layout::registry`]
    ///   and filters `type_id` itself).
    fn node_scan(&self, g: GraphId, labels: &LabelSet, at: TimeScope) -> SqlFrag;

    /// The relationships of `types` in graph `g`, as a relation to alias: rows
    /// with `id`, `type_id`, `src`, `dst` and the property columns.
    ///
    /// v1: `Under(t)` → `og_data.ve_<t>`, or `og_data.vep_<t>` when packed — the
    /// union view over `og_data.e_<sub>` (and `h_` for temporal types), created
    /// on first use like the node views (`ensure_view(t, true)`). v1 compiles
    /// no other form: a relationship without one type is read by id
    /// ([`Layout::element_json_of_id`]).
    fn edge_scan(&self, g: GraphId, types: &LabelSet, at: TimeScope) -> SqlFrag;

    /// The registry of every element of a kind, in every graph: rows with `id`
    /// and `type_id` (and `src`, `dst` for relationships). For callers that
    /// filter by type themselves.
    ///
    /// v1: `og_data.og_node` / `og_data.og_edge`. TypeQL's `isa` scans
    /// `og_data.og_node <a>` and adds `<a>.type_id = ANY(…)`.
    fn registry(&self, kind: Kind) -> SqlFrag;

    /// The role players of n-ary relations: rows `(edge_id, role_id, player_id)`.
    ///
    /// v1: `og_data.og_role_player` (TypeQL role patterns; ADR-016).
    fn role_players(&self) -> SqlFrag;

    // ------------------------------------------------------------------ TypeQL attributes (K4)
    //
    // TypeQL keeps an attribute as an instance of its own, identified by type and
    // value (spec 010 FR-016), owned through the internal `$has` relationship type
    // (`typeql::schema::HAS_TYPE`). Which attribute types a pattern means — subtypes,
    // abstract ones — is the compiler's; these say where the instances and the
    // ownerships are.

    /// The instances of attribute type `t` itself (not its subtypes) as a
    /// `SELECT` of rows `(id, val)`, for the caller to glue with `UNION ALL`;
    /// `None` when `t` keeps no instances (abstract).
    ///
    /// v1 (`typeql/compile.rs` `concrete_attr_tables`): `SELECT id, val FROM
    /// <storage_table>` — the table `og_catalog.type.storage_table` names,
    /// `og_data.a_<t>`; one catalog lookup (SPI).
    fn attr_values(&self, t: TypeId) -> Option<SqlFrag>;

    /// The attributes the element whose id is `owner` owns, among the attribute
    /// types `attr_types` (subtypes already expanded by the caller), joined to
    /// `values` — rows `(id, val)`, from [`Layout::attr_values`] — aliased
    /// `<h>v`. The caller puts [`OwnershipJoin::from`] after `FROM`, the
    /// conditions in its `WHERE` (joined with ` AND `, its own conditions on
    /// `<h>v` after them) and selects from `<h>v`. `h` is a fresh alias prefix:
    /// the layout's own scans are `<h>` followed by a letter other than `v`.
    ///
    /// v1 (`pat_has`, `attr_scalar`, `attr_list`): the `$has` adjacency
    /// segments — from `og_data.og_adj <h>a CROSS JOIN LATERAL unnest(<h>a.nbr)
    /// AS <h>u(nbr) JOIN <values> <h>v ON <h>v.id = <h>u.nbr`, conditions
    /// `<h>a.src = <owner>`, `<h>a.dir = 'o'::"char"`, `<h>a.etype = <has_type>`,
    /// `<label_of_inline(<h>u.nbr)> = ANY(ARRAY[<attr_types>]::int4[])`.
    fn ownership_join(
        &self,
        owner: &SqlExpr,
        has_type: TypeId,
        attr_types: &[TypeId],
        values: &SqlFrag,
        h: &str,
    ) -> OwnershipJoin;

    /// The type id of the element whose id is `id`, as an expression the
    /// planner can see through (no function call) — the same value as
    /// [`Layout::label_of`]; TypeQL's type filters use it.
    ///
    /// v1 (`typeql/compile.rs` `type_of_id`): `(((<id>) >> 36) & 262143)::int4`
    /// (`id::TYPE_SHIFT`, `id::TYPE_MASK`).
    fn label_of_inline(&self, id: &SqlExpr) -> SqlExpr;

    // ------------------------------------------------------------------ hops

    /// One hop out of the node whose id is `start`, as a relation of rows
    /// `(nbr, eid)`: the node at the other end and the relationship. The caller
    /// joins it `LATERAL` and aliases it; `inner` is an alias the caller minted
    /// for the layout's own scan inside the fragment.
    ///
    /// `types` is the relationship types to follow (`None`: any), closed under
    /// subtypes by the caller. `temporal` is the subset of `types` that must be
    /// read at the session's instant — the caller got it from
    /// [`Layout::temporal_types`] and needs it for its own guards too.
    ///
    /// v1 (`stored_hop_rows`): the adjacency segments,
    /// `SELECT u.nbr, u.eid FROM og_data.og_adj <inner>, LATERAL unnest(<inner>.nbr,
    /// <inner>.eid) AS u(nbr, eid) WHERE <inner>.src = <start> AND <dir>` followed by
    /// ` AND <inner>.etype = ANY(ARRAY[<ids>]::int4[])` when `types` is given;
    /// `<dir>` is `<inner>.dir = 'o'::"char"`, `<inner>.dir = 'i'::"char"` or
    /// `<inner>.dir IN ('o','i')`. With `temporal` non-empty the segments are
    /// narrowed with ` AND <inner>.etype <> ALL(ARRAY[<temporal>]::int4[])` and,
    /// per temporal type, `UNION ALL SELECT <te>.dst, <te>.id FROM <edge relation>
    /// <te> WHERE <te>.src = <start>` (out) and the mirror image (in), each `<te>`
    /// from `names.fresh("te")` in that order; the edge relation is
    /// `temporal::edge_relation`: current rows plus the open and closed history
    /// rows visible at the instant.
    fn expand(
        &self,
        start: &SqlExpr,
        types: Option<&[TypeId]>,
        dir: Dir,
        temporal: &[TypeId],
        inner: &str,
        names: &mut dyn Aliases,
    ) -> SqlFrag;

    /// [`Layout::expand`] over every relationship type except `except` (K2):
    /// the stored half of a hop without one type where the virtual
    /// relationships (ADR-029) are joined separately — `except` is their types.
    /// `temporal` is as for `expand` (all of the graph's temporal
    /// relationship types, the caller having asked for `None`).
    ///
    /// v1 (`join_rel_union`): `expand`'s text with ` AND <inner>.etype <>
    /// ALL(ARRAY[<except>]::int4[])` where `expand` puts its `= ANY(…)`. The
    /// adjacency segments hold no virtual relationship, so the exclusion
    /// changes no answer; it is kept so the statement stays what it was.
    fn expand_except(
        &self,
        start: &SqlExpr,
        except: &[TypeId],
        dir: Dir,
        temporal: &[TypeId],
        inner: &str,
        names: &mut dyn Aliases,
    ) -> SqlFrag;

    /// A variable-length hop from the node whose id is `start`, as a
    /// set-returning call to join `LATERAL`: rows `(node, path)` for
    /// [`PathSem::Trails`] (`path` the relationship ids in order), `(node,
    /// depth)` for [`PathSem::Reach`].
    ///
    /// v1: `og_vlp(<start>, <types>, '<d>'::"char", <min>, <max>)` or the same
    /// arguments to `og_reach`, with `<types>` = `ARRAY[<ids>]::int4[]` or
    /// `NULL::int4[]` and `<d>` = `o` / `i` / `b`. Both read the adjacency
    /// segments (present only: the compiler refuses a variable-length hop over
    /// a temporal type at an instant).
    fn reach(
        &self,
        start: &SqlExpr,
        types: Option<&[TypeId]>,
        dir: Dir,
        min: u32,
        max: u32,
        sem: PathSem,
    ) -> SqlFrag;

    /// The relationships a [`PathSem::Trails`] hop aliased `walk` went through,
    /// as a jsonb list of relationships (null where an OPTIONAL MATCH found none).
    ///
    /// v1 (`var_hop_rels`): `(CASE WHEN <walk>.path IS NULL THEN NULL ELSE (SELECT
    /// coalesce(jsonb_agg(og_edge_json(vx.e) ORDER BY vx.o), '[]'::jsonb) FROM
    /// unnest(<walk>.path) WITH ORDINALITY AS vx(e, o)) END)`.
    fn walk_rels(&self, walk: &str) -> SqlExpr;

    /// The nodes along that hop after its start (whose id is `start`), in
    /// order, as a jsonb list of nodes.
    ///
    /// v1 (`var_hop_nodes`): a `WITH RECURSIVE pn(i, node)` over `<walk>.path`,
    /// each step the other end of `og_data.og_edge pe`, rendered with
    /// `og_node_json(pn.node)`.
    fn walk_nodes(&self, start: &SqlExpr, walk: &str) -> SqlExpr;

    /// The SQL a breadth-first walk runs per level: given the frontier's node
    /// ids bound to `frontier` (`int8[]`) and the relationship types bound to
    /// `types` (`int4[]`, NULL for any), the neighbours, one column `nbr`.
    ///
    /// v1 (`storage/traverse.rs` `og_reach`): `SELECT a.nbr FROM og_data.og_adj a
    /// WHERE a.src = ANY(<frontier>::int8[]) AND <adj_where(dir, "a", types)>`.
    ///
    /// Spelled out (K5 moved it here): `<dir>` as [`Layout::expand`] writes it
    /// (`a.dir = 'o'::"char"`, `a.dir = 'i'::"char"`, `a.dir IN ('o','i')`),
    /// then ` AND (<types>::int4[] IS NULL OR a.etype = ANY(<types>::int4[]))`.
    ///
    /// The column is `int8[]`: v1 hands back whole adjacency segments and the
    /// walk flattens them, skipping NULL elements. A layout with a row per
    /// relationship answers `ARRAY[<other end>]` per row, or aggregates. A
    /// neighbour may come back more than once and in any order; the walk keeps
    /// its own visited set. `og_reach` prepares the statement once per walk
    /// (`frontier` = `$1`, `types` = `$2`) and runs it once per level.
    fn frontier_step(&self, dir: Dir, frontier: &str, types: &str) -> String;

    // ------------------------------------------------------------------ traversal and storage upkeep (K5)
    //
    // `storage/traverse.rs` walks in Rust (`og_reach`, the backend-local CSR)
    // and `storage/stats.rs` reports on the storage (`og_graph_stats`,
    // `og_degree_distribution`, `og_reorganize`, `og_check_integrity`). The
    // walks, and those SQL functions' names, arguments and result shapes, are
    // the same for every layout; what is read, and what the storage can say
    // about itself, is the layout's. The statistics methods run SPI.

    /// The SQL that reads the whole topology once, for a walk compiled into
    /// memory (`og_csr_build`): with the relationship types bound to `types`
    /// (`int4[]`, NULL for any), rows `(src int8, dir text, nbr int8[])` —
    /// `dir` `'o'` for relationships out of `src`, `'i'` for those into it,
    /// `nbr` the nodes at their other ends. Both directions must be present:
    /// the reverse index (`og_csr_hops`) is compiled from the `'i'` rows.
    /// `og_csr_build` binds `types` as `$1`.
    ///
    /// v1: `SELECT a.src, a.dir::text, a.nbr FROM og_data.og_adj a WHERE a.dir
    /// IN ('o','i') AND (<types>::int4[] IS NULL OR a.etype =
    /// ANY(<types>::int4[]))` — every adjacency segment; v1 already stores an
    /// `'i'` segment for each `'o'` one.
    fn topology_scan(&self, types: &str) -> String;

    /// The document `og_graph_stats(<graph>)` returns for graph `g`, whose name
    /// is `graph`: at least `graph`, `nodes`, `edges` and `types` (`[{name,
    /// kind, abstract, instances}]` ordered by name), plus what the layout can
    /// say about its own storage.
    ///
    /// v1: `nodes` / `edges` count the registries (`og_data.og_node` /
    /// `og_edge`) joined to `g`'s types; `instances` is a type's nodes plus its
    /// relationships; `adjacency` is `{segments, avg_fill, chunk_size,
    /// packing_ratio, chunked_supernodes}` over all of `og_data.og_adj` —
    /// every graph's segments, not only `g`'s.
    fn graph_stats(&self, g: GraphId, graph: &str) -> Value;

    /// The out-degree histogram of graph `g` for `og_degree_distribution`:
    /// one `{bucket, nodes, max_degree}` per bucket, in bucket order.
    ///
    /// v1: `width_bucket(<degree>, 0, 1024, 8)`, the degree being the sum of a
    /// node's `'o'` segment counts, over the nodes of `g` that have any.
    fn degree_distribution(&self, g: GraphId) -> Vec<Value>;

    /// Compact graph `g`'s storage (`og_reorganize`); returns how many units
    /// were rewritten. A layout with nothing to compact returns 0.
    ///
    /// v1: each `(src, etype, dir)` of `g` whose segments would fit in fewer
    /// chunks of `store_v1::adjacency::CHUNK` is rewritten densely, one
    /// statement per group; the count is of groups.
    fn reorganize(&self, g: GraphId) -> i64;

    /// Structural inconsistencies in the storage, every graph
    /// (`og_check_integrity`): rows `(kind, entity_id, detail)`, none being the
    /// pass condition.
    ///
    /// v1: four checks, in this order and at most 100 rows each —
    /// `dangling_adjacency` (a segment names a relationship the registry does
    /// not have), `missing_adjacency` (a relationship missing from its
    /// source's `'o'` or its target's `'i'` segments),
    /// `segment_length_mismatch` (`n` against the array lengths),
    /// `orphan_node` (a registry row whose type the catalog does not have).
    fn check_integrity(&self) -> Vec<(String, Option<ElementId>, String)>;

    // ------------------------------------------------------------------ values

    /// Property `key` of the row aliased `alias`, scanned as type `scanned`
    /// (`None`: a row of the registry, type unknown at compile time).
    ///
    /// v1 (`prop_sql`): `_id` → `<alias>.id` (`int8`); a property the type's
    /// view has a column for → `<alias>.<column>` with the column's type (the
    /// column `og_catalog.property.column_name` names, `p_<prop>`); one a
    /// wide view packed → a typed read of `<alias>.__ext -> '<key>'`
    /// (`views::Layout::read`); any other key of a typed row →
    /// `(<alias>.__ext->>'<key>')`; of an untyped row →
    /// `(og_node_json(<alias>.id)->>'<key>')` (type unknown).
    fn prop(&self, alias: &str, scanned: Option<TypeId>, key: &str) -> PropRead;

    /// The same, keeping the JSON type of the value (a number stays a number).
    ///
    /// v1 (`prop_sql_json`): `_id` → `to_jsonb(<alias>.id)`; a column →
    /// `to_jsonb(<alias>.<column>)`; packed → `(<alias>.__ext -> '<key>')`; other
    /// keys `(<alias>.__ext -> '<key>')` or `(og_node_json(<alias>.id) -> '<key>')`.
    fn prop_json(&self, alias: &str, scanned: Option<TypeId>, key: &str) -> SqlExpr;

    /// The SQL type property `key` reads as on the rows of type `t` (its
    /// subtypes included), `None` when no type under `t` declares it (K2). For
    /// a value carried through `WITH` as jsonb, whose reads the compiler casts
    /// back to that type — no row alias is involved.
    ///
    /// v1 (`carried_prop_type`): the SQL type of `key` in `views::view_properties(t)`
    /// — the column's type in `t`'s union view, the common type where subtypes
    /// disagree (`views::common_type`). A lookup by name only: `_id`, which
    /// [`Layout::prop`] answers for every row, is not declared, so it is `None`.
    fn prop_type(&self, t: TypeId, key: &str) -> Option<String>;

    /// The row aliased `alias` as the JSON Bolt expects: `{_id, _type, …props}`
    /// for a node, `{_id, _type, _src, _dst, …props}` for a relationship; null
    /// for a row an OPTIONAL MATCH left empty.
    ///
    /// v1 (`node_json`, `rel_json`): with a type, `(CASE WHEN <alias>.id IS NULL
    /// THEN NULL ELSE jsonb_strip_nulls(jsonb_build_object('_id', <alias>.id,
    /// '_type', og_type_name(<alias>.type_id), [ '_src', <alias>.src, '_dst',
    /// <alias>.dst, ] '<prop>', to_jsonb(<alias>.<column>) …)) ||
    /// COALESCE(<alias>.__ext,'{}'::jsonb) END)` — `jsonb_build_object` split
    /// into chunks of 50 pairs joined with `||` (`cypher::compile::json_object`); a node
    /// without a type → `og_node_json(<alias>.id)`.
    fn element_json(&self, alias: &str, scanned: Option<TypeId>, kind: Kind) -> SqlExpr;

    /// An element as that JSON, from its id alone.
    ///
    /// v1: `og_node_json(<id>)` / `og_edge_json(<id>)` (SQL functions in
    /// `sql/access.sql` that look the type up in the registry and read its table).
    fn element_json_of_id(&self, id: &SqlExpr, kind: Kind) -> SqlExpr;

    /// The id of the row aliased `alias`. v1: `<alias>.id`.
    fn element_id(&self, alias: &str) -> SqlExpr;

    /// The type id of the row aliased `alias`. v1: `<alias>.type_id`.
    fn element_type(&self, alias: &str) -> SqlExpr;

    /// The type id of the element whose id is `id`, in SQL.
    ///
    /// v1: `og_id_type(<id>)` — the type is bits 36..54 of the id (`id.rs`).
    fn label_of(&self, id: &SqlExpr) -> SqlExpr;

    /// The type id of an element, in Rust. v1: `id::id_type(id)`.
    fn type_of_element(&self, id: ElementId) -> TypeId;

    /// The id of a virtual relationship (ADR-029) between the rows whose ids are
    /// `src` and `dst`: no row stores it, so it is derived from its type and its
    /// two ends and the same pair always names the same relationship.
    ///
    /// v1 (`virtual_rel_id`): `og_make_id(0, <rel_type>,
    /// hashtextextended(<src>::text || ':' || <dst>::text, 0) & 68719476735)`
    /// (the 36-bit local part).
    fn synthetic_rel_id(&self, rel_type: TypeId, src: &SqlExpr, dst: &SqlExpr) -> SqlExpr;

    // ------------------------------------------------------------------ what the compiler asks

    /// The temporal types among `among` (`None`: all of graph `g`'s types of
    /// that kind): those whose rows must be read at the session's instant
    /// rather than from what only knows the present. The compiler uses it to
    /// route hops ([`Layout::expand`]) and to refuse what cannot answer for an
    /// instant (`og_require_current(…)`).
    ///
    /// v1: nodes → `temporal::temporal_tables(g, 'n')`; relationships among a
    /// list → those whose table is `og_data.e_*` and that are temporal
    /// (`temporal_rel_types`), all of them → `temporal::temporal_tables(g, 'e')`.
    /// Catalog lookups (SPI).
    fn temporal_types(&self, g: GraphId, kind: Kind, among: Option<&[TypeId]>) -> Vec<TypeId>;

    /// The planner's estimate of how many nodes and relationships there are,
    /// for cost decisions the compiler makes (`prefer_reachability`). `None`
    /// when there are no statistics to answer with.
    ///
    /// v1: `SELECT (SELECT reltuples FROM pg_class WHERE oid =
    /// 'og_data.og_node'::regclass), (SELECT reltuples FROM pg_class WHERE oid =
    /// 'og_data.og_edge'::regclass)` — the whole database, not one graph;
    /// `None` unless both are positive.
    fn size_estimate(&self, g: GraphId) -> Option<(f64, f64)>;

    /// Whether SQL compiled since the last call depends on something a
    /// rollback can take away, so it must not be cached (`compile_cached`).
    /// Clears the flag.
    ///
    /// v1: `views::take_created_view()` — a union view created by this
    /// transaction disappears if it rolls back.
    fn take_uncacheable(&self) -> bool;

    // ------------------------------------------------------------------ writes
    //
    // The write path executes; it does not hand back statements. v1 spends four
    // to six SPI statements on one node or relationship (registry, type table,
    // adjacency in both directions, id allocation, history, audit), with
    // decisions between them (property promotion, `declare_new_props`), and K3
    // must keep their number and order. A layout that writes one statement per
    // operation (v2) implements these the same way.
    //
    // The v1 bodies are `storage/mod.rs`'s public functions, unchanged.

    /// Create a node of entity type `t` (named `type_name`, for messages) in
    /// graph `g`; returns its id. v1: `storage::create_node_inner`.
    fn create_node(&self, g: GraphId, t: TypeId, type_name: &str, props: Value) -> ElementId;

    /// Create a relationship of type `t` from `src` to `dst`; returns its id.
    /// v1: `storage::create_edge_inner`.
    fn create_edge(
        &self,
        g: GraphId,
        t: TypeId,
        type_name: &str,
        src: ElementId,
        dst: ElementId,
        props: Value,
    ) -> ElementId;

    /// Merge `props` into a node's properties (a null removes one).
    /// v1: `storage::set_node_props_inner`.
    fn set_node_props(&self, id: ElementId, props: Value);

    /// The same for a relationship. v1: `storage::set_edge_props_inner`.
    fn set_edge_props(&self, id: ElementId, props: Value);

    /// Delete a node; returns how many were deleted. v1: `storage::delete_node_inner`.
    fn delete_node(&self, id: ElementId) -> i64;

    /// Delete a relationship; returns how many were deleted.
    /// v1: `storage::delete_edge_inner`.
    fn delete_edge(&self, id: ElementId) -> i64;

    /// How many relationships touch node `id`, both directions: what DELETE
    /// without DETACH checks.
    ///
    /// v1 (`cypher/mod.rs`): `SELECT COALESCE(sum(n),0)::int8 FROM og_data.og_adj
    /// WHERE src = $1`.
    fn degree(&self, id: ElementId) -> i64;

    /// Refuse to write a virtual type (its rows live in a source system).
    /// v1: `storage::refuse_virtual_write`.
    fn refuse_virtual_write(&self, t: TypeId);

    /// The catalog has just renamed type `t` from `old` to `new` (Cypher's
    /// `REMOVE n:Old SET n:New`); bring along whatever the layout named after
    /// the type.
    ///
    /// v1 (`cypher::rename_label`): the alias view `og_data."<old>"` is dropped
    /// (`crate::store_v1::type_tables::drop_alias_view`) and, when the type has a table, made again as
    /// `og_data."<new>"` over it (`types::storage_table`, `crate::store_v1::type_tables::ensure_alias_view`).
    fn type_renamed(&self, t: TypeId, old: &str, new: &str);

    // ------------------------------------------------------------------ writes: TypeQL
    //
    // The storage steps `typeql/write.rs` used to run itself (P1 K3 names them,
    // K4 moves write.rs onto them; `reports/ontological-v2/P1-K3-api.md`). Each
    // v1 body is write.rs's statement moved unchanged — text, order, what a
    // failure does — so that write.rs can keep its catalog checks (type kind,
    // `owns`, value type, value constraints, roles) and its own error wording
    // around them. Unlike `create_node`/`create_edge` these do no property
    // planning, no privilege check and no counting: TypeQL never did.

    /// Where the rows of type `t` are stored, looked up once and handed to the
    /// methods below; `None` when the type has no storage of its own (abstract).
    ///
    /// v1: `types::storage_table(t)` — `SELECT storage_table FROM
    /// og_catalog.type WHERE type_id = $1`.
    fn type_store(&self, t: TypeId) -> Option<TypeStore>;

    /// Create a bare instance (no properties) of type `t`, an entity type or a
    /// relation type stored as a node; returns its id. The caller has checked
    /// the type and refused a virtual one ([`Layout::refuse_virtual_write`]).
    ///
    /// v1 (`typeql/write.rs` `create_instance`): `alloc_id(t)`, then `INSERT
    /// INTO og_data.og_node (id, type_id) VALUES ($1, $2)` (failure:
    /// [`WriteError::Register`]), then `INSERT INTO <table> (id) VALUES ($1)`
    /// ([`WriteError::Store`]).
    fn create_instance(&self, t: TypeId, store: &TypeStore) -> Result<ElementId, WriteError>;

    /// The attribute instance in `store` holding `value` — a SQL literal of the
    /// attribute's value type that the caller built (`'fiction'`, `42`,
    /// `1.5::float8`, `'2024-01-01'::date`).
    ///
    /// v1: `SELECT id FROM <table> WHERE val = <value>` (`spiu::one`); `Err`
    /// is the SPI error's text.
    fn find_attribute(&self, store: &TypeStore, value: &SqlExpr) -> Result<Option<ElementId>, String>;

    /// Create the attribute instance of type `t` holding `value`; returns its id.
    ///
    /// v1 (`intern_attribute`): `alloc_id(t)`, `INSERT INTO og_data.og_node (id,
    /// type_id) VALUES ($1, $2)` ([`WriteError::Register`]), `INSERT INTO
    /// <table> (id, val) VALUES ($1, <value>)` ([`WriteError::Store`]).
    fn create_attribute(&self, t: TypeId, store: &TypeStore, value: &SqlExpr) -> Result<ElementId, WriteError>;

    /// An owner other than `except`, of one of `owner_types`, that owns through
    /// ownership type `has` an attribute of `store` holding `value` — the
    /// `@key` / `@unique` check.
    ///
    /// v1 (`check_ownership_constraints`): `SELECT e.src FROM og_data.og_edge e
    /// JOIN <table> a ON a.id = e.dst WHERE e.type_id = $1 AND a.val = <value>
    /// AND e.src <> $2 AND ((e.src >> 36) & 262143)::int4 = ANY($3) LIMIT 1`
    /// (the shift and mask are `id::TYPE_SHIFT` / `TYPE_MASK`; line breaks as
    /// in write.rs).
    fn other_owner(
        &self,
        has: TypeId,
        store: &TypeStore,
        value: &SqlExpr,
        except: ElementId,
        owner_types: &[TypeId],
    ) -> Result<Option<ElementId>, String>;

    /// How many attributes of one of `attr_types` `owner` owns through `has` —
    /// the `@card` check.
    ///
    /// v1: `SELECT count(*) FROM og_data.og_edge e WHERE e.type_id = $1 AND
    /// e.src = $2 AND ((e.dst >> 36) & 262143)::int4 = ANY($3)`; no row reads as 0.
    fn count_owned(&self, has: TypeId, owner: ElementId, attr_types: &[TypeId]) -> Result<i64, String>;

    /// The attributes of one of `attr_types` that `owner` owns through `has`
    /// (what `update` replaces). A failed lookup reads as none.
    ///
    /// v1 (`run_update`): `SELECT dst FROM og_data.og_edge WHERE type_id = $1
    /// AND src = $2 AND ((dst >> 36) & 262143)::int4 = ANY($3)`.
    fn owned_attributes(&self, has: TypeId, owner: ElementId, attr_types: &[TypeId]) -> Vec<ElementId>;

    /// The ownerships through `has` that touch `id`, as owner or as attribute:
    /// `(relationship, owner, attribute)`. A failed lookup reads as none.
    ///
    /// v1 (`delete_instance`): `SELECT id, src, dst FROM og_data.og_edge WHERE
    /// type_id = $1 AND (src = $2 OR dst = $2)`.
    fn ownerships_of(&self, has: TypeId, id: ElementId) -> Vec<(ElementId, ElementId, ElementId)>;

    /// Make `owner` own `attr` through `has`, unless it already does.
    ///
    /// v1 (`link_has`, after `schema::ensure_has_type`): `SELECT id FROM
    /// og_data.og_edge WHERE type_id = $1 AND src = $2 AND dst = $3 LIMIT 1`; if
    /// none, `types::storage_table(has)` (panics "$has has no table" without
    /// one), `alloc_id(has)`, `INSERT INTO og_data.og_edge (id, type_id, src,
    /// dst) VALUES ($1, $2, $3, $4)` ([`WriteError::Register`]), `INSERT INTO
    /// <table> (id, src, dst) VALUES ($1, $2, $3)` ([`WriteError::Store`]), then
    /// the adjacency segments out of `owner` and into `attr` (`adjacency::append`).
    fn link_has(&self, has: TypeId, owner: ElementId, attr: ElementId) -> Result<(), WriteError>;

    /// Remove that ownership, if there is one. Failures are ignored, as
    /// TypeQL always has.
    ///
    /// v1 (`unlink_has`): `SELECT id FROM og_data.og_edge WHERE type_id = $1 AND
    /// src = $2 AND dst = $3`; if found, `adjacency::remove` out of `owner` and
    /// into `attr`, `DELETE FROM <table of has> WHERE id = $1`, `DELETE FROM
    /// og_data.og_edge WHERE id = $1`.
    fn unlink_has(&self, has: TypeId, owner: ElementId, attr: ElementId);

    /// Whether relation instance `rel` has a player in `role` already (a
    /// positional player takes the first free role).
    ///
    /// v1 (`add_role_player`): `SELECT 1 FROM og_data.og_role_player WHERE
    /// edge_id = $1 AND role_id = $2`, read as `int8` through `spiu::one`; a
    /// failed read is "not filled".
    fn role_filled(&self, rel: ElementId, role: RoleId) -> bool;

    /// Add `player` to relation instance `rel` in `role`; being there already
    /// is not an error.
    ///
    /// v1 (`typeql/write.rs` `add_role_player`): `INSERT INTO
    /// og_data.og_role_player (edge_id, role_id, player_id)` / `VALUES ($1, $2,
    /// $3) ON CONFLICT DO NOTHING` — write.rs's text; `og_add_role_player`
    /// writes the same row with its own text and stays as it is.
    fn add_role_player(&self, rel: ElementId, role: RoleId, player: ElementId) -> Result<(), String>;

    /// Take `player` out of `rel` in any of `roles`.
    ///
    /// v1 (`run_delete`): `DELETE FROM og_data.og_role_player WHERE edge_id = $1
    /// AND player_id = $2 AND role_id = ANY($3)`.
    fn remove_role_player(&self, rel: ElementId, player: ElementId, roles: &[RoleId]) -> Result<(), String>;

    /// The relation instances `player` plays a role in. A failed lookup reads
    /// as none.
    ///
    /// v1 (`delete_instance`): `SELECT DISTINCT edge_id FROM
    /// og_data.og_role_player WHERE player_id = $1`.
    fn relations_played(&self, player: ElementId) -> Vec<ElementId>;

    /// Remove an instance's own storage once the caller has removed what
    /// pointed at it (its ownerships, the relations it plays in): the role
    /// players it has as a relation, its row, its entry. Refuses a virtual type.
    /// Failures of the deletes are ignored, as TypeQL always has.
    ///
    /// v1 (the tail of `delete_instance`, in this order): `DELETE FROM
    /// og_data.og_role_player WHERE edge_id = $1`, `storage::refuse_virtual_write`,
    /// `types::storage_table`, `DELETE FROM <table> WHERE id = $1`, `DELETE FROM
    /// og_data.og_node WHERE id = $1`, `DELETE FROM og_data.og_adj WHERE src = $1`.
    fn remove_instance(&self, id: ElementId);
}

// ---------------------------------------------------------------------- write-side types (K3)

/// A role of a relation type, as `og_catalog.role.role_id` numbers it.
pub type RoleId = i32;

/// Where one type's rows are stored, as [`Layout::type_store`] found it: looked
/// up once and handed back to the write methods that need it, so a run of
/// writes on one type does not look it up again. Opaque outside this module —
/// only a layout makes one and only a layout reads it.
///
/// v1: the type's table, `og_catalog.type.storage_table` (`og_data.a_12` …).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeStore(String);

impl TypeStore {
    /// For a layout: what it found where a type's rows are stored.
    pub fn new(where_stored: impl Into<String>) -> Self {
        TypeStore(where_stored.into())
    }
    /// For the layout that made it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Which step of a write failed, for callers that word the two failures
/// differently (TypeQL's "creating 'x' failed" / "creating 'x' = v failed").
/// The text is the database error's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteError {
    /// Recording the element (v1: its registry row in `og_data.og_node` /
    /// `og_edge`).
    Register(String),
    /// Storing the element's own row (v1: the type's table).
    Store(String),
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WriteError::Register(e) | WriteError::Store(e) => f.write_str(e),
        }
    }
}

/// The layout this build stores graphs in: v1. Each extension crate will pick
/// its own once the crates are split (K7); `ontological` is v1.
pub fn active() -> &'static dyn Layout {
    &crate::store_v1::layout::LayoutV1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_text_round_trips() {
        let f = SqlFrag::new("some_schema.some_view");
        assert_eq!(f.to_string(), "some_schema.some_view");
        assert_eq!(SqlExpr::from("n1.id").as_str(), "n1.id");
        assert_eq!(SqlExpr::from(String::from("x")).into_string(), "x");
    }

    #[test]
    fn dir_follows_the_cypher_ast() {
        use crate::cypher::ast;
        assert_eq!(Dir::from(&ast::Dir::Out), Dir::Out);
        assert_eq!(Dir::from(&ast::Dir::In), Dir::In);
        assert_eq!(Dir::from(&ast::Dir::Both), Dir::Both);
    }

    #[test]
    fn the_layout_is_object_safe() {
        // Naming the type proves it. Taking `active()` would build v1's vtable,
        // and with it link every SPI call its methods reach into this binary,
        // which has no PostgreSQL to resolve them against (macOS refuses to
        // load it: "symbol not found in flat namespace").
        fn takes(_: Option<&dyn Layout>) {}
        takes(None);
    }
}
