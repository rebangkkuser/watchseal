// SPDX-License-Identifier: GPL-3.0-or-later
//! Monitoring sources and what the calling process can actually use.
//!
//! Every probe runs in a forked child: on Android app processes (Termux), a
//! syscall outside the seccomp allow-list kills the caller with SIGSYS, and
//! that must only ever kill the child.

use crate::common::err_text;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;

pub struct Source {
    pub name: &'static str,
    /// true = the daemon really uses it, false = only planned
    pub implemented: bool,
    probe: fn() -> Result<String, String>,
}

pub static SOURCES: [Source; 7] = [
    Source { name: "proc", implemented: true, probe: probe_proc },
    Source { name: "inotify", implemented: false, probe: probe_inotify },
    Source { name: "fanotify", implemented: false, probe: probe_fanotify },
    Source { name: "klogctl", implemented: false, probe: probe_klogctl },
    Source { name: "kmsg", implemented: false, probe: probe_kmsg },
    Source { name: "audit", implemented: false, probe: probe_audit },
    Source { name: "logcat", implemented: false, probe: probe_logcat },
];

// Raw syscall numbers, so we do not depend on what the libc crate exports.
#[cfg(target_arch = "aarch64")]
const NR_FANOTIFY_INIT: libc::c_long = 262;
#[cfg(target_arch = "aarch64")]
const NR_SYSLOG: libc::c_long = 116;
#[cfg(target_arch = "x86_64")]
const NR_FANOTIFY_INIT: libc::c_long = 300;
#[cfg(target_arch = "x86_64")]
const NR_SYSLOG: libc::c_long = 103;
#[cfg(target_arch = "arm")]
const NR_FANOTIFY_INIT: libc::c_long = 367;
#[cfg(target_arch = "arm")]
const NR_SYSLOG: libc::c_long = 103;
#[cfg(target_arch = "x86")]
const NR_FANOTIFY_INIT: libc::c_long = 338;
#[cfg(target_arch = "x86")]
const NR_SYSLOG: libc::c_long = 103;

fn probe_proc() -> Result<String, String> {
    fs::read_dir("/proc").map(|_| String::new()).map_err(|e| crate::common::err_string(&e))
}

fn probe_inotify() -> Result<String, String> {
    // SAFETY: plain syscall wrapper; the fd is closed right away.
    unsafe {
        let fd = libc::inotify_init1(libc::IN_CLOEXEC);
        if fd < 0 {
            return Err(err_text());
        }
        libc::close(fd);
    }
    Ok(String::new())
}

fn probe_fanotify() -> Result<String, String> {
    // SAFETY: FAN_CLOEXEC (1) | FAN_CLASS_NOTIF (0), O_RDONLY.
    unsafe {
        let fd = libc::syscall(NR_FANOTIFY_INIT, 1 as libc::c_long, libc::O_RDONLY as libc::c_long);
        if fd < 0 {
            return Err(err_text());
        }
        libc::close(fd as i32);
    }
    Ok(String::new())
}

fn probe_klogctl() -> Result<String, String> {
    // SYSLOG_ACTION_READ_ALL (3) reads without consuming the ring buffer.
    let mut c = [0u8; 1];
    // SAFETY: buffer of 1 byte, length 1.
    let r = unsafe { libc::syscall(NR_SYSLOG, 3 as libc::c_long, c.as_mut_ptr(), 1 as libc::c_long) };
    if r < 0 {
        return Err(err_text());
    }
    Ok(String::new())
}

fn probe_kmsg() -> Result<String, String> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open("/dev/kmsg")
        .map(|_| String::new())
        .map_err(|e| crate::common::err_string(&e))
}

fn probe_audit() -> Result<String, String> {
    // SAFETY: plain socket(2); NETLINK_AUDIT is 9.
    unsafe {
        let fd = libc::socket(libc::AF_NETLINK, libc::SOCK_RAW | libc::SOCK_CLOEXEC, 9);
        if fd < 0 {
            return Err(err_text());
        }
        libc::close(fd);
    }
    Ok("socket opens; reading events not verified".to_string())
}

fn probe_logcat() -> Result<String, String> {
    // SAFETY: path is a NUL-terminated literal.
    let r = unsafe { libc::access(b"/system/bin/logcat\0".as_ptr() as *const libc::c_char, libc::X_OK) };
    if r < 0 {
        return Err(err_text());
    }
    Ok(String::new())
}

/// Runs the source's probe in a child process. Ok carries an optional note,
/// Err carries the reason it is unavailable (including "killed by SIGSYS").
pub fn probe(s: &Source) -> Result<String, String> {
    let _ = std::io::stdout().flush();
    // SAFETY: the program is single-threaded here, so fork() is sound; the
    // child only runs the probe, writes the pipe and calls _exit().
    unsafe {
        let mut fds = [0i32; 2];
        if libc::pipe(fds.as_mut_ptr()) < 0 {
            return Err(err_text());
        }
        let pid = libc::fork();
        if pid < 0 {
            let e = err_text();
            libc::close(fds[0]);
            libc::close(fds[1]);
            return Err(e);
        }
        if pid == 0 {
            libc::close(fds[0]);
            let (code, text) = match (s.probe)() {
                Ok(t) => (0, t),
                Err(t) => (1, t),
            };
            libc::write(fds[1], text.as_ptr() as *const libc::c_void, text.len());
            libc::_exit(code);
        }
        libc::close(fds[1]);
        let mut buf = [0u8; 128];
        let n = libc::read(fds[0], buf.as_mut_ptr() as *mut libc::c_void, buf.len());
        libc::close(fds[0]);
        let text = String::from_utf8_lossy(&buf[..n.max(0) as usize]).into_owned();

        let mut status = 0;
        loop {
            let r = libc::waitpid(pid, &mut status, 0);
            if r < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            break;
        }
        if libc::WIFSIGNALED(status) {
            let sig = libc::WTERMSIG(status);
            return Err(format!(
                "blocked by the system (signal {}{}), likely seccomp",
                sig,
                if sig == libc::SIGSYS { "/SIGSYS" } else { "" }
            ));
        }
        if libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0 {
            Ok(text)
        } else {
            Err(text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boom() -> Result<String, String> {
        unsafe { libc::raise(libc::SIGSYS) };
        Ok(String::new())
    }

    // fork() inside the multi-threaded test harness can deadlock on malloc, so
    // run it alone:  cargo test -- --ignored --test-threads=1
    #[test]
    #[ignore]
    fn sigsys_only_kills_the_probe_child() {
        let s = Source { name: "boom", implemented: false, probe: boom };
        let e = probe(&s).unwrap_err();
        assert!(e.contains("SIGSYS"), "{}", e);
    }
}
