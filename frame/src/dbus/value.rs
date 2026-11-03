//! `Value`: one D-Bus value, with readers that see through variants.

use super::types::Type;

/// One D-Bus value. Containers carry their element types so empty ones still have a signature.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Byte(u8),
    Bool(bool),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    F64(f64),
    Str(String),
    Path(String),
    Sig(String),
    /// `ay`, kept compact (udisks sends paths as byte strings). Decoding `ay` always gives this.
    Bytes(Vec<u8>),
    /// Any other array; the `Type` is the element type.
    Array(Type, Vec<Value>),
    /// `a{kv}`: key type, value type, entries in wire order.
    Dict(Type, Type, Vec<(Value, Value)>),
    Struct(Vec<Value>),
    Variant(Box<Value>),
}

impl Value {
    pub fn str(s: &str) -> Value {
        Value::Str(s.to_string())
    }

    pub fn path(s: &str) -> Value {
        Value::Path(s.to_string())
    }

    pub fn variant(v: Value) -> Value {
        Value::Variant(Box::new(v))
    }

    /// `a{sv}` from pairs; values get wrapped in variants. `dict_sv(vec![])` = empty options.
    pub fn dict_sv(pairs: Vec<(&str, Value)>) -> Value {
        let entries = pairs
            .into_iter()
            .map(|(k, v)| (Value::str(k), Value::variant(v)));
        Value::Dict(Type::Str, Type::Variant, entries.collect())
    }

    /// The type of this value (signature source when sending).
    pub fn ty(&self) -> Type {
        match self {
            Value::Byte(_) => Type::Byte,
            Value::Bool(_) => Type::Bool,
            Value::I16(_) => Type::I16,
            Value::U16(_) => Type::U16,
            Value::I32(_) => Type::I32,
            Value::U32(_) => Type::U32,
            Value::I64(_) => Type::I64,
            Value::U64(_) => Type::U64,
            Value::F64(_) => Type::F64,
            Value::Str(_) => Type::Str,
            Value::Path(_) => Type::Path,
            Value::Sig(_) => Type::Sig,
            Value::Bytes(_) => Type::Array(Box::new(Type::Byte)),
            Value::Array(t, _) => Type::Array(Box::new(t.clone())),
            Value::Dict(k, v, _) => Type::Dict(Box::new(k.clone()), Box::new(v.clone())),
            Value::Struct(fields) => Type::Struct(fields.iter().map(Value::ty).collect()),
            Value::Variant(_) => Type::Variant,
        }
    }

    /// This value with any variant layers removed.
    pub fn inner(&self) -> &Value {
        let mut v = self;
        while let Value::Variant(inner) = v {
            v = inner;
        }
        v
    }

    /// `s`, `o` or `g`.
    pub fn as_str(&self) -> Option<&str> {
        match self.inner() {
            Value::Str(s) | Value::Path(s) | Value::Sig(s) => Some(s),
            _ => None,
        }
    }

    /// Any unsigned integer (`y q u t`).
    pub fn as_u64(&self) -> Option<u64> {
        match *self.inner() {
            Value::Byte(n) => Some(n.into()),
            Value::U16(n) => Some(n.into()),
            Value::U32(n) => Some(n.into()),
            Value::U64(n) => Some(n),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match *self.inner() {
            Value::Bool(b) => Some(b),
            _ => None,
        }
    }

    /// `ay` (as decoded; a hand-built `Array(Type::Byte, ..)` gives `None`).
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self.inner() {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self.inner() {
            Value::Array(_, items) => Some(items),
            _ => None,
        }
    }

    pub fn as_struct(&self) -> Option<&[Value]> {
        match self.inner() {
            Value::Struct(fields) => Some(fields),
            _ => None,
        }
    }

    pub fn as_dict(&self) -> Option<&[(Value, Value)]> {
        match self.inner() {
            Value::Dict(_, _, entries) => Some(entries),
            _ => None,
        }
    }

    /// Looks up a string or path key in a dict; the result has variants removed.
    /// Chains: `objects.get(path)?.get(iface)?.get("Size")?.as_u64()`.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_dict()?
            .iter()
            .find(|(k, _)| matches!(k, Value::Str(s) | Value::Path(s) if s == key))
            .map(|(_, v)| v.inner())
    }
}
