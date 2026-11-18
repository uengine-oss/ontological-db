//! Recursive-descent Cypher parser — spec 003 FR-001..FR-008.
//!
//! Unsupported syntax fails loudly at parse time with the offending construct
//! named (FR-008): silently reinterpreting a query is worse than rejecting it.

use super::ast::*;
use super::lexer::{Lexer, Tok, Token};

/// How many `.` segments a function name may carry: `genai.vector.encode` needs
/// two, and the bound is what keeps a long property path from being scanned to
/// the end of the query before it is ruled out as a call.
const MAX_NAMESPACE_DEPTH: usize = 3;

pub struct Parser {
    toks: Vec<Token>,
    i: usize,
    src: String,
    /// Inside the `WHERE` of a list comprehension, `|` starts the projection
    /// (`[x IN xs WHERE x:A | x.name]`), so it cannot also separate label
    /// alternatives there.
    in_comprehension_filter: usize,
    /// Inside `CALL { … }`: a `}` ends the query being parsed.
    subquery_depth: usize,
}

type PResult<T> = Result<T, String>;

pub fn parse(src: &str) -> PResult<Query> {
    let toks = Lexer::new(src).tokenize()?;
    let mut p = Parser { toks, i: 0, src: src.to_string(), in_comprehension_filter: 0, subquery_depth: 0 };
    let q = p.parse_query()?;
    p.expect_eof()?;
    Ok(q)
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.i.min(self.toks.len() - 1)].tok
    }

    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.i + n).min(self.toks.len() - 1)].tok
    }

    fn bump(&mut self) -> Tok {
        let t = self.toks[self.i.min(self.toks.len() - 1)].tok.clone();
        if self.i < self.toks.len() - 1 {
            self.i += 1;
        }
        t
    }

    fn at_kw(&self, k: &str) -> bool {
        matches!(self.peek(), Tok::Keyword(s) if s == k)
    }

    fn at_punct(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::Punct(s) if s == p)
    }

    fn eat_kw(&mut self, k: &str) -> bool {
        if self.at_kw(k) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_punct(&mut self, p: &str) -> bool {
        if self.at_punct(p) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect_punct(&mut self, p: &str) -> PResult<()> {
        if self.eat_punct(p) {
            Ok(())
        } else {
            Err(self.err(&format!("expected '{p}'")))
        }
    }

    fn expect_kw(&mut self, k: &str) -> PResult<()> {
        if self.eat_kw(k) {
            Ok(())
        } else {
            Err(self.err(&format!("expected '{}'", k.to_uppercase())))
        }
    }

    fn expect_eof(&self) -> PResult<()> {
        if matches!(self.peek(), Tok::Eof) {
            Ok(())
        } else {
            Err(self.err("unexpected trailing input"))
        }
    }

    fn err(&self, msg: &str) -> String {
        let pos = self.toks[self.i.min(self.toks.len() - 1)].pos;
        let snippet: String = self.src.chars().skip(pos.saturating_sub(12)).take(36).collect();
        format!("{msg} at offset {pos}, near: …{snippet}…")
    }

    /// Identifier in a position where keywords are also acceptable names.
    ///
    /// A keyword's `tok` carries the lowercased spelling so keyword matching is
    /// case-insensitive, but here the word is a *name* — `[r:CONTAINS]`,
    /// `(n:Order)` — and names are case-sensitive. Take the source spelling.
    fn name(&mut self) -> PResult<String> {
        let at = self.i.min(self.toks.len() - 1);
        match self.bump() {
            Tok::Ident(s) | Tok::QuotedIdent(s) => Ok(s),
            Tok::Keyword(_) => Ok(self.toks[at].raw.clone()),
            _ => {
                self.i -= 1;
                Err(self.err("expected a name"))
            }
        }
    }

    // ----------------------------------------------------------------------
    // Query structure
    // ----------------------------------------------------------------------

    fn parse_query(&mut self) -> PResult<Query> {
        let mut clauses = Vec::new();
        loop {
            match self.peek().clone() {
                Tok::Keyword(k) => match k.as_str() {
                    "match" => clauses.push(self.parse_match(false)?),
                    "optional" if self.peek_at(1) == &Tok::Keyword("call".into()) => {
                        self.bump();
                        let c = self.parse_call()?;
                        let Clause::Subquery { scope, body, .. } = c else {
                            return Err(self.err("OPTIONAL CALL is supported for subqueries (`OPTIONAL CALL { … }`), not procedures"));
                        };
                        clauses.push(Clause::Subquery { scope, body, optional: true });
                    }
                    "optional" => {
                        self.bump();
                        self.expect_kw("match")?;
                        self.i -= 1;
                        self.bump();
                        clauses.push(self.parse_match_body(true)?);
                    }
                    "unwind" => clauses.push(self.parse_unwind()?),
                    "with" => clauses.push(self.parse_with()?),
                    "return" => clauses.push(self.parse_return()?),
                    "create" => clauses.push(self.parse_create()?),
                    "merge" => clauses.push(self.parse_merge()?),
                    "set" => clauses.push(self.parse_set()?),
                    "remove" => clauses.push(self.parse_remove()?),
                    "delete" | "detach" => clauses.push(self.parse_delete()?),
                    "call" => clauses.push(self.parse_call()?),
                    "drop" => clauses.push(self.parse_drop()?),
                    "union" => break,
                    other => {
                        return Err(self.err(&format!("unexpected clause '{}'", other.to_uppercase())))
                    }
                },
                // `SHOW` is matched by spelling rather than reserved, for the
                // same reason INDEX and CONSTRAINT are not keywords: reserving
                // it would stop anyone from having a property or label called
                // `show`. It can only start a clause, so recognising it here
                // costs nothing elsewhere.
                Tok::Ident(s) if s.eq_ignore_ascii_case("show") => {
                    clauses.extend(self.parse_show()?)
                }
                Tok::Ident(s) if s.eq_ignore_ascii_case("foreach") && self.peek_at(1) == &Tok::Punct("(".into()) => {
                    clauses.push(self.parse_foreach()?)
                }
                Tok::Eof => break,
                Tok::Punct(p) if p == "}" && self.subquery_depth > 0 => break,
                _ => return Err(self.err("expected a clause keyword")),
            }
        }

        if clauses.is_empty() {
            return Err("empty query".into());
        }

        let union = if self.eat_kw("union") {
            let all = self.eat_kw("all");
            Some((all, Box::new(self.parse_query()?)))
        } else {
            None
        };

        Ok(Query { clauses, union })
    }

    fn parse_match(&mut self, optional: bool) -> PResult<Clause> {
        self.expect_kw("match")?;
        self.parse_match_body(optional)
    }

    fn parse_match_body(&mut self, optional: bool) -> PResult<Clause> {
        let mut patterns = vec![self.parse_pattern()?];
        while self.eat_punct(",") {
            patterns.push(self.parse_pattern()?);
        }
        let where_ = if self.eat_kw("where") { Some(self.parse_expr()?) } else { None };
        Ok(Clause::Match { patterns, optional, where_ })
    }

    fn parse_unwind(&mut self) -> PResult<Clause> {
        self.expect_kw("unwind")?;
        let expr = self.parse_expr()?;
        self.expect_kw("as")?;
        let alias = self.name()?;
        Ok(Clause::Unwind { expr, alias })
    }

    fn parse_with(&mut self) -> PResult<Clause> {
        self.expect_kw("with")?;
        let proj = self.parse_projection()?;
        let where_ = if self.eat_kw("where") { Some(self.parse_expr()?) } else { None };
        Ok(Clause::With { proj, where_ })
    }

    fn parse_return(&mut self) -> PResult<Clause> {
        self.expect_kw("return")?;
        Ok(Clause::Return(self.parse_projection()?))
    }

    fn parse_projection(&mut self) -> PResult<Projection> {
        let mut proj = Projection { distinct: self.eat_kw("distinct"), ..Default::default() };
        loop {
            let expr = if self.at_punct("*") {
                self.bump();
                Expr::Star
            } else {
                self.parse_expr()?
            };
            let alias = if self.eat_kw("as") { Some(self.name()?) } else { None };
            proj.items.push(ReturnItem { expr, alias });
            if !self.eat_punct(",") {
                break;
            }
        }
        if self.eat_kw("order") {
            self.expect_kw("by")?;
            loop {
                let expr = self.parse_expr()?;
                let desc = if self.eat_kw("desc") {
                    true
                } else {
                    self.eat_kw("asc");
                    false
                };
                proj.order.push(OrderItem { expr, desc });
                if !self.eat_punct(",") {
                    break;
                }
            }
        }
        if self.eat_kw("skip") {
            proj.skip = Some(self.parse_expr()?);
        }
        if self.eat_kw("limit") {
            proj.limit = Some(self.parse_expr()?);
        }
        Ok(proj)
    }

    // ----------------------------------------------------------------------
    // Words that are not reserved
    //
    // DDL reads like English — INDEX, FOR, REQUIRE, OPTIONS — but reserving
    // those spellings would take them away from anyone who wants a property
    // called `text`. So they are matched here as ordinary words instead.
    // ----------------------------------------------------------------------

    fn word_at(&self, n: usize) -> Option<String> {
        match self.peek_at(n) {
            Tok::Ident(s) | Tok::QuotedIdent(s) => Some(s.to_ascii_lowercase()),
            Tok::Keyword(s) => Some(s.clone()),
            _ => None,
        }
    }

    fn at_word(&self, w: &str) -> bool {
        self.word_at(0).as_deref() == Some(w)
    }

    fn eat_word(&mut self, w: &str) -> bool {
        if self.at_word(w) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect_word(&mut self, w: &str) -> PResult<()> {
        if self.eat_word(w) {
            Ok(())
        } else {
            Err(self.err(&format!("expected '{}'", w.to_uppercase())))
        }
    }

    /// `IF NOT EXISTS` / `IF EXISTS`, both optional.
    fn eat_if_exists(&mut self, negated: bool) -> bool {
        let save = self.i;
        if !self.eat_word("if") {
            return false;
        }
        if negated && !self.eat_kw("not") {
            self.i = save;
            return false;
        }
        if !self.eat_kw("exists") {
            self.i = save;
            return false;
        }
        true
    }

    // ----------------------------------------------------------------------
    // CALL
    // ----------------------------------------------------------------------

    /// `SHOW CONSTRAINTS` / `SHOW INDEXES`, with the optional
    /// `YIELD … [WHERE …]` tail Neo4j allows.
    ///
    /// These are administration *commands*, not procedures, so a driver cannot
    /// reach them through `CALL`. Tools ask for them constantly — a schema
    /// browser lists them, and any code that wants to know which labels carry a
    /// declared key has nowhere else to look — so refusing the syntax makes the
    /// database look like it has no constraints rather than like it does not
    /// speak the command.
    ///
    /// Desugared into the procedure machinery: the command becomes a `Call`
    /// with the same yielded columns, so `YIELD`, `WHERE`, `RETURN` and joins
    /// all behave as they do everywhere else and nothing new has to be planned.
    fn parse_show(&mut self) -> PResult<Vec<Clause>> {
        self.bump(); // SHOW — an identifier, not a reserved word
        let what = self.name()?.to_ascii_lowercase();
        let name = match what.as_str() {
            "constraint" | "constraints" => "show.constraints",
            "index" | "indexes" | "indices" => "show.indexes",
            other => {
                return Err(self.err(&format!(
                    "SHOW {} is not supported (SHOW CONSTRAINTS | SHOW INDEXES)",
                    other.to_uppercase()
                )))
            }
        };
        // Neo4j accepts a trailing verbosity word; it changes only how much is
        // printed, and we always return the same columns.
        let _ = self.eat_kw("brief") || self.eat_kw("verbose");

        let columns = show_columns(name);
        let mut yields: Vec<(String, String)> = Vec::new();
        if self.eat_kw("yield") {
            loop {
                let col = self.name()?;
                let alias = if self.eat_kw("as") { self.name()? } else { col.clone() };
                yields.push((col, alias));
                if !self.eat_punct(",") {
                    break;
                }
            }
        }

        let explicit_yield = !yields.is_empty();
        if !explicit_yield {
            yields = columns.iter().map(|c| ((*c).to_string(), (*c).to_string())).collect();
        }

        let mut out = vec![Clause::Call { name: name.into(), args: Vec::new(), yields: yields.clone() }];

        // `YIELD … WHERE p` filters the yielded rows. `WITH * WHERE p` is
        // exactly that, and it already exists.
        if self.eat_kw("where") {
            let pred = self.parse_expr()?;
            out.push(Clause::With {
                proj: projection_of(&yields),
                where_: Some(pred),
            });
        }

        // Bare `SHOW CONSTRAINTS` is a complete query in Neo4j. Without an
        // explicit YIELD the caller wrote no RETURN either, so supply one.
        if !explicit_yield && !self.at_kw("return") {
            out.push(Clause::Return(projection_of(&yields)));
        }
        Ok(out)
    }

    fn parse_call(&mut self) -> PResult<Clause> {
        self.expect_kw("call")?;
        // `CALL { … }`, `CALL (a, b) { … }`, `CALL (*) { … }` — a subquery.
        let scope = if self.at_punct("(") && self.scope_clause_ahead() {
            self.bump();
            let mut names = Vec::new();
            if self.eat_punct("*") {
                names.push("*".to_string());
            } else if !self.at_punct(")") {
                loop {
                    names.push(self.name()?);
                    if !self.eat_punct(",") {
                        break;
                    }
                }
            }
            self.expect_punct(")")?;
            Some(names)
        } else {
            None
        };
        if self.at_punct("{") || scope.is_some() {
            self.expect_punct("{")?;
            self.subquery_depth += 1;
            let body = self.parse_query();
            self.subquery_depth -= 1;
            let body = body?;
            self.expect_punct("}")?;
            if self.at_kw("in") && self.word_at(1).as_deref() == Some("transactions") {
                return Err(self.err(
                    "CALL { … } IN TRANSACTIONS is not supported: it commits each batch on its own, and a \
                     statement here runs in one transaction. Send the batches as separate statements, or \
                     leave out IN TRANSACTIONS to write them all in this one",
                ));
            }
            return Ok(Clause::Subquery { scope, body: Box::new(body), optional: false });
        }
        let mut name = self.name()?;
        while self.eat_punct(".") {
            name.push('.');
            name.push_str(&self.name()?);
        }
        let mut args = Vec::new();
        if self.eat_punct("(") {
            if !self.at_punct(")") {
                loop {
                    args.push(self.parse_expr()?);
                    if !self.eat_punct(",") {
                        break;
                    }
                }
            }
            self.expect_punct(")")?;
        }
        let mut yields = Vec::new();
        if self.eat_kw("yield") {
            loop {
                let col = self.name()?;
                let alias = if self.eat_kw("as") { self.name()? } else { col.clone() };
                yields.push((col, alias));
                if !self.eat_punct(",") {
                    break;
                }
            }
        }
        Ok(Clause::Call { name, args, yields })
    }

    // ----------------------------------------------------------------------
    // Index / constraint DDL
    // ----------------------------------------------------------------------

    /// Is what follows `CREATE` a DDL statement rather than a pattern?
    fn create_is_ddl(&self) -> bool {
        match self.word_at(1).as_deref() {
            Some("index" | "constraint" | "lookup") => true,
            // `CREATE VECTOR INDEX`, `CREATE FULLTEXT INDEX`, `CREATE TEXT INDEX`…
            Some("vector" | "fulltext" | "text" | "point" | "range") => {
                self.word_at(2).as_deref() == Some("index")
            }
            _ => false,
        }
    }

    fn parse_ddl_create(&mut self) -> PResult<Clause> {
        self.expect_kw("create")?;
        let kind = if self.eat_word("vector") {
            IndexKind::Vector
        } else if self.eat_word("fulltext") {
            IndexKind::Fulltext
        } else if self.eat_word("text") || self.eat_word("point") || self.eat_word("range") {
            IndexKind::Btree
        } else {
            IndexKind::Btree
        };

        if self.eat_word("constraint") {
            return self.parse_constraint_body();
        }
        self.eat_word("lookup");
        self.expect_word("index")?;

        // `CREATE INDEX name IF NOT EXISTS FOR …` — the name is optional.
        let mut if_not_exists = self.eat_if_exists(true);
        let name = if !self.at_word("for") && !if_not_exists {
            let n = Some(self.name()?);
            if_not_exists = self.eat_if_exists(true);
            n
        } else {
            None
        };

        self.expect_word("for")?;
        let (on_relationship, label) = self.parse_ddl_target()?;
        self.expect_kw("on")?;
        let props = self.parse_ddl_props()?;
        let options = if self.eat_word("options") { self.parse_ddl_options()? } else { Vec::new() };

        Ok(Clause::Ddl(Ddl::CreateIndex {
            name,
            kind,
            on_relationship,
            label,
            props,
            options,
            if_not_exists,
        }))
    }

    fn parse_constraint_body(&mut self) -> PResult<Clause> {
        let mut if_not_exists = self.eat_if_exists(true);
        let name = if !self.at_word("for") && !if_not_exists {
            let n = Some(self.name()?);
            if_not_exists = self.eat_if_exists(true);
            n
        } else {
            None
        };
        self.expect_word("for")?;
        let (_, label) = self.parse_ddl_target()?;
        // Neo4j 5 says REQUIRE; Neo4j 4 said ASSERT. Both mean this.
        if !self.eat_word("require") {
            self.expect_word("assert")?;
        }
        let props = self.parse_ddl_props()?;
        self.expect_kw("is")?;
        let kind = if self.eat_word("unique") {
            ConstraintKind::Unique
        } else if self.eat_kw("not") {
            self.expect_kw("null")?;
            ConstraintKind::NotNull
        } else if self.eat_word("node") || self.eat_word("relationship") {
            self.expect_word("key")?;
            ConstraintKind::NodeKey
        } else {
            return Err(self.err("expected UNIQUE, NOT NULL or NODE KEY"));
        };
        Ok(Clause::Ddl(Ddl::CreateConstraint { name, label, props, kind, if_not_exists }))
    }

    /// `(n:Label)` or `()-[r:TYPE]-()`. Returns (is_relationship, label).
    fn parse_ddl_target(&mut self) -> PResult<(bool, String)> {
        if self.at_punct("(") && self.peek_at(1) == &Tok::Punct(")".into()) {
            // ()-[r:TYPE]-()
            let pat = self.parse_pattern()?;
            for elem in &pat.elems {
                if let PatElem::Rel(rp) = elem {
                    if let Some(t) = rp.types.first() {
                        return Ok((true, t.clone()));
                    }
                }
            }
            return Err(self.err("expected a relationship type in the index target"));
        }
        self.expect_punct("(")?;
        let _var = self.name()?;
        self.expect_punct(":")?;
        let label = self.name()?;
        self.expect_punct(")")?;
        Ok((false, label))
    }

    /// The property list of an index or constraint. Cypher writes it three
    /// ways and all three mean the same thing:
    ///   `(n.a, n.b)`        — index, and multi-property constraints
    ///   `EACH [n.a, n.b]`   — full-text
    ///   `n.a`               — a single-property constraint, no brackets
    fn parse_ddl_props(&mut self) -> PResult<Vec<String>> {
        let each = self.eat_word("each");
        let close = if each || self.at_punct("[") {
            self.expect_punct("[")?;
            Some("]")
        } else if self.at_punct("(") {
            self.bump();
            Some(")")
        } else {
            None
        };
        let mut props = Vec::new();
        loop {
            let _var = self.name()?;
            self.expect_punct(".")?;
            props.push(self.name()?);
            if close.is_none() || !self.eat_punct(",") {
                break;
            }
        }
        if let Some(c) = close {
            self.expect_punct(c)?;
        }
        Ok(props)
    }

    /// `OPTIONS {indexConfig: {`vector.dimensions`: 1536, …}}` — flattened, so
    /// the caller reads `vector.dimensions` whatever nesting it arrived in.
    fn parse_ddl_options(&mut self) -> PResult<Vec<(String, Expr)>> {
        let kv = self.parse_prop_map()?;
        let mut out = Vec::new();
        fn flatten(kv: Vec<(String, Expr)>, out: &mut Vec<(String, Expr)>) {
            for (k, v) in kv {
                match v {
                    Expr::Map(inner) => flatten(inner, out),
                    other => out.push((k.to_ascii_lowercase(), other)),
                }
            }
        }
        flatten(kv, &mut out);
        Ok(out)
    }

    fn parse_drop(&mut self) -> PResult<Clause> {
        self.expect_kw("drop")?;
        let constraint = if self.eat_word("constraint") {
            true
        } else {
            self.expect_word("index")?;
            false
        };
        let name = self.name()?;
        let if_exists = self.eat_if_exists(false);
        Ok(Clause::Ddl(Ddl::Drop { name, if_exists, constraint }))
    }

    fn parse_create(&mut self) -> PResult<Clause> {
        if self.create_is_ddl() {
            return self.parse_ddl_create();
        }
        self.expect_kw("create")?;
        let mut pats = vec![self.parse_pattern()?];
        while self.eat_punct(",") {
            pats.push(self.parse_pattern()?);
        }
        Ok(Clause::Create(pats))
    }

    fn parse_merge(&mut self) -> PResult<Clause> {
        self.expect_kw("merge")?;
        let pattern = self.parse_pattern()?;
        let mut on_create = Vec::new();
        let mut on_match = Vec::new();
        while self.at_kw("on") {
            self.bump();
            if self.eat_kw("create") {
                self.expect_kw("set")?;
                on_create.extend(self.parse_set_items()?);
            } else if self.eat_kw("match") {
                self.expect_kw("set")?;
                on_match.extend(self.parse_set_items()?);
            } else {
                return Err(self.err("expected ON CREATE or ON MATCH"));
            }
        }
        Ok(Clause::Merge { pattern, on_create, on_match })
    }

    fn parse_set(&mut self) -> PResult<Clause> {
        self.expect_kw("set")?;
        Ok(Clause::Set(self.parse_set_items()?))
    }

    fn parse_remove(&mut self) -> PResult<Clause> {
        self.expect_kw("remove")?;
        let mut items = Vec::new();
        loop {
            let var = self.name()?;
            if self.eat_punct(".") {
                let prop = self.name()?;
                items.push(SetOp::Prop { var, prop, value: Expr::Lit(Lit::Null) });
            } else if self.at_punct(":") {
                let mut labels = Vec::new();
                while self.eat_punct(":") {
                    labels.push(self.name()?);
                }
                items.push(SetOp::Label { var, labels });
            } else {
                return Err(self.err("REMOVE expects a property or a label"));
            }
            if !self.eat_punct(",") {
                break;
            }
        }
        Ok(Clause::Remove(items))
    }

    fn parse_set_items(&mut self) -> PResult<Vec<SetOp>> {
        let mut items = Vec::new();
        loop {
            let var = self.name()?;
            if self.eat_punct(".") {
                let prop = self.name()?;
                self.expect_punct("=")?;
                items.push(SetOp::Prop { var, prop, value: self.parse_expr()? });
            } else if self.at_punct(":") {
                let mut labels = Vec::new();
                while self.eat_punct(":") {
                    labels.push(self.name()?);
                }
                items.push(SetOp::Label { var, labels });
            } else if self.eat_punct("+=") {
                items.push(SetOp::Map { var, value: self.parse_expr()?, merge: true });
            } else if self.eat_punct("=") {
                items.push(SetOp::Map { var, value: self.parse_expr()?, merge: false });
            } else {
                return Err(self.err("SET expects `var.prop = …`, `var = {…}`, `var += {…}` or `var:Label`"));
            }
            if !self.eat_punct(",") {
                break;
            }
        }
        Ok(items)
    }

    /// `FOREACH (x IN list | clause …)` — the clauses are writes.
    fn parse_foreach(&mut self) -> PResult<Clause> {
        self.bump(); // FOREACH — an identifier, not a reserved word
        self.expect_punct("(")?;
        let var = self.name()?;
        self.expect_kw("in")?;
        let list = self.parse_expr()?;
        self.expect_punct("|")?;
        let mut body = Vec::new();
        while !self.at_punct(")") {
            let c = match self.peek().clone() {
                Tok::Keyword(k) => match k.as_str() {
                    "create" => self.parse_create()?,
                    "merge" => self.parse_merge()?,
                    "set" => self.parse_set()?,
                    "remove" => self.parse_remove()?,
                    "delete" | "detach" => self.parse_delete()?,
                    other => {
                        return Err(self.err(&format!(
                            "FOREACH may only hold writes (CREATE, MERGE, SET, REMOVE, DELETE, FOREACH), not {}",
                            other.to_uppercase()
                        )))
                    }
                },
                Tok::Ident(s) if s.eq_ignore_ascii_case("foreach") => self.parse_foreach()?,
                _ => return Err(self.err("expected a write clause or ')' in FOREACH")),
            };
            if matches!(c, Clause::Ddl(_)) {
                return Err(self.err("a schema command cannot be part of FOREACH"));
            }
            body.push(c);
        }
        self.expect_punct(")")?;
        if body.is_empty() {
            return Err(self.err("FOREACH needs at least one clause after '|'"));
        }
        Ok(Clause::Foreach { var, list, body })
    }

    fn parse_delete(&mut self) -> PResult<Clause> {
        let detach = self.eat_kw("detach");
        self.expect_kw("delete")?;
        let mut exprs = vec![self.parse_expr()?];
        while self.eat_punct(",") {
            exprs.push(self.parse_expr()?);
        }
        Ok(Clause::Delete { exprs, detach })
    }

    // ----------------------------------------------------------------------
    // Patterns
    // ----------------------------------------------------------------------

    fn parse_pattern(&mut self) -> PResult<Pattern> {
        // `p = (a)-[r]->(b)`
        let path_var = if matches!(self.peek(), Tok::Ident(_)) && self.peek_at(1) == &Tok::Punct("=".into())
        {
            let v = self.name()?;
            self.bump();
            Some(v)
        } else {
            None
        };

        let mut elems = vec![PatElem::Node(self.parse_node_pat()?)];
        loop {
            if self.at_punct("-") || self.at_punct("<-") || self.at_punct("<") {
                let rel = self.parse_rel_pat()?;
                let node = self.parse_node_pat()?;
                elems.push(PatElem::Rel(rel));
                elems.push(PatElem::Node(node));
            } else {
                break;
            }
        }
        Ok(Pattern { path_var, elems })
    }

    fn parse_node_pat(&mut self) -> PResult<NodePat> {
        self.expect_punct("(")?;
        let var = if matches!(self.peek(), Tok::Ident(_) | Tok::QuotedIdent(_)) {
            Some(self.name()?)
        } else {
            None
        };
        let mut labels = Vec::new();
        // `:A:B` — all of them — and `:A|B` — any of them: one group per `:`.
        let mut label_groups: Vec<Vec<String>> = Vec::new();
        while self.eat_punct(":") {
            let first = self.name()?;
            labels.push(first.clone());
            let mut group = vec![first];
            // `:A|B` multi-label alternatives
            while self.eat_punct("|") {
                let alt = self.name()?;
                labels.push(alt.clone());
                group.push(alt);
            }
            label_groups.push(group);
        }
        let props = if self.at_punct("{") { self.parse_prop_map()? } else { Vec::new() };
        self.expect_punct(")")?;
        Ok(NodePat { var, labels, label_groups, props })
    }

    fn parse_rel_pat(&mut self) -> PResult<RelPat> {
        let mut dir = Dir::Both;
        if self.eat_punct("<-") {
            dir = Dir::In;
        } else if self.eat_punct("-") {
            // direction decided by the trailing arrow
        } else {
            return Err(self.err("expected a relationship pattern"));
        }

        let mut var = None;
        let mut types = Vec::new();
        let mut props = Vec::new();
        let mut range = None;

        if self.eat_punct("[") {
            if matches!(self.peek(), Tok::Ident(_) | Tok::QuotedIdent(_)) {
                var = Some(self.name()?);
            }
            if self.eat_punct(":") {
                types.push(self.name()?);
                while self.eat_punct("|") {
                    self.eat_punct(":");
                    types.push(self.name()?);
                }
            }
            if self.eat_punct("*") {
                let min = if let Tok::Int(n) = self.peek().clone() {
                    self.bump();
                    n as u32
                } else {
                    1
                };
                let max = if self.eat_punct("..") {
                    if let Tok::Int(n) = self.peek().clone() {
                        self.bump();
                        n as u32
                    } else {
                        super::MAX_VAR_LENGTH
                    }
                } else if min > 0 && !self.at_punct("]") {
                    min
                } else {
                    min
                };
                range = Some((min, max.max(min)));
            }
            if self.at_punct("{") {
                props = self.parse_prop_map()?;
            }
            self.expect_punct("]")?;
        }

        if self.eat_punct("->") {
            dir = if dir == Dir::In { Dir::Both } else { Dir::Out };
        } else if self.eat_punct("-") {
            // undirected tail; keep In if we opened with '<-'
            if dir != Dir::In {
                dir = Dir::Both;
            }
        } else {
            return Err(self.err("expected '->' or '-' to close the relationship pattern"));
        }

        Ok(RelPat { var, types, dir, props, range })
    }

    fn parse_prop_map(&mut self) -> PResult<Vec<(String, Expr)>> {
        self.expect_punct("{")?;
        let mut out = Vec::new();
        if self.eat_punct("}") {
            return Ok(out);
        }
        loop {
            let k = self.name()?;
            self.expect_punct(":")?;
            out.push((k, self.parse_expr()?));
            if !self.eat_punct(",") {
                break;
            }
        }
        self.expect_punct("}")?;
        Ok(out)
    }

    // ----------------------------------------------------------------------
    // Expressions
    // ----------------------------------------------------------------------

    pub fn parse_expr(&mut self) -> PResult<Expr> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_xor()?;
        while self.eat_kw("or") {
            lhs = Expr::Binary(BinOp::Or, Box::new(lhs), Box::new(self.parse_xor()?));
        }
        Ok(lhs)
    }

    fn parse_xor(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_and()?;
        while self.eat_kw("xor") {
            lhs = Expr::Binary(BinOp::Xor, Box::new(lhs), Box::new(self.parse_and()?));
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_not()?;
        while self.eat_kw("and") {
            lhs = Expr::Binary(BinOp::And, Box::new(lhs), Box::new(self.parse_not()?));
        }
        Ok(lhs)
    }

    fn parse_not(&mut self) -> PResult<Expr> {
        if self.eat_kw("not") {
            Ok(Expr::Not(Box::new(self.parse_not()?)))
        } else {
            self.parse_comparison()
        }
    }

    fn parse_comparison(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_additive()?;
        loop {
            let op = if self.at_punct("=") {
                BinOp::Eq
            } else if self.at_punct("<>") || self.at_punct("!=") {
                BinOp::Ne
            } else if self.at_punct("<=") {
                BinOp::Le
            } else if self.at_punct(">=") {
                BinOp::Ge
            } else if self.at_punct("<") {
                BinOp::Lt
            } else if self.at_punct(">") {
                BinOp::Gt
            } else if self.at_punct("=~") {
                BinOp::Regex
            } else if self.at_kw("in") {
                BinOp::In
            } else if self.at_kw("starts") {
                BinOp::StartsWith
            } else if self.at_kw("ends") {
                BinOp::EndsWith
            } else if self.at_kw("contains") {
                BinOp::Contains
            } else if self.at_kw("is") {
                self.bump();
                let negated = self.eat_kw("not");
                self.expect_kw("null")?;
                lhs = Expr::IsNull(Box::new(lhs), !negated);
                continue;
            } else {
                break;
            };
            self.bump();
            if matches!(op, BinOp::StartsWith | BinOp::EndsWith) {
                self.expect_kw("with")?;
            }
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(self.parse_additive()?));
        }
        Ok(lhs)
    }

    fn parse_additive(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_multiplicative()?;
        loop {
            let op = if self.at_punct("+") {
                BinOp::Add
            } else if self.at_punct("-") {
                BinOp::Sub
            } else if self.at_punct("||") {
                BinOp::Concat
            } else {
                break;
            };
            self.bump();
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(self.parse_multiplicative()?));
        }
        Ok(lhs)
    }

    fn parse_multiplicative(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_power()?;
        loop {
            let op = if self.at_punct("*") {
                BinOp::Mul
            } else if self.at_punct("/") {
                BinOp::Div
            } else if self.at_punct("%") {
                BinOp::Mod
            } else {
                break;
            };
            self.bump();
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(self.parse_power()?));
        }
        Ok(lhs)
    }

    fn parse_power(&mut self) -> PResult<Expr> {
        let lhs = self.parse_unary()?;
        if self.at_punct("^") {
            self.bump();
            return Ok(Expr::Binary(BinOp::Pow, Box::new(lhs), Box::new(self.parse_power()?)));
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> PResult<Expr> {
        if self.at_punct("-") {
            self.bump();
            return Ok(Expr::Neg(Box::new(self.parse_unary()?)));
        }
        if self.at_punct("+") {
            self.bump();
            return self.parse_unary();
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> PResult<Expr> {
        let mut e = self.parse_primary()?;
        loop {
            if self.at_punct(".") {
                self.bump();
                let p = self.name()?;
                e = Expr::Prop(Box::new(e), p);
            } else if self.at_punct("[") {
                self.bump();
                // `xs[i]` is one element; `xs[a..b]`, `xs[..b]`, `xs[a..]` a sub-list.
                let from = if self.at_punct("..") { None } else { Some(self.parse_expr()?) };
                if self.eat_punct("..") {
                    let to = if self.at_punct("]") { None } else { Some(Box::new(self.parse_expr()?)) };
                    self.expect_punct("]")?;
                    e = Expr::Slice { list: Box::new(e), from: from.map(Box::new), to };
                } else {
                    self.expect_punct("]")?;
                    let idx = from.expect("an index was parsed");
                    e = Expr::Func { name: "element_at".into(), args: vec![e, idx], distinct: false };
                }
            } else if self.at_punct(":") && matches!(e, Expr::Var(_)) && self.name_at(1) {
                // `n:A` in an expression is a label predicate: `WHERE n:A OR n:B`.
                // Nothing else can follow a variable with `:` here — map keys and
                // pattern labels are read by their own rules, never as expressions.
                let Expr::Var(var) = e else { unreachable!() };
                e = Expr::HasLabels { var, alts: self.parse_label_alternatives()? };
            } else {
                break;
            }
        }
        Ok(e)
    }

    /// Is the token `n` ahead something `name()` accepts?
    fn name_at(&self, n: usize) -> bool {
        matches!(self.peek_at(n), Tok::Ident(_) | Tok::QuotedIdent(_) | Tok::Keyword(_))
    }

    /// `:A`, `:A:B` / `:A&B` (all of them), `:A|B` / `:A|:B` (any of them).
    /// `&` binds tighter than `|`, as in Cypher's label expressions.
    fn parse_label_alternatives(&mut self) -> PResult<Vec<Vec<String>>> {
        self.expect_punct(":")?;
        let mut alts = vec![vec![self.name()?]];
        loop {
            if (self.at_punct(":") || self.at_punct("&")) && self.name_at(1) {
                self.bump();
                let n = self.name()?;
                alts.last_mut().expect("one alternative at least").push(n);
            } else if self.in_comprehension_filter == 0
                && self.at_punct("|")
                && (self.name_at(1) || (self.peek_at(1) == &Tok::Punct(":".into()) && self.name_at(2)))
            {
                self.bump();
                self.eat_punct(":");
                alts.push(vec![self.name()?]);
            } else {
                break;
            }
        }
        Ok(alts)
    }

    fn parse_primary(&mut self) -> PResult<Expr> {
        match self.peek().clone() {
            Tok::Int(n) => {
                self.bump();
                Ok(Expr::Lit(Lit::Int(n)))
            }
            Tok::Float(f) => {
                self.bump();
                Ok(Expr::Lit(Lit::Float(f)))
            }
            Tok::Str(s) => {
                self.bump();
                Ok(Expr::Lit(Lit::Str(s)))
            }
            Tok::Param(p) => {
                self.bump();
                Ok(Expr::Param(p))
            }
            Tok::Keyword(k) if k == "true" => {
                self.bump();
                Ok(Expr::Lit(Lit::Bool(true)))
            }
            Tok::Keyword(k) if k == "false" => {
                self.bump();
                Ok(Expr::Lit(Lit::Bool(false)))
            }
            Tok::Keyword(k) if k == "null" => {
                self.bump();
                Ok(Expr::Lit(Lit::Null))
            }
            Tok::Keyword(k) if k == "case" => self.parse_case(),
            // COUNT and EXISTS are lexed as keywords but behave as functions —
            // *when called*. Without the parenthesis they are an ordinary name,
            // which is how `RETURN count(*) AS count ORDER BY count DESC` gets
            // to refer back to its own projection.
            Tok::Keyword(k) if (k == "count" || k == "exists") => {
                let name = k.clone();
                self.bump();
                if self.at_punct("{") {
                    let kind = if name == "count" { SubqueryKind::Count } else { SubqueryKind::Exists };
                    return self.parse_pattern_subquery(kind);
                }
                if name == "exists" && self.at_punct("(") {
                    // `exists((a)-[:R]->())`: the Neo4j 4 spelling of a pattern predicate.
                    let save = self.i;
                    self.bump();
                    if let Some(pat) = self.try_relationship_pattern() {
                        if self.eat_punct(")") {
                            return Ok(Expr::Subquery {
                                kind: SubqueryKind::Exists,
                                patterns: vec![pat],
                                where_: None,
                                project: None,
                            });
                        }
                    }
                    self.i = save;
                }
                if self.at_punct("(") {
                    self.parse_call_args(&name)
                } else {
                    Ok(Expr::Var(name))
                }
            }
            Tok::Punct(p) if p == "(" => {
                // `(a)-[:R]->(:B)` in an expression is a pattern predicate; a
                // parenthesised expression otherwise.
                if let Some(pat) = self.try_relationship_pattern() {
                    return Ok(Expr::Subquery {
                        kind: SubqueryKind::Exists,
                        patterns: vec![pat],
                        where_: None,
                        project: None,
                    });
                }
                self.bump();
                let e = self.parse_expr()?;
                self.expect_punct(")")?;
                Ok(e)
            }
            Tok::Punct(p) if p == "[" => {
                self.bump();
                // `[(a)-[:R]->(b) WHERE … | b.name]` — a pattern comprehension.
                if self.at_punct("(") {
                    let save = self.i;
                    if let Some(pat) = self.try_relationship_pattern() {
                        if self.at_kw("where") || self.at_punct("|") {
                            let where_ = if self.eat_kw("where") {
                                self.in_comprehension_filter += 1;
                                let f = self.parse_expr();
                                self.in_comprehension_filter -= 1;
                                Some(Box::new(f?))
                            } else {
                                None
                            };
                            let project = if self.eat_punct("|") { Some(Box::new(self.parse_expr()?)) } else { None };
                            self.expect_punct("]")?;
                            return Ok(Expr::Subquery {
                                kind: SubqueryKind::Collect,
                                patterns: vec![pat],
                                where_,
                                project,
                            });
                        }
                    }
                    self.i = save;
                }
                // `[x IN xs …]` is a comprehension, `[a, b, c]` a literal list.
                // One token of lookahead separates them: a bare name followed by
                // IN can only be the comprehension's binder.
                if matches!(self.peek(), Tok::Ident(_)) && self.peek_at(1) == &Tok::Keyword("in".into())
                {
                    let var = self.name()?;
                    self.expect_kw("in")?;
                    let source = Box::new(self.parse_expr()?);
                    let filter = if self.eat_kw("where") {
                        self.in_comprehension_filter += 1;
                        let f = self.parse_expr();
                        self.in_comprehension_filter -= 1;
                        Some(Box::new(f?))
                    } else {
                        None
                    };
                    let project = if self.eat_punct("|") {
                        Some(Box::new(self.parse_expr()?))
                    } else {
                        None
                    };
                    self.expect_punct("]")?;
                    return Ok(Expr::ListComp { var, source, filter, project });
                }
                let mut items = Vec::new();
                if !self.at_punct("]") {
                    loop {
                        items.push(self.parse_expr()?);
                        if !self.eat_punct(",") {
                            break;
                        }
                    }
                }
                self.expect_punct("]")?;
                Ok(Expr::List(items))
            }
            Tok::Punct(p) if p == "{" => {
                let kv = self.parse_prop_map()?;
                Ok(Expr::Map(kv))
            }
            // any/all/none/single take a binder, not an argument list:
            // `any(x IN xs WHERE p)`. `all` is a keyword, the rest are not.
            Tok::Keyword(k) if k == "all" && self.list_pred_ahead() => {
                self.bump();
                self.parse_list_pred(ListPredKind::All)
            }
            // `reduce(acc = init, x IN list | expr)`.
            Tok::Ident(name)
                if name.eq_ignore_ascii_case("reduce")
                    && self.peek_at(1) == &Tok::Punct("(".into())
                    && matches!(self.peek_at(2), Tok::Ident(_))
                    && self.peek_at(3) == &Tok::Punct("=".into()) =>
            {
                self.bump();
                self.expect_punct("(")?;
                let acc = self.name()?;
                self.expect_punct("=")?;
                let init = Box::new(self.parse_expr()?);
                self.expect_punct(",")?;
                let var = self.name()?;
                self.expect_kw("in")?;
                let list = Box::new(self.parse_expr()?);
                self.expect_punct("|")?;
                let expr = Box::new(self.parse_expr()?);
                self.expect_punct(")")?;
                Ok(Expr::Reduce { acc, init, var, list, expr })
            }
            Tok::Ident(name)
                if matches!(name.to_ascii_lowercase().as_str(), "any" | "none" | "single")
                    && self.list_pred_ahead() =>
            {
                let kind = match name.to_ascii_lowercase().as_str() {
                    "any" => ListPredKind::Any,
                    "none" => ListPredKind::None,
                    _ => ListPredKind::Single,
                };
                self.bump();
                self.parse_list_pred(kind)
            }
            Tok::Ident(name) if name.eq_ignore_ascii_case("collect") && self.peek_at(1) == &Tok::Punct("{".into()) => {
                self.bump();
                self.parse_pattern_subquery(SubqueryKind::Collect)
            }
            Tok::Ident(name) => {
                self.bump();
                // Namespaced function: `vector.similarity(…)` — and namespaces
                // nest, so `genai.vector.encode(…)` is one name rather than a
                // property of a property of `genai`.
                //
                // Which one it is can only be known at the end of the run: a
                // chain of `.name` is a function name if it arrives at `(`, and
                // a property path otherwise. So the chain is measured first and
                // consumed only once that is settled.
                let mut full = name.clone();
                let mut segments = 0usize;
                loop {
                    let at = segments * 2;
                    let is_segment = self.peek_at(at) == &Tok::Punct(".".into())
                        && matches!(self.peek_at(at + 1), Tok::Ident(_));
                    if !is_segment || segments >= MAX_NAMESPACE_DEPTH {
                        segments = 0;
                        break;
                    }
                    segments += 1;
                    if self.peek_at(segments * 2) == &Tok::Punct("(".into()) {
                        break;
                    }
                }
                for _ in 0..segments {
                    self.bump();
                    full.push('.');
                    full.push_str(&self.name()?);
                }
                if self.at_punct("(") {
                    self.parse_call_args(&full)
                } else if self.at_punct("{") {
                    // `ds { .name }` in an expression is a map projection. The
                    // same spelling inside a pattern is an inline property
                    // filter, but patterns are parsed elsewhere, so there is no
                    // ambiguity to resolve here.
                    self.parse_map_projection(full)
                } else {
                    Ok(Expr::Var(full))
                }
            }
            _ => Err(self.err("expected an expression")),
        }
    }

    /// After `CALL`, is `( … )` a scope clause followed by `{` — rather than
    /// the start of a procedure name? (A procedure name never starts with `(`,
    /// so only the shape has to be checked.)
    fn scope_clause_ahead(&self) -> bool {
        let mut n = 1;
        loop {
            match self.peek_at(n) {
                Tok::Punct(p) if p == ")" => return self.peek_at(n + 1) == &Tok::Punct("{".into()),
                Tok::Punct(p) if p == "," || p == "*" => {}
                Tok::Ident(_) | Tok::QuotedIdent(_) | Tok::Keyword(_) => {}
                _ => return false,
            }
            n += 1;
            if n > 256 {
                return false;
            }
        }
    }

    /// A pattern with at least one relationship, if one starts here; the
    /// position is left where it was when none does.
    fn try_relationship_pattern(&mut self) -> Option<Pattern> {
        let save = self.i;
        match self.parse_pattern() {
            Ok(p) if p.path_var.is_none() && p.elems.len() > 1 => Some(p),
            _ => {
                self.i = save;
                None
            }
        }
    }

    /// `{ [MATCH] pattern, … [WHERE …] }` after EXISTS or COUNT.
    fn parse_pattern_subquery(&mut self, kind: SubqueryKind) -> PResult<Expr> {
        self.expect_punct("{")?;
        self.eat_kw("match");
        let mut patterns = vec![self.parse_pattern()?];
        while self.eat_punct(",") {
            patterns.push(self.parse_pattern()?);
        }
        let where_ = if self.eat_kw("where") { Some(Box::new(self.parse_expr()?)) } else { None };
        // A RETURN is what COLLECT { … } collects; in EXISTS / COUNT it does
        // not change the answer.
        let project = if self.eat_kw("return") { Some(Box::new(self.parse_expr()?)) } else { None };
        if !self.at_punct("}") {
            return Err(self.err(
                "only `{ [MATCH] pattern [WHERE …] [RETURN expr] }` is supported inside \
                 EXISTS / COUNT / COLLECT — not WITH, UNION or further clauses",
            ));
        }
        self.bump();
        if kind == SubqueryKind::Collect && project.is_none() {
            return Err(self.err("COLLECT { … } needs a RETURN"));
        }
        let project = if kind == SubqueryKind::Collect { project } else { None };
        Ok(Expr::Subquery { kind, patterns, where_, project })
    }

    /// Does the call ahead look like `f(x IN …)` rather than `f(expr, …)`?
    fn list_pred_ahead(&self) -> bool {
        self.peek_at(1) == &Tok::Punct("(".into())
            && matches!(self.peek_at(2), Tok::Ident(_))
            && self.peek_at(3) == &Tok::Keyword("in".into())
    }

    fn parse_list_pred(&mut self, kind: ListPredKind) -> PResult<Expr> {
        self.expect_punct("(")?;
        let var = self.name()?;
        self.expect_kw("in")?;
        let source = Box::new(self.parse_expr()?);
        self.expect_kw("where")?;
        let filter = Box::new(self.parse_expr()?);
        self.expect_punct(")")?;
        Ok(Expr::ListPred { kind, var, source, filter })
    }

    fn parse_map_projection(&mut self, var: String) -> PResult<Expr> {
        self.expect_punct("{")?;
        let mut items = Vec::new();
        if !self.at_punct("}") {
            loop {
                if self.eat_punct(".") {
                    if self.eat_punct("*") {
                        items.push(MapProjItem::All);
                    } else {
                        items.push(MapProjItem::Prop(self.name()?));
                    }
                } else {
                    let key = self.name()?;
                    self.expect_punct(":")?;
                    items.push(MapProjItem::Entry(key, self.parse_expr()?));
                }
                if !self.eat_punct(",") {
                    break;
                }
            }
        }
        self.expect_punct("}")?;
        Ok(Expr::MapProjection { var, items })
    }

    fn parse_call_args(&mut self, name: &str) -> PResult<Expr> {
        self.expect_punct("(")?;
        let distinct = self.eat_kw("distinct");
        let mut args = Vec::new();
        if self.at_punct("*") {
            self.bump();
            args.push(Expr::Star);
        } else if !self.at_punct(")") {
            loop {
                args.push(self.parse_expr()?);
                if !self.eat_punct(",") {
                    break;
                }
            }
        }
        self.expect_punct(")")?;
        Ok(Expr::Func { name: name.to_string(), args, distinct })
    }

    fn parse_case(&mut self) -> PResult<Expr> {
        self.expect_kw("case")?;
        let operand = if !self.at_kw("when") { Some(Box::new(self.parse_expr()?)) } else { None };
        let mut whens = Vec::new();
        while self.eat_kw("when") {
            let cond = self.parse_expr()?;
            self.expect_kw("then")?;
            whens.push((cond, self.parse_expr()?));
        }
        let else_ = if self.eat_kw("else") { Some(Box::new(self.parse_expr()?)) } else { None };
        self.expect_kw("end")?;
        Ok(Expr::Case { operand, whens, else_ })
    }
}


/// Columns `SHOW …` yields, in Neo4j's order and spelling. Kept next to the
/// parser because the bare form has to project them without a `YIELD`.
fn show_columns(which: &str) -> &'static [&'static str] {
    match which {
        "show.constraints" => &["id", "name", "type", "entityType", "labelsOrTypes", "properties"],
        _ => &["id", "name", "state", "type", "entityType", "labelsOrTypes", "properties"],
    }
}

/// A projection that passes the yielded columns straight through.
fn projection_of(yields: &[(String, String)]) -> Projection {
    Projection {
        items: yields
            .iter()
            .map(|(_, alias)| ReturnItem {
                expr: Expr::Var(alias.clone()),
                alias: Some(alias.clone()),
            })
            .collect(),
        distinct: false,
        order: Vec::new(),
        skip: None,
        limit: None,
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_match_return() {
        let q = parse("MATCH (a:Person)-[:KNOWS]->(b) WHERE a.age > 30 RETURN b.name AS n").unwrap();
        assert_eq!(q.clauses.len(), 2);
    }

    #[test]
    fn var_length() {
        let q = parse("MATCH (a)-[:R*1..3]->(b) RETURN b").unwrap();
        let Clause::Match { patterns, .. } = &q.clauses[0] else { panic!() };
        let PatElem::Rel(r) = &patterns[0].elems[1] else { panic!() };
        assert_eq!(r.range, Some((1, 3)));
    }

    #[test]
    fn direction_parsing() {
        let q = parse("MATCH (a)<-[:R]-(b) RETURN a").unwrap();
        let Clause::Match { patterns, .. } = &q.clauses[0] else { panic!() };
        let PatElem::Rel(r) = &patterns[0].elems[1] else { panic!() };
        assert_eq!(r.dir, Dir::In);
    }

    #[test]
    fn aggregates_detected() {
        let q = parse("MATCH (a) RETURN count(a), a.x").unwrap();
        let Clause::Return(p) = &q.clauses[1] else { panic!() };
        assert!(p.items[0].expr.is_aggregate());
        assert!(!p.items[1].expr.is_aggregate());
    }

    #[test]
    fn rejects_unknown_clause_loudly() {
        let e = parse("FROBNICATE (x)").unwrap_err();
        assert!(e.contains("unexpected") || e.contains("expected"));
        let q = parse("MATCH (n) FOREACH (x IN [1, 2] | SET n.y = x CREATE (:T {v: x}))").unwrap();
        assert!(matches!(&q.clauses[1], Clause::Foreach { body, .. } if body.len() == 2));
        assert!(parse("FOREACH (x IN [1] | MATCH (n) SET n.y = 1)").is_err());
    }

    #[test]
    fn call_subqueries() {
        let q = parse("MATCH (p) CALL { WITH p MATCH (p)-->(q) RETURN count(q) AS n } RETURN p, n").unwrap();
        assert!(matches!(&q.clauses[1], Clause::Subquery { scope: None, optional: false, body } if body.clauses.len() == 3));
        let q = parse("MATCH (p) CALL (p) { MATCH (p)-->(q) RETURN q } RETURN q").unwrap();
        assert!(matches!(&q.clauses[1], Clause::Subquery { scope: Some(s), .. } if s == &vec!["p".to_string()]));
        let q = parse("MATCH (p) OPTIONAL CALL (*) { RETURN 1 AS x UNION RETURN 2 AS x } RETURN x").unwrap();
        assert!(matches!(&q.clauses[1], Clause::Subquery { optional: true, body, .. } if body.union.is_some()));
        // A map literal inside does not end the body; a procedure call is still a procedure call.
        parse("CALL { WITH 1 AS a RETURN {k: a} AS m } RETURN m").unwrap();
        assert!(matches!(&parse("CALL db.labels() YIELD label RETURN label").unwrap().clauses[0], Clause::Call { .. }));
        let e = parse("UNWIND [1] AS i CALL { WITH i CREATE (:T) } IN TRANSACTIONS OF 10 ROWS").unwrap_err();
        assert!(e.contains("IN TRANSACTIONS is not supported"), "{e}");
        assert!(parse("OPTIONAL CALL db.labels()").is_err());
    }

    #[test]
    fn string_predicates() {
        parse("MATCH (a) WHERE a.name STARTS WITH 'x' AND a.b CONTAINS 'y' RETURN a").unwrap();
    }

    fn return_expr(q: &str) -> Expr {
        let q = parse(q).unwrap();
        let Some(Clause::Return(p)) = q.clauses.last() else { panic!() };
        p.items[0].expr.clone()
    }

    #[test]
    fn slices_and_indexes() {
        let Expr::Slice { from, to, .. } = return_expr("RETURN [1,2,3][1..2] AS x") else { panic!() };
        assert!(from.is_some() && to.is_some());
        let Expr::Slice { from, to, .. } = return_expr("RETURN [1,2,3][..2] AS x") else { panic!() };
        assert!(from.is_none() && to.is_some());
        let Expr::Slice { from, to, .. } = return_expr("RETURN [1,2,3][1..] AS x") else { panic!() };
        assert!(from.is_some() && to.is_none());
        assert!(matches!(return_expr("RETURN [1,2,3][1] AS x"), Expr::Func { ref name, .. } if name == "element_at"));
        // A slice of an aggregate still groups.
        assert!(return_expr("MATCH (a) RETURN collect(a.x)[0..3]").is_aggregate());
    }

    #[test]
    fn label_predicates() {
        let q = parse("MATCH (n) WHERE n:A OR NOT n:B:C AND n:D|E RETURN n:A AS isA").unwrap();
        let Clause::Match { where_: Some(w), .. } = &q.clauses[0] else { panic!() };
        let Expr::Binary(BinOp::Or, l, r) = w else { panic!("{w:?}") };
        assert!(matches!(&**l, Expr::HasLabels { alts, .. } if alts == &vec![vec!["A".to_string()]]));
        let Expr::Binary(BinOp::And, nb, de) = &**r else { panic!() };
        assert!(matches!(&**nb, Expr::Not(x) if matches!(&**x, Expr::HasLabels { alts, .. }
            if alts == &vec![vec!["B".to_string(), "C".to_string()]])));
        assert!(matches!(&**de, Expr::HasLabels { alts, .. } if alts.len() == 2));
        assert!(matches!(return_expr("MATCH (n) RETURN n:A AS isA"), Expr::HasLabels { .. }));
        // In a comprehension's WHERE, `|` is the projection.
        let Expr::ListComp { filter: Some(f), project: Some(_), .. } =
            return_expr("WITH [1] AS xs RETURN [x IN xs WHERE x:A | x.name] AS n") else { panic!() };
        assert!(matches!(&*f, Expr::HasLabels { alts, .. } if alts.len() == 1));
        // A map value is not a label predicate.
        parse("RETURN {a: 1, b: 'x'} AS m").unwrap();
    }

    #[test]
    fn node_labels_keep_all_apart_from_any() {
        let node = |q: &str| {
            let q = parse(q).unwrap();
            let Clause::Match { patterns, .. } = &q.clauses[0] else { panic!() };
            let PatElem::Node(n) = &patterns[0].elems[0] else { panic!() };
            (n.labels.clone(), n.label_groups.clone())
        };
        let s = |xs: &[&[&str]]| xs.iter().map(|g| g.iter().map(|l| l.to_string()).collect::<Vec<_>>()).collect::<Vec<_>>();
        // `labels` stays the flat list the engine has always read.
        assert_eq!(node("MATCH (n:A:B) RETURN n"), (vec!["A".into(), "B".into()], s(&[&["A"], &["B"]])));
        assert_eq!(node("MATCH (n:A|B) RETURN n"), (vec!["A".into(), "B".into()], s(&[&["A", "B"]])));
        assert_eq!(node("MATCH (n:A|B:C) RETURN n").1, s(&[&["A", "B"], &["C"]]));
        assert_eq!(node("MATCH (n {k: 1}) RETURN n"), (vec![], vec![]));
    }
}
