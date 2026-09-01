//! Cypher abstract syntax tree — spec 003.

#[derive(Debug, Clone, PartialEq)]
pub enum Dir {
    Out,
    In,
    Both,
}

#[derive(Debug, Clone)]
pub struct NodePat {
    pub var: Option<String>,
    pub labels: Vec<String>,
    pub props: Vec<(String, Expr)>,
}

#[derive(Debug, Clone)]
pub struct RelPat {
    pub var: Option<String>,
    pub types: Vec<String>,
    pub dir: Dir,
    pub props: Vec<(String, Expr)>,
    /// `*min..max` — `None` means a single hop.
    pub range: Option<(u32, u32)>,
}

#[derive(Debug, Clone)]
pub enum PatElem {
    Node(NodePat),
    Rel(RelPat),
}

#[derive(Debug, Clone)]
pub struct Pattern {
    /// Optional path variable: `p = (a)-[r]->(b)`
    pub path_var: Option<String>,
    pub elems: Vec<PatElem>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Lit {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Null,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Xor,
    In,
    StartsWith,
    EndsWith,
    Contains,
    Regex,
    Concat,
}

#[derive(Debug, Clone)]
pub enum Expr {
    Lit(Lit),
    Param(String),
    Var(String),
    Prop(Box<Expr>, String),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    Neg(Box<Expr>),
    IsNull(Box<Expr>, bool),
    /// `n:Label` used as a predicate — `WHERE (n:Aggregate AND …)`.
    /// Distinct from a pattern's label, which constrains the match itself.
    HasLabel(Box<Expr>, Vec<String>),
    /// A pattern used as a predicate — `WHERE NOT (us)-[:IMPLEMENTS]->(:BC)`.
    /// True when at least one match exists, correlated to the enclosing query.
    PatternPred(Box<Pattern>),
    Func { name: String, args: Vec<Expr>, distinct: bool },
    List(Vec<Expr>),
    Map(Vec<(String, Expr)>),
    Case { operand: Option<Box<Expr>>, whens: Vec<(Expr, Expr)>, else_: Option<Box<Expr>> },
    /// `[x IN list WHERE pred | projection]` — the `WHERE` and the `|` half are
    /// each optional, so this also covers the plain map form `[x IN list | e]`.
    ListComp {
        var: String,
        source: Box<Expr>,
        filter: Option<Box<Expr>>,
        project: Option<Box<Expr>>,
    },
    /// `any(x IN list WHERE pred)` and its siblings.
    ListPred { kind: ListPredKind, var: String, source: Box<Expr>, filter: Box<Expr> },
    /// Map projection — `n { .name, .age, total: count(x), .* }`.
    MapProjection { var: String, items: Vec<MapProjItem> },
    /// `labels(n)` style pseudo-property used by RETURN projection
    Star,
}

#[derive(Debug, Clone)]
pub enum MapProjItem {
    /// `.name` — that property of the projected element.
    Prop(String),
    /// `.*` — every property it has.
    All,
    /// `key: expr` — a computed entry.
    Entry(String, Expr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListPredKind {
    Any,
    All,
    None,
    Single,
}

#[derive(Debug, Clone)]
pub struct ReturnItem {
    pub expr: Expr,
    pub alias: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OrderItem {
    pub expr: Expr,
    pub desc: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Projection {
    pub items: Vec<ReturnItem>,
    pub distinct: bool,
    pub order: Vec<OrderItem>,
    pub skip: Option<Expr>,
    pub limit: Option<Expr>,
}

#[derive(Debug, Clone)]
pub enum SetOp {
    /// `SET a.name = expr`
    Prop { var: String, prop: String, value: Expr },
    /// `SET a += {..}` / `SET a = {..}`
    Map { var: String, value: Expr, merge: bool },
    /// `SET a:Label`
    Label { var: String, labels: Vec<String> },
}

#[derive(Debug, Clone)]
pub enum Clause {
    Match { patterns: Vec<Pattern>, optional: bool, where_: Option<Expr> },
    Unwind { expr: Expr, alias: String },
    With { proj: Projection, where_: Option<Expr> },
    Return(Projection),
    Create(Vec<Pattern>),
    Merge { pattern: Pattern, on_create: Vec<SetOp>, on_match: Vec<SetOp> },
    Set(Vec<SetOp>),
    Remove(Vec<SetOp>),
    Delete { exprs: Vec<Expr>, detach: bool },
    /// `CALL ns.proc(args) YIELD a, b AS c` — the Neo4j procedure surface.
    /// Procedures are resolved by their Neo4j name against a fixed registry;
    /// there is no user-defined procedure mechanism (see `compat::procs`).
    Call { name: String, args: Vec<Expr>, yields: Vec<(String, String)> },
    /// Index and constraint DDL. Cypher applications send these at startup, so
    /// accepting them is part of speaking Cypher, not an extra.
    Ddl(Ddl),
}

#[derive(Debug, Clone)]
pub enum Ddl {
    CreateIndex {
        name: Option<String>,
        kind: IndexKind,
        /// `true` for `FOR ()-[r:T]-()`, the relationship form.
        on_relationship: bool,
        label: String,
        props: Vec<String>,
        options: Vec<(String, Expr)>,
        if_not_exists: bool,
    },
    CreateConstraint {
        name: Option<String>,
        label: String,
        props: Vec<String>,
        kind: ConstraintKind,
        if_not_exists: bool,
    },
    Drop { name: String, if_exists: bool, constraint: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexKind {
    /// Plain lookup index — Neo4j's `RANGE`/`TEXT`/`POINT`/default all land here.
    Btree,
    Vector,
    Fulltext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintKind {
    Unique,
    NotNull,
    NodeKey,
}

#[derive(Debug, Clone)]
pub struct Query {
    pub clauses: Vec<Clause>,
    /// `UNION` / `UNION ALL` continuation
    pub union: Option<(bool, Box<Query>)>,
}

impl Expr {
    pub fn is_aggregate(&self) -> bool {
        match self {
            Expr::Func { name, args, .. } => {
                const AGGS: &[&str] =
                    &["count", "sum", "avg", "min", "max", "collect", "stdev", "percentile"];
                AGGS.contains(&name.to_ascii_lowercase().as_str())
                    || args.iter().any(|a| a.is_aggregate())
            }
            Expr::Binary(_, a, b) => a.is_aggregate() || b.is_aggregate(),
            Expr::Not(a) | Expr::Neg(a) | Expr::IsNull(a, _) | Expr::HasLabel(a, _) => a.is_aggregate(),
            Expr::PatternPred(_) => false,
            Expr::List(xs) => xs.iter().any(|x| x.is_aggregate()),
            Expr::Map(kv) => kv.iter().any(|(_, v)| v.is_aggregate()),
            Expr::Case { operand, whens, else_ } => {
                operand.as_ref().is_some_and(|o| o.is_aggregate())
                    || whens.iter().any(|(a, b)| a.is_aggregate() || b.is_aggregate())
                    || else_.as_ref().is_some_and(|e| e.is_aggregate())
            }
            _ => false,
        }
    }

    /// Best-effort display name for an un-aliased RETURN item, matching
    /// openCypher's column naming.
    pub fn default_alias(&self) -> String {
        match self {
            Expr::Var(v) => v.clone(),
            Expr::Prop(b, p) => match &**b {
                Expr::Var(v) => format!("{v}.{p}"),
                _ => p.clone(),
            },
            Expr::Func { name, args, .. } => {
                let inner =
                    args.iter().map(|a| a.default_alias()).collect::<Vec<_>>().join(", ");
                format!("{name}({inner})")
            }
            Expr::Lit(Lit::Str(s)) => format!("\"{s}\""),
            Expr::Lit(Lit::Int(i)) => i.to_string(),
            Expr::Lit(Lit::Float(f)) => f.to_string(),
            Expr::Lit(Lit::Bool(b)) => b.to_string(),
            Expr::Lit(Lit::Null) => "NULL".into(),
            Expr::Param(p) => format!("${p}"),
            Expr::Star => "*".into(),
            _ => "expr".into(),
        }
    }
}
