//! The decoded AD2 value model, plus display helpers.

#[derive(PartialEq, Debug, Clone)]
pub enum Value {
    Nil,
    Bool(bool),
    Number(f64),
    Vector(f64, f64, f64),
    Angle(f64, f64, f64),
    // Raw bytes, never String. Lua strings carry no encoding guarantee and
    // players put arbitrary bytes in entity names; from_utf8_lossy would
    // rewrite those and break the round trip.
    Str(Vec<u8>),
    // An arena index. Note there is no separate Ref variant: AD2 decides
    // between "write the table" and "write a back-reference" purely by
    // whether the encoder has reached this table before. That's a property
    // of the traversal, not of the data, so it is not stored here.
    Table(usize),
}

#[derive(PartialEq, Debug, Clone)]
pub enum Node {
    Table(Vec<(Value, Value)>), // keyed, type byte 255
    Array(Vec<Value>),          // positional, type byte 254
}

pub fn table_index(v: &Value) -> Option<usize> {
    match v {
        Value::Table(i) => Some(*i),
        _ => None,
    }
}

pub fn key_matches(k: &Value, name: &str) -> bool {
    matches!(k, Value::Str(s) if s.as_slice() == name.as_bytes())
}

pub fn escape(bytes: &[u8]) -> String {
    let mut s = String::new();
    for &b in bytes {
        match b {
            b'\r' => s.push_str("\\r"),
            b'\n' => s.push_str("\\n"),
            b'\t' => s.push_str("\\t"),
            0x20..=0x7E => s.push(b as char),
            _ => s.push_str(&format!("\\x{b:02X}")),
        }
    }
    s
}

pub fn hexline(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X} ")).collect()
}

pub fn short(v: &Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => format!("{n}"),
        Value::Vector(x, y, z) => format!("Vector({x:.3}, {y:.3}, {z:.3})"),
        Value::Angle(p, y, r) => format!("Angle({p:.3}, {y:.3}, {r:.3})"),
        Value::Str(b) => format!("\"{}\"", escape(b)),
        // t3 is an arena id, deliberately not called "table #3" any more —
        // AD2's numbering is assigned at encode time and the two are no
        // longer guaranteed to line up after an edit.
        Value::Table(i) => format!("t{i}"),
    }
}

/// `seen` guards against re-expanding a shared table and against cycles,
/// which matters now that Ref is gone and every occurrence looks identical.
pub fn dump(
    v: &Value,
    arena: &[Node],
    depth: usize,
    max_depth: usize,
    max_items: usize,
    seen: &mut Vec<bool>,
) {
    let pad = "  ".repeat(depth);

    let Some(index) = table_index(v) else {
        println!("{pad}{}", short(v));
        return;
    };

    if seen[index] {
        println!("{pad}t{index} (already shown)");
        return;
    }
    if depth >= max_depth {
        println!("{pad}t{index} (...)");
        return;
    }
    seen[index] = true;

    match &arena[index] {
        Node::Table(entries) => {
            println!("{pad}t{index}: {} keys", entries.len());
            for (k, val) in entries.iter().take(max_items) {
                if table_index(val).is_some() {
                    println!("{pad}  {} =", short(k));
                    dump(val, arena, depth + 2, max_depth, max_items, seen);
                } else {
                    println!("{pad}  {} = {}", short(k), short(val));
                }
            }
            if entries.len() > max_items {
                println!("{pad}  ... {} more", entries.len() - max_items);
            }
        }
        Node::Array(items) => {
            println!("{pad}t{index}: {} items", items.len());
            for item in items.iter().take(max_items) {
                dump(item, arena, depth + 1, max_depth, max_items, seen);
            }
            if items.len() > max_items {
                println!("{pad}  ... {} more", items.len() - max_items);
            }
        }
    }
}
