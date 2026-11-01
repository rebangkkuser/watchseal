// SPDX-License-Identifier: GPL-3.0-or-later
use std::ffi::CString;
use std::fs;
use std::io;

/// print!/println! that ignore a closed stdout (e.g. `watchsealctl status | head`)
/// instead of panicking with "Broken pipe".
#[macro_export]
macro_rules! out {
    ($($a:tt)*) => {{
        use std::io::Write as _;
        let _ = write!(std::io::stdout(), $($a)*);
    }};
}

#[macro_export]
macro_rules! outln {
    ($($a:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout(), $($a)*);
    }};
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// Runtime paths can be overridden at build time for host-side tests:
//   WS_LOG_PATH=/tmp/ws/sealmsg cargo build
pub const LOG_PATH: &str = match option_env!("WS_LOG_PATH") {
    Some(p) => p,
    None => "/data/sealmsg",
};
pub const PID_PATH: &str = match option_env!("WS_PID_PATH") {
    Some(p) => p,
    None => "/data/sealmsg.pid",
};
pub const LOCK_PATH: &str = match option_env!("WS_LOCK_PATH") {
    Some(p) => p,
    None => "/data/sealmsg.lock",
};

/// Process arguments as strings (lossy, never panics on odd bytes).
pub fn args() -> Vec<String> {
    std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

/// Pid stored in the pid file, or 0 if it cannot be read.
pub fn read_pidfile() -> i32 {
    fs::read_to_string(PID_PATH)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Matches a long option written as `-name`, `--name` or `--name=value`.
/// Returns `Some(None)` for a bare flag and `Some(Some(v))` for `=value`.
pub fn opt_match<'a>(arg: &'a str, name: &str) -> Option<Option<&'a str>> {
    let rest = arg.strip_prefix("--").or_else(|| arg.strip_prefix('-'))?;
    let rest = rest.strip_prefix(name)?;
    if rest.is_empty() {
        Some(None)
    } else {
        rest.strip_prefix('=').map(Some)
    }
}

/// Last OS error as plain text, without Rust's " (os error N)" suffix.
pub fn err_text() -> String {
    err_string(&io::Error::last_os_error())
}

pub fn err_string(e: &io::Error) -> String {
    let s = e.to_string();
    match s.find(" (os error") {
        Some(i) => s[..i].to_string(),
        None => s,
    }
}

/// Current local time formatted with strftime(3).
pub fn strftime_now(fmt: &str) -> String {
    let Ok(cfmt) = CString::new(fmt) else {
        return String::new();
    };
    // SAFETY: plain libc time calls with valid, initialised buffers.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        let mut buf = [0u8; 64];
        let n = libc::strftime(
            buf.as_mut_ptr() as *mut libc::c_char,
            buf.len(),
            cfmt.as_ptr(),
            &tm,
        );
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opt_match_forms() {
        assert_eq!(opt_match("-lang=pt", "lang"), Some(Some("pt")));
        assert_eq!(opt_match("--lang=pt", "lang"), Some(Some("pt")));
        assert_eq!(opt_match("-lang", "lang"), Some(None));
        assert_eq!(opt_match("--lang", "lang"), Some(None));
        assert_eq!(opt_match("--lang=", "lang"), Some(Some("")));
        assert_eq!(opt_match("--language", "lang"), None);
        assert_eq!(opt_match("lang", "lang"), None);
        assert_eq!(opt_match("-c", "lang"), None);
    }
}
