//! AGE calling conventions, checked with the driver the Bolt gateway uses
//! (`postgres` 0.19, extended protocol, binary parameters and results).
//!
//!   AGESB_DSN='host=127.0.0.1 port=55439 dbname=postgres user=robo password=... options=-csearch_path=ag_catalog,public' \
//!     cargo run --release
//!
//! What it shows:
//! * `options=-csearch_path=ag_catalog,...` in the connection string is enough
//!   for every Cypher statement (no SET, no role setting).
//! * The third cypher() argument is a bare `$1`; the server infers its type as
//!   agtype. A small `ToSql` wrapper sends agtype in binary: one version byte
//!   (1) followed by the agtype text (JSON). Same for `FromSql` on the way back.
//! * agtype text keeps `::vertex` / `::edge` / `::path` / `::numeric`
//!   annotations and the int/float distinction (`1` vs `1.0`).
use postgres::types::{to_sql_checked, FromSql, IsNull, ToSql, Type};
use postgres::{Client, NoTls};
use std::error::Error;

/// agtype value as its text form (JSON plus AGE's `::vertex`-style annotations).
#[derive(Debug)]
struct Agtype(String);

impl ToSql for Agtype {
    fn to_sql(&self, _ty: &Type, out: &mut postgres::types::private::BytesMut)
        -> Result<IsNull, Box<dyn Error + Sync + Send>> {
        out.extend_from_slice(&[1u8]); // agtype_recv: version 1, then text
        out.extend_from_slice(self.0.as_bytes());
        Ok(IsNull::No)
    }
    fn accepts(ty: &Type) -> bool { ty.name() == "agtype" }
    to_sql_checked!();
}

impl<'a> FromSql<'a> for Agtype {
    fn from_sql(_ty: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn Error + Sync + Send>> {
        match raw.split_first() {
            Some((1, text)) => Ok(Agtype(std::str::from_utf8(text)?.to_owned())),
            _ => Err("unexpected agtype binary version".into()),
        }
    }
    fn accepts(ty: &Type) -> bool { ty.name() == "agtype" }
}

/// Dollar-quote client Cypher with a tag that the text does not contain
/// (AGE accepts only a dollar-quoted constant as the query argument).
fn dollar_quote(cypher: &str) -> String {
    let mut n = 0u32;
    loop {
        let tag = format!("$cy{n}$");
        if !cypher.contains(&tag) { return format!("{tag}{cypher}{tag}"); }
        n += 1;
    }
}

fn cypher_sql(graph: &str, cypher: &str, cols: &[&str]) -> String {
    let defs: Vec<String> = cols.iter().map(|c| format!("\"{c}\" ag_catalog.agtype")).collect();
    format!(
        "SELECT * FROM ag_catalog.cypher('{graph}', {}, $1) AS ({})",
        dollar_quote(cypher), defs.join(", ")
    )
}

fn main() -> Result<(), Box<dyn Error>> {
    let dsn = std::env::var("AGESB_DSN")?;
    let mut c = Client::connect(&dsn, NoTls)?;
    let sp: String = c.query_one("SELECT current_setting('search_path')", &[])?.get(0);
    println!("connected, search_path = {sp}");

    c.batch_execute("SELECT ag_catalog.create_graph('agesb_rust')")?;

    // write with parameters, including a brand-new vertex label and edge label
    let sql = cypher_sql("agesb_rust",
        "CREATE (a:RustNode {name: $a, tags: $tags})-[r:RUST_REL {w: $w}]->(b:RustNode {name: $b}) RETURN a, r, b",
        &["a", "r", "b"]);
    let params = Agtype(r#"{"a": "Order", "b": "OrderLine", "w": 1.5, "tags": ["x", "y"]}"#.into());
    let row = c.query_one(sql.as_str(), &[&params])?;
    let stmt = c.prepare(sql.as_str())?;
    println!("param type inferred by the server: {}", stmt.params()[0].name());
    for (i, col) in ["a", "r", "b"].iter().enumerate() {
        let v: Agtype = row.get(i);
        println!("{col} = {}", v.0);
    }

    // read: path, scalars, list with a node, a value with $$ inside the query text
    let sql = cypher_sql("agesb_rust",
        "MATCH p = (a:RustNode {name: $n})-[*1..2]->(b) RETURN p, length(p), 1, 1.0, 'has $$ and $cy0$ inside', [a, 2]",
        &["p", "len", "i", "f", "s", "l"]);
    let row = c.query_one(sql.as_str(), &[&Agtype(r#"{"n": "Order"}"#.into())])?;
    for (i, col) in ["p", "len", "i", "f", "s", "l"].iter().enumerate() {
        let v: Agtype = row.get(i);
        println!("{col} = {}", v.0);
    }

    // the same row as jsonb (list-wrap keeps scalars working; annotations are dropped)
    let sql = format!(
        "SELECT (ag_catalog.agtype_build_list(p)::jsonb)->0, (ag_catalog.agtype_build_list(i)::jsonb)->0 FROM ({}) q",
        cypher_sql("agesb_rust", "MATCH p = (:RustNode)-[:RUST_REL]->() RETURN p, 1", &["p", "i"]));
    let row = c.query_one(sql.as_str(), &[&Agtype("{}".into())])?;
    let p: serde_json_value::Raw = row.get(0);
    let i: serde_json_value::Raw = row.get(1);
    println!("as jsonb: p = {}, i = {}", p.0, i.0);

    // a text-typed parameter is refused (third argument must be agtype $1)
    let bad = c.query(
        "SELECT * FROM ag_catalog.cypher('agesb_rust', $$ RETURN $x $$, $1::text::ag_catalog.agtype) AS (x ag_catalog.agtype)",
        &[&r#"{"x": 1}"#]);
    println!("text parameter + cast: {}", bad.err()
        .map(|e| e.as_db_error().map(|d| d.message().to_owned()).unwrap_or_else(|| e.to_string()))
        .unwrap_or_else(|| "accepted".into()));

    c.batch_execute("SELECT ag_catalog.drop_graph('agesb_rust', true)")?;
    println!("ok");
    Ok(())
}

/// jsonb read without the serde feature: binary jsonb is version byte 1 + text.
mod serde_json_value {
    use postgres::types::{FromSql, Type};
    use std::error::Error;
    pub struct Raw(pub serde_json::Value);
    impl<'a> FromSql<'a> for Raw {
        fn from_sql(_ty: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn Error + Sync + Send>> {
            let (_, text) = raw.split_first().ok_or("empty jsonb")?;
            Ok(Raw(serde_json::from_slice(text)?))
        }
        fn accepts(ty: &Type) -> bool { *ty == Type::JSONB }
    }
}
