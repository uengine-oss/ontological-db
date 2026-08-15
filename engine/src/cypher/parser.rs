//! Recursive-descent Cypher parser — spec 003 FR-001..FR-008.
//!
//! Unsupported syntax fails loudly at parse time with the offending construct
//! named (FR-008): silently reinterpreting a query is worse than rejecting it.

use super::ast::*;
use super::lexer::{Lexer, Tok, Token};

pub struct Parser {
    toks: Vec<Token>,
    i: usize,
    src: String,
}

type PResult<T> = Result<T, String>;

pub fn parse(src: &str) -> PResult<Query> {
    let toks = Lexer::new(src).tokenize()?;
    let mut p = Parser { toks, i: 0, src: src.to_string() };
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
    fn name(&mut self) -> PResult<String> {
        match self.bump() {
            Tok::Ident(s) | Tok::QuotedIdent(s) => Ok(s),
            Tok::Keyword(s) => Ok(s),
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
                    "call" => {
                        return Err(self.err(
                            "CALL procedures are not supported yet; use the SQL function surface \
                             (og_*) for administrative operations",
                        ))
                    }
                    "union" => break,
                    other => {
                        return Err(self.err(&format!("unexpected clause '{}'", other.to_uppercase())))
                    }
                },
                Tok::Eof => break,
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

    fn parse_create(&mut self) -> PResult<Clause> {
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
        while self.eat_punct(":") {
            labels.push(self.name()?);
            // `:A|B` multi-label alternatives
            while self.eat_punct("|") {
                labels.push(self.name()?);
            }
        }
        let props = if self.at_punct("{") { self.parse_prop_map()? } else { Vec::new() };
        self.expect_punct(")")?;
        Ok(NodePat { var, labels, props })
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
                let idx = self.parse_expr()?;
                self.expect_punct("]")?;
                e = Expr::Func { name: "element_at".into(), args: vec![e, idx], distinct: false };
            } else {
                break;
            }
        }
        Ok(e)
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
            Tok::Keyword(k) if k == "count" => {
                // COUNT is lexed as a keyword but behaves as a function.
                self.bump();
                self.parse_call_args("count")
            }
            Tok::Keyword(k) if k == "exists" => {
                self.bump();
                self.parse_call_args("exists")
            }
            Tok::Punct(p) if p == "(" => {
                self.bump();
                let e = self.parse_expr()?;
                self.expect_punct(")")?;
                Ok(e)
            }
            Tok::Punct(p) if p == "[" => {
                self.bump();
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
            Tok::Ident(name) => {
                self.bump();
                // namespaced function: vector.similarity(...)
                let mut full = name.clone();
                while self.at_punct(".")
                    && matches!(self.peek_at(1), Tok::Ident(_))
                    && self.peek_at(2) == &Tok::Punct("(".into())
                {
                    self.bump();
                    full.push('.');
                    full.push_str(&self.name()?);
                }
                if self.at_punct("(") {
                    self.parse_call_args(&full)
                } else {
                    Ok(Expr::Var(full))
                }
            }
            _ => Err(self.err("expected an expression")),
        }
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
        let e = parse("FOREACH (x IN [1] | SET x.y = 1)").unwrap_err();
        assert!(e.contains("unexpected") || e.contains("expected"));
    }

    #[test]
    fn string_predicates() {
        parse("MATCH (a) WHERE a.name STARTS WITH 'x' AND a.b CONTAINS 'y' RETURN a").unwrap();
    }
}
