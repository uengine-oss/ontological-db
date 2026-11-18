//! agtype, the value type of Apache AGE, as the gateway reads and writes it.
//!
//! AGE sends agtype over the wire as one version byte (1) followed by its text
//! form (`agtype_send`, `src/backend/utils/adt/agtype.c`), and reads it back the
//! same way (`agtype_recv`). The text form is JSON with annotations: a vertex is
//! `{"id": …, "label": …, "properties": {…}}::vertex`, an edge also carries
//! `start_id`/`end_id` and is `::edge`, a path is a list of them `::path`, a
//! decimal is `…::numeric`, and floats may be `NaN`, `Infinity`, `-Infinity`.
//!
//! [`parse`] turns that text into [`Ag`], and [`Ag::to_bolt`] into the Bolt
//! value a Neo4j driver expects: a vertex becomes a `Node`, an edge a
//! `Relationship`, a path a `Path`.
//!
//! Element ids: Bolt 4.4 carries an integer id only, and drivers derive the
//! element id from it (`str(id)`). The integer is AGE's graphid — the label's
//! number in the top 16 bits, the row's sequence below — so it is unique in the
//! graph and stable for the life of the element, and `elementId(n)` is
//! rewritten to `toString(id(n))` so that the two agree.

use crate::packstream::{string, Value};
use postgres::types::{to_sql_checked, FromSql, IsNull, ToSql, Type};
use std::collections::BTreeMap;
use std::error::Error;

const NODE: u8 = 0x4E;
const RELATIONSHIP: u8 = 0x52;
const UNBOUND_RELATIONSHIP: u8 = 0x72;
const PATH: u8 = 0x50;

/// One agtype value.
#[derive(Debug, Clone, PartialEq)]
pub enum Ag {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    /// `…::numeric`: kept as written.
    Numeric(String),
    Str(String),
    List(Vec<Ag>),
    Map(Vec<(String, Ag)>),
    Vertex { id: i64, label: String, props: Vec<(String, Ag)> },
    Edge { id: i64, label: String, start: i64, end: i64, props: Vec<(String, Ag)> },
    Path(Vec<Ag>),
}

impl Ag {
    /// The value as Bolt carries it.
    pub fn to_bolt(&self) -> Value {
        match self {
            Ag::Null => Value::Null,
            Ag::Bool(b) => Value::Bool(*b),
            Ag::Int(i) => Value::Int(*i),
            Ag::Float(f) => Value::Float(*f),
            // Bolt has no decimal; a float is what a driver can hold.
            Ag::Numeric(s) => s.parse::<f64>().map(Value::Float).unwrap_or_else(|_| string(s.clone())),
            Ag::Str(s) => Value::String(s.clone()),
            Ag::List(xs) => Value::List(xs.iter().map(Ag::to_bolt).collect()),
            Ag::Map(kv) => Value::Map(kv.iter().map(|(k, v)| (k.clone(), v.to_bolt())).collect()),
            Ag::Vertex { id, label, props } => Value::Struct(
                NODE,
                vec![Value::Int(*id), Value::List(labels_of(label)), props_of(props)],
            ),
            Ag::Edge { id, label, start, end, props } => Value::Struct(
                RELATIONSHIP,
                vec![Value::Int(*id), Value::Int(*start), Value::Int(*end), string(label.clone()), props_of(props)],
            ),
            Ag::Path(elems) => path(elems).unwrap_or_else(|| Value::List(elems.iter().map(Ag::to_bolt).collect())),
        }
    }

    /// The graph id, for a vertex or an edge.
    pub fn element_id(&self) -> Option<i64> {
        match self {
            Ag::Vertex { id, .. } | Ag::Edge { id, .. } => Some(*id),
            _ => None,
        }
    }

    /// As JSON (for values carried from one statement to the next as a
    /// parameter): graph elements are not expected here.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Ag::Null => serde_json::Value::Null,
            Ag::Bool(b) => serde_json::Value::Bool(*b),
            Ag::Int(i) => serde_json::Value::from(*i),
            Ag::Float(f) => serde_json::Number::from_f64(*f).map(serde_json::Value::Number).unwrap_or(serde_json::Value::Null),
            Ag::Numeric(s) => s.parse::<f64>().ok().and_then(serde_json::Number::from_f64).map(serde_json::Value::Number).unwrap_or(serde_json::Value::Null),
            Ag::Str(s) => serde_json::Value::String(s.clone()),
            Ag::List(xs) | Ag::Path(xs) => serde_json::Value::Array(xs.iter().map(Ag::to_json).collect()),
            Ag::Map(kv) => serde_json::Value::Object(kv.iter().map(|(k, v)| (k.clone(), v.to_json())).collect()),
            Ag::Vertex { id, .. } | Ag::Edge { id, .. } => serde_json::Value::from(*id),
        }
    }
}

/// AGE's default vertex label (a vertex created without one) is the empty
/// name; Neo4j's unlabelled node has no labels.
fn labels_of(label: &str) -> Vec<Value> {
    if label.is_empty() || label == "_ag_label_vertex" {
        Vec::new()
    } else {
        vec![string(label.to_string())]
    }
}

fn props_of(props: &[(String, Ag)]) -> Value {
    // Neo4j has no property whose value is null: such a key is absent.
    Value::Map(props.iter().filter(|(_, v)| *v != Ag::Null).map(|(k, v)| (k.clone(), v.to_bolt())).collect::<BTreeMap<_, _>>())
}

/// A path — vertex, edge, vertex, … — as Bolt's Path structure: the distinct
/// nodes, the distinct relationships (unbound), and the walk as indices into
/// both (a relationship index is negative when the walk crosses it against its
/// direction). `None` when the list is not shaped like a path.
fn path(elems: &[Ag]) -> Option<Value> {
    if elems.is_empty() || elems.len() % 2 == 0 {
        return None;
    }
    let mut node_ids: Vec<i64> = Vec::new();
    let mut node_vals: Vec<Value> = Vec::new();
    let mut rel_ids: Vec<i64> = Vec::new();
    let mut rel_vals: Vec<Value> = Vec::new();
    let mut walk: Vec<i64> = Vec::new();
    let mut node_index = |n: &Ag| -> Option<i64> {
        let Ag::Vertex { id, .. } = n else { return None };
        Some(match node_ids.iter().position(|x| x == id) {
            Some(i) => i as i64,
            None => {
                node_ids.push(*id);
                node_vals.push(n.to_bolt());
                (node_ids.len() - 1) as i64
            }
        })
    };
    node_index(&elems[0])?;
    let mut i = 1;
    while i < elems.len() {
        let Ag::Edge { id, label, start, props, .. } = &elems[i] else { return None };
        let at = match rel_ids.iter().position(|x| x == id) {
            Some(k) => k,
            None => {
                rel_ids.push(*id);
                rel_vals.push(Value::Struct(UNBOUND_RELATIONSHIP, vec![Value::Int(*id), string(label.clone()), props_of(props)]));
                rel_ids.len() - 1
            }
        } as i64
            + 1;
        let Ag::Vertex { id: from, .. } = &elems[i - 1] else { return None };
        walk.push(if from == start { at } else { -at });
        walk.push(node_index(&elems[i + 1])?);
        i += 2;
    }
    Some(Value::Struct(
        PATH,
        vec![Value::List(node_vals), Value::List(rel_vals), Value::List(walk.into_iter().map(Value::Int).collect())],
    ))
}

// ------------------------------------------------------------------ text

/// Parse agtype's text form.
pub fn parse(text: &str) -> Result<Ag, String> {
    let mut p = P { s: text.as_bytes(), i: 0 };
    let v = p.value()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(format!("agtype: trailing input at {}", p.i));
    }
    Ok(v)
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn eat(&mut self, lit: &str) -> bool {
        if self.s[self.i..].starts_with(lit.as_bytes()) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self) -> Result<Ag, String> {
        self.ws();
        let v = match self.s.get(self.i) {
            None => return Err("agtype: unexpected end".into()),
            Some(b'{') => Ag::Map(self.object()?),
            Some(b'[') => Ag::List(self.array()?),
            Some(b'"') => Ag::Str(self.string()?),
            Some(_) if self.eat("null") => Ag::Null,
            Some(_) if self.eat("true") => Ag::Bool(true),
            Some(_) if self.eat("false") => Ag::Bool(false),
            Some(_) if self.eat("NaN") => Ag::Float(f64::NAN),
            Some(_) if self.eat("Infinity") => Ag::Float(f64::INFINITY),
            Some(_) if self.eat("-Infinity") => Ag::Float(f64::NEG_INFINITY),
            Some(_) => self.number()?,
        };
        self.ws();
        if !self.eat("::") {
            return Ok(v);
        }
        let start = self.i;
        while self.i < self.s.len() && self.s[self.i].is_ascii_alphabetic() {
            self.i += 1;
        }
        let ann = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("");
        Ok(match (ann, v) {
            ("vertex", Ag::Map(m)) => {
                let (id, label, props) = entity(m)?;
                Ag::Vertex { id, label, props }
            }
            ("edge", Ag::Map(m)) => {
                let start = get_int(&m, "start_id")?;
                let end = get_int(&m, "end_id")?;
                let (id, label, props) = entity(m)?;
                Ag::Edge { id, label, start, end, props }
            }
            ("path", Ag::List(xs)) => Ag::Path(xs),
            ("numeric", Ag::Int(i)) => Ag::Numeric(i.to_string()),
            ("numeric", Ag::Float(f)) => Ag::Numeric(f.to_string()),
            ("numeric", Ag::Numeric(n)) => Ag::Numeric(n),
            ("float", Ag::Int(i)) => Ag::Float(i as f64),
            ("integer", v) | ("float", v) => v,
            (other, _) => return Err(format!("agtype: unknown annotation ::{other}")),
        })
    }

    fn object(&mut self) -> Result<Vec<(String, Ag)>, String> {
        self.i += 1; // {
        let mut out = Vec::new();
        self.ws();
        if self.eat("}") {
            return Ok(out);
        }
        loop {
            self.ws();
            let k = self.string()?;
            self.ws();
            if !self.eat(":") {
                return Err(format!("agtype: expected ':' at {}", self.i));
            }
            let v = self.value()?;
            out.push((k, v));
            self.ws();
            if self.eat(",") {
                continue;
            }
            if self.eat("}") {
                return Ok(out);
            }
            return Err(format!("agtype: expected ',' or '}}' at {}", self.i));
        }
    }

    fn array(&mut self) -> Result<Vec<Ag>, String> {
        self.i += 1; // [
        let mut out = Vec::new();
        self.ws();
        if self.eat("]") {
            return Ok(out);
        }
        loop {
            out.push(self.value()?);
            self.ws();
            if self.eat(",") {
                continue;
            }
            if self.eat("]") {
                return Ok(out);
            }
            return Err(format!("agtype: expected ',' or ']' at {}", self.i));
        }
    }

    fn string(&mut self) -> Result<String, String> {
        if self.s.get(self.i) != Some(&b'"') {
            return Err(format!("agtype: expected a string at {}", self.i));
        }
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        while let Some(&c) = self.s.get(self.i) {
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).map_err(|e| e.to_string()),
                b'\\' => {
                    let e = *self.s.get(self.i).ok_or("agtype: bad escape")?;
                    self.i += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp) && self.eat("\\u") {
                                let lo = self.hex4()?;
                                cp = 0x10000 + ((cp - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF);
                            }
                            let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        other => out.push(other),
                    }
                }
                other => out.push(other),
            }
        }
        Err("agtype: unterminated string".into())
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let h = self.s.get(self.i..self.i + 4).ok_or("agtype: short \\u escape")?;
        self.i += 4;
        u32::from_str_radix(std::str::from_utf8(h).map_err(|e| e.to_string())?, 16).map_err(|e| e.to_string())
    }

    fn number(&mut self) -> Result<Ag, String> {
        let start = self.i;
        let mut float = false;
        if matches!(self.s.get(self.i), Some(b'-') | Some(b'+')) {
            self.i += 1;
        }
        while let Some(&c) = self.s.get(self.i) {
            match c {
                b'0'..=b'9' => {}
                b'.' | b'e' | b'E' => float = true,
                b'-' | b'+' if float => {}
                _ => break,
            }
            self.i += 1;
        }
        let t = std::str::from_utf8(&self.s[start..self.i]).map_err(|e| e.to_string())?;
        if t.is_empty() || t == "-" {
            return Err(format!("agtype: unexpected character at {start}"));
        }
        if float {
            t.parse::<f64>().map(Ag::Float).map_err(|e| format!("agtype: {e}: {t}"))
        } else {
            match t.parse::<i64>() {
                Ok(i) => Ok(Ag::Int(i)),
                Err(_) => Ok(Ag::Numeric(t.to_string())),
            }
        }
    }
}

fn get_int(m: &[(String, Ag)], key: &str) -> Result<i64, String> {
    match m.iter().find(|(k, _)| k == key) {
        Some((_, Ag::Int(i))) => Ok(*i),
        _ => Err(format!("agtype: entity without an integer {key}")),
    }
}

fn entity(m: Vec<(String, Ag)>) -> Result<(i64, String, Vec<(String, Ag)>), String> {
    let id = get_int(&m, "id")?;
    if !m.iter().any(|(k, _)| k == "label") {
        return Err("agtype: entity without a label".into());
    }
    let mut label = String::new();
    let mut props = Vec::new();
    for (k, v) in m {
        match (k.as_str(), v) {
            ("label", Ag::Str(s)) => label = s,
            ("properties", Ag::Map(p)) => props = p,
            _ => {}
        }
    }
    Ok((id, label, props))
}

// ------------------------------------------------------------------ wire

/// An agtype parameter, sent in agtype's binary form: version 1, then text.
#[derive(Debug)]
pub struct AgParam(pub String);

impl ToSql for AgParam {
    fn to_sql(&self, _ty: &Type, out: &mut postgres::types::private::BytesMut) -> Result<IsNull, Box<dyn Error + Sync + Send>> {
        out.extend_from_slice(&[1u8]);
        out.extend_from_slice(self.0.as_bytes());
        Ok(IsNull::No)
    }

    fn accepts(ty: &Type) -> bool {
        ty.name() == "agtype"
    }

    to_sql_checked!();
}

/// An agtype result column, as its text form.
#[derive(Debug)]
pub struct AgText(pub String);

impl<'a> FromSql<'a> for AgText {
    fn from_sql(_ty: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn Error + Sync + Send>> {
        match raw.split_first() {
            Some((1, rest)) => Ok(AgText(std::str::from_utf8(rest)?.to_string())),
            Some((v, _)) => Err(format!("unsupported agtype version {v}").into()),
            None => Err("empty agtype".into()),
        }
    }

    fn accepts(ty: &Type) -> bool {
        ty.name() == "agtype"
    }
}

/// A JSON parameter map as agtype text. agtype's text form is a superset of
/// JSON, so the JSON text is already agtype.
pub fn params_text(v: &serde_json::Value) -> String {
    v.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars_and_containers() {
        assert_eq!(parse("1").unwrap(), Ag::Int(1));
        assert_eq!(parse("-7").unwrap(), Ag::Int(-7));
        assert_eq!(parse("1.5").unwrap(), Ag::Float(1.5));
        assert_eq!(parse("2.0::numeric").unwrap(), Ag::Numeric("2".into()));
        assert!(matches!(parse("NaN").unwrap(), Ag::Float(f) if f.is_nan()));
        assert_eq!(parse("-Infinity").unwrap(), Ag::Float(f64::NEG_INFINITY));
        assert_eq!(parse("null").unwrap(), Ag::Null);
        assert_eq!(parse(r#""a\"b\\c\né😀""#).unwrap(), Ag::Str("a\"b\\c\né😀".into()));
        assert_eq!(parse("\"회의실\"").unwrap(), Ag::Str("회의실".into()));
        assert_eq!(
            parse(r#"{"a": [1, true, null], "b": {}}"#).unwrap(),
            Ag::Map(vec![("a".into(), Ag::List(vec![Ag::Int(1), Ag::Bool(true), Ag::Null])), ("b".into(), Ag::Map(vec![]))])
        );
    }

    #[test]
    fn a_vertex_is_a_node() {
        let v = parse(r#"{"id": 844424930131969, "label": "Person", "properties": {"name": "Ann", "gone": null}}::vertex"#).unwrap();
        assert_eq!(v.element_id(), Some(844424930131969));
        let Value::Struct(sig, f) = v.to_bolt() else { panic!() };
        assert_eq!(sig, NODE);
        assert_eq!(f[0], Value::Int(844424930131969));
        assert_eq!(f[1], Value::List(vec![string("Person")]));
        // A null-valued key is not a property.
        assert_eq!(f[2], Value::Map([("name".to_string(), string("Ann"))].into_iter().collect()));
        // The default label is no label.
        let Value::Struct(_, f) = parse(r#"{"id": 281474976710657, "label": "", "properties": {}}::vertex"#).unwrap().to_bolt() else {
            panic!()
        };
        assert_eq!(f[1], Value::List(vec![]));
    }

    #[test]
    fn an_edge_is_a_relationship() {
        let e = parse(r#"{"id": 1125899906842625, "label": "KNOWS", "end_id": 2, "start_id": 1, "properties": {"w": 0.5}}::edge"#).unwrap();
        let Value::Struct(sig, f) = e.to_bolt() else { panic!() };
        assert_eq!(sig, RELATIONSHIP);
        assert_eq!(&f[..4], &[Value::Int(1125899906842625), Value::Int(1), Value::Int(2), string("KNOWS")]);
    }

    #[test]
    fn a_path_is_a_path() {
        let p = parse(
            r#"[{"id": 1, "label": "A", "properties": {}}::vertex, {"id": 10, "label": "R", "end_id": 1, "start_id": 2, "properties": {}}::edge, {"id": 2, "label": "A", "properties": {}}::vertex]::path"#,
        )
        .unwrap();
        let Value::Struct(sig, f) = p.to_bolt() else { panic!() };
        assert_eq!(sig, PATH);
        // From 1 to 2 against the edge (2 → 1): index -1.
        assert_eq!(f[2], Value::List(vec![Value::Int(-1), Value::Int(1)]));
        let (Value::List(nodes), Value::List(rels)) = (&f[0], &f[1]) else { panic!() };
        assert_eq!((nodes.len(), rels.len()), (2, 1));
    }

    #[test]
    fn bad_text_is_an_error() {
        assert!(parse("{\"a\": }").is_err());
        assert!(parse("[1, 2").is_err());
        assert!(parse("{\"id\": 1}::vertex").is_err());
        assert!(parse("1 2").is_err());
    }
}
