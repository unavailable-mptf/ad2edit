//! Decodes and encodes the AD2 body byte stream.
//!
//! Type bytes (revision 5):
//!   255 table  254 array  253 true  252 false  251 f64
//!   250 Vector 249 Angle  248 long string (u32 len)
//!   247 backref (signed i16, 1-based)  246 nil / terminator
//!   0 empty string  1-245 string of that length

use crate::value::{Node, Value};

pub struct Decoded {
    pub root: Value,
    pub arena: Vec<Node>,
    pub consumed: usize,
}

pub fn decode(data: &[u8]) -> Result<Decoded, String> {
    let mut r = Reader {
        data,
        pos: 0,
        arena: Vec::new(),
        numbered: Vec::new(),
    };
    let root = r.value()?;
    Ok(Decoded {
        root,
        arena: r.arena,
        consumed: r.pos,
    })
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    arena: Vec<Node>,
    // AD2 table number (1-based, in encounter order) -> arena index.
    // Needed only while reading, to resolve back-references. After decoding
    // it is thrown away, because the numbering is regenerated on write.
    numbered: Vec<usize>,
}

impl<'a> Reader<'a> {
    fn u8(&mut self) -> Result<u8, String> {
        let b = *self
            .data
            .get(self.pos)
            .ok_or_else(|| format!("unexpected end of data at offset {}", self.pos))?;
        self.pos += 1;
        Ok(b)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.pos + n;
        if end > self.data.len() {
            return Err(format!(
                "wanted {} bytes at offset {}, only {} remain",
                n,
                self.pos,
                self.data.len() - self.pos
            ));
        }
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn f64le(&mut self) -> Result<f64, String> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn u32le(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i16le(&mut self) -> Result<i16, String> {
        Ok(i16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn value(&mut self) -> Result<Value, String> {
        let tag = self.u8()?;
        match tag {
            255 => self.container(true),
            254 => self.container(false),
            253 => Ok(Value::Bool(true)),
            252 => Ok(Value::Bool(false)),
            251 => Ok(Value::Number(self.f64le()?)),
            250 => Ok(Value::Vector(self.f64le()?, self.f64le()?, self.f64le()?)),
            249 => Ok(Value::Angle(self.f64le()?, self.f64le()?, self.f64le()?)),
            248 => {
                let len = self.u32le()? as usize;
                Ok(Value::Str(self.take(len)?.to_vec()))
            }
            247 => {
                // Resolve straight to the arena index. A back-reference and a
                // definition become indistinguishable in the model, which is
                // exactly what we want.
                let n = self.i16le()?;
                if n < 1 {
                    return Err(format!("bad table reference {n}"));
                }
                let slot = n as usize - 1;
                let arena_index = *self
                    .numbered
                    .get(slot)
                    .ok_or_else(|| format!("reference to table #{n}, which hasn't been seen yet"))?;
                Ok(Value::Table(arena_index))
            }
            246 => Ok(Value::Nil),
            0 => Ok(Value::Str(Vec::new())),
            n => Ok(Value::Str(self.take(n as usize)?.to_vec())),
        }
    }

    fn container(&mut self, keyed: bool) -> Result<Value, String> {
        let index = self.arena.len();
        self.arena.push(Node::Array(Vec::new())); // placeholder
                                                  // Claim the next AD2 number BEFORE reading contents — nested tables
                                                  // must be numbered after their parent.
        self.numbered.push(index);

        let node = if keyed {
            let mut entries = Vec::new();
            loop {
                let key = self.value()?;
                if matches!(key, Value::Nil) {
                    break; // nil in key position ends the table
                }
                let val = self.value()?;
                entries.push((key, val));
            }
            Node::Table(entries)
        } else {
            let mut items = Vec::new();
            loop {
                let val = self.value()?;
                if matches!(val, Value::Nil) {
                    break;
                }
                items.push(val);
            }
            Node::Array(items)
        };

        self.arena[index] = node;
        Ok(Value::Table(index))
    }
}

/// Assigns AD2 table numbers by pre-order first encounter, mirroring the Lua
/// exactly. Because numbering is derived here rather than stored, any edit to
/// the arena — insert, delete, duplicate, reorder — produces correct
/// back-references with no separate renumbering pass.
pub fn encode(root: &Value, arena: &[Node]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut numbers: Vec<Option<i16>> = vec![None; arena.len()];
    let mut next: i32 = 0;
    write_value(root, arena, &mut out, &mut numbers, &mut next)?;
    Ok(out)
}

fn write_value(
    v: &Value,
    arena: &[Node],
    out: &mut Vec<u8>,
    numbers: &mut Vec<Option<i16>>,
    next: &mut i32,
) -> Result<(), String> {
    match v {
        Value::Nil => out.push(246),
        Value::Bool(true) => out.push(253),
        Value::Bool(false) => out.push(252),
        Value::Number(n) => {
            out.push(251);
            out.extend_from_slice(&n.to_le_bytes());
        }
        Value::Vector(x, y, z) => {
            out.push(250);
            for c in [x, y, z] {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        Value::Angle(p, y, r) => {
            out.push(249);
            for c in [p, y, r] {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        Value::Str(bytes) => {
            // < 246, matching the Lua: a 246-byte string must use the long
            // form, because 246 means nil.
            if bytes.len() < 246 {
                out.push(bytes.len() as u8);
            } else {
                out.push(248);
                out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            }
            out.extend_from_slice(bytes);
        }
        Value::Table(i) => {
            let i = *i;
            if i >= arena.len() {
                return Err(format!("value points at arena slot {i}, which doesn't exist"));
            }

            // Seen before? Emit a reference and stop.
            if let Some(n) = numbers[i] {
                out.push(247);
                out.extend_from_slice(&n.to_le_bytes());
                return Ok(());
            }

            // First encounter: claim the next number, then write the body.
            *next += 1;
            if *next > i16::MAX as i32 {
                return Err(format!(
                    "more than {} tables; AD2 writes table numbers as a signed short",
                    i16::MAX
                ));
            }
            numbers[i] = Some(*next as i16);

            match &arena[i] {
                Node::Table(entries) => {
                    out.push(255);
                    for (k, val) in entries {
                        write_value(k, arena, out, numbers, next)?;
                        write_value(val, arena, out, numbers, next)?;
                    }
                }
                Node::Array(items) => {
                    out.push(254);
                    for item in items {
                        write_value(item, arena, out, numbers, next)?;
                    }
                }
            }
            out.push(246);
        }
    }
    Ok(())
}
