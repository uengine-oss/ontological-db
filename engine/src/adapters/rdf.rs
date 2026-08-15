//! RDF parsing, mapping and serialisation — spec 006.
//!
//! Deliberately a *subset*: N-Triples plus the Turtle constructs that appear in
//! real ontology files (prefixes, `a`, predicate/object lists, string and typed
//! literals, language tags). Anything else is preserved in the overflow table
//! and counted in the report rather than quietly discarded.

use crate::catalog::{labeling, types};
use pgrx::prelude::*;
use serde_json::{json, Value};
use std::collections::HashMap;

pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
pub const RDFS_SUBCLASS: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
pub const RDFS_DOMAIN: &str = "http://www.w3.org/2000/01/rdf-schema#domain";
pub const RDFS_RANGE: &str = "http://www.w3.org/2000/01/rdf-schema#range";
pub const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";
pub const OWL_CLASS: &str = "http://www.w3.org/2002/07/owl#Class";
pub const RDFS_CLASS: &str = "http://www.w3.org/2000/01/rdf-schema#Class";
pub const OWL_OBJECT_PROP: &str = "http://www.w3.org/2002/07/owl#ObjectProperty";
pub const OWL_DATA_PROP: &str = "http://www.w3.org/2002/07/owl#DatatypeProperty";
pub const OWL_TRANSITIVE: &str = "http://www.w3.org/2002/07/owl#TransitiveProperty";
pub const OWL_SYMMETRIC: &str = "http://www.w3.org/2002/07/owl#SymmetricProperty";
pub const OWL_FUNCTIONAL: &str = "http://www.w3.org/2002/07/owl#FunctionalProperty";
pub const OWL_INVERSE: &str = "http://www.w3.org/2002/07/owl#inverseOf";

#[derive(Debug, Clone, PartialEq)]
pub enum Term {
    Iri(String),
    Blank(String),
    Literal { value: String, datatype: Option<String>, lang: Option<String> },
}

impl Term {
    pub fn as_iri(&self) -> Option<&str> {
        match self {
            Term::Iri(s) => Some(s),
            _ => None,
        }
    }
    pub fn text(&self) -> String {
        match self {
            Term::Iri(s) => s.clone(),
            Term::Blank(s) => format!("_:{s}"),
            Term::Literal { value, .. } => value.clone(),
        }
    }
}

pub struct Triple {
    pub s: Term,
    pub p: Term,
    pub o: Term,
}

// --------------------------------------------------------------------------
// Parsing
// --------------------------------------------------------------------------

struct RdfParser<'a> {
    src: &'a [u8],
    pos: usize,
    prefixes: HashMap<String, String>,
    base: String,
}

pub fn parse(doc: &str) -> Result<(Vec<Triple>, HashMap<String, String>), String> {
    let mut p = RdfParser {
        src: doc.as_bytes(),
        pos: 0,
        prefixes: HashMap::new(),
        base: String::new(),
    };
    let triples = p.parse_document()?;
    Ok((triples, p.prefixes))
}

impl<'a> RdfParser<'a> {
    fn skip_ws(&mut self) {
        loop {
            match self.src.get(self.pos) {
                Some(c) if c.is_ascii_whitespace() => self.pos += 1,
                Some(b'#') => {
                    while let Some(c) = self.src.get(self.pos) {
                        self.pos += 1;
                        if *c == b'\n' {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
    }

    fn starts_with(&self, s: &str) -> bool {
        self.src[self.pos..].starts_with(s.as_bytes())
    }

    fn parse_document(&mut self) -> Result<Vec<Triple>, String> {
        let mut out = Vec::new();
        loop {
            self.skip_ws();
            if self.pos >= self.src.len() {
                return Ok(out);
            }
            if self.starts_with("@prefix") || self.starts_with("PREFIX") {
                self.parse_prefix()?;
                continue;
            }
            if self.starts_with("@base") || self.starts_with("BASE") {
                self.pos += 5;
                self.skip_ws();
                if let Term::Iri(i) = self.parse_term()? {
                    self.base = i;
                }
                self.skip_ws();
                if self.src.get(self.pos) == Some(&b'.') {
                    self.pos += 1;
                }
                continue;
            }
            self.parse_statement(&mut out)?;
        }
    }

    fn parse_prefix(&mut self) -> Result<(), String> {
        self.pos += 7;
        self.skip_ws();
        let start = self.pos;
        while self.src.get(self.pos).is_some_and(|c| *c != b':') {
            self.pos += 1;
        }
        let name = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
        self.pos += 1; // ':'
        self.skip_ws();
        let Term::Iri(iri) = self.parse_term()? else {
            return Err("@prefix expects an IRI".into());
        };
        self.prefixes.insert(name, iri);
        self.skip_ws();
        if self.src.get(self.pos) == Some(&b'.') {
            self.pos += 1;
        }
        Ok(())
    }

    fn parse_statement(&mut self, out: &mut Vec<Triple>) -> Result<(), String> {
        let subject = self.parse_term()?;
        loop {
            self.skip_ws();
            let predicate = if self.starts_with("a ") || self.starts_with("a\t") {
                self.pos += 1;
                Term::Iri(RDF_TYPE.into())
            } else {
                self.parse_term()?
            };
            loop {
                self.skip_ws();
                let object = self.parse_term()?;
                out.push(Triple {
                    s: subject.clone(),
                    p: predicate.clone(),
                    o: object,
                });
                self.skip_ws();
                match self.src.get(self.pos) {
                    Some(b',') => {
                        self.pos += 1;
                        continue;
                    }
                    _ => break,
                }
            }
            self.skip_ws();
            match self.src.get(self.pos) {
                Some(b';') => {
                    self.pos += 1;
                    self.skip_ws();
                    if self.src.get(self.pos) == Some(&b'.') {
                        self.pos += 1;
                        return Ok(());
                    }
                    continue;
                }
                Some(b'.') => {
                    self.pos += 1;
                    return Ok(());
                }
                None => return Ok(()),
                Some(c) => {
                    return Err(format!(
                        "unexpected '{}' at offset {} (expected ';', ',' or '.')",
                        *c as char, self.pos
                    ))
                }
            }
        }
    }

    fn parse_term(&mut self) -> Result<Term, String> {
        self.skip_ws();
        match self.src.get(self.pos) {
            Some(b'<') => {
                self.pos += 1;
                let start = self.pos;
                while self.src.get(self.pos).is_some_and(|c| *c != b'>') {
                    self.pos += 1;
                }
                let iri = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
                self.pos += 1;
                Ok(Term::Iri(self.resolve(&iri)))
            }
            Some(b'_') => {
                self.pos += 2; // "_:"
                let start = self.pos;
                while self
                    .src
                    .get(self.pos)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-')
                {
                    self.pos += 1;
                }
                Ok(Term::Blank(
                    String::from_utf8_lossy(&self.src[start..self.pos]).into_owned(),
                ))
            }
            Some(b'"') | Some(b'\'') => self.parse_literal(),
            Some(c) if c.is_ascii_digit() || *c == b'-' || *c == b'+' => {
                let start = self.pos;
                while self
                    .src
                    .get(self.pos)
                    .is_some_and(|c| c.is_ascii_digit() || *c == b'.' || *c == b'-' || *c == b'+' || *c == b'e')
                {
                    self.pos += 1;
                }
                let v = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
                let dt = if v.contains('.') {
                    "http://www.w3.org/2001/XMLSchema#decimal"
                } else {
                    "http://www.w3.org/2001/XMLSchema#integer"
                };
                Ok(Term::Literal { value: v, datatype: Some(dt.into()), lang: None })
            }
            Some(_) => {
                // prefixed name  foaf:Person  |  :Local  |  true / false
                let start = self.pos;
                while self.src.get(self.pos).is_some_and(|c| {
                    c.is_ascii_alphanumeric() || b":_-.#/".contains(c)
                }) {
                    self.pos += 1;
                }
                let raw = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
                let raw = raw.trim_end_matches('.').to_string();
                self.pos = start + raw.len();
                if raw == "true" || raw == "false" {
                    return Ok(Term::Literal {
                        value: raw,
                        datatype: Some("http://www.w3.org/2001/XMLSchema#boolean".into()),
                        lang: None,
                    });
                }
                match raw.split_once(':') {
                    Some((pfx, local)) => {
                        let base = self
                            .prefixes
                            .get(pfx)
                            .cloned()
                            .unwrap_or_else(|| format!("urn:prefix:{pfx}#"));
                        Ok(Term::Iri(format!("{base}{local}")))
                    }
                    None => Ok(Term::Iri(self.resolve(&raw))),
                }
            }
            None => Err("unexpected end of document".into()),
        }
    }

    fn parse_literal(&mut self) -> Result<Term, String> {
        let quote = self.src[self.pos];
        // Triple-quoted long strings.
        let long = self.src[self.pos..].starts_with(&[quote, quote, quote]);
        self.pos += if long { 3 } else { 1 };
        let mut value = String::new();
        loop {
            let Some(c) = self.src.get(self.pos).copied() else {
                return Err("unterminated literal".into());
            };
            if c == b'\\' {
                self.pos += 1;
                let esc = self.src.get(self.pos).copied().unwrap_or(b'\\');
                self.pos += 1;
                value.push(match esc {
                    b'n' => '\n',
                    b't' => '\t',
                    b'r' => '\r',
                    other => other as char,
                });
                continue;
            }
            if c == quote {
                if long {
                    if self.src[self.pos..].starts_with(&[quote, quote, quote]) {
                        self.pos += 3;
                        break;
                    }
                } else {
                    self.pos += 1;
                    break;
                }
            }
            if c < 0x80 {
                value.push(c as char);
                self.pos += 1;
            } else {
                let len = if c >= 0xF0 {
                    4
                } else if c >= 0xE0 {
                    3
                } else {
                    2
                };
                let end = (self.pos + len).min(self.src.len());
                value.push_str(&String::from_utf8_lossy(&self.src[self.pos..end]));
                self.pos = end;
            }
        }

        let mut datatype = None;
        let mut lang = None;
        if self.src.get(self.pos) == Some(&b'@') {
            self.pos += 1;
            let start = self.pos;
            while self
                .src
                .get(self.pos)
                .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'-')
            {
                self.pos += 1;
            }
            lang = Some(String::from_utf8_lossy(&self.src[start..self.pos]).into_owned());
        } else if self.src[self.pos..].starts_with(b"^^") {
            self.pos += 2;
            if let Term::Iri(i) = self.parse_term()? {
                datatype = Some(i);
            }
        }
        Ok(Term::Literal { value, datatype, lang })
    }

    fn resolve(&self, iri: &str) -> String {
        if iri.contains("://") || iri.starts_with("urn:") || self.base.is_empty() {
            iri.to_string()
        } else {
            format!("{}{iri}", self.base)
        }
    }
}

// --------------------------------------------------------------------------
// Mapping onto the type system
// --------------------------------------------------------------------------

fn local_name(iri: &str) -> String {
    iri.rsplit(['#', '/']).next().unwrap_or(iri).to_string()
}

fn xsd_to_property_type(dt: Option<&str>) -> &'static str {
    match dt.map(local_name).as_deref() {
        Some("integer") | Some("int") | Some("long") => "long",
        Some("decimal") | Some("double") | Some("float") => "float",
        Some("boolean") => "bool",
        Some("date") => "date",
        Some("dateTime") => "datetime",
        _ => "string",
    }
}

pub fn load(graph: &str, document: &str, format: &str) -> Value {
    let f = format.to_ascii_lowercase();
    if !["turtle", "ttl", "ntriples", "nt", "n3"].contains(&f.as_str()) {
        error!("unsupported RDF format '{format}' (turtle | ntriples)");
    }
    let (triples, prefixes) = match parse(document) {
        Ok(v) => v,
        Err(e) => error!("RDF parse error: {e}"),
    };
    let gid = types::graph_id(graph);

    for (p, iri) in &prefixes {
        Spi::run_with_args(
            "INSERT INTO og_catalog.prefix (prefix, iri) VALUES ($1,$2)
             ON CONFLICT (prefix) DO UPDATE SET iri = EXCLUDED.iri",
            &[p.as_str().into(), iri.as_str().into()],
        )
        .ok();
    }

    let mut classes = 0i64;
    let mut object_props = 0i64;
    let mut data_props = 0i64;
    let mut subclass = 0i64;
    let mut instances = 0i64;
    let mut facts = 0i64;
    let mut rules = 0i64;
    let mut unmapped = 0i64;

    // ---- pass 1: TBox (classes and properties) --------------------------
    for t in &triples {
        let (Some(s), Some(p)) = (t.s.as_iri(), t.p.as_iri()) else { continue };
        if p == RDF_TYPE {
            match t.o.as_iri() {
                Some(OWL_CLASS) | Some(RDFS_CLASS) => {
                    ensure_type(gid, s, 'e');
                    classes += 1;
                }
                Some(OWL_OBJECT_PROP) => {
                    ensure_type(gid, s, 'r');
                    object_props += 1;
                }
                Some(OWL_DATA_PROP) => {
                    data_props += 1;
                }
                Some(OWL_TRANSITIVE) | Some(OWL_SYMMETRIC) => {
                    ensure_type(gid, s, 'r');
                    let c = if t.o.as_iri() == Some(OWL_TRANSITIVE) {
                        "transitive"
                    } else {
                        "symmetric"
                    };
                    if let Some(tid) = iri_type(gid, s) {
                        Spi::run_with_args(
                            "INSERT INTO og_catalog.rule (rule_id, rel_type_id, characteristic)
                             VALUES (nextval('og_catalog.rule_id_seq')::int4, $1, $2)
                             ON CONFLICT DO NOTHING",
                            &[tid.into(), c.into()],
                        )
                        .ok();
                        rules += 1;
                    }
                }
                _ => {}
            }
        }
    }

    // ---- pass 2: hierarchy ---------------------------------------------
    for t in &triples {
        let (Some(s), Some(p), Some(o)) = (t.s.as_iri(), t.p.as_iri(), t.o.as_iri()) else {
            continue;
        };
        if p == RDFS_SUBCLASS {
            ensure_type(gid, s, 'e');
            ensure_type(gid, o, 'e');
            if let (Some(c), Some(par)) = (iri_type(gid, s), iri_type(gid, o)) {
                Spi::run_with_args(
                    "INSERT INTO og_catalog.type_parent (type_id, parent_id) VALUES ($1,$2)
                     ON CONFLICT DO NOTHING",
                    &[c.into(), par.into()],
                )
                .ok();
                subclass += 1;
            }
        }
    }
    labeling::relabel_graph(gid);

    // ---- pass 3: domain/range → roles ----------------------------------
    for t in &triples {
        let (Some(s), Some(p), Some(o)) = (t.s.as_iri(), t.p.as_iri(), t.o.as_iri()) else {
            continue;
        };
        if p != RDFS_DOMAIN && p != RDFS_RANGE {
            continue;
        }
        let Some(rel) = iri_type(gid, s) else { continue };
        if type_kind_of(rel) != 'r' {
            continue;
        }
        ensure_type(gid, o, 'e');
        let Some(player) = iri_type(gid, o) else { continue };
        let (role, ordinal) = if p == RDFS_DOMAIN { ("subject", 0) } else { ("object", 1) };
        Spi::run_with_args(
            "INSERT INTO og_catalog.role
                (role_id, rel_type_id, name, player_type_id, ordinal)
             VALUES (nextval('og_catalog.role_id_seq')::int4, $1, $2, $3, $4)
             ON CONFLICT (rel_type_id, name) DO UPDATE SET player_type_id = EXCLUDED.player_type_id",
            &[rel.into(), role.into(), player.into(), ordinal.into()],
        )
        .ok();
    }

    // ---- pass 4: ABox ---------------------------------------------------
    let mut node_of: HashMap<String, i64> = HashMap::new();
    for t in &triples {
        let (Some(s), Some(p), Some(o)) = (t.s.as_iri(), t.p.as_iri(), t.o.as_iri()) else {
            continue;
        };
        if p == RDF_TYPE
            && !matches!(o, OWL_CLASS | RDFS_CLASS | OWL_OBJECT_PROP | OWL_DATA_PROP
                            | OWL_TRANSITIVE | OWL_SYMMETRIC | OWL_FUNCTIONAL)
        {
            ensure_type(gid, o, 'e');
            let Some(tid) = iri_type(gid, o) else { continue };
            let id = crate::storage::create_node_inner(
                gid,
                tid,
                &local_name(o),
                json!({ "iri": s }),
            );
            node_of.insert(s.to_string(), id);
            Spi::run_with_args(
                "INSERT INTO og_data.og_iri (entity_id, iri) VALUES ($1,$2)
                 ON CONFLICT (iri) DO NOTHING",
                &[id.into(), s.into()],
            )
            .ok();
            instances += 1;
        }
    }

    // ---- pass 5: facts ---------------------------------------------------
    for t in &triples {
        let (Some(s), Some(p)) = (t.s.as_iri(), t.p.as_iri()) else {
            unmapped += 1;
            record_overflow(gid, t, "non-IRI subject or predicate");
            continue;
        };
        if matches!(p, RDF_TYPE | RDFS_SUBCLASS | RDFS_DOMAIN | RDFS_RANGE) {
            continue;
        }
        let Some(&sid) = node_of.get(s) else {
            if !matches!(t.o, Term::Literal { .. }) || iri_type(gid, s).is_none() {
                unmapped += 1;
                record_overflow(gid, t, "subject has no rdf:type in this document");
            }
            continue;
        };

        match &t.o {
            Term::Iri(oi) => match node_of.get(oi) {
                Some(&did) => {
                    ensure_type(gid, p, 'r');
                    if let Some(rel) = iri_type(gid, p) {
                        crate::storage::create_edge_inner(
                            gid,
                            rel,
                            &local_name(p),
                            sid,
                            did,
                            json!({}),
                        );
                        facts += 1;
                    }
                }
                None => {
                    unmapped += 1;
                    record_overflow(gid, t, "object IRI is not an instance in this document");
                }
            },
            Term::Literal { value, datatype, lang } => {
                let tid = crate::id::id_type(sid);
                let tname = crate::storage::type_name_of(tid);
                let pname = local_name(p);
                let ptype = xsd_to_property_type(datatype.as_deref());
                ensure_property(graph, &tname, &pname, ptype);
                let mut payload = serde_json::Map::new();
                payload.insert(pname.clone(), json!(value));
                if let Some(l) = lang {
                    payload.insert(format!("{pname}@lang"), json!(l));
                }
                if let Some(d) = datatype {
                    payload.insert(format!("{pname}^^"), json!(d));
                }
                crate::storage::set_node_props_inner(sid, Value::Object(payload));
                facts += 1;
            }
            Term::Blank(_) => {
                unmapped += 1;
                record_overflow(gid, t, "blank node object");
            }
        }
    }

    json!({
        "graph": graph,
        "triples_read": triples.len(),
        "classes": classes,
        "object_properties": object_props,
        "datatype_properties": data_props,
        "subclass_axioms": subclass,
        "instances": instances,
        "facts": facts,
        "inference_rules": rules,
        "unmapped": unmapped,
        "note": if unmapped > 0 {
            "unmapped triples were preserved verbatim — see og_mapping_report(graph)"
        } else { "all triples mapped" },
    })
}

fn record_overflow(gid: i32, t: &Triple, reason: &str) {
    Spi::run_with_args(
        "INSERT INTO og_data.og_triple_overflow (graph_id, subject, predicate, object, reason)
         VALUES ($1,$2,$3,$4,$5)",
        &[
            gid.into(),
            t.s.text().into(),
            t.p.text().into(),
            t.o.text().into(),
            reason.into(),
        ],
    )
    .ok();
}

fn iri_type(gid: i32, iri: &str) -> Option<i32> {
    crate::spiu::one::<i32>(
        "SELECT type_id FROM og_catalog.type WHERE graph_id = $1 AND iri = $2",
        &[gid.into(), iri.into()],
    )
    .ok()
    .flatten()
}

fn type_kind_of(tid: i32) -> char {
    crate::spiu::one::<String>(
        "SELECT kind::text FROM og_catalog.type WHERE type_id = $1",
        &[tid.into()],
    )
    .ok()
    .flatten()
    .and_then(|s| s.chars().next())
    .unwrap_or('e')
}

fn ensure_type(gid: i32, iri: &str, kind: char) {
    if iri_type(gid, iri).is_some() {
        return;
    }
    let mut name = local_name(iri);
    if name.is_empty() {
        name = iri.to_string();
    }
    // Name collision with a different IRI: keep both, disambiguate the newcomer.
    if types::try_type_id(gid, &name).is_some() {
        name = format!("{name}_{}", crate::catalog::types::edit_distance(iri, ""));
    }
    let graph = crate::spiu::one::<String>(
        "SELECT name FROM og_catalog.graph WHERE graph_id = $1",
        &[gid.into()],
    )
    .ok()
    .flatten()
    .unwrap_or_default();
    let kind_s = if kind == 'r' { "relation" } else { "entity" };
    Spi::run_with_args(
        "SELECT og_create_type($1,$2,$3,'{}'::text[],false)",
        &[graph.into(), name.clone().into(), kind_s.into()],
    )
    .ok();
    Spi::run_with_args(
        "UPDATE og_catalog.type SET iri = $2 WHERE graph_id = $3 AND name = $1",
        &[name.into(), iri.into(), gid.into()],
    )
    .ok();
}

fn ensure_property(graph: &str, type_name: &str, prop: &str, ptype: &str) {
    let gid = types::graph_id(graph);
    let Some(tid) = types::try_type_id(gid, type_name) else { return };
    let exists = crate::spiu::one::<bool>(
        "SELECT true FROM og_catalog.property WHERE type_id = $1 AND name = $2",
        &[tid.into(), prop.into()],
    )
    .ok()
    .flatten()
    .unwrap_or(false);
    if exists {
        return;
    }
    Spi::run_with_args(
        "SELECT og_add_property($1,$2,$3,$4,false,false)",
        &[graph.into(), type_name.into(), prop.into(), ptype.into()],
    )
    .ok();
}

// --------------------------------------------------------------------------
// Serialisation
// --------------------------------------------------------------------------

pub fn dump(graph: &str, format: &str) -> String {
    let gid = types::graph_id(graph);
    let mut out = String::new();
    let ntriples = matches!(format.to_ascii_lowercase().as_str(), "ntriples" | "nt");

    let prefixes: Vec<(String, String)> = Spi::connect(|client| {
        client
            .select("SELECT prefix, iri FROM og_catalog.prefix ORDER BY prefix", None, &[])
            .map(|rows| {
                rows.filter_map(|r| Some((r.get::<String>(1).ok()??, r.get::<String>(2).ok()??)))
                    .collect()
            })
            .unwrap_or_default()
    });
    if !ntriples {
        for (p, i) in &prefixes {
            out.push_str(&format!("@prefix {p}: <{i}> .\n"));
        }
        out.push('\n');
    }

    // TBox
    let types_rows: Vec<(String, String, String, Option<String>)> = Spi::connect(|client| {
        client
            .select(
                "SELECT t.name, t.kind::text, COALESCE(t.iri, 'urn:og:' || t.name),
                        (SELECT COALESCE(p.iri, 'urn:og:' || p.name) FROM og_catalog.type p
                           JOIN og_catalog.type_parent tp ON tp.parent_id = p.type_id
                          WHERE tp.type_id = t.type_id LIMIT 1)
                   FROM og_catalog.type t WHERE t.graph_id = $1 ORDER BY t.name",
                None,
                &[gid.into()],
            )
            .expect("type dump failed")
            .filter_map(|r| {
                Some((
                    r.get::<String>(1).ok()??,
                    r.get::<String>(2).ok()??,
                    r.get::<String>(3).ok()??,
                    r.get::<String>(4).ok().flatten(),
                ))
            })
            .collect()
    });
    for (_name, kind, iri, parent) in &types_rows {
        let class = if kind == "r" { OWL_OBJECT_PROP } else { OWL_CLASS };
        out.push_str(&format!("<{iri}> <{RDF_TYPE}> <{class}> .\n"));
        if let Some(p) = parent {
            out.push_str(&format!("<{iri}> <{RDFS_SUBCLASS}> <{p}> .\n"));
        }
    }

    // ABox
    let nodes: Vec<(i64, String, Value)> = Spi::connect(|client| {
        client
            .select(
                "SELECT n.id, COALESCE(t.iri, 'urn:og:' || t.name), og_node_json(n.id)
                   FROM og_data.og_node n
                   JOIN og_catalog.type t ON t.type_id = n.type_id
                  WHERE t.graph_id = $1",
                None,
                &[gid.into()],
            )
            .expect("node dump failed")
            .filter_map(|r| {
                Some((
                    r.get::<i64>(1).ok()??,
                    r.get::<String>(2).ok()??,
                    r.get::<pgrx::JsonB>(3).ok()??.0,
                ))
            })
            .collect()
    });
    for (id, tiri, doc) in &nodes {
        let subject = doc
            .get("iri")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("urn:og:node:{id}"));
        out.push_str(&format!("<{subject}> <{RDF_TYPE}> <{tiri}> .\n"));
        if let Value::Object(m) = doc {
            for (k, v) in m {
                if k.starts_with('_') || k == "iri" || k.contains('@') || k.contains("^^") {
                    continue;
                }
                let lit = match v {
                    Value::String(s) => format!("\"{}\"", s.replace('"', "\\\"")),
                    Value::Null => continue,
                    other => format!("\"{other}\""),
                };
                out.push_str(&format!("<{subject}> <urn:og:prop:{k}> {lit} .\n"));
            }
        }
    }

    let edges: Vec<(String, String, String)> = Spi::connect(|client| {
        client
            .select(
                "SELECT COALESCE(si.iri, 'urn:og:node:' || e.src),
                        COALESCE(t.iri, 'urn:og:' || t.name),
                        COALESCE(di.iri, 'urn:og:node:' || e.dst)
                   FROM og_data.og_edge e
                   JOIN og_catalog.type t ON t.type_id = e.type_id
                   LEFT JOIN og_data.og_iri si ON si.entity_id = e.src
                   LEFT JOIN og_data.og_iri di ON di.entity_id = e.dst
                  WHERE t.graph_id = $1",
                None,
                &[gid.into()],
            )
            .expect("edge dump failed")
            .filter_map(|r| {
                Some((
                    r.get::<String>(1).ok()??,
                    r.get::<String>(2).ok()??,
                    r.get::<String>(3).ok()??,
                ))
            })
            .collect()
    });
    for (s, p, o) in &edges {
        out.push_str(&format!("<{s}> <{p}> <{o}> .\n"));
    }

    // Round-trip fidelity: re-emit whatever we could not map on the way in.
    let overflow: Vec<(String, String, String)> = Spi::connect(|client| {
        client
            .select(
                "SELECT subject, predicate, object FROM og_data.og_triple_overflow
                  WHERE graph_id = $1",
                None,
                &[gid.into()],
            )
            .map(|rows| {
                rows.filter_map(|r| {
                    Some((
                        r.get::<String>(1).ok()??,
                        r.get::<String>(2).ok()??,
                        r.get::<String>(3).ok()??,
                    ))
                })
                .collect()
            })
            .unwrap_or_default()
    });
    for (s, p, o) in &overflow {
        let obj = if o.starts_with("http") || o.starts_with("urn:") {
            format!("<{o}>")
        } else {
            format!("\"{}\"", o.replace('"', "\\\""))
        };
        out.push_str(&format!("<{s}> <{p}> {obj} .\n"));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_turtle_prefixes_and_types() {
        let doc = r#"
            @prefix ex: <http://example.org/> .
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            ex:Vehicle a <http://www.w3.org/2002/07/owl#Class> .
            ex:Car rdfs:subClassOf ex:Vehicle ; a <http://www.w3.org/2002/07/owl#Class> .
            ex:car1 a ex:Car ; ex:model "Model 3" ; ex:year 2024 .
        "#;
        let (triples, prefixes) = parse(doc).unwrap();
        assert_eq!(prefixes.get("ex").map(String::as_str), Some("http://example.org/"));
        assert!(triples.iter().any(|t| t.p.as_iri() == Some(RDFS_SUBCLASS)));
        assert!(triples
            .iter()
            .any(|t| matches!(&t.o, Term::Literal { value, .. } if value == "Model 3")));
    }

    #[test]
    fn language_tags_survive() {
        let doc = r#"<urn:a> <urn:label> "색"@ko ."#;
        let (t, _) = parse(doc).unwrap();
        match &t[0].o {
            Term::Literal { value, lang, .. } => {
                assert_eq!(value, "색");
                assert_eq!(lang.as_deref(), Some("ko"));
            }
            _ => panic!("expected a literal"),
        }
    }
}
