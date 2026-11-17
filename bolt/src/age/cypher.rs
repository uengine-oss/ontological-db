//! A Cypher statement, re-printed as the Cypher Apache AGE accepts.
//!
//! The statement is parsed with the engine's own parser (`cypher-syntax`, the
//! same lexer, parser and AST `og_cypher()` uses), which answers the two
//! questions the gateway asks the engine on the default backend: the RETURN
//! column names (`og_cypher_columns`) and whether it writes (`og_cypher_check`).
//! The tree is then printed back as text, with the rewrites AGE needs.
//!
//! What is rewritten, and why (`FEATURE-COMPARISON.md` §4, Robo's column):
//!
//! | Neo4j | AGE |
//! |---|---|
//! | `datetime()`, `localdatetime()`, `date()` | a parameter the gateway fills with the statement's clock, as ISO 8601 text (AGE has no temporal type) |
//! | `datetime($p)` | a parameter holding `$p` written the way PostgreSQL writes a `timestamptz` |
//! | `datetime(x)` | `x` (dates are ISO text here) |
//! | `randomUUID()` | a UUID built from `rand()` in Cypher, one per evaluation |
//! | `elementId(x)` | `toString(id(x))` — what a driver makes the element id from on Bolt 4.4 |
//! | `n:L` as a predicate | `label(n) = 'L'` |
//! | `-[:A\|B]-` in MATCH, EXISTS, COUNT | `-[v]-` and `type(v) IN ['A', 'B']` in that clause's WHERE |
//! | `(n:A\|B)` in MATCH | `(n)` and `label(n) IN ['A', 'B']` |
//! | `xs[i]`, `a \|\| b` | as written (`\|\|` as `+`, which is what it means for text and lists) |
//! | `RETURN *` | the variables in scope, sorted, as Neo4j names the columns |
//!
//! Operator precedence differs (AGE binds `IN`, `IS NULL`, `STARTS WITH`,
//! `CONTAINS`, `=~` tighter than arithmetic, and `XOR` tighter than `AND`), so
//! every operand that is not atomic is parenthesised.
//!
//! What cannot be said in AGE's Cypher at all — `FOREACH`, `CALL { … }`, a label
//! changed by `SET`/`REMOVE` — is not printed here: `split.rs` cuts the statement
//! around it and the gateway runs the parts in one transaction.

use cypher_syntax::ast::*;
use serde_json::{Map, Value as Json};

pub type R<T> = Result<T, String>;

/// Words AGE's grammar reserves (`src/include/parser/cypher_kwlist.h`) and the
/// engine's lexer's: a name spelled like one is back-quoted.
const RESERVED: &[&str] = &[
    "all", "analyze", "and", "any", "as", "asc", "ascending", "by", "call", "case", "coalesce", "contains", "count",
    "create", "delete", "desc", "descending", "detach", "distinct", "else", "end", "ends", "exists", "explain",
    "false", "in", "is", "limit", "match", "merge", "none", "not", "null", "on", "operator", "optional", "or",
    "order", "reduce", "remove", "return", "set", "single", "skip", "starts", "then", "true", "union", "unwind",
    "verbose", "when", "where", "with", "xor", "yield", "drop", "foreach", "load", "csv",
];

/// The prefix of every name the rewrites introduce. Never a column: `RETURN *`
/// leaves them out.
pub const FRESH: &str = "__og_";

/// The parameter that carries the statement's clock.
pub const NOW_PARAM: &str = "__og_now";
pub const TODAY_PARAM: &str = "__og_today";

/// A name as AGE reads it.
pub fn ident(name: &str) -> String {
    let plain = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if plain && !RESERVED.contains(&name.to_ascii_lowercase().as_str()) {
        name.to_string()
    } else {
        format!("`{}`", name.replace('`', "``"))
    }
}

/// A string literal as AGE reads it.
pub fn str_lit(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

fn float_lit(f: f64) -> R<String> {
    if !f.is_finite() {
        return Err("a literal that is not a finite number".into());
    }
    let s = format!("{f:?}");
    Ok(if s.contains('.') {
        s
    } else if let Some(at) = s.find('e') {
        format!("{}.0{}", &s[..at], &s[at..])
    } else {
        format!("{s}.0")
    })
}

/// What a statement needs besides its text.
#[derive(Default)]
pub struct Printer<'a> {
    /// The statement's parameters, for the rewrites that read one.
    pub params: Option<&'a Map<String, Json>>,
    /// Parameters the rewrites add (the clock, canonical datetimes).
    pub extra: Map<String, Json>,
    /// The statement's clock, ISO 8601 UTC, as PostgreSQL writes a timestamptz.
    pub now: String,
    fresh: usize,
}

impl<'a> Printer<'a> {
    pub fn new(params: Option<&'a Map<String, Json>>, now: String) -> Self {
        Printer { params, extra: Map::new(), now, fresh: 0 }
    }

    pub fn fresh(&mut self, what: &str) -> String {
        self.fresh += 1;
        format!("{FRESH}{what}{}", self.fresh)
    }

    // ---------------------------------------------------------------- query

    pub fn query(&mut self, q: &Query) -> R<String> {
        let mut out = Vec::new();
        let mut scope: Vec<String> = Vec::new();
        for c in &q.clauses {
            out.push(self.clause(c, &scope)?);
            scope = scope_after(&scope, c);
        }
        let mut s = out.join(" ");
        if let Some((all, tail)) = &q.union {
            s.push_str(if *all { " UNION ALL " } else { " UNION " });
            s.push_str(&self.query(tail)?);
        }
        Ok(s)
    }

    pub fn clause(&mut self, c: &Clause, scope: &[String]) -> R<String> {
        Ok(match c {
            Clause::Match { patterns, optional, where_ } => {
                let mut patterns = patterns.clone();
                let mut preds = Vec::new();
                for p in &mut patterns {
                    self.lift_alternatives(p, &mut preds);
                }
                let where_ = and_all(where_.clone(), preds);
                let mut s = String::from(if *optional { "OPTIONAL MATCH " } else { "MATCH " });
                s.push_str(&self.patterns(&patterns)?);
                if let Some(w) = &where_ {
                    s.push_str(" WHERE ");
                    s.push_str(&self.expr(w)?);
                }
                s
            }
            Clause::Unwind { expr, alias } => format!("UNWIND {} AS {}", self.operand(expr)?, ident(alias)),
            Clause::With { proj, where_ } => {
                let mut s = format!("WITH {}", self.projection(proj, scope)?);
                if let Some(w) = where_ {
                    s.push_str(" WHERE ");
                    s.push_str(&self.expr(w)?);
                }
                s
            }
            Clause::Return(proj) => format!("RETURN {}", self.projection(proj, scope)?),
            Clause::Create(pats) => {
                for p in pats {
                    no_alternatives(p, "CREATE")?;
                }
                format!("CREATE {}", self.patterns(pats)?)
            }
            Clause::Merge { pattern, on_create, on_match } => {
                no_alternatives(pattern, "MERGE")?;
                let mut s = format!("MERGE {}", self.pattern(pattern)?);
                if !on_create.is_empty() {
                    s.push_str(" ON CREATE SET ");
                    s.push_str(&self.set_items(on_create)?);
                }
                if !on_match.is_empty() {
                    s.push_str(" ON MATCH SET ");
                    s.push_str(&self.set_items(on_match)?);
                }
                s
            }
            Clause::Set(items) => format!("SET {}", self.set_items(items)?),
            Clause::Remove(items) => {
                let mut parts = Vec::new();
                for it in items {
                    match it {
                        SetOp::Prop { var, prop, .. } => parts.push(format!("{}.{}", ident(var), ident(prop))),
                        _ => return Err(unsupported("REMOVE of a label")),
                    }
                }
                format!("REMOVE {}", parts.join(", "))
            }
            Clause::Delete { exprs, detach } => {
                let xs = exprs.iter().map(|e| self.operand(e)).collect::<R<Vec<_>>>()?;
                format!("{}DELETE {}", if *detach { "DETACH " } else { "" }, xs.join(", "))
            }
            Clause::Call { name, .. } => return Err(unsupported(&format!("CALL {name}() inside a query"))),
            Clause::Foreach { .. } => return Err(unsupported("FOREACH here")),
            Clause::Subquery { .. } => return Err(unsupported("CALL { … } here")),
            Clause::Ddl(_) => return Err("a schema command cannot be combined with other clauses".into()),
        })
    }

    fn set_items(&mut self, items: &[SetOp]) -> R<String> {
        let mut parts = Vec::new();
        for it in items {
            parts.push(match it {
                SetOp::Prop { var, prop, value } => format!("{}.{} = {}", ident(var), ident(prop), self.operand(value)?),
                SetOp::Map { var, value, merge } => {
                    format!("{} {} {}", ident(var), if *merge { "+=" } else { "=" }, self.operand(value)?)
                }
                SetOp::Label { .. } => return Err(unsupported("SET of a label")),
            });
        }
        Ok(parts.join(", "))
    }

    fn projection(&mut self, p: &Projection, scope: &[String]) -> R<String> {
        let mut s = String::new();
        if p.distinct {
            s.push_str("DISTINCT ");
        }
        let mut items = Vec::new();
        for it in &p.items {
            if matches!(it.expr, Expr::Star) {
                // `*`: every variable in scope, by name, sorted as Neo4j orders
                // the columns. The rewrites' own variables stay out of sight.
                let mut names: Vec<&String> = scope.iter().filter(|v| !v.starts_with(FRESH)).collect();
                names.sort();
                names.dedup();
                if names.is_empty() {
                    return Err("RETURN * with no variables in scope".into());
                }
                items.extend(names.into_iter().map(|n| ident(n)));
                continue;
            }
            let e = self.expr(&it.expr)?;
            items.push(match &it.alias {
                Some(a) => format!("{e} AS {}", ident(a)),
                None => e,
            });
        }
        s.push_str(&items.join(", "));
        if !p.order.is_empty() {
            let mut keys = Vec::new();
            for o in &p.order {
                keys.push(format!("{}{}", self.expr(&o.expr)?, if o.desc { " DESC" } else { "" }));
            }
            s.push_str(" ORDER BY ");
            s.push_str(&keys.join(", "));
        }
        if let Some(e) = &p.skip {
            s.push_str(" SKIP ");
            s.push_str(&self.operand(e)?);
        }
        if let Some(e) = &p.limit {
            s.push_str(" LIMIT ");
            s.push_str(&self.operand(e)?);
        }
        Ok(s)
    }

    // ------------------------------------------------------------- patterns

    /// `-[:A|B]-` and `(n:A|B)`: AGE takes one label or type per element. The
    /// element gets a variable (a fresh one if it had none) and the choice
    /// becomes a predicate.
    pub fn lift_alternatives(&mut self, p: &mut Pattern, preds: &mut Vec<Expr>) {
        for el in &mut p.elems {
            match el {
                PatElem::Rel(r) if r.types.len() > 1 => {
                    let var = r.var.clone().unwrap_or_else(|| self.fresh("r"));
                    r.var = Some(var.clone());
                    let types = Expr::List(r.types.drain(..).map(|t| Expr::Lit(Lit::Str(t))).collect());
                    let type_in = |v: Expr| {
                        Expr::Binary(BinOp::In, Box::new(Expr::Func { name: "type".into(), args: vec![v], distinct: false }), Box::new(types.clone()))
                    };
                    preds.push(if r.range.is_some() {
                        // A variable-length relationship is a list of them.
                        let x = self.fresh("x");
                        Expr::ListPred {
                            kind: ListPredKind::All,
                            var: x.clone(),
                            source: Box::new(Expr::Var(var)),
                            filter: Box::new(type_in(Expr::Var(x))),
                        }
                    } else {
                        type_in(Expr::Var(var))
                    });
                }
                PatElem::Node(n) if n.labels.len() > 1 => {
                    let var = n.var.clone().unwrap_or_else(|| self.fresh("n"));
                    n.var = Some(var.clone());
                    let labels = Expr::List(n.labels.drain(..).map(|t| Expr::Lit(Lit::Str(t))).collect());
                    preds.push(Expr::Binary(
                        BinOp::In,
                        Box::new(Expr::Func { name: "label".into(), args: vec![Expr::Var(var)], distinct: false }),
                        Box::new(labels),
                    ));
                }
                _ => {}
            }
        }
    }

    fn patterns(&mut self, ps: &[Pattern]) -> R<String> {
        Ok(ps.iter().map(|p| self.pattern(p)).collect::<R<Vec<_>>>()?.join(", "))
    }

    pub fn pattern(&mut self, p: &Pattern) -> R<String> {
        let mut s = String::new();
        if let Some(v) = &p.path_var {
            s.push_str(&ident(v));
            s.push_str(" = ");
        }
        for el in &p.elems {
            match el {
                PatElem::Node(n) => {
                    s.push('(');
                    if let Some(v) = &n.var {
                        s.push_str(&ident(v));
                    }
                    match n.labels.as_slice() {
                        [] => {}
                        [l] => {
                            s.push(':');
                            s.push_str(&ident(l));
                        }
                        _ => return Err(unsupported("a node with more than one label (a vertex in AGE has one)")),
                    }
                    if !n.props.is_empty() {
                        if n.var.is_some() || !n.labels.is_empty() {
                            s.push(' ');
                        }
                        s.push_str(&self.map(&n.props)?);
                    }
                    s.push(')');
                }
                PatElem::Rel(r) => {
                    s.push_str(if r.dir == Dir::In { "<-[" } else { "-[" });
                    if let Some(v) = &r.var {
                        s.push_str(&ident(v));
                    }
                    match r.types.as_slice() {
                        [] => {}
                        [t] => {
                            s.push(':');
                            s.push_str(&ident(t));
                        }
                        _ => return Err(unsupported("a relationship with alternative types here")),
                    }
                    if let Some((lo, hi)) = r.range {
                        s.push_str(&format!("*{lo}..{hi}"));
                    }
                    if !r.props.is_empty() {
                        s.push(' ');
                        s.push_str(&self.map(&r.props)?);
                    }
                    s.push_str(if r.dir == Dir::Out { "]->" } else { "]-" });
                }
            }
        }
        Ok(s)
    }

    fn map(&mut self, kv: &[(String, Expr)]) -> R<String> {
        let mut parts = Vec::new();
        for (k, v) in kv {
            parts.push(format!("{}: {}", ident(k), self.expr(v)?));
        }
        Ok(format!("{{{}}}", parts.join(", ")))
    }

    // ---------------------------------------------------------- expressions

    /// An expression that can stand where an operand goes without changing
    /// what binds to what.
    fn atomic(e: &Expr) -> bool {
        !matches!(e, Expr::Binary(..) | Expr::Not(_) | Expr::Neg(_) | Expr::IsNull(..) | Expr::HasLabels { .. })
    }

    /// `e`, parenthesised unless atomic.
    pub fn operand(&mut self, e: &Expr) -> R<String> {
        let s = self.expr(e)?;
        Ok(if Self::atomic(e) { s } else { format!("({s})") })
    }

    pub fn expr(&mut self, e: &Expr) -> R<String> {
        Ok(match e {
            Expr::Lit(l) => match l {
                Lit::Int(i) => i.to_string(),
                Lit::Float(f) => float_lit(*f)?,
                Lit::Str(s) => str_lit(s),
                Lit::Bool(b) => b.to_string(),
                Lit::Null => "null".into(),
            },
            Expr::Param(p) => format!("${}", param_name(p)?),
            Expr::Var(v) => ident(v),
            Expr::Prop(b, p) => format!("{}.{}", self.operand(b)?, ident(p)),
            Expr::Binary(op, a, b) => {
                let sym = match op {
                    BinOp::Add | BinOp::Concat => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    BinOp::Div => "/",
                    BinOp::Mod => "%",
                    BinOp::Pow => "^",
                    BinOp::Eq => "=",
                    BinOp::Ne => "<>",
                    BinOp::Lt => "<",
                    BinOp::Le => "<=",
                    BinOp::Gt => ">",
                    BinOp::Ge => ">=",
                    BinOp::And => "AND",
                    BinOp::Or => "OR",
                    BinOp::Xor => "XOR",
                    BinOp::In => "IN",
                    BinOp::StartsWith => "STARTS WITH",
                    BinOp::EndsWith => "ENDS WITH",
                    BinOp::Contains => "CONTAINS",
                    BinOp::Regex => "=~",
                };
                // A run of the same AND / OR needs no parentheses inside.
                let side = |this: &mut Self, x: &Expr| -> R<String> {
                    match (op, x) {
                        (BinOp::And, Expr::Binary(BinOp::And, ..)) | (BinOp::Or, Expr::Binary(BinOp::Or, ..)) => this.expr(x),
                        _ => this.operand(x),
                    }
                };
                format!("{} {sym} {}", side(self, a)?, side(self, b)?)
            }
            Expr::Not(a) => format!("NOT {}", self.operand(a)?),
            Expr::Neg(a) => format!("-{}", self.operand(a)?),
            Expr::IsNull(a, is_null) => format!("{} IS {}NULL", self.operand(a)?, if *is_null { "" } else { "NOT " }),
            Expr::Func { name, args, distinct } => self.func(name, args, *distinct)?,
            Expr::List(xs) => {
                format!("[{}]", xs.iter().map(|x| self.expr(x)).collect::<R<Vec<_>>>()?.join(", "))
            }
            Expr::Map(kv) => self.map(kv)?,
            Expr::Case { operand, whens, else_ } => {
                let mut s = String::from("CASE");
                if let Some(o) = operand {
                    s.push(' ');
                    s.push_str(&self.operand(o)?);
                }
                for (w, t) in whens {
                    s.push_str(&format!(" WHEN {} THEN {}", self.expr(w)?, self.expr(t)?));
                }
                if let Some(e) = else_ {
                    s.push_str(&format!(" ELSE {}", self.expr(e)?));
                }
                s.push_str(" END");
                s
            }
            Expr::ListComp { var, source, filter, project } => {
                let mut s = format!("[{} IN {}", ident(var), self.operand(source)?);
                if let Some(f) = filter {
                    s.push_str(&format!(" WHERE {}", self.expr(f)?));
                }
                if let Some(p) = project {
                    s.push_str(&format!(" | {}", self.expr(p)?));
                }
                s.push(']');
                s
            }
            Expr::ListPred { kind, var, source, filter } => {
                let k = match kind {
                    ListPredKind::Any => "any",
                    ListPredKind::All => "all",
                    ListPredKind::None => "none",
                    ListPredKind::Single => "single",
                };
                format!("{k}({} IN {} WHERE {})", ident(var), self.operand(source)?, self.expr(filter)?)
            }
            Expr::MapProjection { var, items } => {
                let mut parts = Vec::new();
                for it in items {
                    parts.push(match it {
                        MapProjItem::Prop(p) => format!(".{}", ident(p)),
                        MapProjItem::All => ".*".into(),
                        MapProjItem::Entry(k, v) => format!("{}: {}", ident(k), self.expr(v)?),
                    });
                }
                format!("{}{{{}}}", ident(var), parts.join(", "))
            }
            Expr::Slice { list, from, to } => {
                let f = match from {
                    Some(x) => self.expr(x)?,
                    None => String::new(),
                };
                let t = match to {
                    Some(x) => self.expr(x)?,
                    None => String::new(),
                };
                format!("{}[{f}..{t}]", self.operand(list)?)
            }
            Expr::HasLabels { var, alts } => {
                // One label per vertex in AGE: `n:A:B` holds only when A and B
                // are the same name.
                let label = format!("label({})", ident(var));
                let one = |ls: &Vec<String>| -> String {
                    ls.iter().map(|l| format!("{label} = {}", str_lit(l))).collect::<Vec<_>>().join(" AND ")
                };
                if alts.len() == 1 && alts[0].len() == 1 {
                    one(&alts[0])
                } else if alts.iter().all(|a| a.len() == 1) {
                    format!("{label} IN [{}]", alts.iter().map(|a| str_lit(&a[0])).collect::<Vec<_>>().join(", "))
                } else {
                    alts.iter().map(|a| format!("({})", one(a))).collect::<Vec<_>>().join(" OR ")
                }
            }
            Expr::Reduce { acc, init, var, list, expr } => format!(
                "reduce({} = {}, {} IN {} | {})",
                ident(acc),
                self.expr(init)?,
                ident(var),
                self.operand(list)?,
                self.expr(expr)?
            ),
            Expr::Subquery { kind, patterns, where_, project: _ } => {
                let mut patterns = patterns.clone();
                let mut preds = Vec::new();
                for p in &mut patterns {
                    self.lift_alternatives(p, &mut preds);
                }
                let where_ = and_all(where_.as_deref().cloned(), preds);
                match kind {
                    SubqueryKind::Exists if patterns.len() == 1 && where_.is_none() && patterns[0].path_var.is_none() => {
                        format!("EXISTS({})", self.pattern(&patterns[0])?)
                    }
                    SubqueryKind::Exists | SubqueryKind::Count => {
                        let mut s = format!(
                            "{} {{ MATCH {}",
                            if *kind == SubqueryKind::Exists { "EXISTS" } else { "COUNT" },
                            self.patterns(&patterns)?
                        );
                        if let Some(w) = &where_ {
                            s.push_str(&format!(" WHERE {}", self.expr(w)?));
                        }
                        s.push_str(" }");
                        s
                    }
                    SubqueryKind::Collect => return Err(unsupported("a pattern comprehension or COLLECT { … }")),
                }
            }
            Expr::Star => "*".into(),
        })
    }

    fn func(&mut self, name: &str, args: &[Expr], distinct: bool) -> R<String> {
        let lname = name.to_ascii_lowercase();
        match (lname.as_str(), args) {
            // `xs[i]` comes out of the parser as a call.
            ("element_at", [list, idx]) => return Ok(format!("{}[{}]", self.operand(list)?, self.expr(idx)?)),
            ("datetime" | "localdatetime", []) => {
                self.extra.insert(NOW_PARAM.into(), Json::String(self.now.clone()));
                return Ok(format!("${NOW_PARAM}"));
            }
            ("date", []) => {
                self.extra.insert(TODAY_PARAM.into(), Json::String(self.now[..10].to_string()));
                return Ok(format!("${TODAY_PARAM}"));
            }
            ("datetime" | "localdatetime", [Expr::Param(p)]) => {
                // The value as PostgreSQL would have written it back.
                let given = self.params.and_then(|m| m.get(p)).cloned().unwrap_or(Json::Null);
                let canon = match &given {
                    Json::String(s) => canonical_datetime(s).map(Json::String).unwrap_or(given.clone()),
                    other => other.clone(),
                };
                let name = format!("{FRESH}dt_{}", param_name(p)?);
                self.extra.insert(name.clone(), canon);
                return Ok(format!("${name}"));
            }
            ("datetime" | "localdatetime", [Expr::Lit(Lit::Str(s))]) => {
                return Ok(str_lit(&canonical_datetime(s).unwrap_or_else(|| s.clone())))
            }
            ("datetime" | "localdatetime", [x]) => return self.expr(x),
            ("date", [x]) => return Ok(format!("left(toString({}), 10)", self.expr(x)?)),
            ("randomuuid", []) => return Ok(uuid_expr()),
            ("elementid", [x]) => return Ok(format!("toString(id({}))", self.expr(x)?)),
            ("nullif", [a, b]) => {
                let (a, b) = (self.operand(a)?, self.operand(b)?);
                return Ok(format!("CASE WHEN {a} = {b} THEN null ELSE {a} END"));
            }
            _ => {}
        }
        let mut s = String::new();
        // A namespaced name goes out as written; AGE takes two parts at most and
        // says so.
        s.push_str(&name.split('.').map(ident_fn).collect::<Vec<_>>().join("."));
        s.push('(');
        if distinct {
            s.push_str("DISTINCT ");
        }
        let xs = args.iter().map(|a| self.expr(a)).collect::<R<Vec<_>>>()?;
        s.push_str(&xs.join(", "));
        s.push(')');
        Ok(s)
    }
}

/// A function name: AGE's keywords that are also functions (`count`,
/// `exists`, `coalesce`) are spelled bare.
fn ident_fn(n: &str) -> String {
    if matches!(n.to_ascii_lowercase().as_str(), "count" | "exists" | "coalesce" | "any" | "all" | "none" | "single" | "reduce") {
        n.to_string()
    } else {
        ident(n)
    }
}

fn param_name(p: &str) -> R<String> {
    if p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !p.is_empty() {
        Ok(p.to_string())
    } else {
        Err(format!("parameter name ${p} cannot be passed to AGE"))
    }
}

pub fn unsupported(what: &str) -> String {
    format!("{what} is not supported on the AGE backend")
}

fn no_alternatives(p: &Pattern, clause: &str) -> R<()> {
    for el in &p.elems {
        match el {
            PatElem::Rel(r) if r.types.len() > 1 => return Err(format!("{clause} needs one relationship type")),
            PatElem::Node(n) if n.labels.len() > 1 => {
                return Err(unsupported(&format!("{clause} of a node with more than one label")))
            }
            _ => {}
        }
    }
    Ok(())
}

/// `w AND p1 AND p2 …`.
pub fn and_all(w: Option<Expr>, preds: Vec<Expr>) -> Option<Expr> {
    let mut out = w;
    for p in preds {
        out = Some(match out {
            None => p,
            Some(x) => Expr::Binary(BinOp::And, Box::new(x), Box::new(p)),
        });
    }
    out
}

/// A version-4 UUID, built in Cypher from `rand()` so that every evaluation —
/// every row of an `UNWIND` — gets its own. AGE has no `randomUUID()`.
fn uuid_expr() -> String {
    "reduce(s = '', i IN range(0, 35) | s + CASE WHEN i IN [8, 13, 18, 23] THEN '-' WHEN i = 14 THEN '4' \
     WHEN i = 19 THEN substring('89ab', toInteger(floor(rand() * 4)), 1) \
     ELSE substring('0123456789abcdef', toInteger(floor(rand() * 16)), 1) END)"
        .into()
}

/// An ISO 8601 date-time as PostgreSQL writes a `timestamptz` on a UTC session
/// (`2026-01-11T09:00:00+00:00`, fraction without trailing zeros). `None` when
/// it does not read as one.
pub fn canonical_datetime(s: &str) -> Option<String> {
    let b = s.trim();
    let date = b.get(..10)?;
    let (y, m, d) = (date.get(..4)?.parse::<i64>().ok()?, date.get(5..7)?.parse::<i64>().ok()?, date.get(8..10)?.parse::<i64>().ok()?);
    if date.as_bytes()[4] != b'-' || date.as_bytes()[7] != b'-' {
        return None;
    }
    let rest = &b[10..];
    let (mut hh, mut mi, mut ss, mut frac, mut off) = (0i64, 0i64, 0i64, String::new(), 0i64);
    if !rest.is_empty() {
        let rest = rest.strip_prefix('T').or_else(|| rest.strip_prefix(' '))?;
        hh = rest.get(..2)?.parse().ok()?;
        mi = rest.get(3..5)?.parse().ok()?;
        let mut tail = rest.get(5..)?;
        if let Some(t) = tail.strip_prefix(':') {
            ss = t.get(..2)?.parse().ok()?;
            tail = &t[2..];
            if let Some(t) = tail.strip_prefix('.') {
                let n = t.bytes().take_while(u8::is_ascii_digit).count();
                frac = t[..n].to_string();
                tail = &t[n..];
            }
        }
        // a zone name in brackets is not something to compute with here
        let tail = tail.split('[').next().unwrap_or("");
        if tail == "Z" || tail.is_empty() {
            off = 0;
        } else {
            let sign = if tail.starts_with('-') { -1 } else if tail.starts_with('+') { 1 } else { return None };
            let digits: String = tail[1..].chars().filter(|c| c.is_ascii_digit()).collect();
            let oh: i64 = digits.get(..2)?.parse().ok()?;
            let om: i64 = digits.get(2..4).unwrap_or("0").parse().ok()?;
            off = sign * (oh * 3600 + om * 60);
        }
    }
    // to UTC
    let days = days_from_civil(y, m, d);
    let secs = days * 86_400 + hh * 3600 + mi * 60 + ss - off;
    let (day, tod) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(day);
    let mut frac = frac;
    frac.truncate(6);
    let frac = frac.trim_end_matches('0');
    Some(format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}{}+00:00",
        tod / 3600,
        (tod / 60) % 60,
        tod % 60,
        if frac.is_empty() { String::new() } else { format!(".{frac}") }
    ))
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// The clock now, written as [`canonical_datetime`] writes one.
pub fn now_text() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = t.as_secs() as i64;
    let micros = t.subsec_micros();
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let tod = secs.rem_euclid(86_400);
    let frac = format!("{micros:06}");
    let frac = frac.trim_end_matches('0');
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}{}+00:00",
        tod / 3600,
        (tod / 60) % 60,
        tod % 60,
        if frac.is_empty() { String::new() } else { format!(".{frac}") }
    )
}

// ------------------------------------------------------------------ scope

/// The variables a pattern binds.
pub fn pattern_vars(p: &Pattern, out: &mut Vec<String>) {
    if let Some(v) = &p.path_var {
        out.push(v.clone());
    }
    for el in &p.elems {
        let v = match el {
            PatElem::Node(n) => &n.var,
            PatElem::Rel(r) => &r.var,
        };
        if let Some(v) = v {
            if !out.contains(v) {
                out.push(v.clone());
            }
        }
    }
}

/// The column a projection item becomes (`og_cypher_columns`' rule).
pub fn item_name(it: &ReturnItem) -> String {
    it.alias.clone().unwrap_or_else(|| it.expr.default_alias())
}

/// The variables in scope after a clause.
pub fn scope_after(scope: &[String], c: &Clause) -> Vec<String> {
    let mut out = scope.to_vec();
    match c {
        Clause::Match { patterns, .. } | Clause::Create(patterns) => {
            for p in patterns {
                pattern_vars(p, &mut out);
            }
        }
        Clause::Merge { pattern, .. } => pattern_vars(pattern, &mut out),
        Clause::Unwind { alias, .. } => {
            if !out.contains(alias) {
                out.push(alias.clone());
            }
        }
        Clause::With { proj, .. } | Clause::Return(proj) => {
            let star = proj.items.iter().any(|i| matches!(i.expr, Expr::Star));
            if !star {
                out.clear();
            }
            for it in &proj.items {
                if matches!(it.expr, Expr::Star) {
                    continue;
                }
                let n = item_name(it);
                if !out.contains(&n) {
                    out.push(n);
                }
            }
        }
        Clause::Call { yields, .. } => {
            for (_, a) in yields {
                if !out.contains(a) {
                    out.push(a.clone());
                }
            }
        }
        Clause::Subquery { body, .. } => {
            if let Some(Clause::Return(proj)) = body.clauses.last() {
                for it in &proj.items {
                    let n = item_name(it);
                    if !out.contains(&n) {
                        out.push(n);
                    }
                }
            }
        }
        _ => {}
    }
    out
}

/// The variables in scope after the clauses.
pub fn scope_of(clauses: &[Clause]) -> Vec<String> {
    let mut s = Vec::new();
    for c in clauses {
        s = scope_after(&s, c);
    }
    s
}

/// The result columns, in RETURN order — what the engine's
/// `og_cypher_columns()` answers, and `RETURN *` spelled out.
pub fn columns(q: &Query) -> Vec<String> {
    let Some(at) = q.clauses.iter().rposition(|c| matches!(c, Clause::Return(_))) else { return Vec::new() };
    let Clause::Return(proj) = &q.clauses[at] else { unreachable!() };
    let scope = scope_of(&q.clauses[..at]);
    let mut out = Vec::new();
    for it in &proj.items {
        if matches!(it.expr, Expr::Star) {
            let mut names: Vec<String> = scope.iter().filter(|v| !v.starts_with(FRESH)).cloned().collect();
            names.sort();
            names.dedup();
            out.extend(names);
        } else {
            out.push(item_name(it));
        }
    }
    out
}

/// Does the statement write? The engine's `is_write`, on the same tree.
pub fn is_write(q: &Query) -> bool {
    if let Some((_, tail)) = &q.union {
        if is_write(tail) {
            return true;
        }
    }
    q.clauses.iter().any(|c| {
        matches!(
            c,
            Clause::Create(_)
                | Clause::Merge { .. }
                | Clause::Set(_)
                | Clause::Remove(_)
                | Clause::Delete { .. }
                | Clause::Foreach { .. }
                | Clause::Ddl(_)
        ) || matches!(c, Clause::Subquery { body, .. } if is_write(body))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn print(q: &str) -> String {
        let ast = cypher_syntax::parse(q).unwrap();
        let mut p = Printer::new(None, "2026-11-18T01:02:03.5+00:00".into());
        p.query(&ast).unwrap()
    }

    #[test]
    fn plain_queries_come_back_as_they_were() {
        assert_eq!(print("MATCH (a:Person {name: $n})-[r:KNOWS]->(b) WHERE a.age > 30 RETURN b.name AS n"),
            "MATCH (a:Person {name: $n})-[r:KNOWS]->(b) WHERE a.age > 30 RETURN b.name AS n");
        assert_eq!(print("OPTIONAL MATCH (a)<-[:R*1..3]-(b) RETURN count(*) AS count ORDER BY count DESC LIMIT 5"),
            "OPTIONAL MATCH (a)<-[:R*1..3]-(b) RETURN count(*) AS `count` ORDER BY `count` DESC LIMIT 5");
        assert_eq!(print("MERGE (n:L {k: 1}) ON CREATE SET n.c = 1 ON MATCH SET n.m = 2 SET n += $p RETURN n"),
            "MERGE (n:L {k: 1}) ON CREATE SET n.c = 1 ON MATCH SET n.m = 2 SET n += $p RETURN n");
        assert_eq!(print("UNWIND $rows AS r WITH r WHERE r.x IS NOT NULL MATCH (n) WHERE n.id = r.id DETACH DELETE n"),
            "UNWIND $rows AS r WITH r WHERE r.x IS NOT NULL MATCH (n) WHERE n.id = r.id DETACH DELETE n");
        // `*..` gets the engine's bound, as on v1; the engine's parser reads a
        // bare `*` as one hop, and so it goes out.
        assert_eq!(print("MATCH (a)-[*..]->(b) RETURN b"), "MATCH (a)-[*1..8]->(b) RETURN b");
        assert_eq!(print("MATCH (a)-[*]->(b) RETURN b"), "MATCH (a)-[*1..1]->(b) RETURN b");
        assert_eq!(print("RETURN [x IN $xs WHERE x > 1 | x * 2] AS d, n {.a, .*, k: 1} AS m, xs[0] AS h, xs[1..] AS t"),
            "RETURN [x IN $xs WHERE x > 1 | x * 2] AS d, n{.a, .*, k: 1} AS m, xs[0] AS h, xs[1..] AS t");
    }

    #[test]
    fn precedence_is_spelled_out() {
        // AGE binds IN and STARTS WITH tighter than `+`, and XOR tighter than AND.
        assert_eq!(print("RETURN a + b IN xs AS v"), "RETURN (a + b) IN xs AS v");
        assert_eq!(print("RETURN n.s STARTS WITH $p + 'x' AS v"), "RETURN n.s STARTS WITH ($p + 'x') AS v");
        assert_eq!(print("RETURN a AND b XOR c AS v"), "RETURN (a AND b) XOR c AS v");
        assert_eq!(print("MATCH (n) WHERE n.a = 1 AND n.b = 2 AND NOT n.c = 3 RETURN n"),
            "MATCH (n) WHERE (n.a = 1) AND (n.b = 2) AND (NOT (n.c = 3)) RETURN n");
        // The engine's parser reads `-x ^ 2` as `(-x) ^ 2`; that is what goes out.
        assert_eq!(print("RETURN -x ^ 2 AS v, 1.0 AS f, 1e21 AS g, 'it\\'s' AS s"),
            "RETURN (-x) ^ 2 AS v, 1.0 AS f, 1.0e21 AS g, 'it\\'s' AS s");
    }

    #[test]
    fn neo4j_functions_become_age_ones() {
        let ast = cypher_syntax::parse("CREATE (n:N {at: datetime(), id: randomUUID(), c: datetime($c), d: datetime(n.x)}) RETURN elementId(n) AS e").unwrap();
        let params: Map<String, Json> = [("c".to_string(), Json::String("2026-01-11T18:00:00+09:00".into()))].into_iter().collect();
        let mut p = Printer::new(Some(&params), "2026-11-18T01:02:03.5+00:00".into());
        let s = p.query(&ast).unwrap();
        assert!(s.starts_with("CREATE (n:N {at: $__og_now, id: reduce(s = '', i IN range(0, 35) |"), "{s}");
        assert!(s.ends_with("c: $__og_dt_c, d: n.x}) RETURN toString(id(n)) AS e"), "{s}");
        assert_eq!(p.extra.get("__og_now"), Some(&Json::String("2026-11-18T01:02:03.5+00:00".into())));
        assert_eq!(p.extra.get("__og_dt_c"), Some(&Json::String("2026-01-11T09:00:00+00:00".into())));
    }

    #[test]
    fn label_predicates_and_type_alternatives() {
        assert_eq!(print("MATCH (n) WHERE n:A OR NOT n:B RETURN n"),
            "MATCH (n) WHERE (label(n) = 'A') OR (NOT (label(n) = 'B')) RETURN n");
        assert_eq!(print("MATCH (n) WHERE n:A|B RETURN n"), "MATCH (n) WHERE label(n) IN ['A', 'B'] RETURN n");
        assert_eq!(print("MATCH (a)-[:X|Y]->(b) WHERE a.k = 1 RETURN b"),
            "MATCH (a)-[__og_r1]->(b) WHERE (a.k = 1) AND (type(__og_r1) IN ['X', 'Y']) RETURN b");
        assert_eq!(print("MATCH (a)-[r:X|:Y*1..2]->(b) RETURN b"),
            "MATCH (a)-[r*1..2]->(b) WHERE all(__og_x1 IN r WHERE type(__og_x1) IN ['X', 'Y']) RETURN b");
        assert_eq!(print("MATCH (a) WHERE NOT (a)-[:X|Y]->() RETURN a"),
            "MATCH (a) WHERE NOT EXISTS { MATCH (a)-[__og_r1]->() WHERE type(__og_r1) IN ['X', 'Y'] } RETURN a");
        assert_eq!(print("MATCH (a) WHERE (a)-[:X]->(:B) RETURN a"), "MATCH (a) WHERE EXISTS((a)-[:X]->(:B)) RETURN a");
        assert_eq!(print("MATCH (n:A|B) RETURN n"), "MATCH (n) WHERE label(n) IN ['A', 'B'] RETURN n");
    }

    #[test]
    fn columns_and_writes_are_the_engines() {
        let q = cypher_syntax::parse("MATCH (a)-[r]->(b) WITH a, b, count(r) AS c RETURN a.name, c, b").unwrap();
        assert_eq!(columns(&q), vec!["a.name", "c", "b"]);
        assert!(!is_write(&q));
        let q = cypher_syntax::parse("MATCH (b)-[r]->(a) RETURN *").unwrap();
        assert_eq!(columns(&q), vec!["a", "b", "r"]);
        assert_eq!(print("MATCH (b)-[r]->(a) RETURN *"), "MATCH (b)-[r]->(a) RETURN a, b, r");
        assert!(is_write(&cypher_syntax::parse("MATCH (n) SET n.x = 1").unwrap()));
        assert!(columns(&cypher_syntax::parse("CREATE (n)").unwrap()).is_empty());
    }

    #[test]
    fn names_that_are_words_are_quoted() {
        assert_eq!(ident("end"), "`end`");
        assert_eq!(ident("회의실"), "`회의실`");
        assert_eq!(ident("a`b"), "`a``b`");
        assert_eq!(ident("plain_1"), "plain_1");
        assert_eq!(str_lit("a'b\\c\n"), "'a\\'b\\\\c\\n'");
    }

    #[test]
    fn datetimes_are_written_as_postgres_writes_them() {
        assert_eq!(canonical_datetime("2026-01-11T09:00:00Z").as_deref(), Some("2026-01-11T09:00:00+00:00"));
        assert_eq!(canonical_datetime("2026-01-11T09:00:00.120+09:00").as_deref(), Some("2026-01-11T00:00:00.12+00:00"));
        assert_eq!(canonical_datetime("2026-01-01").as_deref(), Some("2026-01-01T00:00:00+00:00"));
        assert_eq!(canonical_datetime("2026-03-01T00:30:00+01:00").as_deref(), Some("2026-02-28T23:30:00+00:00"));
        assert_eq!(canonical_datetime("not a date"), None);
        let now = now_text();
        assert!(now.ends_with("+00:00") && now.as_bytes()[10] == b'T', "{now}");
    }
}
