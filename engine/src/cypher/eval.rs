//! Expression evaluation for the write path.
//!
//! Read queries compile expressions into SQL. Write clauses (`CREATE`, `SET`,
//! `MERGE`) need values *in Rust* to build property payloads, so they get this
//! small evaluator over already-bound rows.

use super::ast::*;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub type Env = HashMap<String, Value>;

pub fn eval(e: &Expr, env: &Env, params: &Value) -> Result<Value, String> {
    Ok(match e {
        Expr::Lit(Lit::Int(i)) => json!(i),
        Expr::Lit(Lit::Float(f)) => json!(f),
        Expr::Lit(Lit::Str(s)) => json!(s),
        Expr::Lit(Lit::Bool(b)) => json!(b),
        Expr::Lit(Lit::Null) => Value::Null,
        Expr::Param(p) => params.get(p).cloned().unwrap_or(Value::Null),
        Expr::Var(v) => env.get(v).cloned().unwrap_or(Value::Null),
        Expr::Prop(base, p) => {
            let b = eval(base, env, params)?;
            b.get(p).cloned().unwrap_or(Value::Null)
        }
        Expr::List(xs) => {
            Value::Array(xs.iter().map(|x| eval(x, env, params)).collect::<Result<_, _>>()?)
        }
        Expr::Map(kv) => {
            let mut m = Map::new();
            for (k, v) in kv {
                m.insert(k.clone(), eval(v, env, params)?);
            }
            Value::Object(m)
        }
        Expr::Neg(a) => match eval(a, env, params)? {
            Value::Number(n) => n
                .as_f64()
                .map(|f| json!(-f))
                .unwrap_or(Value::Null),
            _ => Value::Null,
        },
        Expr::Not(a) => json!(!truthy(&eval(a, env, params)?)),
        Expr::IsNull(a, want) => json!(eval(a, env, params)?.is_null() == *want),
        // 라벨 판정은 타입 카탈로그가 필요하므로 상수 평가 경로에서는 답할 수 없다.
        Expr::HasLabel(..) => return Err("label predicate is not constant-foldable".into()),
        Expr::PatternPred(..) => return Err("pattern predicate is not constant-foldable".into()),
        Expr::Binary(op, l, r) => {
            let a = eval(l, env, params)?;
            let b = eval(r, env, params)?;
            binary(*op, &a, &b)
        }
        Expr::Case { operand, whens, else_ } => {
            let subject = match operand {
                Some(o) => Some(eval(o, env, params)?),
                None => None,
            };
            for (cond, val) in whens {
                let hit = match &subject {
                    Some(s) => *s == eval(cond, env, params)?,
                    None => truthy(&eval(cond, env, params)?),
                };
                if hit {
                    return eval(val, env, params);
                }
            }
            match else_ {
                Some(e) => eval(e, env, params)?,
                None => Value::Null,
            }
        }
        Expr::Func { name, args, .. } => {
            let a: Vec<Value> =
                args.iter().map(|x| eval(x, env, params)).collect::<Result<_, _>>()?;
            func(name, &a)?
        }
        // The write path evaluates over already-bound rows, so a list
        // comprehension here has a real list in hand and can just be folded.
        Expr::ListComp { var, source, filter, project } => {
            let src = eval(source, env, params)?;
            let Value::Array(items) = src else { return Ok(Value::Array(vec![])) };
            // The binder is scoped to the comprehension, and `eval` reads a
            // shared environment, so bind it in a copy rather than reaching
            // into the caller's row.
            let mut local = env.clone();
            let mut out = Vec::new();
            for item in items {
                local.insert(var.clone(), item.clone());
                let keep = match filter {
                    Some(f) => truthy(&eval(f, &local, params)?),
                    None => true,
                };
                if keep {
                    out.push(match project {
                        Some(p) => eval(p, &local, params)?,
                        None => item,
                    });
                }
            }
            Value::Array(out)
        }
        Expr::ListPred { kind, var, source, filter } => {
            let src = eval(source, env, params)?;
            let items = match src {
                Value::Array(xs) => xs,
                _ => vec![],
            };
            let mut local = env.clone();
            let mut hits = 0usize;
            let total = items.len();
            for item in items {
                local.insert(var.clone(), item);
                if truthy(&eval(filter, &local, params)?) {
                    hits += 1;
                }
            }
            json!(match kind {
                ListPredKind::Any => hits > 0,
                ListPredKind::None => hits == 0,
                ListPredKind::Single => hits == 1,
                ListPredKind::All => hits == total,
            })
        }
        Expr::MapProjection { var, items } => {
            let subject = env.get(var).cloned().unwrap_or(Value::Null);
            let mut out = Map::new();
            for item in items {
                match item {
                    MapProjItem::All => {
                        if let Value::Object(o) = &subject {
                            for (k, v) in o {
                                // The identity fields are ours, not the user's
                                // properties — `.*` must not hand them back.
                                if !k.starts_with('_') {
                                    out.insert(k.clone(), v.clone());
                                }
                            }
                        }
                    }
                    MapProjItem::Prop(p) => {
                        out.insert(p.clone(), subject.get(p).cloned().unwrap_or(Value::Null));
                    }
                    MapProjItem::Entry(k, e) => {
                        out.insert(k.clone(), eval(e, env, params)?);
                    }
                }
            }
            Value::Object(out)
        }
        Expr::Star => Value::Null,
    })
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Null => false,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn binary(op: BinOp, a: &Value, b: &Value) -> Value {
    use BinOp::*;
    match op {
        Add => match (num(a), num(b)) {
            (Some(x), Some(y)) => json!(x + y),
            _ => json!(format!("{}{}", as_str(a), as_str(b))),
        },
        Sub | Mul | Div | Mod | Pow => match (num(a), num(b)) {
            (Some(x), Some(y)) => json!(match op {
                Sub => x - y,
                Mul => x * y,
                Div => {
                    if y == 0.0 {
                        return Value::Null;
                    } else {
                        x / y
                    }
                }
                Mod => {
                    if y == 0.0 {
                        return Value::Null;
                    } else {
                        x % y
                    }
                }
                _ => x.powf(y),
            }),
            _ => Value::Null,
        },
        Eq => json!(a == b),
        Ne => json!(a != b),
        Lt | Le | Gt | Ge => match (num(a), num(b)) {
            (Some(x), Some(y)) => json!(match op {
                Lt => x < y,
                Le => x <= y,
                Gt => x > y,
                _ => x >= y,
            }),
            _ => {
                let (x, y) = (as_str(a), as_str(b));
                json!(match op {
                    Lt => x < y,
                    Le => x <= y,
                    Gt => x > y,
                    _ => x >= y,
                })
            }
        },
        And => json!(truthy(a) && truthy(b)),
        Or => json!(truthy(a) || truthy(b)),
        Xor => json!(truthy(a) != truthy(b)),
        Concat => json!(format!("{}{}", as_str(a), as_str(b))),
        StartsWith => json!(as_str(a).starts_with(&as_str(b))),
        EndsWith => json!(as_str(a).ends_with(&as_str(b))),
        Contains => json!(as_str(a).contains(&as_str(b))),
        Regex => json!(false),
        In => match b {
            Value::Array(xs) => json!(xs.contains(a)),
            _ => json!(false),
        },
    }
}

fn as_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Transaction time as ISO-8601, read from PostgreSQL rather than the wall
/// clock so a write clause and the surrounding SQL agree on "now".
fn now_iso() -> String {
    crate::spiu::one::<String>("SELECT now()::text", &[])
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn now_epoch() -> i64 {
    crate::spiu::one::<i64>("SELECT extract(epoch from now())::int8", &[])
        .ok()
        .flatten()
        .unwrap_or(0)
}

fn func(name: &str, a: &[Value]) -> Result<Value, String> {
    Ok(match name.to_ascii_lowercase().as_str() {
        "toupper" | "upper" => json!(as_str(&a[0]).to_uppercase()),
        "tolower" | "lower" => json!(as_str(&a[0]).to_lowercase()),
        "trim" => json!(as_str(&a[0]).trim()),
        "tostring" => json!(as_str(&a[0])),
        "tointeger" => num(&a[0]).map(|f| json!(f as i64)).unwrap_or(Value::Null),
        "tofloat" => num(&a[0]).map(|f| json!(f)).unwrap_or(Value::Null),
        "coalesce" => a.iter().find(|v| !v.is_null()).cloned().unwrap_or(Value::Null),
        "size" | "length" => match &a[0] {
            Value::Array(xs) => json!(xs.len()),
            Value::String(s) => json!(s.chars().count()),
            _ => Value::Null,
        },
        "abs" => num(&a[0]).map(|f| json!(f.abs())).unwrap_or(Value::Null),
        "id" => a[0].get("_id").cloned().unwrap_or(Value::Null),
        "elementid" => a[0]
            .get("_id")
            .map(|v| json!(v.to_string()))
            .unwrap_or(Value::Null),
        "type" => a[0].get("_type").cloned().unwrap_or(Value::Null),
        // Cypher's labels() is a list. On the write path the bound row carries
        // only the concrete type name, so the list is that one name — enough for
        // `x IN labels(n)` inside ON MATCH SET, which is where this is reached.
        "labels" => match a[0].get("_type") {
            Some(t) => json!([t]),
            None => Value::Null,
        },
        "keys" => match &a[0] {
            Value::Object(o) => json!(o.keys().collect::<Vec<_>>()),
            _ => Value::Null,
        },
        // `SET n.created_at = datetime()` is the single most common write-clause
        // function there is. Returning null here — which is what happened before
        // — loses the value silently, which is worse than refusing it.
        "timestamp" => json!(now_epoch()),
        "datetime" => json!(now_iso()),
        other => return Err(format!("function '{other}' is not available in a write clause")),
    })
}
