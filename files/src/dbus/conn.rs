//! `Conn`: one bus connection. Blocking calls for clients, `read_ready` for an event loop.

use super::message::{Kind, Message, NO_REPLY, frame_len};
use super::value::Value;
use super::{Error, transport};
use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

const BUS: &str = "org.freedesktop.DBus";
const BUS_PATH: &str = "/org/freedesktop/DBus";

/// Socket reads per `read_ready`, so a flood cannot stall the event loop (poll wakes again).
const DRAIN_READS: usize = 64;

pub struct Conn {
    sock: UnixStream,
    name: String,
    serial: u32,
    /// Bytes read but not yet a whole message.
    rbuf: Vec<u8>,
    /// Whole messages not handed out yet.
    queue: VecDeque<Message>,
    /// How long `call` waits for its reply (default 25 s, like libdbus).
    pub timeout: Duration,
}

impl Conn {
    pub fn system() -> Result<Conn, Error> {
        Conn::open(&transport::system_address())
    }

    pub fn session() -> Result<Conn, Error> {
        Conn::open(&transport::session_address()?)
    }

    /// Connects to a bus address, logs in and says Hello.
    pub fn open(addr: &str) -> Result<Conn, Error> {
        let mut sock = transport::connect(addr)?;
        let timeout = Duration::from_secs(25);
        sock.set_read_timeout(Some(timeout))?;
        transport::authenticate(&mut sock)?;
        let mut conn = Conn {
            sock,
            name: String::new(),
            serial: 0,
            rbuf: Vec::new(),
            queue: VecDeque::new(),
            timeout,
        };
        let reply = conn.call(BUS, BUS_PATH, "org.freedesktop.DBus.Hello", &[])?;
        let name = reply.first().and_then(Value::as_str);
        conn.name = name
            .ok_or(Error::Malformed("Hello reply without a name"))?
            .to_string();
        Ok(conn)
    }

    /// Our unique name on the bus (":1.42").
    #[cfg(test)]
    pub fn unique_name(&self) -> &str {
        &self.name
    }

    /// For poll. Also drain `read_ready` after each `call`: queued messages don't wake poll.
    pub fn fd(&self) -> RawFd {
        self.sock.as_raw_fd()
    }

    /// Sends `msg` with the next serial; returns that serial.
    pub fn send(&mut self, mut msg: Message) -> Result<u32, Error> {
        self.serial = self.serial.wrapping_add(1).max(1);
        msg.serial = self.serial;
        self.sock.write_all(&msg.encode()?)?;
        Ok(msg.serial)
    }

    /// Calls `method` ("org.freedesktop.DBus.ListNames" = interface + member) and waits for
    /// the reply. Unrelated messages that arrive meanwhile are queued for `read_ready`.
    pub fn call(
        &mut self,
        dest: &str,
        path: &str,
        method: &str,
        args: &[Value],
    ) -> Result<Vec<Value>, Error> {
        let (iface, member) = method
            .rsplit_once('.')
            .ok_or(Error::Invalid("method needs an interface"))?;
        let serial = self.send(Message::call(dest, path, iface, member, args.to_vec()))?;
        let deadline = Instant::now() + self.timeout;
        loop {
            while let Some(m) = self.next_buffered()? {
                let answer = matches!(m.kind, Kind::Return | Kind::Error);
                if answer && m.reply_serial == Some(serial) {
                    return reply_result(m);
                }
                self.queue.push_back(m);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(Error::Timeout);
            }
            self.sock.set_read_timeout(Some(left))?;
            self.fill()?;
        }
    }

    /// `AddMatch`, e.g. "type='signal',interface='org.freedesktop.DBus.ObjectManager'".
    pub fn add_match(&mut self, rule: &str) -> Result<(), Error> {
        let method = "org.freedesktop.DBus.AddMatch";
        self.call(BUS, BUS_PATH, method, &[Value::str(rule)])
            .map(drop)
    }

    /// Takes a well-known name; `Error::NameTaken` if another connection has it.
    pub fn request_name(&mut self, name: &str) -> Result<(), Error> {
        const DO_NOT_QUEUE: u32 = 4;
        let args = [Value::str(name), Value::U32(DO_NOT_QUEUE)];
        let reply = self.call(BUS, BUS_PATH, "org.freedesktop.DBus.RequestName", &args)?;
        match reply.first().and_then(Value::as_u64) {
            Some(1 | 4) => Ok(()), // primary owner / already owner
            _ => Err(Error::NameTaken(name.to_string())),
        }
    }

    /// Everything that has arrived, without blocking; queued messages first.
    /// Call when `fd()` is readable and after each `call`.
    pub fn read_ready(&mut self) -> Result<Vec<Message>, Error> {
        self.sock.set_nonblocking(true)?;
        let mut read = Ok(true);
        for _ in 0..DRAIN_READS {
            read = self.fill();
            if !matches!(read, Ok(true)) {
                break;
            }
        }
        self.sock.set_nonblocking(false)?;
        read?;
        while let Some(m) = self.next_buffered()? {
            self.queue.push_back(m);
        }
        Ok(self.queue.drain(..).collect())
    }

    // shortcut: no built-in Introspectable/Peer answers; services reply UnknownMethod
    // themselves, add Peer.Ping here if a client ever needs it.
    /// Answers the call `msg` (skipped when it asked for no reply).
    pub fn reply(&mut self, msg: &Message, args: &[Value]) -> Result<(), Error> {
        if msg.flags & NO_REPLY != 0 {
            return Ok(());
        }
        self.send(Message::method_return(msg, args.to_vec()))
            .map(drop)
    }

    /// Answers the call `msg` with a D-Bus error, e.g. "org.freedesktop.DBus.Error.UnknownMethod".
    pub fn reply_error(&mut self, msg: &Message, name: &str, text: &str) -> Result<(), Error> {
        if msg.flags & NO_REPLY != 0 {
            return Ok(());
        }
        self.send(Message::error(msg, name, text)).map(drop)
    }

    /// One read into `rbuf`. `Ok(false)`: nothing there yet (nonblocking or timed out).
    fn fill(&mut self) -> Result<bool, Error> {
        let mut chunk = [0u8; 32 * 1024];
        loop {
            match self.sock.read(&mut chunk) {
                Ok(0) => return Err(Error::Closed),
                Ok(n) => {
                    self.rbuf.extend_from_slice(&chunk[..n]);
                    return Ok(true);
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    return Ok(false);
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Cuts the next whole message off `rbuf`, if one is there.
    fn next_buffered(&mut self) -> Result<Option<Message>, Error> {
        let Some(head) = self.rbuf.get(..16) else {
            return Ok(None);
        };
        let len = frame_len(head)?;
        if self.rbuf.len() < len {
            return Ok(None);
        }
        let msg = Message::decode(&self.rbuf[..len]);
        self.rbuf.drain(..len);
        msg.map(Some)
    }
}

fn reply_result(m: Message) -> Result<Vec<Value>, Error> {
    if m.kind != Kind::Error {
        return Ok(m.body);
    }
    let text = m
        .body
        .first()
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Err(Error::Remote {
        name: m.error_name.unwrap_or_default(),
        text,
    })
}
