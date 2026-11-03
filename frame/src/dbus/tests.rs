//! Wire-format tests, plus live tests against the real session and system bus.

use super::message::frame_len;
use super::types::{parse_sig, parse_single};
use super::wire::{Reader, Writer};
use super::*;
use std::time::{Duration, Instant};

const BUS: &str = "org.freedesktop.DBus";
const BUS_PATH: &str = "/org/freedesktop/DBus";
const UDISKS: &str = "org.freedesktop.UDisks2";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
const SDA1: &str = "/org/freedesktop/UDisks2/block_devices/sda1";

fn body_bytes(v: &Value) -> Vec<u8> {
    let mut w = Writer::default();
    w.put(&v.ty(), v).unwrap();
    w.buf
}

fn sig_of(v: &Value) -> String {
    let mut s = String::new();
    v.ty().write_sig(&mut s);
    s
}

fn roundtrip(mut m: Message) -> Message {
    m.serial = 3;
    m.sender = Some(":1.9".into());
    assert_eq!(Message::decode(&m.encode().unwrap()).unwrap(), m);
    m
}

/// The Hello every client sends first: 128 bytes, byte for byte as libdbus emits it.
#[test]
fn hello_matches_spec_layout() {
    let expected: &[u8] = b"l\x01\x00\x01\x00\x00\x00\x00\x01\x00\x00\x00m\x00\x00\x00\
\x01\x01o\x00\x15\x00\x00\x00/org/freedesktop/DBus\x00\x00\x00\
\x02\x01s\x00\x14\x00\x00\x00org.freedesktop.DBus\x00\x00\x00\x00\
\x03\x01s\x00\x05\x00\x00\x00Hello\x00\x00\x00\
\x06\x01s\x00\x14\x00\x00\x00org.freedesktop.DBus\x00\x00\x00\x00";
    let mut m = Message::call(BUS, BUS_PATH, BUS, "Hello", vec![]);
    m.serial = 1;
    let bytes = m.encode().unwrap();
    assert_eq!(bytes.len(), 128);
    assert_eq!(bytes, expected);
    assert_eq!(frame_len(&bytes).unwrap(), 128);
    assert_eq!(Message::decode(&bytes).unwrap(), m);
}

#[test]
fn alignment_rules() {
    let s = Value::Struct(vec![Value::Byte(7), Value::U32(1)]);
    assert_eq!(body_bytes(&s), [7, 0, 0, 0, 1, 0, 0, 0]);
    // Empty arrays still pad to the element alignment; the length excludes that padding.
    assert_eq!(body_bytes(&Value::Array(Type::U64, vec![])), [0; 8]);
    assert_eq!(body_bytes(&Value::Array(Type::I32, vec![])), [0; 4]);
    let opts = Value::dict_sv(vec![("a", Value::U32(5))]);
    let expected = [
        16, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, b'a', 0, 1, b'u', 0, 0, 0, 0, 5, 0, 0, 0,
    ];
    assert_eq!(body_bytes(&opts), expected);
    // Filesystem.Mount(a{sv}) without options: body = length 0 + 4 bytes padding.
    let fs = "org.freedesktop.UDisks2.Filesystem";
    let mut m = Message::call(UDISKS, SDA1, fs, "Mount", vec![Value::dict_sv(vec![])]);
    m.serial = 2;
    let bytes = m.encode().unwrap();
    assert_eq!(bytes[4..8], 8u32.to_le_bytes());
    assert_eq!(bytes[bytes.len() - 8..], [0; 8]);
    assert_eq!(Message::decode(&bytes).unwrap(), m);
}

#[test]
fn roundtrip_all_types() {
    let v = Value::Struct(vec![
        Value::Byte(1),
        Value::Bool(true),
        Value::I16(-2),
        Value::U16(3),
        Value::I32(-4),
        Value::U32(5),
        Value::I64(-6),
        Value::U64(7),
        Value::F64(0.5),
        Value::str("ü"),
        Value::path("/a/b_1"),
        Value::Sig("a{sv}(ii)".into()),
        Value::Bytes(vec![1, 2, 3]),
        Value::Array(Type::Str, vec![]),
        Value::variant(Value::variant(Value::Array(Type::I16, vec![Value::I16(9)]))),
    ]);
    assert_eq!(sig_of(&v), "(ybnqiuxtdsogayasv)");
    assert_eq!(Reader::new(&body_bytes(&v), false).get(&v.ty()).unwrap(), v);
}

/// GetManagedObjects reply body: one block device with Block + Filesystem.
fn managed_objects() -> Value {
    let block = Value::dict_sv(vec![
        ("Device", Value::Bytes(b"/dev/sda1\0".to_vec())),
        ("Size", Value::U64(512 << 30)),
    ]);
    let aay = Type::Array(Box::new(Type::Byte));
    let mounts = Value::Array(aay, vec![Value::Bytes(b"/mnt/x\0".to_vec())]);
    let fs = Value::dict_sv(vec![("MountPoints", mounts)]);
    let props = Type::Dict(Box::new(Type::Str), Box::new(Type::Variant));
    let ifaces = Value::Dict(
        Type::Str,
        props.clone(),
        vec![
            (Value::str("org.freedesktop.UDisks2.Block"), block),
            (Value::str("org.freedesktop.UDisks2.Filesystem"), fs),
        ],
    );
    let ifaces_ty = Type::Dict(Box::new(Type::Str), Box::new(props));
    Value::Dict(Type::Path, ifaces_ty, vec![(Value::path(SDA1), ifaces)])
}

#[test]
fn udisks_managed_objects_roundtrip() {
    let objects = managed_objects();
    assert_eq!(sig_of(&objects), "a{oa{sa{sv}}}");
    let mut call = Message::call(
        UDISKS,
        "/org/freedesktop/UDisks2",
        OBJECT_MANAGER,
        "x",
        vec![],
    );
    call.serial = 7;
    call.sender = Some(":1.5".into());
    let reply = roundtrip(Message::method_return(&call, vec![objects]));
    assert_eq!(
        (reply.reply_serial, reply.destination.as_deref()),
        (Some(7), Some(":1.5"))
    );
    let sda1 = reply.body[0].get(SDA1).unwrap();
    let block = sda1.get("org.freedesktop.UDisks2.Block").unwrap();
    assert_eq!(block.get("Size").and_then(Value::as_u64), Some(512 << 30));
    assert_eq!(
        block.get("Device").and_then(Value::as_bytes),
        Some(&b"/dev/sda1\0"[..])
    );
    let fs = sda1.get("org.freedesktop.UDisks2.Filesystem").unwrap();
    let mounts = fs.get("MountPoints").and_then(Value::as_array).unwrap();
    assert_eq!(mounts[0].as_bytes(), Some(&b"/mnt/x\0"[..]));
}

#[test]
fn udisks_signals_and_portal_shapes_roundtrip() {
    let root = "/org/freedesktop/UDisks2";
    let ifaces = managed_objects().as_dict().unwrap()[0].1.clone();
    let added = vec![Value::path(SDA1), ifaces];
    roundtrip(Message::signal(
        root,
        OBJECT_MANAGER,
        "InterfacesAdded",
        added,
    ));
    let names = Value::Array(Type::Str, vec![Value::str("org.freedesktop.UDisks2.Block")]);
    let removed = vec![Value::path(SDA1), names];
    roundtrip(Message::signal(
        root,
        OBJECT_MANAGER,
        "InterfacesRemoved",
        removed,
    ));
    // FileChooser.OpenFile(o s s s a{sv}) with a filter list a(sa(us)) → (u a{sv}).
    let pattern = Value::Struct(vec![Value::U32(0), Value::str("*.png")]);
    let rule = Type::Struct(vec![Type::U32, Type::Str]);
    let filter = Value::Struct(vec![
        Value::str("Images"),
        Value::Array(rule, vec![pattern]),
    ]);
    let filters = Value::Array(filter.ty(), vec![filter]);
    let opts = Value::dict_sv(vec![("filters", filters), ("multiple", Value::Bool(true))]);
    let path = "/org/freedesktop/portal/desktop/request/1_2/t";
    let args = vec![
        Value::path(path),
        Value::str("app"),
        Value::str(""),
        Value::str("Open"),
        opts,
    ];
    let fc = "org.freedesktop.impl.portal.FileChooser";
    let call = roundtrip(Message::call("org.gabe.Files", path, fc, "OpenFile", args));
    assert_eq!(sig_of(&Value::Struct(call.body.clone())), "(osssa{sv})");
    let uris = Value::Array(Type::Str, vec![Value::str("file:///tmp/a.png")]);
    let results = vec![Value::U32(0), Value::dict_sv(vec![("uris", uris)])];
    roundtrip(Message::method_return(&call, results));
}

#[test]
fn dict_lookup() {
    let opts = Value::dict_sv(vec![
        ("modal", Value::Bool(true)),
        ("label", Value::str("Open")),
    ]);
    assert_eq!(opts.get("modal").and_then(Value::as_bool), Some(true));
    assert_eq!(opts.get("label").and_then(Value::as_str), Some("Open"));
    assert!(opts.get("missing").is_none());
    assert!(Value::U32(1).get("x").is_none());
}

#[test]
fn truncated_and_corrupted_messages_are_rejected() {
    let mut m = Message::signal("/a", "a.b", "C", vec![managed_objects(), Value::F64(1.0)]);
    m.serial = 1;
    let bytes = m.encode().unwrap();
    for n in 0..bytes.len() {
        assert!(Message::decode(&bytes[..n]).is_err(), "prefix {n}");
    }
    let mut longer = bytes.clone();
    longer.push(0);
    assert!(Message::decode(&longer).is_err());
    // Flipping any byte must never panic (it may still decode, e.g. a changed letter).
    for i in 0..bytes.len() {
        let mut b = bytes.clone();
        b[i] ^= 0xff;
        let _ = Message::decode(&b);
        let _ = frame_len(&b);
    }
}

#[test]
fn bad_headers_are_rejected() {
    let mut m = Message::call(BUS, BUS_PATH, BUS, "Hello", vec![]);
    m.serial = 1;
    let good = m.encode().unwrap();
    let with = |i: usize, b: u8| {
        let mut x = good.clone();
        x[i] = b;
        Message::decode(&x)
    };
    assert!(with(0, b'X').is_err(), "byte order");
    assert!(with(1, 9).is_err(), "message type");
    assert!(with(3, 2).is_err(), "version");
    assert!(with(8, 0).is_err(), "serial 0");
    m.member = None;
    assert!(
        Message::decode(&m.encode().unwrap()).is_err(),
        "call without member"
    );
    let mut head = good[..16].to_vec();
    head[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(frame_len(&head).is_err(), "over 128 MiB");
}

#[test]
fn bad_values_are_rejected() {
    let get = |sig: &str, bytes: &[u8]| Reader::new(bytes, false).get(&parse_single(sig).unwrap());
    assert!(get("b", &[2, 0, 0, 0]).is_err(), "bool 2");
    assert!(
        get("s", &[1, 0, 0, 0, b'a', 1]).is_err(),
        "no NUL terminator"
    );
    assert!(get("s", &[2, 0, 0, 0, b'a', 0, 0]).is_err(), "NUL inside");
    assert!(get("s", &[1, 0, 0, 0, 0xff, 0]).is_err(), "not UTF-8");
    assert!(get("o", &[1, 0, 0, 0, b'x', 0]).is_err(), "not a path");
    assert!(get("ay", &[0xff, 0xff, 0xff, 0x0f]).is_err(), "over 64 MiB");
    assert!(
        get("ai", &[8, 0, 0, 0, 1, 0, 0, 0]).is_err(),
        "length past the end"
    );
    assert!(
        get("ai", &[2, 0, 0, 0, 1, 0, 0, 0]).is_err(),
        "element crosses the end"
    );
    assert!(
        get("(yu)", &[1, 9, 0, 0, 1, 0, 0, 0]).is_err(),
        "nonzero padding"
    );
    assert!(get("v", &[1, b'{', 0]).is_err(), "bad variant signature");
    assert!(
        get("v", &[2, b'y', b'y', 0, 1, 2]).is_err(),
        "variant with two types"
    );
    let mut deep = [1, b'v', 0].repeat(100);
    deep.extend_from_slice(&[1, b'y', 0, 5]);
    assert!(get("v", &deep).is_err(), "variants nested too deep");
    for sig in ["a", "(", "()", "a{vs}", "a{s}", "a{sss}", "{sv}", "z", ")"] {
        assert!(parse_sig(sig).is_err(), "{sig}");
    }
    assert!(
        parse_sig(&("a".repeat(65) + "y")).is_err(),
        "arrays nested too deep"
    );
    assert!(parse_sig(&"y".repeat(256)).is_err(), "signature over 255");
    assert_eq!(parse_sig("a{sv}ia(ss)").unwrap().len(), 3);
}

#[test]
fn bad_values_are_not_sent() {
    let mut w = Writer::default();
    assert!(w.put(&Type::U32, &Value::str("x")).is_err());
    let mixed = Value::Array(Type::U32, vec![Value::I32(1)]);
    assert!(w.put(&Type::Array(Box::new(Type::U32)), &mixed).is_err());
    assert!(w.put(&Type::Path, &Value::path("no/slash")).is_err());
    assert!(w.put(&Type::Str, &Value::str("a\0b")).is_err());
    let empty_struct = Message::call(BUS, BUS_PATH, BUS, "X", vec![Value::Struct(vec![])]);
    assert!(empty_struct.encode().is_err());
}

#[test]
fn big_endian_messages_decode() {
    let bytes = [
        b'B', 2, 0, 1, 0, 0, 0, 4, 0, 0, 0, 1, 0, 0, 0, 15, // fixed header
        5, 1, b'u', 0, 0, 0, 0, 9, // REPLY_SERIAL = 9
        8, 1, b'g', 0, 1, b'u', 0, 0, // SIGNATURE = "u", padding
        1, 2, 3, 4, // body
    ];
    assert_eq!(frame_len(&bytes).unwrap(), bytes.len());
    let m = Message::decode(&bytes).unwrap();
    assert_eq!((m.kind, m.reply_serial), (Kind::Return, Some(9)));
    assert_eq!(m.body, [Value::U32(0x0102_0304)]);
}

#[test]
fn addresses() {
    assert_eq!(transport::unescape("/run/a%2cb%3D").unwrap(), b"/run/a,b=");
    assert!(transport::unescape("%z1").is_err() && transport::unescape("%4").is_err());
    assert!(matches!(
        transport::connect("tcp:host=x,port=1"),
        Err(Error::Address(_))
    ));
    assert!(matches!(
        transport::connect("unix:path=/nonexistent/bus"),
        Err(Error::Io(_))
    ));
}

#[test]
fn live_session_bus_lists_names() {
    let mut c = Conn::session().unwrap();
    assert!(c.unique_name().starts_with(':'));
    let names = c
        .call(BUS, BUS_PATH, "org.freedesktop.DBus.ListNames", &[])
        .unwrap();
    assert!(
        names[0]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_str() == Some(BUS))
    );
    let err = c
        .call(BUS, BUS_PATH, "org.freedesktop.DBus.NoSuchMethod", &[])
        .unwrap_err();
    assert!(
        err.to_string()
            .starts_with("org.freedesktop.DBus.Error.UnknownMethod"),
        "{err}"
    );
    // Calling ourselves: nobody answers, so it times out; the call itself gets queued.
    c.timeout = Duration::from_millis(200);
    let me = c.unique_name().to_string();
    let err = c.call(&me, "/x", "org.gabe.FilesTest.Nobody", &[]);
    assert!(matches!(err, Err(Error::Timeout)), "{err:?}");
    let queued = c.read_ready().unwrap();
    assert!(queued.iter().any(|m| m.member.as_deref() == Some("Nobody")));
}

#[test]
fn live_system_bus_answers() {
    let mut c = Conn::system().unwrap();
    let id = c
        .call(BUS, BUS_PATH, "org.freedesktop.DBus.GetId", &[])
        .unwrap();
    assert_eq!(id[0].as_str().map(str::len), Some(32));
    let names = c
        .call(BUS, BUS_PATH, "org.freedesktop.DBus.ListNames", &[])
        .unwrap();
    assert!(
        names[0]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_str() == Some(BUS))
    );
}

/// A service and a client on the session bus: RequestName, calls, an error reply, a signal.
#[test]
fn live_session_service_roundtrip() {
    let name = format!("org.gabe.FilesTest.p{}", std::process::id());
    let mut server = Conn::session().unwrap();
    server.request_name(&name).unwrap();
    server
        .add_match("type='signal',interface='org.gabe.FilesTest'")
        .unwrap();
    let mut client = Conn::session().unwrap();
    assert!(matches!(
        client.request_name(&name),
        Err(Error::NameTaken(_))
    ));
    let client_name = client.unique_name().to_string();
    let service = std::thread::spawn(move || serve(server, &client_name));
    let path = "/org/gabe/FilesTest";
    let args = [Value::str("hi"), Value::dict_sv(vec![("n", Value::U32(3))])];
    let echo = client
        .call(&name, path, "org.gabe.FilesTest.Echo", &args)
        .unwrap();
    assert_eq!(echo[0].as_str(), Some("hi"));
    assert_eq!(echo[1].get("n").and_then(Value::as_u64), Some(3));
    let err = client
        .call(&name, path, "org.gabe.FilesTest.Fail", &[])
        .unwrap_err();
    assert_eq!(err.to_string(), "org.gabe.FilesTest.Nope: no");
    let ping = Message::signal(path, "org.gabe.FilesTest", "Ping", vec![]);
    client.send(ping).unwrap();
    assert_eq!(service.join().unwrap(), 3);
}

/// Test service: echoes Echo, fails anything else, stops at the Ping signal.
/// Returns how many messages from `client` it handled.
fn serve(mut conn: Conn, client: &str) -> usize {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut handled = 0;
    while Instant::now() < deadline {
        for m in conn.read_ready().unwrap() {
            if m.sender.as_deref() != Some(client) {
                continue; // NameAcquired and friends come from the bus itself
            }
            handled += 1;
            match (m.kind, m.member.as_deref()) {
                (Kind::Call, Some("Echo")) => conn.reply(&m, &m.body).unwrap(),
                (Kind::Call, _) => conn
                    .reply_error(&m, "org.gabe.FilesTest.Nope", "no")
                    .unwrap(),
                (Kind::Signal, Some("Ping")) => return handled,
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    handled
}
