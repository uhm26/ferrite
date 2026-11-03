//! The polkit side: register as this session's agent, turn calls into `Event`s, answer them.

use frame::dbus::{Conn, Error, Kind, Message, Value};

const POLKIT: &str = "org.freedesktop.PolicyKit1";
const AUTHORITY: &str = "/org/freedesktop/PolicyKit1/Authority";
const AGENT_IFACE: &str = "org.freedesktop.PolicyKit1.AuthenticationAgent";
/// Where we serve the agent interface.
pub const PATH: &str = "/org/ferrite/auth";
pub const CANCELLED: &str = "org.freedesktop.PolicyKit1.Error.Cancelled";
pub const FAILED: &str = "org.freedesktop.PolicyKit1.Error.Failed";

/// Who polkit lets answer a request.
#[derive(Debug, PartialEq)]
pub enum Identity {
    User(u32),
    Group(u32),
}

/// One `BeginAuthentication`; answer it with `Conn::reply`/`reply_error` on `call`.
#[derive(Debug)]
pub struct Request {
    pub call: Message,
    pub action: String,
    pub message: String,
    pub cookie: String,
    pub identities: Vec<Identity>,
    /// The program that asks (from /proc), when polkit says which one.
    pub program: Option<String>,
}

pub enum Event {
    Begin(Box<Request>),
    Cancel { cookie: String },
}

/// Registers `conn` as the agent of login session `session`.
pub fn register(conn: &mut Conn, session: &str) -> Result<(), Error> {
    let subject = Value::Struct(vec![
        Value::str("unix-session"),
        Value::dict_sv(vec![("session-id", Value::str(session))]),
    ]);
    let args = [subject, Value::str(&locale()), Value::str(PATH)];
    let method = "org.freedesktop.PolicyKit1.Authority.RegisterAuthenticationAgent";
    conn.call(POLKIT, AUTHORITY, method, &args).map(drop)
}

fn locale() -> String {
    std::env::var("LANG").unwrap_or_else(|_| "C".into())
}

/// Turns one incoming message into an event; answers anything else itself.
pub fn handle(conn: &mut Conn, msg: Message) -> Result<Option<Event>, Error> {
    if msg.kind != Kind::Call {
        return Ok(None);
    }
    let ours = msg.interface.as_deref() == Some(AGENT_IFACE);
    match (ours, msg.member.as_deref()) {
        (true, Some("BeginAuthentication")) => match parse_begin(&msg) {
            Some(req) => Ok(Some(Event::Begin(Box::new(req)))),
            None => conn
                .reply_error(&msg, FAILED, "bad arguments")
                .map(|()| None),
        },
        (true, Some("CancelAuthentication")) => {
            let cookie = msg.body.first().and_then(Value::as_str).unwrap_or("");
            let event = Event::Cancel {
                cookie: cookie.into(),
            };
            conn.reply(&msg, &[])?;
            Ok(Some(event))
        }
        _ => {
            let unknown = "org.freedesktop.DBus.Error.UnknownMethod";
            conn.reply_error(&msg, unknown, "auth serves the polkit agent interface only")?;
            Ok(None)
        }
    }
}

/// `(s action_id, s message, s icon, a{ss} details, s cookie, a(sa{sv}) identities)`.
/// `None` when its arguments don't fit.
fn parse_begin(call: &Message) -> Option<Request> {
    let [action, message, _icon, details, cookie, ids] = call.body.as_slice() else {
        return None;
    };
    let s = |v: &Value| v.as_str().map(str::to_string);
    let pid = |k| details.get(k)?.as_str()?.parse::<u32>().ok();
    Some(Request {
        call: call.clone(),
        action: s(action)?,
        message: s(message)?,
        cookie: s(cookie)?,
        identities: ids.as_array()?.iter().filter_map(identity).collect(),
        program: pid("polkit.subject-pid").or_else(|| pid("polkit.caller-pid")).and_then(comm),
    })
}

/// The short name of process `pid` ("files").
fn comm(pid: u32) -> Option<String> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(s.trim_end().to_string()).filter(|s| !s.is_empty())
}

fn identity(v: &Value) -> Option<Identity> {
    let [kind, details] = v.as_struct()? else {
        return None;
    };
    match kind.as_str()? {
        "unix-user" => Some(Identity::User(
            details.get("uid")?.as_u64()?.try_into().ok()?,
        )),
        "unix-group" => Some(Identity::Group(
            details.get("gid")?.as_u64()?.try_into().ok()?,
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(kind: &str, key: &str, n: u32) -> Value {
        Value::Struct(vec![
            Value::str(kind),
            Value::dict_sv(vec![(key, Value::U32(n))]),
        ])
    }

    #[test]
    fn begin_arguments_parse() {
        let ids = vec![id("unix-user", "uid", 1000), id("unix-group", "gid", 998)];
        let ty = ids[0].ty();
        let pid = std::process::id().to_string();
        let details = vec![(Value::str("polkit.subject-pid"), Value::str(&pid))];
        let body = vec![
            Value::str("org.freedesktop.udisks2.filesystem-mount-system"),
            Value::str("Authentication is required"),
            Value::str(""),
            Value::Dict(frame::dbus::Type::Str, frame::dbus::Type::Str, details),
            Value::str("cookie-1"),
            Value::Array(ty, ids),
        ];
        let msg = Message::call("x", "/", AGENT_IFACE, "BeginAuthentication", body);
        let req = parse_begin(&msg).expect("parses");
        assert_eq!(req.action, "org.freedesktop.udisks2.filesystem-mount-system");
        assert_eq!(req.cookie, "cookie-1");
        assert_eq!(req.identities, [Identity::User(1000), Identity::Group(998)]);
        assert_eq!(req.program, comm(std::process::id()));
        assert!(req.program.is_some());
        let short = Message::call("x", "/", AGENT_IFACE, "BeginAuthentication", vec![]);
        assert!(parse_begin(&short).is_none());
    }
}
