//! The dupe as JSON, for anything that would rather not parse text.
//! Vectors and angles are tagged so the types survive: {"$vec":[x,y,z]},
//! {"$ang":[p,y,r]}. Strings are lossy UTF-8; a non-UTF-8 string (E2 code
//! with byte 163/128) is given as {"$bytes":[..]}.

use serde_json::{json, Map, Value as J};

use crate::dupe::Dupe;
use crate::value::{short, Node, Value};

pub fn value_to_json(dupe: &Dupe, v: &Value, seen: &mut Vec<bool>) -> J {
    match v {
        Value::Nil => J::Null,
        Value::Bool(b) => J::Bool(*b),
        Value::Number(n) => json!(n),
        Value::Str(b) => match std::str::from_utf8(b) {
            Ok(s) => J::String(s.to_owned()),
            Err(_) => json!({ "$bytes": b }),
        },
        Value::Vector(x, y, z) => json!({ "$vec": [x, y, z] }),
        Value::Angle(p, y, r) => json!({ "$ang": [p, y, r] }),
        Value::Table(t) => {
            if seen.get(*t).copied().unwrap_or(false) {
                return json!({ "$ref": t });
            }
            if *t < seen.len() {
                seen[*t] = true;
            }
            match &dupe.arena[*t] {
                Node::Array(items) => J::Array(items.iter().map(|i| value_to_json(dupe, i, seen)).collect()),
                Node::Table(entries) => {
                    let mut m = Map::new();
                    for (k, val) in entries {
                        m.insert(short(k).trim_matches('"').to_owned(), value_to_json(dupe, val, seen));
                    }
                    J::Object(m)
                }
            }
        }
    }
}

pub fn dupe_to_json(dupe: &Dupe) -> J {
    let mut seen = vec![false; dupe.arena.len()];
    value_to_json(dupe, &dupe.root, &mut seen)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixture_serialises_with_typed_vectors() {
        let d = crate::fixture::fixture();
        let j = dupe_to_json(&d);
        let head = &j["HeadEnt"];
        assert_eq!(head["Index"], json!(10.0));
        assert_eq!(head["Pos"]["$vec"], json!([0.0, 0.0, 0.0]));
        assert!(j["Entities"]["10"]["Class"].as_str() == Some("acf_baseplate"));
        assert!(j["Constraints"].is_array());
    }
}
