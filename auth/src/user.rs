//! Which user answers a request, and that user's login name (getpwuid_r, so NSS users work).

use crate::agent::Identity;
use std::ffi::{CStr, c_char, c_int};

/// pwd.h: struct passwd (glibc, x86_64).
#[repr(C)]
struct Passwd {
    name: *mut c_char,
    passwd: *mut c_char,
    uid: u32,
    gid: u32,
    gecos: *mut c_char,
    dir: *mut c_char,
    shell: *mut c_char,
}

unsafe extern "C" {
    fn getuid() -> u32;
    fn getpwuid_r(
        uid: u32,
        pwd: *mut Passwd,
        buf: *mut c_char,
        len: usize,
        result: *mut *mut Passwd,
    ) -> c_int;
}

/// We answer as ourselves when polkit allows it, else as the first user it lists.
/// shortcut: no picker between several admins; add one if a machine has more than one.
pub fn pick(ids: &[Identity]) -> Option<u32> {
    // SAFETY: getuid takes no arguments, touches no memory and cannot fail.
    let me = unsafe { getuid() };
    let users = || ids.iter().filter_map(|i| if let Identity::User(u) = i { Some(*u) } else { None });
    users().find(|&u| u == me).or_else(|| users().next())
}

/// Login name of `uid`, or `None` if no such user.
pub fn name(uid: u32) -> Option<String> {
    let mut buf = vec![0 as c_char; 16 * 1024];
    let mut pwd = std::mem::MaybeUninit::<Passwd>::uninit();
    let mut found = std::ptr::null_mut();
    // SAFETY: all pointers are valid for the call; buf.len() is its real size.
    let ret = unsafe { getpwuid_r(uid, pwd.as_mut_ptr(), buf.as_mut_ptr(), buf.len(), &mut found) };
    if ret != 0 || found.is_null() {
        return None;
    }
    // SAFETY: success: `found` points at `pwd`, whose name is a C string inside `buf`.
    let name = unsafe { CStr::from_ptr((*found).name) };
    name.to_str().ok().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_has_a_name_and_ourselves_win() {
        assert_eq!(name(0).as_deref(), Some("root"));
        // SAFETY: as in `pick`.
        let me = unsafe { getuid() };
        let ids = [Identity::Group(1), Identity::User(0), Identity::User(me)];
        assert_eq!(pick(&ids), Some(me));
        assert_eq!(pick(&[Identity::Group(1)]), None);
    }
}
