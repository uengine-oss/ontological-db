//! TypeQL abstract syntax — spec 010 T002.
//!
//! A TypeQL query is a **pipeline of stages**, not a tree of clauses. Each stage
//! consumes the row stream the previous one produced. Keeping that shape in the
//! AST is what makes `sort` before `limit` mean something different from `limit`
//! before `sort` (FR-036) without any special-casing later.

use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Query {
    pub stages: Vec<Stage>,
}

#[derive(Clone, Debug)]
pub enum Stage {
    Define(Vec<Definition>),
    Undefine(Vec<Definition>),
    Match(Vec<Pat>),
    Insert(Vec<Pat>),
    Put(Vec<Pat>),
    Update(Vec<Pat>),
    Delete(Vec<DeleteItem>),
    Select(Vec<String>),
    Sort(Vec<(String, bool)>),
    Limit(i64),
    Offset(i64),
    Distinct,
    Reduce { assigns: Vec<Reduction>, groupby: Vec<String> },
    Fetch(FetchDoc),
}

// --------------------------------------------------------------------------
// Schema
// --------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Entity,
    Relation,
    Attribute,
}

#[derive(Clone, Debug, Default)]
pub struct Annotations {
    pub abstract_: bool,
    pub key: bool,
    pub unique: bool,
    pub card: Option<(i64, Option<i64>)>,
    pub values: Vec<Lit>,
    pub range: Option<(Option<Lit>, Option<Lit>)>,
}

#[derive(Clone, Debug)]
pub enum Definition {
    /// `entity book @abstract, owns title, plays contribution:work;`
    Type {
        kind: Kind,
        name: String,
        sub: Option<String>,
        value_type: Option<String>,
        anns: Annotations,
        owns: Vec<Owns>,
        relates: Vec<Relates>,
        plays: Vec<Plays>,
    },
    /// `fun name($a: t) -> { r }: match ... return ...;`
    Function(FunctionDef),
}

#[derive(Clone, Debug)]
pub struct Owns {
    pub attribute: String,
    pub anns: Annotations,
}

#[derive(Clone, Debug)]
pub struct Relates {
    pub role: String,
    /// `relates author as contributor` — the specialised supertype role.
    pub specialises: Option<String>,
    pub anns: Annotations,
}

#[derive(Clone, Debug)]
pub struct Plays {
    pub relation: String,
    pub role: String,
}

#[derive(Clone, Debug)]
pub struct FunctionDef {
    pub name: String,
    pub params: Vec<(String, String)>,
    /// `-> { book }` is a stream, `-> double` is a single value.
    pub returns_stream: bool,
    pub return_types: Vec<String>,
    /// Verbatim source of the whole declaration, preserved for round-tripping.
    pub source: String,
}

// --------------------------------------------------------------------------
// Patterns
// --------------------------------------------------------------------------

/// A normalised pattern primitive. The parser flattens TypeQL's comma-separated
/// statements into these so the compiler never has to re-derive the subject.
#[derive(Clone, Debug)]
pub enum Pat {
    /// `$x isa book` — `exact` is the `isa!` form.
    Isa { var: String, ty: String, exact: bool },
    /// `$owner has genre $g` / `has genre "sf"` / `has price > 10`
    Has { owner: String, attr: Option<String>, val: HasVal },
    /// One role player of a relation instance.
    Role { rel: String, role: Option<String>, player: String },
    /// A relation instance whose role players are given positionally.
    Cmp { lhs: Expr, op: CmpOp, rhs: Expr },
    Let { var: String, expr: Expr },
    Or(Vec<Vec<Pat>>),
    Not(Vec<Pat>),
    Sub { var: String, ty: String, exact: bool },
    Label { var: String, name: String },
    /// `$x isa order-line` where the instance is *created* with these attributes
    /// (insert only) — carried as `Has` plus this marker for anonymous subjects.
    IsaAnon { var: String, ty: String },
}

#[derive(Clone, Debug)]
pub enum HasVal {
    /// `has genre $g` — binds the attribute instance.
    Var(String),
    /// `has genre "sf"`, `has price > 10`
    Cmp(CmpOp, Expr),
    /// `has genre` — existence only.
    Any,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Like,
    Contains,
}

impl CmpOp {
    pub fn sql(self) -> &'static str {
        match self {
            CmpOp::Eq => "=",
            CmpOp::Ne => "<>",
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
            CmpOp::Like => "~",
            CmpOp::Contains => "ILIKE",
        }
    }
}

#[derive(Clone, Debug)]
pub struct DeleteItem {
    /// `delete $x;` removes the instance; `delete has $a of $x;` removes one
    /// ownership; `delete links (role: $p) of $r;` removes one role player.
    pub kind: DeleteKind,
}

#[derive(Clone, Debug)]
pub enum DeleteKind {
    Instance(String),
    Has { owner: String, attr: String },
    Links { rel: String, role: Option<String>, player: String },
}

// --------------------------------------------------------------------------
// Expressions
// --------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Lit {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    DateTime(String),
}

#[derive(Clone, Debug)]
pub enum Expr {
    Lit(Lit),
    Var(String),
    /// `$book.title`
    Attr(String, String),
    Bin(Box<Expr>, &'static str, Box<Expr>),
    Neg(Box<Expr>),
    Call(String, Vec<Expr>),
}

#[derive(Clone, Debug)]
pub struct Reduction {
    pub var: String,
    pub agg: String,
    pub arg: Option<String>,
}

// --------------------------------------------------------------------------
// Fetch
// --------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct FetchDoc {
    /// Ordered so the emitted JSON is deterministic; BTreeMap would reorder the
    /// author's keys, which makes diffing against documented output harder.
    pub entries: Vec<(String, FetchVal)>,
}

#[derive(Clone, Debug)]
pub enum FetchVal {
    Expr(Expr),
    /// `"authors": [ match ...; fetch { ... }; ]`
    SubFetch(Box<Query>),
    Doc(FetchDoc),
    /// `"genres": [ $book.genre ]` — every value of a multi-valued attribute.
    AttrList(String, String),
}

/// Only used by the schema dump, kept here so both sides agree on the shape.
pub type SchemaDump = BTreeMap<String, Vec<String>>;
