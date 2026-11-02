//! Type signatures: "a{sv}" ⇄ `Type` tree, plus the object path rule.

use super::Error;

/// Max container nesting (arrays, structs, variants); the spec allows 32 + 32.
pub const MAX_DEPTH: usize = 64;

/// One complete D-Bus type. `Dict` is a whole `a{kv}`.
#[derive(Clone, Debug, PartialEq)]
pub enum Type {
    Byte,
    Bool,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F64,
    Str,
    Path,
    Sig,
    Variant,
    Array(Box<Type>),
    Dict(Box<Type>, Box<Type>),
    Struct(Vec<Type>),
}

const CODES: [(u8, Type); 13] = [
    (b'y', Type::Byte),
    (b'b', Type::Bool),
    (b'n', Type::I16),
    (b'q', Type::U16),
    (b'i', Type::I32),
    (b'u', Type::U32),
    (b'x', Type::I64),
    (b't', Type::U64),
    (b'd', Type::F64),
    (b's', Type::Str),
    (b'o', Type::Path),
    (b'g', Type::Sig),
    (b'v', Type::Variant),
];

impl Type {
    /// Wire alignment in bytes.
    pub fn align(&self) -> usize {
        match self {
            Type::Byte | Type::Sig | Type::Variant => 1,
            Type::I16 | Type::U16 => 2,
            Type::Bool | Type::I32 | Type::U32 | Type::Str | Type::Path => 4,
            Type::Array(_) | Type::Dict(..) => 4,
            Type::I64 | Type::U64 | Type::F64 | Type::Struct(_) => 8,
        }
    }

    /// Appends this type's signature to `out`.
    pub fn write_sig(&self, out: &mut String) {
        match self {
            Type::Array(t) => {
                out.push('a');
                t.write_sig(out);
            }
            Type::Dict(k, v) => {
                out.push_str("a{");
                k.write_sig(out);
                v.write_sig(out);
                out.push('}');
            }
            Type::Struct(fields) => {
                out.push('(');
                fields.iter().for_each(|t| t.write_sig(out));
                out.push(')');
            }
            basic => {
                if let Some((c, _)) = CODES.iter().find(|(_, t)| t == basic) {
                    out.push(char::from(*c));
                }
            }
        }
    }

    /// Basic types may be dict keys; containers and variants may not.
    fn is_basic(&self) -> bool {
        !matches!(
            self,
            Type::Variant | Type::Array(_) | Type::Dict(..) | Type::Struct(_)
        )
    }
}

/// Parses a signature into its complete types (a message body may hold several).
pub fn parse_sig(sig: &str) -> Result<Vec<Type>, Error> {
    let b = sig.as_bytes();
    if b.len() > 255 {
        return Err(Error::Malformed("signature longer than 255 bytes"));
    }
    let mut pos = 0;
    let mut out = Vec::new();
    while pos < b.len() {
        out.push(parse_one(b, &mut pos, 0)?);
    }
    Ok(out)
}

/// Parses a signature that must be exactly one complete type (variants).
pub fn parse_single(sig: &str) -> Result<Type, Error> {
    let mut types = parse_sig(sig)?;
    match (types.pop(), types.is_empty()) {
        (Some(t), true) => Ok(t),
        _ => Err(Error::Malformed("variant signature is not one type")),
    }
}

fn parse_one(b: &[u8], pos: &mut usize, depth: usize) -> Result<Type, Error> {
    if depth >= MAX_DEPTH {
        return Err(Error::Malformed("signature nested too deep"));
    }
    let c = *b
        .get(*pos)
        .ok_or(Error::Malformed("signature ends early"))?;
    *pos += 1;
    match c {
        b'a' if b.get(*pos) == Some(&b'{') => {
            *pos += 1;
            let k = parse_one(b, pos, depth + 1)?;
            let v = parse_one(b, pos, depth + 1)?;
            if !k.is_basic() || b.get(*pos) != Some(&b'}') {
                return Err(Error::Malformed(
                    "dict entry needs a basic key and one value",
                ));
            }
            *pos += 1;
            Ok(Type::Dict(Box::new(k), Box::new(v)))
        }
        b'a' => Ok(Type::Array(Box::new(parse_one(b, pos, depth + 1)?))),
        b'(' => {
            let mut fields = Vec::new();
            while b.get(*pos) != Some(&b')') {
                fields.push(parse_one(b, pos, depth + 1)?);
            }
            *pos += 1;
            if fields.is_empty() {
                return Err(Error::Malformed("empty struct"));
            }
            Ok(Type::Struct(fields))
        }
        _ => CODES
            .iter()
            .find(|(code, _)| *code == c)
            .map(|(_, t)| t.clone())
            .ok_or(Error::Malformed("unknown type code in signature")),
    }
}

/// Object path rule: "/" or "/seg/seg" with segments of `[A-Za-z0-9_]`.
pub fn valid_path(p: &str) -> bool {
    p == "/"
        || p.strip_prefix('/').is_some_and(|rest| {
            rest.split('/').all(|seg| {
                !seg.is_empty() && seg.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
            })
        })
}
