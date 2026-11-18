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
//! | `(n:A:B)` in MATCH (both labels), A ≠ B | `(n:A)` and `false` — a vertex has one label ([`node_label_test`]) |
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

/// The most hops a variable-length relationship may have without exactly one
/// type: AGE's VLE enumerates trails, and filtering the types afterwards means
/// walking every relationship of the graph first.
pub const MAX_UNTYPED_HOPS: u32 = 3;

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
    /// The variables the expression being printed can see.
    in_scope: Vec<String>,
}

impl<'a> Printer<'a> {
    pub fn new(params: Option<&'a Map<String, Json>>, now: String) -> Self {
        Printer { params, extra: Map::new(), now, fresh: 0, in_scope: Vec::new() }
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
        let mut seen = scope.to_vec();
        for v in scope_after(scope, c) {
            if !seen.contains(&v) {
                seen.push(v);
            }
        }
        self.in_scope = seen;
        // AGE takes a whole-map parameter in SET only once it is bound to a
        // variable (`SET n += $p` is refused; `WITH $p AS p … SET n += p` is
        // not), so such a clause gets a WITH in front that binds it.
        if let Some((with, c2)) = self.bind_map_params(c, scope) {
            return Ok(format!("{with} {}", self.clause_text(&c2, scope)?));
        }
        self.clause_text(c, scope)
    }

    fn bind_map_params(&mut self, c: &Clause, scope: &[String]) -> Option<(String, Clause)> {
        let mut c = c.clone();
        let mut bound: Vec<(String, String)> = Vec::new();
        let mut fix = |items: &mut Vec<SetOp>, this: &mut Self| {
            for it in items.iter_mut() {
                if let SetOp::Map { value: Expr::Param(p), .. } = it {
                    let alias = match bound.iter().find(|(q, _)| q == p) {
                        Some((_, a)) => a.clone(),
                        None => {
                            let a = this.fresh("p");
                            bound.push((p.clone(), a.clone()));
                            a
                        }
                    };
                    if let SetOp::Map { value, .. } = it {
                        *value = Expr::Var(alias);
                    }
                }
            }
        };
        match &mut c {
            Clause::Set(items) => fix(items, self),
            Clause::Merge { on_create, on_match, .. } => {
                fix(on_create, self);
                fix(on_match, self);
            }
            _ => {}
        }
        if bound.is_empty() {
            return None;
        }
        let mut items: Vec<String> = scope.iter().map(|v| ident(v)).collect();
        for (p, a) in &bound {
            items.push(format!("${} AS {}", p, ident(a)));
        }
        Some((format!("WITH {}", items.join(", ")), c))
    }

    fn clause_text(&mut self, c: &Clause, scope: &[String]) -> R<String> {
        Ok(match c {
            Clause::Match { patterns, optional, where_ } => {
                let mut patterns = patterns.clone();
                let mut preds = Vec::new();
                for p in &mut patterns {
                    self.lift_alternatives(p, &mut preds)?;
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
                let key = order_through_projection(&o.expr, &p.items);
                keys.push(format!("{}{}", self.expr(key.as_ref().unwrap_or(&o.expr))?, if o.desc { " DESC" } else { "" }));
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

    /// `-[:A|B]-`, `(n:A|B)` and `(n:A:B)`: AGE takes one label or type per
    /// element. The element gets a variable (a fresh one if it had none) and
    /// the choice becomes a predicate; what a node's labels mean is
    /// [`node_label_test`]'s to say.
    pub fn lift_alternatives(&mut self, p: &mut Pattern, preds: &mut Vec<Expr>) -> R<()> {
        for el in &p.elems {
            if let PatElem::Rel(r) = el {
                if let Some((_, hi)) = r.range {
                    if r.types.len() != 1 && hi > MAX_UNTYPED_HOPS {
                        return Err(format!(
                            "a variable-length relationship with {} and up to {hi} hops is not run on the AGE backend: \
                             AGE filters a variable-length pattern by one type only, and walks every trail of the \
                             rest (*1..30 over every type took the server to its memory limit). Give it one type, \
                             or at most {MAX_UNTYPED_HOPS} hops",
                            if r.types.is_empty() { "no type".to_string() } else { format!("the types {}", r.types.join("|")) }
                        ));
                    }
                }
            }
        }
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
                PatElem::Node(n) => {
                    let groups = label_groups(n);
                    if groups.len() < 2 && groups.iter().all(|g| g.len() < 2) {
                        continue;
                    }
                    let (mut scan, mut pred) = node_label_test(n.var.as_deref().unwrap_or(""), &groups);
                    if pred.is_some() && n.var.is_none() {
                        let var = self.fresh("n");
                        (scan, pred) = node_label_test(&var, &groups);
                        n.var = Some(var);
                    }
                    if let Some(p) = pred {
                        preds.push(p);
                    }
                    n.labels = scan.into_iter().collect();
                    n.label_groups = n.labels.iter().map(|l| vec![l.clone()]).collect();
                }
                _ => {}
            }
        }
        Ok(())
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
                    match distinct(&n.labels).as_slice() {
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
        // A list predicate goes out as `size([…]) > 0`: a comparison.
        !matches!(
            e,
            Expr::Binary(..) | Expr::Not(_) | Expr::Neg(_) | Expr::IsNull(..) | Expr::HasLabels { .. } | Expr::ListPred { .. }
        )
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
            Expr::Binary(op @ (BinOp::Add | BinOp::Sub), a, b) if self.constant_instant(a).is_some() && self.constant_duration(b).is_some() => {
                // `datetime() - duration({minutes: $m})`: both sides are fixed for
                // the statement, so the instant is worked out here and sent as a
                // parameter — AGE has neither temporal type.
                let (secs, micros) = self.constant_instant(a).unwrap();
                let (months, days, dsecs) = self.constant_duration(b).unwrap();
                let sign = if *op == BinOp::Sub { -1 } else { 1 };
                let at = shift_instant(secs, micros, sign * months, sign * days, sign * dsecs);
                self.fresh += 1;
                let name = format!("{FRESH}t{}", self.fresh);
                self.extra.insert(name.clone(), Json::String(at));
                format!("${name}")
            }
            Expr::Binary(op @ (BinOp::Eq | BinOp::In), a, b) if id_test(*op, a, b).is_some() => {
                return self.id_comparison(*op, a, b);
            }
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
                // AGE 1.8.0's any()/all()/none()/single() crash the backend
                // (signal 11) or fail ("attribute 1 of type record has wrong
                // type") when the predicate reads a matched element —
                // `any(l IN $ls WHERE l IN labels(n))`. A list comprehension
                // with the same predicate does not, so count its elements.
                let (v, src) = (ident(var), self.operand(source)?);
                let (pred, cmp) = match kind {
                    ListPredKind::Any => (self.expr(filter)?, "> 0"),
                    ListPredKind::None => (self.expr(filter)?, "= 0"),
                    ListPredKind::Single => (self.expr(filter)?, "= 1"),
                    ListPredKind::All => (format!("NOT ({})", self.expr(filter)?), "= 0"),
                };
                format!("size([{v} IN {src} WHERE {pred}]) {cmp}")
            }
            Expr::MapProjection { var, items } => {
                // Neo4j: a projection of null is null, and `.k` of an element
                // without k is `k: null`. AGE gives `{}` and leaves the key
                // out, so without `.*` the projection goes out as a map.
                let v = ident(var);
                let mut parts = Vec::new();
                let all = items.iter().any(|i| matches!(i, MapProjItem::All));
                for it in items {
                    parts.push(match it {
                        MapProjItem::Prop(p) if all => format!(".{}", ident(p)),
                        MapProjItem::Prop(p) => format!("{}: {v}.{}", ident(p), ident(p)),
                        MapProjItem::All => ".*".into(),
                        MapProjItem::Entry(k, e) => format!("{}: {}", ident(k), self.expr(e)?),
                    });
                }
                let inner = if all { format!("{v}{{{}}}", parts.join(", ")) } else { format!("{{{}}}", parts.join(", ")) };
                format!("CASE WHEN {v} IS NULL THEN null ELSE {inner} END")
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
                    self.lift_alternatives(p, &mut preds)?;
                }
                let where_ = and_all(where_.as_deref().cloned(), preds);
                // AGE's `exists(pattern)` may not name a variable of its own;
                // `EXISTS { MATCH … }` may.
                let mut named = Vec::new();
                for p in &patterns {
                    pattern_vars(p, &mut named);
                }
                let only_known = named.iter().all(|v| self.in_scope.contains(v));
                match kind {
                    SubqueryKind::Exists
                        if patterns.len() == 1 && where_.is_none() && patterns[0].path_var.is_none() && only_known =>
                    {
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

    /// `datetime()`, `datetime($p)`, `datetime('…')` as UTC epoch seconds and
    /// microseconds, when it is fixed for the statement.
    fn constant_instant(&self, e: &Expr) -> Option<(i64, i64)> {
        let Expr::Func { name, args, .. } = e else { return None };
        if !matches!(name.to_ascii_lowercase().as_str(), "datetime" | "localdatetime") {
            return None;
        }
        let text = match args.as_slice() {
            [] => self.now.clone(),
            [Expr::Param(p)] => self.params?.get(p)?.as_str()?.to_string(),
            [Expr::Lit(Lit::Str(s))] => s.clone(),
            _ => return None,
        };
        epoch_of(&canonical_datetime(&text)?)
    }

    /// `duration({days: 1, minutes: $m})` / `duration('P1DT30M')` as (months,
    /// days, seconds), when its parts are numbers known now.
    fn constant_duration(&self, e: &Expr) -> Option<(i64, i64, i64)> {
        let Expr::Func { name, args, .. } = e else { return None };
        if !name.eq_ignore_ascii_case("duration") {
            return None;
        }
        let num = |x: &Expr| -> Option<f64> {
            match x {
                Expr::Lit(Lit::Int(i)) => Some(*i as f64),
                Expr::Lit(Lit::Float(f)) => Some(*f),
                Expr::Param(p) => self.params?.get(p)?.as_f64(),
                Expr::Neg(inner) => match &**inner {
                    Expr::Lit(Lit::Int(i)) => Some(-(*i as f64)),
                    Expr::Lit(Lit::Float(f)) => Some(-*f),
                    _ => None,
                },
                _ => None,
            }
        };
        let (mut months, mut days, mut secs) = (0f64, 0f64, 0f64);
        match args.as_slice() {
            [Expr::Map(kv)] => {
                for (k, v) in kv {
                    let n = num(v)?;
                    match k.to_ascii_lowercase().as_str() {
                        "years" => months += 12.0 * n,
                        "months" => months += n,
                        "weeks" => days += 7.0 * n,
                        "days" => days += n,
                        "hours" => secs += 3600.0 * n,
                        "minutes" => secs += 60.0 * n,
                        "seconds" => secs += n,
                        "milliseconds" => secs += n / 1000.0,
                        _ => return None,
                    }
                }
            }
            [Expr::Lit(Lit::Str(iso))] => {
                let (m, d, s) = parse_iso_duration(iso)?;
                months = m as f64;
                days = d as f64;
                secs = s as f64;
            }
            _ => return None,
        }
        if months.fract() != 0.0 || days.fract() != 0.0 {
            return None;
        }
        Some((months as i64, days as i64, secs.round() as i64))
    }

    /// Comparisons with a node's or relationship's id ([`id_test`]):
    ///
    /// - `id(x) IN list` → `id(coalesce(x)) IN list`. AGE 1.8.0 crashes the
    ///   backend (signal 11) on `id(n) IN $param` and refuses `id(n) IN [1, 2]`
    ///   ("not a common type"); through `coalesce()` the id is an agtype
    ///   integer and both run (ROBO-ON-AGE 11.4).
    /// - `elementId(x) = e` (either side) → `id(x) = <e as an integer>`, which
    ///   AGE answers from the label table's id index. The element id is
    ///   `toString(id(x))` here, and `toString(id(coalesce(x))) = e` is read
    ///   row by row (STUDIO-ON-AGE G4: 28 ms → 2 ms at 25k vertices). A
    ///   parameter or literal that is an integer's text goes as that integer;
    ///   anything else known now keeps the text comparison (the same answer
    ///   for `'012'`, `'x'`, null); an expression becomes `id(x) =
    ///   toInteger(e) AND toString(id(coalesce(x))) = e` — the index finds the
    ///   candidate, the text decides.
    fn id_comparison(&mut self, op: BinOp, a: &Expr, b: &Expr) -> R<String> {
        match id_test(op, a, b).ok_or_else(|| "not an id comparison".to_string())? {
            IdTest::IdIn(x, list) => Ok(format!("id(coalesce({})) IN {}", self.expr(x)?, self.operand(list)?)),
            IdTest::ElementIdEq(x, e) => {
                let xs = self.expr(x)?;
                let text = |this: &mut Self| -> R<String> { Ok(format!("toString(id(coalesce({xs}))) = {}", this.operand(e)?)) };
                match e {
                    Expr::Lit(Lit::Str(s)) => match canonical_int(s) {
                        Some(i) => Ok(format!("id({xs}) = {i}")),
                        None => text(self),
                    },
                    Expr::Lit(_) => text(self),
                    Expr::Param(p) => match self.params.and_then(|m| m.get(p)).and_then(Json::as_str).and_then(canonical_int) {
                        Some(i) => {
                            let name = format!("{FRESH}eid_{}", param_name(p)?);
                            self.extra.insert(name.clone(), Json::from(i));
                            Ok(format!("id({xs}) = ${name}"))
                        }
                        None => text(self),
                    },
                    _ => {
                        let ev = self.operand(e)?;
                        Ok(format!("id({xs}) = toInteger({ev}) AND toString(id(coalesce({xs}))) = {ev}"))
                    }
                }
            }
        }
    }

    fn func(&mut self, name: &str, args: &[Expr], distinct: bool) -> R<String> {
        let lname = name.to_ascii_lowercase();
        match (lname.as_str(), args) {
            // `xs[i]` comes out of the parser as a call.
            ("element_at", [list, idx]) => return Ok(format!("{}[{}]", self.operand(list)?, self.expr(idx)?)),
            // Text a rewrite (reach.rs) has already written in AGE's Cypher.
            (super::reach::RAW, [Expr::Lit(Lit::Str(text))]) => return Ok(text.clone()),
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
            // `id(n)` on a matched vertex is AGE's raw graphid, which
            // toString() refuses; through coalesce() it is an agtype integer.
            ("elementid", [x]) => return Ok(format!("toString(id(coalesce({})))", self.expr(x)?)),
            ("tostring" | "tointeger" | "tofloat", [Expr::Func { name: f, args: inner, .. }])
                if f.eq_ignore_ascii_case("id") && inner.len() == 1 =>
            {
                return Ok(format!("{name}(id(coalesce({})))", self.expr(&inner[0])?));
            }
            // AGE types a matched vertex apart from an agtype one, and
            // coalesce() refuses to mix them ("COALESCE types vertex and agtype
            // cannot be matched"); `head([v])` is the same vertex as agtype.
            ("coalesce", xs) if xs.len() > 1 && xs.iter().any(|x| matches!(x, Expr::Var(_))) => {
                let parts = xs
                    .iter()
                    .map(|x| match x {
                        Expr::Var(v) => Ok(format!("head([{}])", ident(v))),
                        other => self.expr(other),
                    })
                    .collect::<R<Vec<_>>>()?;
                return Ok(format!("coalesce({})", parts.join(", ")));
            }
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

/// An id comparison the printer writes its own way ([`Printer::id_comparison`]).
enum IdTest<'e> {
    /// `id(x) IN list`: x, list.
    IdIn(&'e Expr, &'e Expr),
    /// `elementId(x) = e` or `e = elementId(x)`: x, e.
    ElementIdEq(&'e Expr, &'e Expr),
}

/// `f(x)` (any case) → x.
fn one_arg<'e>(e: &'e Expr, f: &str) -> Option<&'e Expr> {
    match e {
        Expr::Func { name, args, distinct: false } if name.eq_ignore_ascii_case(f) && args.len() == 1 => Some(&args[0]),
        _ => None,
    }
}

fn id_test<'e>(op: BinOp, a: &'e Expr, b: &'e Expr) -> Option<IdTest<'e>> {
    match op {
        BinOp::In => one_arg(a, "id").map(|x| IdTest::IdIn(x, b)),
        BinOp::Eq => match (one_arg(a, "elementid"), one_arg(b, "elementid")) {
            (Some(x), None) => Some(IdTest::ElementIdEq(x, b)),
            (None, Some(x)) => Some(IdTest::ElementIdEq(x, a)),
            _ => None,
        },
        _ => None,
    }
}

/// The integer whose text `s` is, written the way `toString()` writes it
/// (no sign but `-`, no leading zeros).
fn canonical_int(s: &str) -> Option<i64> {
    s.parse::<i64>().ok().filter(|i| i.to_string() == s)
}

/// `ORDER BY m.k` where `m` is this projection's map (`n {.k} AS m`, `{k: e}
/// AS m`): AGE cannot reach into the projected map there ("could not find
/// rte for m"), so the key is read where the map got it — `n.k`, `e`.
fn order_through_projection(e: &Expr, items: &[ReturnItem]) -> Option<Expr> {
    let Expr::Prop(base, key) = e else { return None };
    let Expr::Var(alias) = &**base else { return None };
    let item = items.iter().find(|i| i.alias.as_deref() == Some(alias.as_str()))?;
    match &item.expr {
        Expr::MapProjection { var, items } => items.iter().rev().find_map(|it| match it {
            MapProjItem::Entry(k, v) if k == key => Some(v.clone()),
            MapProjItem::Prop(p) if p == key => Some(Expr::Prop(Box::new(Expr::Var(var.clone())), key.clone())),
            MapProjItem::All => Some(Expr::Prop(Box::new(Expr::Var(var.clone())), key.clone())),
            _ => None,
        }),
        Expr::Map(kv) => kv.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()),
        _ => None,
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
            PatElem::Node(n) => {
                let groups = label_groups(n);
                if groups.iter().any(|g| g.len() > 1) {
                    return Err(format!("{clause} of a node needs its labels, not a choice of them (:A|B)"));
                }
                if distinct(&n.labels).len() > 1 {
                    return Err(unsupported(&format!("{clause} of a node with more than one label")));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// The names, each once, in order.
fn distinct(xs: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for x in xs {
        if !out.contains(x) {
            out.push(x.clone());
        }
    }
    out
}

/// A node pattern's labels as groups (all of the groups, any label within
/// one). The parser fills [`NodePat::label_groups`]; a pattern without it
/// (put together by hand) reads its flat list as all of them.
pub fn label_groups(n: &NodePat) -> Vec<Vec<String>> {
    if n.label_groups.is_empty() && !n.labels.is_empty() {
        n.labels.iter().map(|l| vec![l.clone()]).collect()
    } else {
        n.label_groups.iter().map(|g| distinct(g)).collect()
    }
}

/// What a node pattern's labels ask of AGE, where a vertex has exactly one
/// label: the label to put in the pattern (scanned), and a predicate for the
/// clause's WHERE. `groups` are [`NodePat::label_groups`] — all of the groups,
/// any label within a group — so the vertex's one label must be in every
/// group:
///
/// - one label is in all of them (`(n:A)`, `(n:A:A)`) → that label, no
///   predicate;
/// - several are (`(n:A|B)`) → no label, `label(n) IN [...]`;
/// - none is (`(n:A:B)`, A ≠ B) → nothing can match: the first label and
///   `false`.
///
/// SUITES-ON-AGE B1: the parser used to give `:A:B` and `:A|B` as one list,
/// and both went out as `label(n) IN ['A', 'B']` — `MATCH (n:Lamp:Part)
/// RETURN count(n)` counted the Lamps and the Parts (2; Neo4j 0). This is the
/// one place that decides it, so that a label hierarchy (an umbrella label
/// whose table the others inherit, STUDIO-ON-AGE G1/G2) can change it here.
pub fn node_label_test(var: &str, groups: &[Vec<String>]) -> (Option<String>, Option<Expr>) {
    let Some(first) = groups.first() else { return (None, None) };
    let common: Vec<String> = distinct(first).into_iter().filter(|l| groups.iter().all(|g| g.contains(l))).collect();
    match common.as_slice() {
        [] => (first.first().cloned(), Some(Expr::Lit(Lit::Bool(false)))),
        [one] => (Some(one.clone()), None),
        many => (
            None,
            Some(Expr::Binary(
                BinOp::In,
                Box::new(Expr::Func { name: "label".into(), args: vec![Expr::Var(var.to_string())], distinct: false }),
                Box::new(Expr::List(many.iter().map(|l| Expr::Lit(Lit::Str(l.clone()))).collect())),
            )),
        ),
    }
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

/// A version-4 UUID per evaluation — every row of an `UNWIND` gets its own.
/// AGE has no `randomUUID()`, but passes a schema-qualified name through to
/// PostgreSQL: `gen_random_uuid()` is built in (13+), `concat` makes it text
/// and `toString` agtype. (A UUID put together from `rand()` in a `reduce`
/// is refused inside `ON CREATE SET`: "unrecognized node type: 124".)
fn uuid_expr() -> String {
    "toString(pg_catalog.concat(pg_catalog.gen_random_uuid()))".into()
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

/// A canonical date-time (`canonical_datetime`) as UTC epoch seconds and
/// microseconds.
fn epoch_of(canon: &str) -> Option<(i64, i64)> {
    let y: i64 = canon.get(..4)?.parse().ok()?;
    let m: i64 = canon.get(5..7)?.parse().ok()?;
    let d: i64 = canon.get(8..10)?.parse().ok()?;
    let hh: i64 = canon.get(11..13)?.parse().ok()?;
    let mi: i64 = canon.get(14..16)?.parse().ok()?;
    let ss: i64 = canon.get(17..19)?.parse().ok()?;
    let rest = canon.get(19..)?;
    let micros = match rest.strip_prefix('.') {
        Some(f) => {
            let digits: String = f.chars().take_while(char::is_ascii_digit).collect();
            format!("{digits:0<6}")[..6].parse().ok()?
        }
        None => 0,
    };
    Some((days_from_civil(y, m, d) * 86_400 + hh * 3600 + mi * 60 + ss, micros))
}

/// `P1Y2M3DT4H5M6S` (Neo4j's printed form) as (months, days, seconds).
fn parse_iso_duration(s: &str) -> Option<(i64, i64, i64)> {
    let s = s.trim().strip_prefix('P')?;
    let (date, time) = s.split_once('T').unwrap_or((s, ""));
    let (mut months, mut days, mut secs) = (0i64, 0i64, 0f64);
    let mut num = String::new();
    for c in date.chars() {
        match c {
            '0'..='9' | '-' => num.push(c),
            'Y' => months += 12 * num.parse::<i64>().ok()?,
            'M' => months += num.parse::<i64>().ok()?,
            'W' => days += 7 * num.parse::<i64>().ok()?,
            'D' => days += num.parse::<i64>().ok()?,
            _ => return None,
        }
        if c.is_ascii_alphabetic() {
            num.clear();
        }
    }
    for c in time.chars() {
        match c {
            '0'..='9' | '-' | '.' => num.push(c),
            'H' => secs += 3600.0 * num.parse::<f64>().ok()?,
            'M' => secs += 60.0 * num.parse::<f64>().ok()?,
            'S' => secs += num.parse::<f64>().ok()?,
            _ => return None,
        }
        if c.is_ascii_alphabetic() {
            num.clear();
        }
    }
    Some((months, days, secs.round() as i64))
}

/// An instant moved by a duration — months on the calendar (the day clamped
/// to the month's last), then days, then seconds — written canonically.
fn shift_instant(secs: i64, micros: i64, months: i64, days: i64, dsecs: i64) -> String {
    let (day, tod) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(day);
    let total = y * 12 + (m - 1) + months;
    let (ny, nm) = (total.div_euclid(12), total.rem_euclid(12) + 1);
    let last = days_from_civil(if nm == 12 { ny + 1 } else { ny }, if nm == 12 { 1 } else { nm + 1 }, 1) - days_from_civil(ny, nm, 1);
    let nd = d.min(last);
    let s = (days_from_civil(ny, nm, nd) + days) * 86_400 + tod + dsecs;
    let (day, tod) = (s.div_euclid(86_400), s.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(day);
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

// ------------------------------------------------------- AGE's own names

/// Variable names AGE's generated SQL also uses as column names — of the
/// label tables (`id`, `properties`, `start_id`, `end_id`). A Cypher variable
/// spelled like one makes the SQL ambiguous: `UNWIND $ids AS id MATCH (n {id:
/// id}) RETURN n.id` fails with `column reference "id" is ambiguous`.
const CLASHING: &[&str] = &["id", "properties", "start_id", "end_id"];

/// Rename the variables that clash with AGE's columns. Columns (Bolt fields)
/// are worked out from the statement as sent, before this.
pub fn rename_clashing(q: &Query) -> Query {
    let mut q = q.clone();
    let mut names = Vec::new();
    collect_query(&q, &mut names);
    let map: Vec<(String, String)> = names
        .into_iter()
        .filter(|n| CLASHING.contains(&n.to_ascii_lowercase().as_str()))
        .map(|n| (n.clone(), format!("{FRESH}v_{n}")))
        .collect();
    if !map.is_empty() {
        rename_query(&mut q, &|n: &str| map.iter().find(|(a, _)| a == n).map(|(_, b)| b.clone()));
    }
    q
}

fn collect_query(q: &Query, out: &mut Vec<String>) {
    let mut push = |n: &String| {
        if !out.contains(n) {
            out.push(n.clone())
        }
    };
    for c in &q.clauses {
        let mut vars = Vec::new();
        match c {
            Clause::Match { patterns, .. } | Clause::Create(patterns) => patterns.iter().for_each(|p| pattern_vars(p, &mut vars)),
            Clause::Merge { pattern, .. } => pattern_vars(pattern, &mut vars),
            Clause::Unwind { alias, .. } => vars.push(alias.clone()),
            Clause::With { proj, .. } | Clause::Return(proj) => vars.extend(proj.items.iter().filter_map(|i| i.alias.clone())),
            Clause::Call { yields, .. } => vars.extend(yields.iter().map(|(_, a)| a.clone())),
            Clause::Foreach { var, .. } => vars.push(var.clone()),
            Clause::Subquery { body, .. } => collect_query(body, &mut vars),
            _ => {}
        }
        vars.iter().for_each(&mut push);
    }
    if let Some((_, t)) = &q.union {
        collect_query(t, out);
    }
}

type Rename<'a> = &'a dyn Fn(&str) -> Option<String>;

fn rn(v: &mut String, f: Rename) {
    if let Some(n) = f(v) {
        *v = n;
    }
}

fn rename_query(q: &mut Query, f: Rename) {
    for c in &mut q.clauses {
        rename_clause(c, f);
    }
    if let Some((_, t)) = &mut q.union {
        rename_query(t, f);
    }
}

fn rename_pattern(p: &mut Pattern, f: Rename) {
    if let Some(v) = &mut p.path_var {
        rn(v, f);
    }
    for el in &mut p.elems {
        match el {
            PatElem::Node(n) => {
                if let Some(v) = &mut n.var {
                    rn(v, f);
                }
                n.props.iter_mut().for_each(|(_, e)| rename_expr(e, f));
            }
            PatElem::Rel(r) => {
                if let Some(v) = &mut r.var {
                    rn(v, f);
                }
                r.props.iter_mut().for_each(|(_, e)| rename_expr(e, f));
            }
        }
    }
}

fn rename_proj(p: &mut Projection, f: Rename) {
    for it in &mut p.items {
        rename_expr(&mut it.expr, f);
        if let Some(a) = &mut it.alias {
            rn(a, f);
        }
    }
    p.order.iter_mut().for_each(|o| rename_expr(&mut o.expr, f));
    if let Some(e) = &mut p.skip {
        rename_expr(e, f);
    }
    if let Some(e) = &mut p.limit {
        rename_expr(e, f);
    }
}

fn rename_ops(items: &mut [SetOp], f: Rename) {
    for it in items {
        match it {
            SetOp::Prop { var, value, .. } | SetOp::Map { var, value, .. } => {
                rn(var, f);
                rename_expr(value, f);
            }
            SetOp::Label { var, .. } => rn(var, f),
        }
    }
}

fn rename_clause(c: &mut Clause, f: Rename) {
    match c {
        Clause::Match { patterns, where_, .. } => {
            patterns.iter_mut().for_each(|p| rename_pattern(p, f));
            if let Some(w) = where_ {
                rename_expr(w, f);
            }
        }
        Clause::Unwind { expr, alias } => {
            rename_expr(expr, f);
            rn(alias, f);
        }
        Clause::With { proj, where_ } => {
            rename_proj(proj, f);
            if let Some(w) = where_ {
                rename_expr(w, f);
            }
        }
        Clause::Return(proj) => rename_proj(proj, f),
        Clause::Create(ps) => ps.iter_mut().for_each(|p| rename_pattern(p, f)),
        Clause::Merge { pattern, on_create, on_match } => {
            rename_pattern(pattern, f);
            rename_ops(on_create, f);
            rename_ops(on_match, f);
        }
        Clause::Set(items) | Clause::Remove(items) => rename_ops(items, f),
        Clause::Delete { exprs, .. } => exprs.iter_mut().for_each(|e| rename_expr(e, f)),
        Clause::Call { args, yields, .. } => {
            args.iter_mut().for_each(|e| rename_expr(e, f));
            yields.iter_mut().for_each(|(_, a)| rn(a, f));
        }
        Clause::Foreach { var, list, body } => {
            rn(var, f);
            rename_expr(list, f);
            body.iter_mut().for_each(|c| rename_clause(c, f));
        }
        Clause::Subquery { scope, body, .. } => {
            if let Some(names) = scope {
                names.iter_mut().for_each(|n| rn(n, f));
            }
            rename_query(body, f);
        }
        Clause::Ddl(_) => {}
    }
}

fn rename_expr(e: &mut Expr, f: Rename) {
    match e {
        Expr::Var(v) => rn(v, f),
        Expr::Prop(b, _) | Expr::Not(b) | Expr::Neg(b) | Expr::IsNull(b, _) => rename_expr(b, f),
        Expr::Binary(_, a, b) => {
            rename_expr(a, f);
            rename_expr(b, f);
        }
        Expr::Func { args, .. } | Expr::List(args) => args.iter_mut().for_each(|a| rename_expr(a, f)),
        Expr::Map(kv) => kv.iter_mut().for_each(|(_, v)| rename_expr(v, f)),
        Expr::Case { operand, whens, else_ } => {
            if let Some(o) = operand {
                rename_expr(o, f);
            }
            for (a, b) in whens {
                rename_expr(a, f);
                rename_expr(b, f);
            }
            if let Some(x) = else_ {
                rename_expr(x, f);
            }
        }
        Expr::ListComp { var, source, filter, project } => {
            rn(var, f);
            rename_expr(source, f);
            if let Some(x) = filter {
                rename_expr(x, f);
            }
            if let Some(x) = project {
                rename_expr(x, f);
            }
        }
        Expr::ListPred { var, source, filter, .. } => {
            rn(var, f);
            rename_expr(source, f);
            rename_expr(filter, f);
        }
        Expr::MapProjection { var, items } => {
            rn(var, f);
            for it in items {
                if let MapProjItem::Entry(_, v) = it {
                    rename_expr(v, f);
                }
            }
        }
        Expr::Slice { list, from, to } => {
            rename_expr(list, f);
            if let Some(x) = from {
                rename_expr(x, f);
            }
            if let Some(x) = to {
                rename_expr(x, f);
            }
        }
        Expr::HasLabels { var, .. } => rn(var, f),
        Expr::Reduce { acc, init, var, list, expr } => {
            rn(acc, f);
            rn(var, f);
            rename_expr(init, f);
            rename_expr(list, f);
            rename_expr(expr, f);
        }
        Expr::Subquery { patterns, where_, project, .. } => {
            patterns.iter_mut().for_each(|p| rename_pattern(p, f));
            if let Some(w) = where_ {
                rename_expr(w, f);
            }
            if let Some(p) = project {
                rename_expr(p, f);
            }
        }
        Expr::Lit(_) | Expr::Param(_) | Expr::Star => {}
    }
}

/// A statement that begins with OPTIONAL MATCH gets a row to start from:
/// AGE gives no row when nothing matches (`OPTIONAL MATCH (x:Nope) RETURN
/// x.id` → 0 rows), Neo4j gives one with nulls.
pub fn lead_optional(q: &Query) -> Query {
    let mut q = q.clone();
    if matches!(q.clauses.first(), Some(Clause::Match { optional: true, .. })) {
        q.clauses.insert(
            0,
            Clause::With {
                proj: Projection {
                    items: vec![ReturnItem { expr: Expr::Lit(Lit::Int(1)), alias: Some(format!("{FRESH}one")) }],
                    ..Default::default()
                },
                where_: None,
            },
        );
    }
    if let Some((all, t)) = &q.union {
        q.union = Some((*all, Box::new(lead_optional(t))));
    }
    q
}

// ------------------------------------------------------ aggregation + lists

const AGGREGATES: &[&str] = &["count", "sum", "avg", "min", "max", "collect", "stdev", "stdevp", "percentilecont", "percentiledisc"];

pub(crate) fn is_agg_call(e: &Expr) -> bool {
    matches!(e, Expr::Func { name, .. } if AGGREGATES.contains(&name.to_ascii_lowercase().as_str()))
}

/// Does the expression hold something AGE compiles to a subquery (SubLink):
/// a list comprehension, a list predicate, reduce(), a pattern predicate?
fn has_sublink(e: &Expr) -> bool {
    match e {
        Expr::ListComp { .. } | Expr::ListPred { .. } | Expr::Reduce { .. } | Expr::Subquery { .. } => true,
        Expr::Prop(b, _) | Expr::Not(b) | Expr::Neg(b) | Expr::IsNull(b, _) => has_sublink(b),
        Expr::Binary(_, a, b) => has_sublink(a) || has_sublink(b),
        Expr::Func { args, .. } | Expr::List(args) => args.iter().any(has_sublink),
        Expr::Map(kv) => kv.iter().any(|(_, v)| has_sublink(v)),
        Expr::Case { operand, whens, else_ } => {
            operand.as_deref().is_some_and(has_sublink)
                || whens.iter().any(|(a, b)| has_sublink(a) || has_sublink(b))
                || else_.as_deref().is_some_and(has_sublink)
        }
        Expr::MapProjection { items, .. } => items.iter().any(|i| matches!(i, MapProjItem::Entry(_, v) if has_sublink(v))),
        Expr::Slice { list, from, to } => {
            has_sublink(list) || from.as_deref().is_some_and(has_sublink) || to.as_deref().is_some_and(has_sublink)
        }
        _ => false,
    }
}

/// Every aggregate call in `e` replaced by a fresh variable, returned with
/// the call it stands for.
fn lift_aggs(e: &mut Expr, out: &mut Vec<(String, Expr)>, fresh: &mut dyn FnMut() -> String) {
    if is_agg_call(e) {
        let v = fresh();
        let call = std::mem::replace(e, Expr::Var(v.clone()));
        out.push((v, call));
        return;
    }
    let mut go = |x: &mut Expr| lift_aggs(x, out, fresh);
    match e {
        Expr::Prop(b, _) | Expr::Not(b) | Expr::Neg(b) | Expr::IsNull(b, _) => go(b),
        Expr::Binary(_, a, b) => {
            go(a);
            go(b);
        }
        Expr::Func { args, .. } | Expr::List(args) => args.iter_mut().for_each(go),
        Expr::Map(kv) => kv.iter_mut().for_each(|(_, v)| go(v)),
        Expr::Case { operand, whens, else_ } => {
            if let Some(o) = operand {
                go(o);
            }
            for (a, b) in whens {
                go(a);
                go(b);
            }
            if let Some(x) = else_ {
                go(x);
            }
        }
        Expr::ListComp { source, filter, project, .. } => {
            go(source);
            if let Some(x) = filter {
                go(x);
            }
            if let Some(x) = project {
                go(x);
            }
        }
        Expr::ListPred { source, filter, .. } => {
            go(source);
            go(filter);
        }
        Expr::Reduce { init, list, expr, .. } => {
            go(init);
            go(list);
            go(expr);
        }
        Expr::MapProjection { items, .. } => {
            for it in items {
                if let MapProjItem::Entry(_, v) = it {
                    go(v);
                }
            }
        }
        Expr::Slice { list, from, to } => {
            go(list);
            if let Some(x) = from {
                go(x);
            }
            if let Some(x) = to {
                go(x);
            }
        }
        _ => {}
    }
}

/// An aggregating WITH / RETURN that also holds a list comprehension, a list
/// predicate or a pattern predicate — or an aggregate inside one
/// (`[p IN collect(p) WHERE …]`) — fails in AGE 1.8.0 ("unsupported SubLink",
/// or "`x` must be either part of an explicitly listed key or used inside an
/// aggregate function"). It becomes three projections, which AGE runs:
///
/// 1. `WITH <scope>, <list expressions of the grouping keys and of the
///    aggregates' arguments> AS __og_kN` — no aggregation;
/// 2. `WITH <grouping keys>, <aggregate calls> AS __og_aN` — no lists;
/// 3. the projection as written, reading those variables.
///
/// The groups are the same: each grouping key is computed once per row
/// before step 2 and grouped on as it was.
pub fn split_aggregations(q: &Query) -> Query {
    let mut out = Vec::new();
    let mut scope: Vec<String> = Vec::new();
    let mut n = 0usize;
    for c in &q.clauses {
        let (proj, where_, is_return) = match c {
            Clause::With { proj, where_ } => (proj, where_.clone(), false),
            Clause::Return(proj) => (proj, None, true),
            other => {
                let other = match other {
                    Clause::Subquery { scope: sc, body, optional } => {
                        Clause::Subquery { scope: sc.clone(), body: Box::new(split_aggregations(body)), optional: *optional }
                    }
                    o => o.clone(),
                };
                scope = scope_after(&scope, &other);
                out.push(other);
                continue;
            }
        };
        let aggregating = proj.items.iter().any(|i| i.expr.is_aggregate());
        let lists = proj.items.iter().any(|i| has_sublink(&i.expr));
        if !aggregating || !lists || proj.items.iter().any(|i| matches!(i.expr, Expr::Star)) {
            scope = scope_after(&scope, c);
            out.push(c.clone());
            continue;
        }
        let mut fresh = |what: &str| {
            n += 1;
            format!("{FRESH}{what}{n}")
        };
        let mut pre: Vec<ReturnItem> = scope.iter().map(|v| ReturnItem { expr: Expr::Var(v.clone()), alias: None }).collect();
        let mut agg: Vec<ReturnItem> = Vec::new();
        let mut post: Vec<ReturnItem> = Vec::new();
        for it in &proj.items {
            if it.expr.is_aggregate() {
                let mut e = it.expr.clone();
                let mut calls = Vec::new();
                lift_aggs(&mut e, &mut calls, &mut || fresh("a"));
                for (v, mut call) in calls {
                    if let Expr::Func { args, .. } = &mut call {
                        for a in args.iter_mut() {
                            if has_sublink(a) {
                                let k = fresh("g");
                                pre.push(ReturnItem { expr: a.clone(), alias: Some(k.clone()) });
                                *a = Expr::Var(k);
                            }
                        }
                    }
                    agg.push(ReturnItem { expr: call, alias: Some(v) });
                }
                post.push(ReturnItem { expr: e, alias: it.alias.clone() });
            } else {
                match &it.expr {
                    Expr::Var(v) if it.alias.is_none() || it.alias.as_deref() == Some(v.as_str()) => {
                        agg.push(ReturnItem { expr: it.expr.clone(), alias: None });
                        post.push(it.clone());
                    }
                    e => {
                        let k = fresh("k");
                        if has_sublink(e) {
                            pre.push(ReturnItem { expr: e.clone(), alias: Some(k.clone()) });
                            agg.push(ReturnItem { expr: Expr::Var(k.clone()), alias: None });
                        } else {
                            agg.push(ReturnItem { expr: e.clone(), alias: Some(k.clone()) });
                        }
                        post.push(ReturnItem { expr: Expr::Var(k), alias: Some(item_name(it)) });
                    }
                }
            }
        }
        // ORDER BY may name an item by its expression: read the item instead.
        let printed = |e: &Expr| Printer::new(None, String::new()).expr(e).ok();
        let order = proj
            .order
            .iter()
            .map(|o| {
                let hit = proj.items.iter().find(|i| printed(&i.expr).is_some() && printed(&i.expr) == printed(&o.expr));
                match hit {
                    Some(i) => OrderItem { expr: Expr::Var(item_name(i)), desc: o.desc },
                    None => o.clone(),
                }
            })
            .collect();
        let plain = |items: Vec<ReturnItem>| Projection { items, ..Default::default() };
        if pre.len() > scope.len() {
            out.push(Clause::With { proj: plain(pre), where_: None });
        }
        out.push(Clause::With { proj: plain(agg), where_: None });
        let last = Projection { items: post, distinct: proj.distinct, order, skip: proj.skip.clone(), limit: proj.limit.clone() };
        let rewritten = if is_return { Clause::Return(last) } else { Clause::With { proj: last, where_ } };
        scope = scope_after(&scope, c);
        out.push(rewritten);
    }
    Query { clauses: out, union: q.union.as_ref().map(|(all, t)| (*all, Box::new(split_aggregations(t)))) }
}

// ------------------------------------------------------- MERGE and null

/// A MERGE pattern property whose value may be null: what Neo4j says then.
#[derive(Debug, Clone, PartialEq)]
pub struct NullMerge {
    /// `node` or `relationship`.
    pub kind: &'static str,
    pub key: String,
    /// The element as Neo4j shows it in the message: `(:W {key: null})`.
    pub shape: String,
}

impl NullMerge {
    /// Neo4j's `Neo.ClientError.Statement.SemanticError` text.
    pub fn message(&self) -> String {
        format!("Cannot merge the following {} because of null property value for '{}': {}", self.kind, self.key, self.shape)
    }
}

/// What a MERGE key is, as far as the gateway can tell before sending it.
enum Known {
    Null,
    NotNull,
    Unknown,
}

fn known(e: &Expr, params: Option<&Map<String, Json>>) -> Known {
    /// `$p`, `$p.a.b`: the parameter's value, if it was given.
    fn param_value<'a>(e: &Expr, params: &'a Map<String, Json>) -> Option<Option<&'a Json>> {
        match e {
            Expr::Param(p) => params.get(p).map(Some),
            Expr::Prop(b, k) => match param_value(b, params)? {
                Some(Json::Object(m)) => Some(m.get(k)),
                Some(Json::Null) | None => Some(None),
                // a field of something that is not a map: AGE says what it says
                Some(_) => None,
            },
            _ => None,
        }
    }
    match e {
        Expr::Lit(Lit::Null) => Known::Null,
        Expr::Lit(_) | Expr::List(_) | Expr::Map(_) => Known::NotNull,
        // the statement's clock and a new UUID are never null
        Expr::Func { name, args, .. }
            if args.is_empty()
                && matches!(name.to_ascii_lowercase().as_str(), "datetime" | "localdatetime" | "date" | "timestamp" | "randomuuid") =>
        {
            Known::NotNull
        }
        // not given: AGE reports the parameter missing, as Neo4j does
        Expr::Param(p) if !params.is_some_and(|m| m.contains_key(p)) => Known::NotNull,
        Expr::Param(_) | Expr::Prop(..) => match params.and_then(|m| param_value(e, m)) {
            Some(None) | Some(Some(Json::Null)) => Known::Null,
            Some(Some(_)) => Known::NotNull,
            // not a parameter path, or a parameter not given (AGE says it is missing)
            None => Known::Unknown,
        },
        _ => Known::Unknown,
    }
}

/// `CASE WHEN e IS NULL THEN 1 / (CASE WHEN e IS NULL THEN 0 ELSE 1 END) ELSE e END`:
/// `e`, or a division by zero when it is null. The divisor reads `e`, so
/// PostgreSQL cannot fold the division while planning (a constant `1 / 0` in
/// a branch never taken fails at plan time); and the value is used — the
/// pattern's map is the MERGE's match and its create — so it cannot be
/// dropped as an unused column either.
fn null_guard(e: &Expr) -> Expr {
    let is_null = Expr::IsNull(Box::new(e.clone()), true);
    let zero_if_null = Expr::Case {
        operand: None,
        whens: vec![(is_null.clone(), Expr::Lit(Lit::Int(0)))],
        else_: Some(Box::new(Expr::Lit(Lit::Int(1)))),
    };
    Expr::Case {
        operand: None,
        whens: vec![(is_null, Expr::Binary(BinOp::Div, Box::new(Expr::Lit(Lit::Int(1))), Box::new(zero_if_null)))],
        else_: Some(Box::new(e.clone())),
    }
}

fn node_shape(n: &NodePat, key: &str) -> String {
    let labels: String = distinct(&n.labels).iter().map(|l| format!(":{l}")).collect();
    format!("({labels} {{{key}: null}})")
}

fn rel_shape(r: &RelPat, key: &str) -> String {
    let t: String = r.types.first().map(|t| format!(":{t}")).unwrap_or_default();
    match r.dir {
        Dir::Out => format!("()-[{t} {{{key}: null}}]->()"),
        Dir::In => format!("()<-[{t} {{{key}: null}}]-()"),
        Dir::Both => format!("()-[{t} {{{key}: null}}]-()"),
    }
}

/// MERGE with a null property value (SUITES-ON-AGE N7): Neo4j refuses
/// (`Cannot merge the following node because of null property value for
/// 'key'`), AGE 1.8.0 makes an element without the key — `UNWIND [{x: 1}] AS
/// r MERGE (n:W {key: r.missing}) RETURN n` gives a new `(:W {})` every time,
/// so MERGE stops being idempotent.
///
/// A value known now — a literal, a parameter given, a field of one — is
/// checked here: null is refused with Neo4j's message (`Err`). Any other value
/// goes to AGE as [`null_guard`]`(e)`, which divides by zero when it is null
/// (AGE: `ERRCODE_DIVISION_BY_ZERO`, "division by zero", `agtype_ops.c`
/// `agtype_div`); the gateway reads that error, from a statement holding a
/// guard, as the [`NullMerge`]s returned (`run.rs`). Into FOREACH and
/// CALL { … } bodies and UNION's branches.
pub fn guard_merge_nulls(q: &Query, params: Option<&Map<String, Json>>) -> Result<(Query, Vec<NullMerge>), NullMerge> {
    let mut guards = Vec::new();
    let q = guard_query(q, params, &mut guards)?;
    Ok((q, guards))
}

fn guard_query(q: &Query, params: Option<&Map<String, Json>>, out: &mut Vec<NullMerge>) -> Result<Query, NullMerge> {
    let clauses = guard_clauses(&q.clauses, params, out)?;
    let union = match &q.union {
        Some((all, t)) => Some((*all, Box::new(guard_query(t, params, out)?))),
        None => None,
    };
    Ok(Query { clauses, union })
}

fn guard_clauses(cs: &[Clause], params: Option<&Map<String, Json>>, out: &mut Vec<NullMerge>) -> Result<Vec<Clause>, NullMerge> {
    let mut res = Vec::with_capacity(cs.len());
    for c in cs {
        res.push(match c {
            Clause::Merge { pattern, on_create, on_match } => {
                let mut pattern = pattern.clone();
                for el in &mut pattern.elems {
                    let (props, kind, shape): (&mut Vec<(String, Expr)>, &'static str, Box<dyn Fn(&str) -> String>) = match el {
                        PatElem::Node(n) => {
                            let nn = n.clone();
                            (&mut n.props, "node", Box::new(move |k: &str| node_shape(&nn, k)))
                        }
                        PatElem::Rel(r) => {
                            let rr = r.clone();
                            (&mut r.props, "relationship", Box::new(move |k: &str| rel_shape(&rr, k)))
                        }
                    };
                    for (k, v) in props.iter_mut() {
                        let nm = NullMerge { kind, key: k.clone(), shape: shape(k) };
                        match known(v, params) {
                            Known::Null => return Err(nm),
                            Known::NotNull => {}
                            Known::Unknown => {
                                *v = null_guard(v);
                                out.push(nm);
                            }
                        }
                    }
                }
                Clause::Merge { pattern, on_create: on_create.clone(), on_match: on_match.clone() }
            }
            Clause::Foreach { var, list, body } => {
                Clause::Foreach { var: var.clone(), list: list.clone(), body: guard_clauses(body, params, out)? }
            }
            Clause::Subquery { scope, body, optional } => {
                Clause::Subquery { scope: scope.clone(), body: Box::new(guard_query(body, params, out)?), optional: *optional }
            }
            other => other.clone(),
        });
    }
    Ok(res)
}

// ------------------------------------------------------------- MERGE + SET

/// [`fold_merge_sets`] inside CALL { … } bodies and FOREACH bodies only: at the
/// top level the statement is cut instead (`split.rs`), which avoids a second
/// AGE fault the fold can run into.
pub fn fold_nested_merge_sets(q: &Query) -> Query {
    let clauses = q
        .clauses
        .iter()
        .map(|c| match c {
            Clause::Subquery { scope, body, optional } => {
                Clause::Subquery { scope: scope.clone(), body: Box::new(fold_merge_sets(body)), optional: *optional }
            }
            Clause::Foreach { var, list, body } => {
                Clause::Foreach { var: var.clone(), list: list.clone(), body: fold_merge_set_clauses(body) }
            }
            other => other.clone(),
        })
        .collect();
    Query { clauses, union: q.union.as_ref().map(|(all, t)| (*all, Box::new(fold_nested_merge_sets(t)))) }
}

/// `MERGE … ON CREATE SET a … SET b` → `MERGE … ON CREATE SET a, b ON MATCH SET b`.
///
/// AGE 1.8.0 silently drops a SET (or REMOVE) clause that follows a MERGE
/// whose branch that ran had SET items of its own: the statement returns the
/// new values, but they are not stored (measured: `MATCH (m) MERGE (n:L {k: 1})
/// ON CREATE SET n.a = 1 SET n.b = 2 MERGE (m)-[:R]->(n)` stores `{a, k}`).
/// Items inside ON CREATE / ON MATCH are stored, so the clauses after the MERGE
/// move into both branches — Neo4j runs them after either branch, which is the
/// same order. Recursively into FOREACH bodies and CALL { … } subqueries.
pub fn fold_merge_sets(q: &Query) -> Query {
    Query { clauses: fold_merge_set_clauses(&q.clauses), union: q.union.as_ref().map(|(all, t)| (*all, Box::new(fold_merge_sets(t)))) }
}

/// [`fold_merge_sets`] on a run of clauses.
pub fn fold_merge_set_clauses(cs: &[Clause]) -> Vec<Clause> {
    let mut out: Vec<Clause> = Vec::with_capacity(cs.len());
    let mut i = 0;
    while i < cs.len() {
        match &cs[i] {
            Clause::Merge { pattern, on_create, on_match } if !on_create.is_empty() || !on_match.is_empty() => {
                let (mut oc, mut om) = (on_create.clone(), on_match.clone());
                let mut j = i + 1;
                while j < cs.len() {
                    let items: Vec<SetOp> = match &cs[j] {
                        Clause::Set(items) if !items.iter().any(|x| matches!(x, SetOp::Label { .. })) => items.clone(),
                        Clause::Remove(items) if !items.iter().any(|x| matches!(x, SetOp::Label { .. })) => items
                            .iter()
                            .map(|x| match x {
                                SetOp::Prop { var, prop, .. } => {
                                    SetOp::Prop { var: var.clone(), prop: prop.clone(), value: Expr::Lit(Lit::Null) }
                                }
                                other => other.clone(),
                            })
                            .collect(),
                        _ => break,
                    };
                    oc.extend(items.iter().cloned());
                    om.extend(items);
                    j += 1;
                }
                out.push(Clause::Merge { pattern: pattern.clone(), on_create: oc, on_match: om });
                i = j;
                continue;
            }
            Clause::Foreach { var, list, body } => {
                out.push(Clause::Foreach { var: var.clone(), list: list.clone(), body: fold_merge_set_clauses(body) })
            }
            Clause::Subquery { scope, body, optional } => {
                out.push(Clause::Subquery { scope: scope.clone(), body: Box::new(fold_merge_sets(body)), optional: *optional })
            }
            other => out.push(other.clone()),
        }
        i += 1;
    }
    out
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
        assert_eq!(print("MERGE (n:L {k: 1}) ON CREATE SET n.c = 1 ON MATCH SET n.m = 2 SET n += r RETURN n"),
            "MERGE (n:L {k: 1}) ON CREATE SET n.c = 1 ON MATCH SET n.m = 2 SET n += r RETURN n");
        assert_eq!(print("UNWIND $rows AS r WITH r WHERE r.x IS NOT NULL MATCH (n) WHERE n.id = r.id DETACH DELETE n"),
            "UNWIND $rows AS r WITH r WHERE r.x IS NOT NULL MATCH (n) WHERE n.id = r.id DETACH DELETE n");
        // `*..` gets the engine's bound, as on v1; the engine's parser reads a
        // bare `*` as one hop, and so it goes out.
        assert_eq!(print("MATCH (a)-[:R*..]->(b) RETURN b"), "MATCH (a)-[:R*1..8]->(b) RETURN b");
        assert_eq!(print("MATCH (a)-[*]->(b) RETURN b"), "MATCH (a)-[*1..1]->(b) RETURN b");
        assert_eq!(print("RETURN [x IN $xs WHERE x > 1 | x * 2] AS d, n {.a, .*, k: 1} AS m, xs[0] AS h, xs[1..] AS t"),
            "RETURN [x IN $xs WHERE x > 1 | x * 2] AS d, CASE WHEN n IS NULL THEN null ELSE n{.a, .*, k: 1} END AS m, xs[0] AS h, xs[1..] AS t");
        assert_eq!(print("RETURN n {.a, k: 1} AS m"), "RETURN CASE WHEN n IS NULL THEN null ELSE {a: n.a, k: 1} END AS m");
        assert_eq!(print("MATCH (rm) RETURN rm {.id, .name} AS m ORDER BY m.name, m.x"),
            "MATCH (rm) RETURN CASE WHEN rm IS NULL THEN null ELSE {id: rm.id, name: rm.name} END AS m ORDER BY rm.name, m.x");
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
        assert_eq!(s, "CREATE (n:N {at: $__og_now, id: toString(pg_catalog.concat(pg_catalog.gen_random_uuid())), \
            c: $__og_dt_c, d: n.x}) RETURN toString(id(coalesce(n))) AS e");
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
            "MATCH (a)-[r*1..2]->(b) WHERE size([__og_x1 IN r WHERE NOT (type(__og_x1) IN ['X', 'Y'])]) = 0 RETURN b");
        assert_eq!(print("MATCH (e) WHERE any(l IN $ls WHERE l IN labels(e)) AND none(x IN e.xs WHERE x = 1) RETURN e"),
            "MATCH (e) WHERE (size([l IN $ls WHERE l IN labels(e)]) > 0) AND (size([x IN e.xs WHERE x = 1]) = 0) RETURN e");
        assert_eq!(print("MATCH (a) WHERE NOT (a)-[:X|Y]->() RETURN a"),
            "MATCH (a) WHERE NOT EXISTS { MATCH (a)-[__og_r1]->() WHERE type(__og_r1) IN ['X', 'Y'] } RETURN a");
        assert_eq!(print("MATCH (a) WHERE (a)-[:X]->(:B) RETURN a"), "MATCH (a) WHERE EXISTS((a)-[:X]->(:B)) RETURN a");
        assert_eq!(print("MATCH (a) WHERE exists((b:B)-[:X]->(a)) RETURN a"),
            "MATCH (a) WHERE EXISTS { MATCH (b:B)-[:X]->(a) } RETURN a");
        assert_eq!(print("MATCH (n:A|B) RETURN n"), "MATCH (n) WHERE label(n) IN ['A', 'B'] RETURN n");
        // Long variable-length patterns without one type are refused, not run.
        let mut p = Printer::new(None, "2026-11-18T00:00:00+00:00".into());
        for q in ["MATCH (a)-[:X|Y*1..30]-(b) RETURN b", "MATCH (a)-[*1..4]-(b) RETURN b"] {
            let e = p.query(&cypher_syntax::parse(q).unwrap()).unwrap_err();
            assert!(e.contains("not run on the AGE backend"), "{e}");
        }
        assert_eq!(print("MATCH (a)-[:X*1..30]-(b) RETURN b"), "MATCH (a)-[:X*1..30]-(b) RETURN b");
    }

    #[test]
    fn a_null_merge_key_fails_as_in_neo4j() {
        let params: Map<String, Json> = serde_json::json!({"k": null, "m": {"a": 1}, "v": 5}).as_object().unwrap().clone();
        let guard = |q: &str| guard_merge_nulls(&cypher_syntax::parse(q).unwrap(), Some(&params));
        // known to be null now: refused with Neo4j's words
        assert_eq!(guard("MERGE (n:W {key: null})").unwrap_err().message(),
            "Cannot merge the following node because of null property value for 'key': (:W {key: null})");
        for q in ["MERGE (n:W {key: $k}) RETURN n", "MERGE (n:W {key: $m.b})", "MATCH (a) FOREACH (x IN [1] | MERGE (a)-[:R]->(:W {key: $k}))"] {
            assert_eq!(guard(q).unwrap_err().key, "key", "{q}");
        }
        assert_eq!(guard("MATCH (a), (b) MERGE (a)-[:R {since: $k}]->(b)").unwrap_err().message(),
            "Cannot merge the following relationship because of null property value for 'since': ()-[:R {since: null}]->()");
        // known not to be null, or a parameter not given (AGE reports it missing): as it was
        let (q, g) = guard("MERGE (n:W {key: $v, a: $m.a, c: 'x', t: datetime(), u: $nope})").unwrap();
        assert!(g.is_empty());
        assert_eq!(Printer::new(Some(&params), "2026-11-18T00:00:00+00:00".into()).query(&q).unwrap(),
            "MERGE (n:W {key: $v, a: $m.a, c: 'x', t: $__og_now, u: $nope})");
        // computed: a division by zero when it is null, which the gateway reads back
        let (q, g) = guard("UNWIND [{x: 1}] AS r MERGE (n:W {key: r.missing}) RETURN n").unwrap();
        assert_eq!(g, vec![NullMerge { kind: "node", key: "key".into(), shape: "(:W {key: null})".into() }]);
        assert_eq!(Printer::new(None, String::new()).query(&q).unwrap(),
            "UNWIND [{x: 1}] AS r MERGE (n:W {key: CASE WHEN r.missing IS NULL THEN 1 / CASE WHEN r.missing IS NULL THEN 0 ELSE 1 END ELSE r.missing END}) RETURN n");
        let (_, g) = guard("UNWIND $rows AS r MATCH (a {id: r.a}) MERGE (a)<-[e:R {k: r.k}]-(b:B {id: r.b})").unwrap();
        assert_eq!(g.iter().map(|x| (x.kind, x.shape.as_str())).collect::<Vec<_>>(),
            vec![("relationship", "()<-[:R {k: null}]-()"), ("node", "(:B {id: null})")]);
    }

    #[test]
    fn id_comparisons_use_the_id() {
        let params: Map<String, Json> = [
            ("id".to_string(), Json::from("844424930131969")),
            ("bad".to_string(), Json::from("012")),
            ("n".to_string(), Json::from(12)),
        ]
        .into_iter()
        .collect();
        let pr = |q: &str| {
            let mut p = Printer::new(Some(&params), "2026-11-18T00:00:00+00:00".into());
            let s = p.query(&cypher_syntax::parse(q).unwrap()).unwrap();
            (s, p.extra)
        };
        // G4: an element id that is an integer's text is compared as the id
        let (s, extra) = pr("MATCH (n) WHERE elementId(n) = $id RETURN n");
        assert_eq!(s, "MATCH (n) WHERE id(n) = $__og_eid_id RETURN n");
        assert_eq!(extra["__og_eid_id"], Json::from(844424930131969i64));
        assert_eq!(pr("MATCH (n) WHERE $id = elementId(n) DETACH DELETE n").0, "MATCH (n) WHERE id(n) = $__og_eid_id DETACH DELETE n");
        assert_eq!(pr("MATCH (n) WHERE elementId(n) = '12' RETURN n").0, "MATCH (n) WHERE id(n) = 12 RETURN n");
        // anything else known now keeps the text comparison: the same answer
        for (q, want) in [
            ("MATCH (n) WHERE elementId(n) = $bad RETURN n", "toString(id(coalesce(n))) = $bad"),
            ("MATCH (n) WHERE elementId(n) = $n RETURN n", "toString(id(coalesce(n))) = $n"),
            ("MATCH (n) WHERE elementId(n) = $nope RETURN n", "toString(id(coalesce(n))) = $nope"),
            ("MATCH (n) WHERE elementId(n) = 'x' RETURN n", "toString(id(coalesce(n))) = 'x'"),
        ] {
            assert!(pr(q).0.contains(want), "{q}: {}", pr(q).0);
        }
        // an expression: the index finds it, the text decides
        assert_eq!(pr("UNWIND $items AS item MATCH (t) WHERE elementId(t) = item.tid AND t.x = 1 RETURN t").0,
            "UNWIND $items AS item MATCH (t) WHERE (id(t) = toInteger(item.tid) AND toString(id(coalesce(t))) = item.tid) AND (t.x = 1) RETURN t");
        // the id IN crash: through coalesce()
        assert_eq!(pr("MATCH (n) WHERE id(n) IN $ids RETURN n").0, "MATCH (n) WHERE id(coalesce(n)) IN $ids RETURN n");
        assert_eq!(pr("MATCH (n) WHERE NOT id(n) IN [1, 2] RETURN n").0, "MATCH (n) WHERE NOT (id(coalesce(n)) IN [1, 2]) RETURN n");
        // left as they were: IN of element ids, two element ids
        assert_eq!(pr("MATCH ()-[r]->() WHERE elementId(r) IN $ids RETURN r").0,
            "MATCH ()-[r]->() WHERE toString(id(coalesce(r))) IN $ids RETURN r");
        assert_eq!(pr("MATCH (m), (n) WHERE elementId(m) <> elementId(n) RETURN m").0,
            "MATCH (m), (n) WHERE toString(id(coalesce(m))) <> toString(id(coalesce(n))) RETURN m");
    }

    #[test]
    fn all_labels_are_not_any_label() {
        // B1: (n:A:B) is both — no vertex of AGE's, which has one label.
        assert_eq!(print("MATCH (n:Lamp:Part) RETURN count(n) AS c"), "MATCH (n:Lamp) WHERE false RETURN count(n) AS c");
        assert_eq!(print("MATCH (n:A:B) WHERE n.k = 1 RETURN n"), "MATCH (n:A) WHERE (n.k = 1) AND false RETURN n");
        assert_eq!(print("OPTIONAL MATCH (:A:B)-[:R]->(m) RETURN m"),
            "OPTIONAL MATCH (__og_n1:A)-[:R]->(m) WHERE false RETURN m");
        // the same label twice, or a choice that leaves one, is that label
        assert_eq!(print("MATCH (n:A:A) RETURN n"), "MATCH (n:A) RETURN n");
        assert_eq!(print("MATCH (n:A|B:B) RETURN n"), "MATCH (n:B) RETURN n");
        // a choice is still a choice
        assert_eq!(print("MATCH (n:A|B) RETURN n"), "MATCH (n) WHERE label(n) IN ['A', 'B'] RETURN n");
        assert_eq!(print("MATCH (a) WHERE EXISTS { MATCH (a)-[:X]->(:A:B) } RETURN a"),
            "MATCH (a) WHERE EXISTS { MATCH (a)-[:X]->(__og_n1:A) WHERE false } RETURN a");
        assert!(matches!(node_label_test("n", &[vec!["A".into()], vec!["B".into(), "A".into()]]), (Some(l), None) if l == "A"));
        assert!(matches!(node_label_test("n", &[]), (None, None)));
        // writes: one label, as before
        let mut p = Printer::new(None, "2026-11-18T00:00:00+00:00".into());
        let e = p.query(&cypher_syntax::parse("CREATE (n:A:B)").unwrap()).unwrap_err();
        assert!(e.contains("more than one label"), "{e}");
        let e = p.query(&cypher_syntax::parse("MERGE (n:A|B {k: 1})").unwrap()).unwrap_err();
        assert!(e.contains("not a choice"), "{e}");
        assert_eq!(print("CREATE (n:A:A {k: 1})"), "CREATE (n:A {k: 1})");
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
    fn a_map_parameter_in_set_is_bound_first() {
        assert_eq!(print("MATCH (n {id: $id}) SET n += $props RETURN n"),
            "MATCH (n {id: $id}) WITH n, $props AS __og_p1 SET n += __og_p1 RETURN n");
        assert_eq!(print("MERGE (n:L {id: 1}) ON CREATE SET n = $p ON MATCH SET n += $p"),
            "WITH $p AS __og_p1 MERGE (n:L {id: 1}) ON CREATE SET n = __og_p1 ON MATCH SET n += __og_p1");
        // a map parameter's field is an ordinary value
        assert_eq!(print("MATCH (n) SET n.a = $p.a"), "MATCH (n) SET n.a = $p.a");
    }

    #[test]
    fn a_set_after_merge_moves_into_its_branches() {
        let fold = |q: &str| {
            let ast = fold_merge_sets(&cypher_syntax::parse(q).unwrap());
            Printer::new(None, String::new()).query(&ast).unwrap()
        };
        assert_eq!(fold("MATCH (m) MERGE (n:L {k: 1}) ON CREATE SET n.a = 1 SET n.b = 2 REMOVE n.c MERGE (m)-[:R]->(n)"),
            "MATCH (m) MERGE (n:L {k: 1}) ON CREATE SET n.a = 1, n.b = 2, n.c = null ON MATCH SET n.b = 2, n.c = null MERGE (m)-[:R]->(n)");
        // a MERGE without branches keeps its SET: AGE stores that one
        assert_eq!(fold("MERGE (n:L {k: 1}) SET n.b = 2"), "MERGE (n:L {k: 1}) SET n.b = 2");
    }

    #[test]
    fn names_that_clash_with_age_columns_are_renamed() {
        let ast = rename_clashing(&cypher_syntax::parse("UNWIND $ids AS id MATCH (n {id: id}) RETURN n.id AS id, [x IN n.l | x.id] AS l ORDER BY id").unwrap());
        assert_eq!(Printer::new(None, String::new()).query(&ast).unwrap(),
            "UNWIND $ids AS __og_v_id MATCH (n {id: __og_v_id}) RETURN n.id AS __og_v_id, [x IN n.l | x.id] AS l ORDER BY __og_v_id");
        let ast = lead_optional(&cypher_syntax::parse("OPTIONAL MATCH (x:A) RETURN x.id AS i").unwrap());
        assert_eq!(Printer::new(None, String::new()).query(&ast).unwrap(), "WITH 1 AS __og_one OPTIONAL MATCH (x:A) RETURN x.id AS i");
    }

    #[test]
    fn aggregations_with_lists_come_apart() {
        let p = |q: &str| Printer::new(None, String::new()).query(&split_aggregations(&cypher_syntax::parse(q).unwrap())).unwrap();
        assert_eq!(p("MATCH (c)-[r]-(n) WITH c, [x IN collect(n) WHERE x IS NOT NULL] AS ps RETURN c, ps"),
            "MATCH (c)-[r]-(n) WITH c, collect(n) AS __og_a1 WITH c, [x IN __og_a1 WHERE x IS NOT NULL] AS ps RETURN c, ps");
        assert_eq!(p("MATCH (b)-[:R]->(a) WITH b, collect(a) AS l RETURN b.id AS id, count(*) AS c, [x IN l WHERE x.k = 1] AS f ORDER BY b.id"),
            "MATCH (b)-[:R]->(a) WITH b, collect(a) AS l WITH b, l, [x IN l WHERE x.k = 1] AS __og_k3 \
WITH b.id AS __og_k1, count(*) AS __og_a2, __og_k3 RETURN __og_k1 AS id, __og_a2 AS c, __og_k3 AS f ORDER BY id");
        // nothing to do without both
        assert_eq!(p("MATCH (a) RETURN a.id AS id, count(*) AS c"), "MATCH (a) RETURN a.id AS id, count(*) AS c");
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
    fn datetime_arithmetic_with_fixed_parts_is_worked_out_here() {
        let params: Map<String, Json> = [("m".to_string(), Json::from(30)), ("at".to_string(), Json::from("2026-01-31T10:00:00Z"))].into_iter().collect();
        let mut p = Printer::new(Some(&params), "2026-11-18T01:00:00.5+00:00".into());
        let s = p.query(&cypher_syntax::parse("MATCH (r) WHERE datetime(r.at) < datetime() - duration({minutes: $m}) RETURN datetime($at) + duration('P1M') AS t").unwrap()).unwrap();
        assert_eq!(s, "MATCH (r) WHERE r.at < ($__og_t1) RETURN $__og_t2 AS t");
        assert_eq!(p.extra["__og_t1"], Json::from("2026-11-18T00:30:00.5+00:00"));
        assert_eq!(p.extra["__og_t2"], Json::from("2026-02-28T10:00:00+00:00"));
        // with a part not known now, it goes out as written (and AGE says duration() is unknown)
        let mut p = Printer::new(None, "2026-11-18T01:00:00+00:00".into());
        assert!(p.query(&cypher_syntax::parse("RETURN datetime() - duration({minutes: r.m}) AS t").unwrap()).unwrap().contains("duration("));
        assert_eq!(print("MATCH (a), (b) RETURN coalesce(a, b, null).k AS k"), "MATCH (a), (b) RETURN coalesce(head([a]), head([b]), null).k AS k");
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
