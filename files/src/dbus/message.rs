//! Messages: fixed header, header fields, body; framing on the socket.

use super::Error;
use super::types::{Type, parse_sig};
use super::value::Value;
use super::wire::{MAX_ARRAY, Reader, Writer};

/// Max whole message (spec: 128 MiB).
pub const MAX_MESSAGE: usize = 128 << 20;

/// Header flag: the caller wants no reply.
pub const NO_REPLY: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Call = 1,
    Return = 2,
    Error = 3,
    Signal = 4,
}

/// One message. `serial` is set by `Conn::send`; the body signature comes from `body`.
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub kind: Kind,
    pub flags: u8,
    pub serial: u32,
    pub path: Option<String>,
    pub interface: Option<String>,
    pub member: Option<String>,
    pub error_name: Option<String>,
    pub reply_serial: Option<u32>,
    pub destination: Option<String>,
    pub sender: Option<String>,
    pub body: Vec<Value>,
}

// shortcut: interface/member/bus names are not validated before sending (the bus drops us
// on a bad one); callers pass constants, validate if names ever come from input.
impl Message {
    fn new(kind: Kind, body: Vec<Value>) -> Message {
        Message {
            kind,
            flags: 0,
            serial: 0,
            path: None,
            interface: None,
            member: None,
            error_name: None,
            reply_serial: None,
            destination: None,
            sender: None,
            body,
        }
    }

    pub fn call(dest: &str, path: &str, iface: &str, member: &str, body: Vec<Value>) -> Message {
        let mut m = Message::new(Kind::Call, body);
        m.destination = Some(dest.to_string());
        m.path = Some(path.to_string());
        m.interface = Some(iface.to_string());
        m.member = Some(member.to_string());
        m
    }

    #[cfg(test)]
    pub fn signal(path: &str, iface: &str, member: &str, body: Vec<Value>) -> Message {
        let mut m = Message::new(Kind::Signal, body);
        m.path = Some(path.to_string());
        m.interface = Some(iface.to_string());
        m.member = Some(member.to_string());
        m
    }

    /// Successful answer to the call `to`.
    pub fn method_return(to: &Message, body: Vec<Value>) -> Message {
        let mut m = Message::new(Kind::Return, body);
        m.destination = to.sender.clone();
        m.reply_serial = Some(to.serial);
        m
    }

    /// Error answer to the call `to`; `text` becomes the message body.
    pub fn error(to: &Message, name: &str, text: &str) -> Message {
        let mut m = Message::method_return(to, vec![Value::str(text)]);
        m.kind = Kind::Error;
        m.error_name = Some(name.to_string());
        m
    }

    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut body = Writer::default();
        let mut sig = String::new();
        for v in &self.body {
            let t = v.ty();
            t.write_sig(&mut sig);
            body.put(&t, v)?;
        }
        let mut w = Writer::default();
        let body_len = u32::try_from(body.buf.len()).map_err(|_| Error::Invalid("body too big"))?;
        w.buf
            .extend_from_slice(&[b'l', self.kind as u8, self.flags, 1]);
        w.buf.extend_from_slice(&body_len.to_le_bytes());
        w.buf.extend_from_slice(&self.serial.to_le_bytes());
        w.put(&field_type(), &self.fields(sig))?;
        w.pad(8);
        w.buf.extend_from_slice(&body.buf);
        if w.buf.len() > MAX_MESSAGE {
            return Err(Error::Invalid("message larger than 128 MiB"));
        }
        Ok(w.buf)
    }

    /// Header fields `a(yv)`, in code order.
    fn fields(&self, sig: String) -> Value {
        let s = |code: u8, v: &Option<String>| v.as_deref().map(|s| (code, Value::str(s)));
        let all = [
            self.path.as_deref().map(|p| (1, Value::path(p))),
            s(2, &self.interface),
            s(3, &self.member),
            s(4, &self.error_name),
            self.reply_serial.map(|n| (5, Value::U32(n))),
            s(6, &self.destination),
            s(7, &self.sender),
            (!sig.is_empty()).then_some((8, Value::Sig(sig))),
        ];
        let fields = all.into_iter().flatten();
        let fields =
            fields.map(|(code, v)| Value::Struct(vec![Value::Byte(code), Value::variant(v)]));
        Value::Array(field_type_elem(), fields.collect())
    }

    /// Parses exactly one whole message (see `frame_len`).
    pub fn decode(buf: &[u8]) -> Result<Message, Error> {
        let big_endian = byte_order(buf)?;
        let mut r = Reader::new(buf, big_endian);
        let head = r.take(4)?;
        let kind = match head[1] {
            1 => Kind::Call,
            2 => Kind::Return,
            3 => Kind::Error,
            4 => Kind::Signal,
            _ => return Err(Error::Malformed("unknown message type")),
        };
        if head[3] != 1 {
            return Err(Error::Malformed("unknown protocol version"));
        }
        let body_len = r.u32()? as usize;
        let mut m = Message::new(kind, Vec::new());
        m.flags = head[2];
        m.serial = r.u32()?;
        if m.serial == 0 {
            return Err(Error::Malformed("serial 0"));
        }
        let mut sig = String::new();
        let fields = r.get(&field_type())?;
        for f in fields.as_array().unwrap_or_default() {
            if let Some([Value::Byte(code), Value::Variant(v)]) = f.as_struct() {
                m.set_field(*code, v, &mut sig)?;
            }
        }
        r.align(8)?;
        let body = buf.get(r.pos..).unwrap_or_default();
        if body.len() != body_len {
            return Err(Error::Malformed("body length does not match"));
        }
        m.body = decode_body(body, &sig, big_endian)?;
        m.check_required()?;
        Ok(m)
    }

    fn set_field(&mut self, code: u8, v: &Value, sig: &mut String) -> Result<(), Error> {
        match (code, v) {
            (1, Value::Path(s)) => self.path = Some(s.clone()),
            (2, Value::Str(s)) => self.interface = Some(s.clone()),
            (3, Value::Str(s)) => self.member = Some(s.clone()),
            (4, Value::Str(s)) => self.error_name = Some(s.clone()),
            (5, Value::U32(n)) => self.reply_serial = Some(*n),
            (6, Value::Str(s)) => self.destination = Some(s.clone()),
            (7, Value::Str(s)) => self.sender = Some(s.clone()),
            (8, Value::Sig(s)) => sig.clone_from(s),
            (9, Value::U32(_)) => {} // fd count; we never negotiate fds, so there are none
            (1..=9, _) => return Err(Error::Malformed("header field has the wrong type")),
            _ => {} // unknown fields are ignored (spec)
        }
        Ok(())
    }

    fn check_required(&self) -> Result<(), Error> {
        let ok = match self.kind {
            Kind::Call => self.path.is_some() && self.member.is_some(),
            Kind::Signal => {
                self.path.is_some() && self.interface.is_some() && self.member.is_some()
            }
            Kind::Return => self.reply_serial.is_some(),
            Kind::Error => self.reply_serial.is_some() && self.error_name.is_some(),
        };
        if ok {
            Ok(())
        } else {
            Err(Error::Malformed("required header field missing"))
        }
    }
}

/// Size of the whole message that starts with these (≥ 16) bytes.
pub fn frame_len(head: &[u8]) -> Result<usize, Error> {
    let mut r = Reader::new(head, byte_order(head)?);
    r.take(4)?;
    let body = r.u32()? as usize;
    r.take(4)?;
    let fields = r.u32()? as usize;
    if fields > MAX_ARRAY {
        return Err(Error::Malformed("header fields longer than 64 MiB"));
    }
    let total = (16 + fields).next_multiple_of(8).checked_add(body);
    total
        .filter(|&n| n <= MAX_MESSAGE)
        .ok_or(Error::Malformed("message larger than 128 MiB"))
}

fn byte_order(buf: &[u8]) -> Result<bool, Error> {
    match buf.first() {
        Some(b'l') => Ok(false),
        Some(b'B') => Ok(true),
        _ => Err(Error::Malformed("bad byte order mark")),
    }
}

fn decode_body(body: &[u8], sig: &str, big_endian: bool) -> Result<Vec<Value>, Error> {
    let mut r = Reader::new(body, big_endian);
    let values = parse_sig(sig)?
        .iter()
        .map(|t| r.get(t))
        .collect::<Result<Vec<_>, _>>()?;
    if r.pos != body.len() {
        return Err(Error::Malformed("bytes left after the body"));
    }
    Ok(values)
}

/// Header fields: `a(yv)`.
fn field_type() -> Type {
    Type::Array(Box::new(field_type_elem()))
}

fn field_type_elem() -> Type {
    Type::Struct(vec![Type::Byte, Type::Variant])
}
