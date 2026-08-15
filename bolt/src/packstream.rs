//! PackStream v2 — the value encoding Bolt carries (spec 011, FR-004).
//!
//! Written out rather than pulled from a crate on purpose: the support matrix
//! is a documented deliverable (FR-020), and a dependency would decide it for
//! us.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    List(Vec<Value>),
    Map(BTreeMap<String, Value>),
    /// Signature + fields. Nodes, relationships and paths ride in these.
    Struct(u8, Vec<Value>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn map_get<'a>(&'a self, key: &str) -> Option<&'a Value> {
        match self {
            Value::Map(m) => m.get(key),
            _ => None,
        }
    }
}

pub fn map(pairs: Vec<(&str, Value)>) -> Value {
    Value::Map(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

pub fn string(s: impl Into<String>) -> Value {
    Value::String(s.into())
}

// ---------------------------------------------------------------- encoding

pub fn encode(v: &Value, out: &mut Vec<u8>) {
    match v {
        Value::Null => out.push(0xC0),
        Value::Bool(false) => out.push(0xC2),
        Value::Bool(true) => out.push(0xC3),
        Value::Float(f) => {
            out.push(0xC1);
            out.extend_from_slice(&f.to_be_bytes());
        }
        Value::Int(i) => encode_int(*i, out),
        Value::String(s) => {
            let b = s.as_bytes();
            encode_header(b.len(), 0x80, 0xD0, out);
            out.extend_from_slice(b);
        }
        Value::List(xs) => {
            encode_header(xs.len(), 0x90, 0xD4, out);
            for x in xs {
                encode(x, out);
            }
        }
        Value::Map(m) => {
            encode_header(m.len(), 0xA0, 0xD8, out);
            for (k, val) in m {
                encode(&Value::String(k.clone()), out);
                encode(val, out);
            }
        }
        Value::Struct(sig, fields) => {
            // PackStream v2 structures are tiny-only: at most 15 fields, which
            // every Bolt message and graph type stays within.
            out.push(0xB0 | (fields.len() as u8 & 0x0F));
            out.push(*sig);
            for f in fields {
                encode(f, out);
            }
        }
    }
}

fn encode_int(i: i64, out: &mut Vec<u8>) {
    match i {
        -16..=127 => out.push(i as i8 as u8),
        -128..=127 => {
            out.push(0xC8);
            out.push(i as i8 as u8);
        }
        -32_768..=32_767 => {
            out.push(0xC9);
            out.extend_from_slice(&(i as i16).to_be_bytes());
        }
        -2_147_483_648..=2_147_483_647 => {
            out.push(0xCA);
            out.extend_from_slice(&(i as i32).to_be_bytes());
        }
        _ => {
            out.push(0xCB);
            out.extend_from_slice(&i.to_be_bytes());
        }
    }
}

/// Tiny marker for small sizes, then the 8/16/32-bit forms that follow it.
fn encode_header(len: usize, tiny: u8, base: u8, out: &mut Vec<u8>) {
    if len < 16 {
        out.push(tiny | len as u8);
    } else if len < 256 {
        out.push(base);
        out.push(len as u8);
    } else if len < 65_536 {
        out.push(base + 1);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(base + 2);
        out.extend_from_slice(&(len as u32).to_be_bytes());
    }
}

// ---------------------------------------------------------------- decoding

pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    fn byte(&mut self) -> io::Result<u8> {
        let b = *self
            .buf
            .get(self.pos)
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "packstream: short read"))?;
        self.pos += 1;
        Ok(b)
    }

    fn bytes(&mut self, n: usize) -> io::Result<&'a [u8]> {
        let end = self.pos + n;
        if end > self.buf.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "packstream: short read"));
        }
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn size(&mut self, marker: u8, base: u8) -> io::Result<usize> {
        Ok(match marker - base {
            0 => self.byte()? as usize,
            1 => u16::from_be_bytes(self.bytes(2)?.try_into().unwrap()) as usize,
            _ => u32::from_be_bytes(self.bytes(4)?.try_into().unwrap()) as usize,
        })
    }

    pub fn value(&mut self) -> io::Result<Value> {
        let m = self.byte()?;
        Ok(match m {
            0xC0 => Value::Null,
            0xC2 => Value::Bool(false),
            0xC3 => Value::Bool(true),
            0xC1 => Value::Float(f64::from_be_bytes(self.bytes(8)?.try_into().unwrap())),
            0xC8 => Value::Int(self.byte()? as i8 as i64),
            0xC9 => Value::Int(i16::from_be_bytes(self.bytes(2)?.try_into().unwrap()) as i64),
            0xCA => Value::Int(i32::from_be_bytes(self.bytes(4)?.try_into().unwrap()) as i64),
            0xCB => Value::Int(i64::from_be_bytes(self.bytes(8)?.try_into().unwrap())),
            0x00..=0x7F => Value::Int(m as i64),
            0xF0..=0xFF => Value::Int(m as i8 as i64),
            0x80..=0x8F => self.string(m as usize & 0x0F)?,
            0xD0..=0xD2 => {
                let n = self.size(m, 0xD0)?;
                self.string(n)?
            }
            0x90..=0x9F => self.list(m as usize & 0x0F)?,
            0xD4..=0xD6 => {
                let n = self.size(m, 0xD4)?;
                self.list(n)?
            }
            0xA0..=0xAF => self.dict(m as usize & 0x0F)?,
            0xD8..=0xDA => {
                let n = self.size(m, 0xD8)?;
                self.dict(n)?
            }
            0xB0..=0xBF => {
                let n = m as usize & 0x0F;
                let sig = self.byte()?;
                let mut fields = Vec::with_capacity(n);
                for _ in 0..n {
                    fields.push(self.value()?);
                }
                Value::Struct(sig, fields)
            }
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("packstream: unknown marker 0x{other:02X}"),
                ))
            }
        })
    }

    fn string(&mut self, n: usize) -> io::Result<Value> {
        let b = self.bytes(n)?;
        Ok(Value::String(String::from_utf8_lossy(b).into_owned()))
    }

    fn list(&mut self, n: usize) -> io::Result<Value> {
        let mut xs = Vec::with_capacity(n);
        for _ in 0..n {
            xs.push(self.value()?);
        }
        Ok(Value::List(xs))
    }

    fn dict(&mut self, n: usize) -> io::Result<Value> {
        let mut m = BTreeMap::new();
        for _ in 0..n {
            let k = match self.value()? {
                Value::String(s) => s,
                other => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("packstream: non-string dictionary key {other:?}"),
                    ))
                }
            };
            m.insert(k, self.value()?);
        }
        Ok(Value::Map(m))
    }
}

// ------------------------------------------------------------- chunking

/// Bolt frames a message as chunks of at most 65_535 bytes, ended by a zero
/// chunk. Chunk boundaries are unrelated to message boundaries (FR-003).
pub fn write_message(w: &mut impl Write, msg: &Value) -> io::Result<()> {
    let mut body = Vec::new();
    encode(msg, &mut body);
    for chunk in body.chunks(65_535) {
        w.write_all(&(chunk.len() as u16).to_be_bytes())?;
        w.write_all(chunk)?;
    }
    w.write_all(&[0, 0])?;
    w.flush()
}

/// Read one message, reassembling however many chunks it arrived in.
pub fn read_message(r: &mut impl Read) -> io::Result<Value> {
    let mut body = Vec::new();
    loop {
        let mut hdr = [0u8; 2];
        r.read_exact(&mut hdr)?;
        let n = u16::from_be_bytes(hdr) as usize;
        if n == 0 {
            if body.is_empty() {
                continue; // a no-op chunk between messages
            }
            break;
        }
        let start = body.len();
        body.resize(start + n, 0);
        r.read_exact(&mut body[start..])?;
    }
    Reader::new(&body).value()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(v: Value) {
        let mut buf = Vec::new();
        encode(&v, &mut buf);
        let got = Reader::new(&buf).value().expect("decode");
        assert_eq!(v, got, "round trip failed for {v:?}");
    }

    #[test]
    fn scalars_round_trip() {
        round_trip(Value::Null);
        round_trip(Value::Bool(true));
        round_trip(Value::Bool(false));
        round_trip(Value::Float(3.5));
        round_trip(string("hello"));
        round_trip(string(""));
    }

    #[test]
    fn every_integer_width_round_trips() {
        for i in [0i64, -16, 127, -17, -128, 200, -32_768, 32_767, 100_000, i64::MIN, i64::MAX] {
            round_trip(Value::Int(i));
        }
    }

    #[test]
    fn sizes_cross_every_header_boundary() {
        for n in [0usize, 15, 16, 255, 256, 70_000] {
            round_trip(string("x".repeat(n)));
            round_trip(Value::List(vec![Value::Int(1); n]));
        }
    }

    #[test]
    fn structures_and_maps_nest() {
        round_trip(Value::Struct(
            0x4E,
            vec![
                Value::Int(42),
                Value::List(vec![string("Movie")]),
                map(vec![("title", string("The Matrix")), ("released", Value::Int(1999))]),
            ],
        ));
    }

    #[test]
    fn a_message_survives_chunking() {
        let msg = Value::Struct(0x71, vec![Value::List(vec![string("y".repeat(200_000))])]);
        let mut wire = Vec::new();
        write_message(&mut wire, &msg).expect("write");
        // more than one chunk, and the terminator is there
        assert!(wire.len() > 65_535 * 3);
        let got = read_message(&mut wire.as_slice()).expect("read");
        assert_eq!(msg, got);
    }

    #[test]
    fn utf8_survives() {
        round_trip(string("한글 · émoji 🎬"));
    }
}
