//! Own D-Bus client and service, std only: wire format, messages, one blocking connection.
//!
//! Enough for udisks2 on the system bus and a portal backend on the session bus.
//! Sends little endian, reads both byte orders. No fd passing.

mod conn;
mod message;
#[cfg(test)]
mod tests;
mod transport;
mod types;
mod value;
mod wire;

pub use conn::Conn;
pub use message::{Kind, Message};
pub use types::Type;
pub use value::Value;

use std::fmt;

/// Everything that can go wrong talking D-Bus.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// No usable bus address.
    Address(String),
    /// The bus refused our SASL login; holds its answer.
    Auth(String),
    /// Bad bytes from the other side.
    Malformed(&'static str),
    /// A value we were asked to send does not fit its type.
    Invalid(&'static str),
    /// No reply within `Conn::timeout`.
    Timeout,
    /// The bus hung up.
    Closed,
    /// The peer answered with a D-Bus error.
    Remote {
        name: String,
        text: String,
    },
    /// `request_name`: someone else owns it.
    NameTaken(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "dbus i/o: {e}"),
            Error::Address(a) => write!(f, "dbus address: {a}"),
            Error::Auth(a) => write!(f, "dbus auth rejected: {a}"),
            Error::Malformed(m) => write!(f, "dbus malformed input: {m}"),
            Error::Invalid(m) => write!(f, "dbus invalid value: {m}"),
            Error::Timeout => write!(f, "dbus call timed out"),
            Error::Closed => write!(f, "dbus connection closed"),
            Error::Remote { name, text } => write!(f, "{name}: {text}"),
            Error::NameTaken(n) => write!(f, "dbus name {n} is taken"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
