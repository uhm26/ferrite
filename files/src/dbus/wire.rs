//! Marshalling: `Value` ⇄ bytes with D-Bus alignment rules.
//!
//! Offsets count from the buffer start, which must sit at an 8-aligned message offset
//! (true for the header and for the body).

use super::Error;
use super::types::{MAX_DEPTH, Type, parse_sig, parse_single, valid_path};
use super::value::Value;

/// Max array payload (spec: 64 MiB).
pub const MAX_ARRAY: usize = 64 << 20;

const TRUNCATED: Error = Error::Malformed("truncated");

/// Little-endian encoder.
#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
    depth: usize,
}

impl Writer {
    pub fn pad(&mut self, align: usize) {
        while !self.buf.len().is_multiple_of(align) {
            self.buf.push(0);
        }
    }

    fn num(&mut self, align: usize, bytes: &[u8]) {
        self.pad(align);
        self.buf.extend_from_slice(bytes);
    }

    /// Appends `v`, which must be of type `ty`.
    pub fn put(&mut self, ty: &Type, v: &Value) -> Result<(), Error> {
        match (ty, v) {
            (Type::Byte, Value::Byte(x)) => self.buf.push(*x),
            (Type::Bool, Value::Bool(x)) => self.num(4, &u32::from(*x).to_le_bytes()),
            (Type::I16, Value::I16(x)) => self.num(2, &x.to_le_bytes()),
            (Type::U16, Value::U16(x)) => self.num(2, &x.to_le_bytes()),
            (Type::I32, Value::I32(x)) => self.num(4, &x.to_le_bytes()),
            (Type::U32, Value::U32(x)) => self.num(4, &x.to_le_bytes()),
            (Type::I64, Value::I64(x)) => self.num(8, &x.to_le_bytes()),
            (Type::U64, Value::U64(x)) => self.num(8, &x.to_le_bytes()),
            (Type::F64, Value::F64(x)) => self.num(8, &x.to_le_bytes()),
            (Type::Str, Value::Str(s)) => self.string(s)?,
            (Type::Path, Value::Path(s)) if valid_path(s) => self.string(s)?,
            (Type::Path, Value::Path(_)) => return Err(Error::Invalid("bad object path")),
            (Type::Sig, Value::Sig(s)) => self.sig(s)?,
            (Type::Variant, Value::Variant(inner)) => self.variant(inner)?,
            (Type::Array(t), Value::Bytes(b)) if **t == Type::Byte => self.array(1, |w| {
                w.buf.extend_from_slice(b);
                Ok(())
            })?,
            (Type::Array(t), Value::Array(et, items)) if **t == *et => {
                self.array(t.align(), |w| items.iter().try_for_each(|i| w.put(t, i)))?
            }
            (Type::Dict(k, vt), Value::Dict(dk, dv, entries)) if **k == *dk && **vt == *dv => self
                .array(8, |w| {
                    entries.iter().try_for_each(|(a, b)| {
                        w.pad(8);
                        w.put(k, a)?;
                        w.put(vt, b)
                    })
                })?,
            (Type::Struct(ts), Value::Struct(vs)) if ts.len() == vs.len() => {
                self.pad(8);
                self.nested(|w| ts.iter().zip(vs).try_for_each(|(t, v)| w.put(t, v)))?
            }
            _ => return Err(Error::Invalid("value does not match its signature")),
        }
        Ok(())
    }

    fn string(&mut self, s: &str) -> Result<(), Error> {
        if s.contains('\0') {
            return Err(Error::Invalid("string contains NUL"));
        }
        let len = u32::try_from(s.len()).map_err(|_| Error::Invalid("string too long"))?;
        self.num(4, &len.to_le_bytes());
        self.buf.extend_from_slice(s.as_bytes());
        self.buf.push(0);
        Ok(())
    }

    fn sig(&mut self, s: &str) -> Result<(), Error> {
        parse_sig(s).map_err(|_| Error::Invalid("bad signature"))?;
        self.buf.push(s.len() as u8); // ≤ 255, checked by parse_sig
        self.buf.extend_from_slice(s.as_bytes());
        self.buf.push(0);
        Ok(())
    }

    fn variant(&mut self, v: &Value) -> Result<(), Error> {
        let ty = v.ty();
        let mut sig = String::new();
        ty.write_sig(&mut sig);
        self.sig(&sig)?;
        self.nested(|w| w.put(&ty, v))
    }

    /// Length, padding to the element alignment (even when empty), elements.
    fn array(
        &mut self,
        align: usize,
        items: impl FnOnce(&mut Self) -> Result<(), Error>,
    ) -> Result<(), Error> {
        self.pad(4);
        let at = self.buf.len();
        self.buf.extend_from_slice(&[0; 4]);
        self.pad(align);
        let start = self.buf.len();
        self.nested(items)?;
        let len = self.buf.len() - start;
        if len > MAX_ARRAY {
            return Err(Error::Invalid("array longer than 64 MiB"));
        }
        self.buf[at..at + 4].copy_from_slice(&(len as u32).to_le_bytes());
        Ok(())
    }

    fn nested(&mut self, f: impl FnOnce(&mut Self) -> Result<(), Error>) -> Result<(), Error> {
        if self.depth >= MAX_DEPTH {
            return Err(Error::Invalid("value nested too deep"));
        }
        self.depth += 1;
        let r = f(self);
        self.depth -= 1;
        r
    }
}

// shortcut: a decoded `Value` is 72 bytes, so `av`/`ab` bodies grow up to ~36x their wire
// size (a 128 MiB message → GiBs); fine for udisks/portal replies, add a lower per-message
// cap in `frame_len` if untrusted peers ever send us big messages.
/// Decoder for either byte order. Every length is bounds-checked; bad input gives `Err`.
pub struct Reader<'a> {
    buf: &'a [u8],
    pub pos: usize,
    big_endian: bool,
    depth: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8], big_endian: bool) -> Self {
        Reader {
            buf,
            pos: 0,
            big_endian,
            depth: 0,
        }
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.buf.len())
            .ok_or(TRUNCATED)?;
        let bytes = &self.buf[self.pos..end];
        self.pos = end;
        Ok(bytes)
    }

    /// Skips padding, which must be zero.
    pub fn align(&mut self, align: usize) -> Result<(), Error> {
        let pad = (align - self.pos % align) % align;
        if self.take(pad)?.iter().any(|&b| b != 0) {
            return Err(Error::Malformed("nonzero padding"));
        }
        Ok(())
    }

    /// N aligned bytes, flipped to little endian.
    fn fixed<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        self.align(N)?;
        let mut a: [u8; N] = self.take(N)?.try_into().map_err(|_| TRUNCATED)?;
        if self.big_endian {
            a.reverse();
        }
        Ok(a)
    }

    pub fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.fixed()?))
    }

    /// Reads one value of type `ty`.
    pub fn get(&mut self, ty: &Type) -> Result<Value, Error> {
        Ok(match ty {
            Type::Byte => Value::Byte(u8::from_le_bytes(self.fixed()?)),
            Type::Bool => match self.u32()? {
                0 => Value::Bool(false),
                1 => Value::Bool(true),
                _ => return Err(Error::Malformed("bool is not 0 or 1")),
            },
            Type::I16 => Value::I16(i16::from_le_bytes(self.fixed()?)),
            Type::U16 => Value::U16(u16::from_le_bytes(self.fixed()?)),
            Type::I32 => Value::I32(i32::from_le_bytes(self.fixed()?)),
            Type::U32 => Value::U32(self.u32()?),
            Type::I64 => Value::I64(i64::from_le_bytes(self.fixed()?)),
            Type::U64 => Value::U64(u64::from_le_bytes(self.fixed()?)),
            Type::F64 => Value::F64(f64::from_le_bytes(self.fixed()?)),
            Type::Str => Value::Str(self.string()?),
            Type::Path => Value::Path(
                Some(self.string()?)
                    .filter(|p| valid_path(p))
                    .ok_or(Error::Malformed("bad object path"))?,
            ),
            Type::Sig => Value::Sig(self.sig()?),
            Type::Variant => {
                let t = parse_single(&self.sig()?)?;
                Value::variant(self.nested(|r| r.get(&t))?)
            }
            Type::Array(t) if **t == Type::Byte => {
                let n = self.array_len(1)?;
                Value::Bytes(self.take(n)?.to_vec())
            }
            Type::Array(t) => Value::Array((**t).clone(), self.array(t.align(), |r| r.get(t))?),
            Type::Dict(k, v) => {
                let entries = self.array(8, |r| {
                    r.align(8)?;
                    Ok((r.get(k)?, r.get(v)?))
                })?;
                Value::Dict((**k).clone(), (**v).clone(), entries)
            }
            Type::Struct(ts) => {
                self.align(8)?;
                Value::Struct(self.nested(|r| ts.iter().map(|t| r.get(t)).collect())?)
            }
        })
    }

    fn string(&mut self) -> Result<String, Error> {
        let n = self.u32()? as usize;
        self.text(n)
    }

    fn sig(&mut self) -> Result<String, Error> {
        let n = usize::from(self.take(1)?[0]);
        let s = self.text(n)?;
        parse_sig(&s)?;
        Ok(s)
    }

    /// `n` bytes of UTF-8 followed by a NUL, none inside.
    fn text(&mut self, n: usize) -> Result<String, Error> {
        let bytes = self.take(n)?;
        if self.take(1)? != [0] || bytes.contains(&0) {
            return Err(Error::Malformed(
                "string not NUL-terminated or has NUL inside",
            ));
        }
        String::from_utf8(bytes.to_vec()).map_err(|_| Error::Malformed("string is not UTF-8"))
    }

    /// Array length, then padding to the element alignment (present even when empty).
    fn array_len(&mut self, align: usize) -> Result<usize, Error> {
        let n = self.u32()? as usize;
        if n > MAX_ARRAY {
            return Err(Error::Malformed("array longer than 64 MiB"));
        }
        self.align(align)?;
        Ok(n)
    }

    fn array<T>(
        &mut self,
        align: usize,
        mut item: impl FnMut(&mut Self) -> Result<T, Error>,
    ) -> Result<Vec<T>, Error> {
        let n = self.array_len(align)?;
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.buf.len())
            .ok_or(TRUNCATED)?;
        self.nested(|r| {
            let mut out = Vec::new();
            while r.pos < end {
                out.push(item(r)?); // every element takes ≥ 1 byte, so this ends
            }
            if r.pos != end {
                return Err(Error::Malformed("array element crosses the array end"));
            }
            Ok(out)
        })
    }

    fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, Error>) -> Result<T, Error> {
        if self.depth >= MAX_DEPTH {
            return Err(Error::Malformed("value nested too deep"));
        }
        self.depth += 1;
        let r = f(self);
        self.depth -= 1;
        r
    }
}
