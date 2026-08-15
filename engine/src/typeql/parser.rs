//! TypeQL parser — spec 010 T003..T009.
//!
//! Produces a pipeline of [`Stage`]s. TypeQL statements are comma-separated
//! constraint lists hanging off a subject (`$b isa book, has title $t;`); the
//! parser flattens them into the normalised [`Pat`] primitives in `ast.rs` so
//! that by the time the compiler runs there is no subject to re-derive.

use super::ast::*;
use super::lexer::{lex, Tok, Token};

pub struct Parser {
    toks: Vec<Token>,
    pos: usize,
    anon: usize,
}

type PResult<T> = Result<T, String>;

/// Split a `.tql` script into its queries and parse each one.
///
/// Blocks are terminated by `end;`, which is what TypeDB Console writes; a
/// trailing block without `end;` is accepted too.
pub fn parse_script(src: &str) -> PResult<Vec<Query>> {
    let mut p = Parser { toks: lex(src)?, pos: 0, anon: 0 };
    let mut out = Vec::new();
    while !p.at_eof() {
        if p.eat_kw("end") {
            p.eat_sym(";");
            continue;
        }
        let q = p.query()?;
        if q.stages.is_empty() {
            break;
        }
        out.push(q);
    }
    Ok(out)
}

pub fn parse(src: &str) -> PResult<Query> {
    let qs = parse_script(src)?;
    match qs.len() {
        0 => Err("empty query".into()),
        1 => Ok(qs.into_iter().next().unwrap()),
        n => Err(format!(
            "expected one query, found {n}. use the script entry point to run several"
        )),
    }
}

impl Parser {
    // ----------------------------------------------------------------------
    // Token helpers
    // ----------------------------------------------------------------------

    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek(), Tok::Eof)
    }

    fn here(&self) -> String {
        let t = &self.toks[self.pos];
        format!("line {}:{}", t.line, t.col)
    }

    fn advance(&mut self) -> Tok {
        let t = self.toks[self.pos].tok.clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn is_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Tok::Ident(s) if s == kw)
    }

    fn eat_kw(&mut self, kw: &str) -> bool {
        if self.is_kw(kw) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn is_sym(&self, s: &str) -> bool {
        matches!(self.peek(), Tok::Sym(x) if *x == s)
    }

    fn eat_sym(&mut self, s: &str) -> bool {
        if self.is_sym(s) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect_sym(&mut self, s: &str) -> PResult<()> {
        if self.eat_sym(s) {
            Ok(())
        } else {
            Err(format!("{}: expected '{}', found '{}'", self.here(), s, self.peek()))
        }
    }

    fn ident(&mut self) -> PResult<String> {
        match self.advance() {
            Tok::Ident(s) => Ok(s),
            other => Err(format!("{}: expected a name, found '{}'", self.here(), other)),
        }
    }

    fn var(&mut self) -> PResult<String> {
        match self.advance() {
            Tok::Var(s) => Ok(s),
            other => Err(format!("{}: expected a variable, found '{}'", self.here(), other)),
        }
    }

    fn fresh(&mut self, hint: &str) -> String {
        self.anon += 1;
        format!("__{hint}{}", self.anon)
    }

    // ----------------------------------------------------------------------
    // Query / stages
    // ----------------------------------------------------------------------

    fn query(&mut self) -> PResult<Query> {
        let mut stages = Vec::new();
        loop {
            if self.at_eof() || self.is_kw("end") {
                break;
            }
            let stage = if self.eat_kw("define") {
                Stage::Define(self.definitions()?)
            } else if self.eat_kw("undefine") {
                Stage::Undefine(self.definitions()?)
            } else if self.eat_kw("redefine") {
                Stage::Define(self.definitions()?)
            } else if self.eat_kw("match") {
                Stage::Match(self.patterns_until_stage()?)
            } else if self.eat_kw("insert") {
                Stage::Insert(self.patterns_until_stage()?)
            } else if self.eat_kw("put") {
                Stage::Put(self.patterns_until_stage()?)
            } else if self.eat_kw("update") {
                Stage::Update(self.patterns_until_stage()?)
            } else if self.eat_kw("delete") {
                Stage::Delete(self.delete_items()?)
            } else if self.eat_kw("select") {
                Stage::Select(self.var_list()?)
            } else if self.eat_kw("sort") {
                Stage::Sort(self.sort_keys()?)
            } else if self.eat_kw("limit") {
                let n = self.int_lit()?;
                self.eat_sym(";");
                Stage::Limit(n)
            } else if self.eat_kw("offset") {
                let n = self.int_lit()?;
                self.eat_sym(";");
                Stage::Offset(n)
            } else if self.eat_kw("distinct") {
                self.eat_sym(";");
                Stage::Distinct
            } else if self.eat_kw("reduce") {
                self.reduce_stage()?
            } else if self.eat_kw("fetch") {
                let doc = self.fetch_doc()?;
                self.eat_sym(";");
                Stage::Fetch(doc)
            } else if self.is_kw("with") {
                return Err(format!(
                    "{}: query-local functions ('with fun') are not supported yet \
                     (spec 010 phase 6). declare the function in the schema instead",
                    self.here()
                ));
            } else if self.is_kw("get") || self.is_kw("rule") {
                return Err(format!(
                    "{}: '{}' is TypeDB 2.x syntax. this engine implements TypeQL 3.x \
                     — use 'select'/'fetch' instead of 'get'",
                    self.here(),
                    self.peek()
                ));
            } else {
                if stages.is_empty() {
                    return Err(format!(
                        "{}: expected a query stage (define, match, insert, put, delete, \
                         update, select, sort, limit, offset, distinct, reduce, fetch), \
                         found '{}'",
                        self.here(),
                        self.peek()
                    ));
                }
                break;
            };
            stages.push(stage);
        }
        if self.eat_kw("end") {
            self.eat_sym(";");
        }
        Ok(Query { stages })
    }

    fn int_lit(&mut self) -> PResult<i64> {
        match self.advance() {
            Tok::Int(v) => Ok(v),
            other => Err(format!("{}: expected an integer, found '{}'", self.here(), other)),
        }
    }

    fn var_list(&mut self) -> PResult<Vec<String>> {
        let mut out = vec![self.var()?];
        while self.eat_sym(",") {
            out.push(self.var()?);
        }
        self.eat_sym(";");
        Ok(out)
    }

    fn sort_keys(&mut self) -> PResult<Vec<(String, bool)>> {
        let mut out = Vec::new();
        loop {
            let v = self.var()?;
            let asc = if self.eat_kw("desc") {
                false
            } else {
                self.eat_kw("asc");
                true
            };
            out.push((v, asc));
            if !self.eat_sym(",") {
                break;
            }
        }
        self.eat_sym(";");
        Ok(out)
    }

    fn reduce_stage(&mut self) -> PResult<Stage> {
        let mut assigns = Vec::new();
        let mut groupby = Vec::new();
        loop {
            let var = self.var()?;
            self.expect_sym("=")?;
            let agg = self.ident()?;
            let arg = if self.eat_sym("(") {
                let a = if self.is_sym(")") { None } else { Some(self.var()?) };
                self.expect_sym(")")?;
                a
            } else {
                None
            };
            assigns.push(Reduction { var, agg: agg.to_ascii_lowercase(), arg });
            if self.eat_sym(",") {
                continue;
            }
            break;
        }
        if self.eat_kw("groupby") {
            loop {
                groupby.push(self.var()?);
                if !self.eat_sym(",") {
                    break;
                }
            }
        }
        self.eat_sym(";");
        Ok(Stage::Reduce { assigns, groupby })
    }

    /// True when the current token starts a new stage rather than another
    /// statement of the current one.
    fn at_stage_boundary(&self) -> bool {
        const STAGE_KWS: &[&str] = &[
            "match", "insert", "put", "delete", "update", "select", "sort", "limit", "offset",
            "distinct", "reduce", "fetch", "define", "undefine", "redefine", "end", "with",
            "return",
            // TypeDB 2.x stage keywords. Listed so they terminate the current
            // statement and reach the dispatcher, which rejects them by name
            // instead of failing with a confusing "not a valid statement start".
            "get", "rule",
        ];
        match self.peek() {
            Tok::Eof => true,
            Tok::Ident(s) => STAGE_KWS.contains(&s.as_str()),
            _ => false,
        }
    }

    // ----------------------------------------------------------------------
    // Patterns
    // ----------------------------------------------------------------------

    fn patterns_until_stage(&mut self) -> PResult<Vec<Pat>> {
        let mut out = Vec::new();
        while !self.at_stage_boundary() {
            self.statement(&mut out)?;
        }
        Ok(out)
    }

    fn block(&mut self) -> PResult<Vec<Pat>> {
        self.expect_sym("{")?;
        let mut out = Vec::new();
        while !self.is_sym("}") {
            if self.at_eof() {
                return Err(format!("{}: unterminated '{{' block", self.here()));
            }
            self.statement(&mut out)?;
        }
        self.expect_sym("}")?;
        Ok(out)
    }

    /// One `... ;`-terminated statement, appended to `out` as primitives.
    fn statement(&mut self, out: &mut Vec<Pat>) -> PResult<()> {
        // not { ... };
        if self.eat_kw("not") {
            let inner = self.block()?;
            self.eat_sym(";");
            out.push(Pat::Not(inner));
            return Ok(());
        }

        // { ... } or { ... };
        if self.is_sym("{") {
            let mut branches = vec![self.block()?];
            while self.eat_kw("or") {
                branches.push(self.block()?);
            }
            self.eat_sym(";");
            if branches.len() == 1 {
                out.extend(branches.pop().unwrap());
            } else {
                out.push(Pat::Or(branches));
            }
            return Ok(());
        }

        // let $x = expr;  |  let $x in f(...);
        if self.eat_kw("let") {
            let v = self.var()?;
            if self.eat_kw("in") {
                let e = self.expr()?;
                self.eat_sym(";");
                out.push(Pat::Let { var: v, expr: e });
                return Ok(());
            }
            self.expect_sym("=")?;
            let e = self.expr()?;
            self.eat_sym(";");
            out.push(Pat::Let { var: v, expr: e });
            return Ok(());
        }

        // Subject.
        let subject: String;
        let mut roles: Vec<(Option<String>, String)> = Vec::new();
        let mut subject_type: Option<String> = None;

        if self.is_sym("(") {
            // Anonymous relation: `(role: $x, $y) isa R`
            roles = self.role_list()?;
            subject = self.fresh("rel");
        } else if let Tok::Var(_) = self.peek() {
            subject = self.var()?;
            if self.is_sym("(") {
                roles = self.role_list()?;
            }
        } else if let Tok::Ident(_) = self.peek() {
            // `authoring (work: $b, author: $a)` — typed anonymous relation.
            let label = self.ident()?;
            if self.is_sym("(") {
                roles = self.role_list()?;
                subject_type = Some(label);
                subject = self.fresh("rel");
            } else {
                return Err(format!(
                    "{}: '{label}' is not a valid statement start. a statement begins with a \
                     variable, a relation type followed by role players, or '('",
                    self.here()
                ));
            }
        } else {
            return Err(format!("{}: unexpected '{}' in pattern", self.here(), self.peek()));
        }

        if let Some(t) = &subject_type {
            out.push(Pat::Isa { var: subject.clone(), ty: t.clone(), exact: false });
        }
        for (role, player) in roles {
            out.push(Pat::Role { rel: subject.clone(), role, player });
        }

        // Constraints. A statement may carry none — `authoring (work: $b, author: $a);`
        // is fully described by its subject and role players — and a role-list
        // subject is followed directly by the separating comma, as in
        // `promotion-inclusion (promotion: $p, item: $b), has discount $d;`.
        loop {
            if self.is_sym(";") || self.at_stage_boundary() {
                break;
            }
            if self.eat_sym(",") {
                continue;
            }
            self.constraint(&subject, out)?;
        }
        self.eat_sym(";");
        Ok(())
    }

    fn role_list(&mut self) -> PResult<Vec<(Option<String>, String)>> {
        self.expect_sym("(")?;
        let mut out = Vec::new();
        if self.eat_sym(")") {
            return Ok(out);
        }
        loop {
            // `role: $player` or bare `$player`
            let mut role = None;
            if let Tok::Ident(_) = self.peek() {
                let name = self.ident()?;
                self.expect_sym(":")?;
                role = Some(name);
            }
            let player = self.var()?;
            out.push((role, player));
            if self.eat_sym(",") {
                continue;
            }
            break;
        }
        self.expect_sym(")")?;
        Ok(out)
    }

    fn constraint(&mut self, subject: &str, out: &mut Vec<Pat>) -> PResult<()> {
        if self.is_kw("isa") || self.is_kw("isa!") {
            let exact = self.is_kw("isa!");
            self.advance();
            let ty = self.ident()?;
            out.push(Pat::Isa { var: subject.into(), ty, exact });
            return Ok(());
        }
        if self.is_kw("sub") || self.is_kw("sub!") {
            let exact = self.is_kw("sub!");
            self.advance();
            let ty = self.ident()?;
            out.push(Pat::Sub { var: subject.into(), ty, exact });
            return Ok(());
        }
        if self.eat_kw("label") {
            let name = self.ident()?;
            out.push(Pat::Label { var: subject.into(), name });
            return Ok(());
        }
        if self.eat_kw("links") {
            let roles = self.role_list()?;
            for (role, player) in roles {
                out.push(Pat::Role { rel: subject.into(), role, player });
            }
            return Ok(());
        }
        if self.eat_kw("has") {
            let (attr, val) = self.has_tail()?;
            out.push(Pat::Has { owner: subject.into(), attr, val });
            return Ok(());
        }
        if let Some(op) = self.cmp_op() {
            let rhs = self.expr()?;
            out.push(Pat::Cmp { lhs: Expr::Var(subject.into()), op, rhs });
            return Ok(());
        }
        Err(format!(
            "{}: expected a constraint (isa, has, links, sub, label or a comparison), found '{}'",
            self.here(),
            self.peek()
        ))
    }

    fn has_tail(&mut self) -> PResult<(Option<String>, HasVal)> {
        // `has $a`
        if let Tok::Var(_) = self.peek() {
            let v = self.var()?;
            return Ok((None, HasVal::Var(v)));
        }
        let attr = self.ident()?;
        // `has genre` — existence only.
        if self.is_sym(",") || self.is_sym(";") || self.at_stage_boundary() {
            return Ok((Some(attr), HasVal::Any));
        }
        if let Tok::Var(_) = self.peek() {
            let v = self.var()?;
            return Ok((Some(attr), HasVal::Var(v)));
        }
        if let Some(op) = self.cmp_op() {
            let e = self.expr()?;
            return Ok((Some(attr), HasVal::Cmp(op, e)));
        }
        let e = self.expr()?;
        Ok((Some(attr), HasVal::Cmp(CmpOp::Eq, e)))
    }

    fn cmp_op(&mut self) -> Option<CmpOp> {
        let op = match self.peek() {
            Tok::Sym("==") => CmpOp::Eq,
            Tok::Sym("=") => CmpOp::Eq,
            Tok::Sym("!=") => CmpOp::Ne,
            Tok::Sym("<") => CmpOp::Lt,
            Tok::Sym("<=") => CmpOp::Le,
            Tok::Sym(">") => CmpOp::Gt,
            Tok::Sym(">=") => CmpOp::Ge,
            Tok::Ident(s) if s == "like" => CmpOp::Like,
            Tok::Ident(s) if s == "contains" => CmpOp::Contains,
            _ => return None,
        };
        self.advance();
        Some(op)
    }

    // ----------------------------------------------------------------------
    // Delete
    // ----------------------------------------------------------------------

    fn delete_items(&mut self) -> PResult<Vec<DeleteItem>> {
        let mut out = Vec::new();
        while !self.at_stage_boundary() {
            if self.eat_kw("has") {
                let attr = match self.peek() {
                    Tok::Var(_) => self.var()?,
                    _ => self.ident()?,
                };
                let owner = if self.eat_kw("of") { self.var()? } else { String::new() };
                out.push(DeleteItem { kind: DeleteKind::Has { owner, attr } });
            } else if self.eat_kw("links") {
                let roles = self.role_list()?;
                let rel = if self.eat_kw("of") { self.var()? } else { String::new() };
                for (role, player) in roles {
                    out.push(DeleteItem {
                        kind: DeleteKind::Links { rel: rel.clone(), role, player },
                    });
                }
            } else {
                let v = self.var()?;
                out.push(DeleteItem { kind: DeleteKind::Instance(v) });
            }
            if self.eat_sym(",") {
                continue;
            }
            self.eat_sym(";");
            if self.at_stage_boundary() {
                break;
            }
        }
        Ok(out)
    }

    // ----------------------------------------------------------------------
    // Fetch
    // ----------------------------------------------------------------------

    fn fetch_doc(&mut self) -> PResult<FetchDoc> {
        self.expect_sym("{")?;
        let mut entries = Vec::new();
        if self.eat_sym("}") {
            return Ok(FetchDoc { entries });
        }
        loop {
            let key = match self.advance() {
                Tok::Str(s) => s,
                Tok::Ident(s) => s,
                other => {
                    return Err(format!(
                        "{}: fetch keys must be strings, found '{}'",
                        self.here(),
                        other
                    ))
                }
            };
            self.expect_sym(":")?;
            let val = self.fetch_val()?;
            entries.push((key, val));
            if self.eat_sym(",") {
                if self.is_sym("}") {
                    break;
                }
                continue;
            }
            break;
        }
        self.expect_sym("}")?;
        Ok(FetchDoc { entries })
    }

    fn fetch_val(&mut self) -> PResult<FetchVal> {
        if self.is_sym("{") {
            return Ok(FetchVal::Doc(self.fetch_doc()?));
        }
        if self.eat_sym("[") {
            // Either a sub-fetch pipeline or an all-values attribute projection.
            if self.is_kw("match") {
                let q = self.query()?;
                self.expect_sym("]")?;
                return Ok(FetchVal::SubFetch(Box::new(q)));
            }
            let v = self.var()?;
            self.expect_sym(".")?;
            let a = self.ident()?;
            self.expect_sym("]")?;
            return Ok(FetchVal::AttrList(v, a));
        }
        Ok(FetchVal::Expr(self.expr()?))
    }

    // ----------------------------------------------------------------------
    // Expressions
    // ----------------------------------------------------------------------

    pub fn expr(&mut self) -> PResult<Expr> {
        self.additive()
    }

    fn additive(&mut self) -> PResult<Expr> {
        let mut lhs = self.multiplicative()?;
        loop {
            let op = if self.is_sym("+") {
                "+"
            } else if self.is_sym("-") {
                "-"
            } else {
                break;
            };
            self.advance();
            let rhs = self.multiplicative()?;
            lhs = Expr::Bin(Box::new(lhs), op, Box::new(rhs));
        }
        Ok(lhs)
    }

    fn multiplicative(&mut self) -> PResult<Expr> {
        let mut lhs = self.unary()?;
        loop {
            let op = if self.is_sym("*") {
                "*"
            } else if self.is_sym("/") {
                "/"
            } else if self.is_sym("%") {
                "%"
            } else {
                break;
            };
            self.advance();
            let rhs = self.unary()?;
            lhs = Expr::Bin(Box::new(lhs), op, Box::new(rhs));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> PResult<Expr> {
        if self.eat_sym("-") {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> PResult<Expr> {
        if self.eat_sym("(") {
            let e = self.expr()?;
            self.expect_sym(")")?;
            return Ok(e);
        }
        match self.advance() {
            Tok::Var(v) => {
                if self.is_sym(".") {
                    self.advance();
                    let a = self.ident()?;
                    Ok(Expr::Attr(v, a))
                } else {
                    Ok(Expr::Var(v))
                }
            }
            Tok::Str(s) => Ok(Expr::Lit(Lit::Str(s))),
            Tok::Int(i) => Ok(Expr::Lit(Lit::Int(i))),
            Tok::Float(f) => Ok(Expr::Lit(Lit::Float(f))),
            Tok::DateTime(s) => Ok(Expr::Lit(Lit::DateTime(s))),
            Tok::Ident(name) => {
                if name == "true" {
                    return Ok(Expr::Lit(Lit::Bool(true)));
                }
                if name == "false" {
                    return Ok(Expr::Lit(Lit::Bool(false)));
                }
                if self.eat_sym("(") {
                    let mut args = Vec::new();
                    if !self.is_sym(")") {
                        loop {
                            args.push(self.expr()?);
                            if self.eat_sym(",") {
                                continue;
                            }
                            break;
                        }
                    }
                    self.expect_sym(")")?;
                    return Ok(Expr::Call(name, args));
                }
                Err(format!("{}: unexpected name '{name}' in an expression", self.here()))
            }
            other => Err(format!("{}: unexpected '{}' in an expression", self.here(), other)),
        }
    }

    // ----------------------------------------------------------------------
    // define
    // ----------------------------------------------------------------------

    fn definitions(&mut self) -> PResult<Vec<Definition>> {
        let mut out = Vec::new();
        while !self.at_stage_boundary() {
            if self.is_kw("fun") {
                out.push(Definition::Function(self.function_def()?));
                continue;
            }
            out.push(self.type_def()?);
        }
        Ok(out)
    }

    fn type_def(&mut self) -> PResult<Definition> {
        let kw = self.ident()?;
        let kind = match kw.as_str() {
            "entity" => Kind::Entity,
            "relation" => Kind::Relation,
            "attribute" => Kind::Attribute,
            other => {
                return Err(format!(
                    "{}: expected 'entity', 'relation', 'attribute' or 'fun', found '{other}'",
                    self.here()
                ))
            }
        };
        let name = self.ident()?;
        let mut def = Definition::Type {
            kind,
            name,
            sub: None,
            value_type: None,
            anns: Annotations::default(),
            owns: Vec::new(),
            relates: Vec::new(),
            plays: Vec::new(),
        };

        loop {
            let Definition::Type { sub, value_type, anns, owns, relates, plays, .. } = &mut def
            else {
                unreachable!()
            };

            if self.is_kw("sub") || self.is_kw("sub!") {
                self.advance();
                *sub = Some(self.ident()?);
            } else if self.eat_kw("value") {
                *value_type = Some(self.ident()?);
                self.annotations_into(anns)?;
            } else if self.eat_kw("owns") {
                let attribute = self.ident()?;
                let mut a = Annotations::default();
                self.annotations_into(&mut a)?;
                owns.push(Owns { attribute, anns: a });
            } else if self.eat_kw("relates") {
                let role = self.ident()?;
                let specialises = if self.eat_kw("as") { Some(self.ident()?) } else { None };
                let mut a = Annotations::default();
                self.annotations_into(&mut a)?;
                relates.push(Relates { role, specialises, anns: a });
            } else if self.eat_kw("plays") {
                let relation = self.ident()?;
                self.expect_sym(":")?;
                let role = self.ident()?;
                plays.push(Plays { relation, role });
            } else if self.is_sym("@") {
                self.annotations_into(anns)?;
            } else if self.eat_sym(",") {
                continue;
            } else if self.eat_sym(";") {
                break;
            } else {
                return Err(format!(
                    "{}: unexpected '{}' in a type definition",
                    self.here(),
                    self.peek()
                ));
            }
        }
        Ok(def)
    }

    fn annotations_into(&mut self, anns: &mut Annotations) -> PResult<()> {
        while self.is_sym("@") {
            self.advance();
            let name = self.ident()?;
            match name.as_str() {
                "abstract" => anns.abstract_ = true,
                "key" => anns.key = true,
                "unique" => anns.unique = true,
                "card" => {
                    self.expect_sym("(")?;
                    let lo = self.int_lit()?;
                    self.expect_sym("..")?;
                    let hi = if self.is_sym(")") { None } else { Some(self.int_lit()?) };
                    self.expect_sym(")")?;
                    anns.card = Some((lo, hi));
                }
                "values" => {
                    self.expect_sym("(")?;
                    loop {
                        anns.values.push(self.literal()?);
                        if self.eat_sym(",") {
                            continue;
                        }
                        break;
                    }
                    self.expect_sym(")")?;
                }
                "range" => {
                    self.expect_sym("(")?;
                    let lo = if self.is_sym("..") { None } else { Some(self.literal()?) };
                    self.expect_sym("..")?;
                    let hi = if self.is_sym(")") { None } else { Some(self.literal()?) };
                    self.expect_sym(")")?;
                    anns.range = Some((lo, hi));
                }
                "distinct" | "cascade" | "independent" | "subkey" => {
                    // Accepted and ignored: they constrain nothing we materialise.
                    if self.eat_sym("(") {
                        while !self.eat_sym(")") {
                            if self.at_eof() {
                                return Err(format!("{}: unterminated annotation", self.here()));
                            }
                            self.advance();
                        }
                    }
                }
                other => {
                    return Err(format!(
                        "{}: unknown annotation '@{other}'. supported: @abstract, @key, \
                         @unique, @card, @values, @range",
                        self.here()
                    ))
                }
            }
        }
        Ok(())
    }

    fn literal(&mut self) -> PResult<Lit> {
        match self.advance() {
            Tok::Str(s) => Ok(Lit::Str(s)),
            Tok::Int(i) => Ok(Lit::Int(i)),
            Tok::Float(f) => Ok(Lit::Float(f)),
            Tok::DateTime(s) => Ok(Lit::DateTime(s)),
            Tok::Ident(s) if s == "true" => Ok(Lit::Bool(true)),
            Tok::Ident(s) if s == "false" => Ok(Lit::Bool(false)),
            other => Err(format!("{}: expected a literal, found '{}'", self.here(), other)),
        }
    }

    /// `fun name($p: t, ...) -> { r } : <body> return <...>;`
    ///
    /// The body is captured verbatim rather than parsed: spec 010 phase 6 owns
    /// evaluation, and FR-044 only requires that the declaration survives a
    /// round trip. Capturing it now means loading a real TypeDB schema does not
    /// fail on a feature we have not built yet.
    fn function_def(&mut self) -> PResult<FunctionDef> {
        let start = self.pos;
        self.advance(); // 'fun'
        let name = self.ident()?;
        self.expect_sym("(")?;
        let mut params = Vec::new();
        if !self.is_sym(")") {
            loop {
                let p = self.var()?;
                self.expect_sym(":")?;
                let t = self.ident()?;
                params.push((p, t));
                if self.eat_sym(",") {
                    continue;
                }
                break;
            }
        }
        self.expect_sym(")")?;
        self.expect_sym("->")?;
        let mut returns_stream = false;
        let mut return_types = Vec::new();
        if self.eat_sym("{") {
            returns_stream = true;
            while !self.eat_sym("}") {
                if self.at_eof() {
                    return Err(format!("{}: unterminated function return type", self.here()));
                }
                if let Tok::Ident(t) = self.peek().clone() {
                    return_types.push(t);
                }
                self.advance();
            }
        } else {
            return_types.push(self.ident()?);
        }
        self.expect_sym(":")?;

        // Skip the body: find `return` at brace depth 0, then its `;`.
        let mut depth = 0i32;
        loop {
            if self.at_eof() {
                return Err(format!("{}: function '{name}' has no 'return' clause", self.here()));
            }
            if self.is_sym("{") {
                depth += 1;
            } else if self.is_sym("}") {
                depth -= 1;
            } else if depth == 0 && self.is_kw("return") {
                self.advance();
                let mut d2 = 0i32;
                loop {
                    if self.at_eof() {
                        return Err(format!(
                            "{}: unterminated 'return' in function '{name}'",
                            self.here()
                        ));
                    }
                    if self.is_sym("{") || self.is_sym("(") {
                        d2 += 1;
                    } else if self.is_sym("}") || self.is_sym(")") {
                        d2 -= 1;
                    } else if d2 == 0 && self.is_sym(";") {
                        self.advance();
                        break;
                    }
                    self.advance();
                }
                break;
            }
            self.advance();
        }

        let source = self.toks[start..self.pos]
            .iter()
            .map(|t| t.tok.to_string())
            .collect::<Vec<_>>()
            .join(" ");

        Ok(FunctionDef { name, params, returns_stream, return_types, source })
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn parses_the_bookstore_schema_shape() {
        let src = r#"
        define
        entity book @abstract,
            owns isbn @card(0..2),
            owns isbn-13 @key,
            plays contribution:work;
        entity hardback sub book, owns stock;
        relation authoring sub contribution, relates author as contributor;
        attribute status, value string @values("pending", "paid");
        attribute loyalty-tier, value integer @range(0..5);
        "#;
        let q = parse(src).unwrap();
        let Stage::Define(defs) = &q.stages[0] else { panic!("expected define") };
        assert_eq!(defs.len(), 5);
        let Definition::Type { name, anns, owns, plays, .. } = &defs[0] else { panic!() };
        assert_eq!(name, "book");
        assert!(anns.abstract_);
        assert_eq!(owns.len(), 2);
        assert_eq!(owns[0].anns.card, Some((0, Some(2))));
        assert!(owns[1].anns.key);
        assert_eq!(plays[0].relation, "contribution");
        let Definition::Type { relates, sub, .. } = &defs[2] else { panic!() };
        assert_eq!(sub.as_deref(), Some("contribution"));
        assert_eq!(relates[0].specialises.as_deref(), Some("contributor"));
        let Definition::Type { anns, .. } = &defs[3] else { panic!() };
        assert_eq!(anns.values.len(), 2);
        let Definition::Type { anns, .. } = &defs[4] else { panic!() };
        assert!(anns.range.is_some());
    }

    #[test]
    fn parses_a_match_fetch_pipeline() {
        let src = r#"
        match
          $book isa book, has genre "science fiction";
        fetch {
          "title": $book.title,
          "authors": [ match authoring (work: $book, author: $author); fetch { "name": $author.name }; ],
          "price": $book.price
        };
        "#;
        let q = parse(src).unwrap();
        assert_eq!(q.stages.len(), 2);
        let Stage::Match(pats) = &q.stages[0] else { panic!() };
        assert_eq!(pats.len(), 2);
        let Stage::Fetch(doc) = &q.stages[1] else { panic!() };
        assert_eq!(doc.entries.len(), 3);
        assert!(matches!(doc.entries[1].1, FetchVal::SubFetch(_)));
    }

    #[test]
    fn splits_a_script_into_blocks() {
        let src = r#"
        insert $c isa country, has name "United States";
        end;
        match $c isa country, has name "United States";
        insert $s isa state, has name "California"; (location: $c, located: $s) isa locating;
        end;
        "#;
        let qs = parse_script(src).unwrap();
        assert_eq!(qs.len(), 2);
        assert_eq!(qs[1].stages.len(), 2);
    }

    #[test]
    fn rejects_typedb_2x_syntax_explicitly() {
        let e = parse("match $p isa person; get $p;").unwrap_err();
        assert!(e.contains("2.x"), "{e}");
    }

    #[test]
    fn captures_function_declarations_without_evaluating_them() {
        let src = r#"
        define
        fun best_discount_for_item($item: book, $t: datetime) -> double:
          match
            { $i isa promotion-inclusion, has discount $d; } or { let $d = 0.0; };
          return max($d);
        "#;
        let q = parse(src).unwrap();
        let Stage::Define(defs) = &q.stages[0] else { panic!() };
        let Definition::Function(f) = &defs[0] else { panic!("expected a function") };
        assert_eq!(f.name, "best_discount_for_item");
        assert_eq!(f.params.len(), 2);
        assert!(!f.returns_stream);
    }
}

#[cfg(test)]
mod corpus {
    //! Parse the vendored TypeDB example application verbatim — spec 010 T055/T056.
    //! These files are the upstream ones, byte for byte; if the grammar drifts
    //! from real TypeQL this is where it shows up first.
    use super::*;

    fn read(name: &str) -> String {
        let p = format!("{}/../examples/typedb/bookstore/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {p}: {e}"))
    }

    #[test]
    fn parses_the_upstream_schema() {
        let qs = parse_script(&read("schema.tql")).expect("schema.tql must parse");
        let defs: Vec<&Definition> = qs
            .iter()
            .flat_map(|q| &q.stages)
            .filter_map(|s| match s {
                Stage::Define(d) => Some(d),
                _ => None,
            })
            .flatten()
            .collect();
        let types = defs.iter().filter(|d| matches!(d, Definition::Type { .. })).count();
        let funs = defs.iter().filter(|d| matches!(d, Definition::Function(_))).count();
        // 19 entity + 12 relation + 25 attribute declarations, 7 functions —
        // counted from the upstream file, not from what the parser happens to do.
        assert_eq!(types, 56, "entity+relation+attribute type declarations");
        assert_eq!(funs, 7, "fun declarations");
    }

    #[test]
    fn parses_the_upstream_data() {
        let qs = parse_script(&read("data.tql")).expect("data.tql must parse");
        // The upstream file has exactly 31 `end;`-terminated blocks.
        assert_eq!(qs.len(), 31, "write blocks");
        // Every block must contain at least one write stage, or we silently
        // parsed data as something inert.
        for (i, q) in qs.iter().enumerate() {
            assert!(
                q.stages.iter().any(|s| matches!(
                    s,
                    Stage::Insert(_) | Stage::Put(_) | Stage::Update(_) | Stage::Delete(_)
                )),
                "block {i} has no write stage: {:?}",
                q.stages.iter().map(std::mem::discriminant).collect::<Vec<_>>()
            );
        }
    }
}
