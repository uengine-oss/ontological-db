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
//! **Nothing calls this yet.** K1 only defines it; K2 (Cypher read), K3 (Cypher
//! write and the storage API), K4 (TypeQL) and K5 (traversal) move the code that
//! writes physical names into [`v1::LayoutV1`], one call site at a time, and the
//! generated-SQL mold (`tests/seam/`) checks that every statement comes out
//! byte for byte as before. That is why the methods take what the call sites
//! have in hand today — an alias, a type id, a caller-minted alias for the
//! layout's own scan — and why each method's documentation says exactly what
//! v1 writes: that text is what the moved code must keep producing.
//!
//! Rules for the cards that fill it in (`P1-CARDS.md`): add methods, do not
//! rename or remove them (several cards edit this file in parallel); keep v1's
//! output identical (`scripts/mold.sh check`); semantics — which types a label
//! means, what a pattern matches — stay in the compilers. A layout decides
//! *where* things are, never *what* is asked.
//!
//! Note the name clash: [`crate::cypher::views::Layout`] is an older struct (the
//! columns of one union view). Inside `views.rs` refer to this trait by its path.

pub mod v1;

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
    /// v1 (`cypher/compile.rs` `bind_node`, `graph_nodes`; `cypher/views.rs`):
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
    fn frontier_step(&self, dir: Dir, frontier: &str, types: &str) -> String;

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
}

/// The layout this build stores graphs in: v1. Each extension crate will pick
/// its own once the crates are split (K7); `ontological` is v1.
pub fn active() -> &'static dyn Layout {
    &v1::LayoutV1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_text_round_trips() {
        let f = SqlFrag::new("og_data.v_1");
        assert_eq!(f.to_string(), "og_data.v_1");
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
