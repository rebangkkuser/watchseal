// SPDX-License-Identifier: GPL-3.0-or-later
//! watchsealctl - query and control watchseald.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::process;
use std::thread::sleep;
use std::time::Duration;

use watchseal::common::{
    args, err_string, opt_match, read_pidfile, strftime_now, LOCK_PATH, LOG_PATH,
};
use watchseal::i18n::{self, fmt, msg, Msg};
use watchseal::sources::{self, SOURCES};

const PROG: &str = "watchsealctl";

fn fail_open(path: &str, e: &io::Error) {
    eprint!("{}", fmt(Msg::CannotOpen, &[&PROG, &path, &err_string(e)]));
    if e.kind() == io::ErrorKind::PermissionDenied {
        eprint!("{}", msg(Msg::HintRoot));
    }
}

fn export_failed(e: &io::Error) {
    eprint!("{}", fmt(Msg::ExportFailed, &[&PROG, &err_string(e)]));
}

// ---- status ---------------------------------------------------------------

fn cmd_status() -> i32 {
    match File::open(LOCK_PATH) {
        // no lock file at all: the daemon never ran
        Err(e) if e.kind() == io::ErrorKind::NotFound => watchseal::out!("{}", msg(Msg::StDaemonStopped)),
        Err(_) => watchseal::out!("{}", msg(Msg::StDaemonUnknown)),
        Ok(f) => {
            let fd = f.as_raw_fd();
            // SAFETY: valid fd owned by `f`. A shared lock only succeeds if
            // the daemon does not hold its exclusive one.
            if unsafe { libc::flock(fd, libc::LOCK_SH | libc::LOCK_NB) } == 0 {
                unsafe { libc::flock(fd, libc::LOCK_UN) };
                watchseal::out!("{}", msg(Msg::StDaemonStopped));
            } else if io::Error::last_os_error().raw_os_error() == Some(libc::EWOULDBLOCK) {
                watchseal::out!("{}", fmt(Msg::StDaemonRunning, &[&read_pidfile()]));
            } else {
                watchseal::out!("{}", msg(Msg::StDaemonUnknown));
            }
        }
    }

    watchseal::out!("{}", fmt(Msg::StLang, &[&i18n::current(), &i18n::available()]));
    watchseal::out!("{}", msg(Msg::StSources));
    for s in SOURCES.iter() {
        let (ok, note) = match sources::probe(s) {
            Ok(n) => (true, n),
            Err(n) => (false, n),
        };
        watchseal::outln!(
            "  {:<9} {:<13} {:<12}{}{}",
            s.name,
            msg(if ok { Msg::StAvailable } else { Msg::StUnavailable }),
            msg(if s.implemented { Msg::StImplemented } else { Msg::StPlanned }),
            if note.is_empty() { "" } else { " - " },
            note
        );
    }
    0
}

// ---- log ------------------------------------------------------------------

fn cmd_log(follow: bool, clear: bool) -> i32 {
    if clear {
        // Truncate in place (no create): the daemon appends with O_APPEND, so
        // it keeps writing at the new end.
        return match OpenOptions::new().write(true).truncate(true).open(LOG_PATH) {
            Ok(_) => {
                watchseal::out!("{}", msg(Msg::LogCleared));
                0
            }
            Err(e) => {
                fail_open(LOG_PATH, &e);
                1
            }
        };
    }

    let mut f = match File::open(LOG_PATH) {
        Ok(f) => f,
        Err(e) => {
            fail_open(LOG_PATH, &e);
            return 1;
        }
    };
    let mut buf = [0u8; 8192];
    let mut pos: u64 = 0;
    let stdout = io::stdout();
    let mut out = stdout.lock();
    loop {
        match f.read(&mut buf) {
            Ok(n) if n > 0 => {
                if out.write_all(&buf[..n]).and_then(|_| out.flush()).is_err() {
                    break; // reader went away (e.g. `| head`)
                }
                pos += n as u64;
                continue;
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                eprintln!("{}: read: {}", PROG, err_string(&e));
                return 1;
            }
        }
        if !follow {
            break;
        }
        if let Ok(m) = f.metadata() {
            if m.len() < pos {
                // the log was cleared: start over from the top
                if f.seek(SeekFrom::Start(0)).is_ok() {
                    pos = 0;
                }
                continue;
            }
        }
        sleep(Duration::from_millis(200));
    }
    0
}

// ---- export ---------------------------------------------------------------

fn cmd_export(dir: &str) -> i32 {
    if let Err(e) = fs::create_dir_all(dir) {
        export_failed(&e);
        return 1;
    }
    let mut input = match File::open(LOG_PATH) {
        Ok(f) => f,
        Err(e) => {
            fail_open(LOG_PATH, &e);
            return 1;
        }
    };
    let size = match input.metadata() {
        Ok(m) => m.len(),
        Err(e) => {
            export_failed(&e);
            return 1;
        }
    };

    // Never overwrite an earlier export: create_new, numeric suffix on clash.
    let stamp = strftime_now("%Y%m%d-%H%M%S");
    let mut chosen: Option<(String, File)> = None;
    for i in 0..100 {
        let name = if i == 0 {
            format!("sealmsg-{}.log", stamp)
        } else {
            format!("sealmsg-{}-{}.log", stamp, i + 1)
        };
        let path = Path::new(dir).join(name).to_string_lossy().into_owned();
        match OpenOptions::new().write(true).create_new(true).mode(0o644).open(&path) {
            Ok(f) => {
                chosen = Some((path, f));
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                fail_open(&path, &e);
                return 1;
            }
        }
    }
    let Some((path, mut out)) = chosen else {
        export_failed(&io::Error::from(io::ErrorKind::AlreadyExists));
        return 1;
    };

    // The daemon keeps appending: copy only the size seen now, and drop any
    // trailing half-written line so the export ends on a complete record.
    let mut left = size;
    let mut copied: u64 = 0;
    let mut last_nl_end: u64 = 0;
    let mut buf = [0u8; 8192];
    while left > 0 {
        let want = left.min(buf.len() as u64) as usize;
        let n = match input.read(&mut buf[..want]) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                export_failed(&e);
                let _ = fs::remove_file(&path);
                return 1;
            }
        };
        if let Err(e) = out.write_all(&buf[..n]) {
            export_failed(&e);
            let _ = fs::remove_file(&path);
            return 1;
        }
        if let Some(k) = buf[..n].iter().rposition(|&b| b == b'\n') {
            last_nl_end = copied + k as u64 + 1;
        }
        copied += n as u64;
        left -= n as u64;
    }
    if copied > last_nl_end {
        let _ = out.set_len(last_nl_end);
    }
    let _ = out.sync_all();
    watchseal::outln!("{}", path);
    0
}

// ---- main -----------------------------------------------------------------

fn main() {
    let argv = args();
    let mut cmd: Option<String> = None;
    let mut path: Option<String> = None;
    let mut lang: Option<String> = None;
    let mut bad: Option<String> = None;
    let (mut follow, mut clear, mut help) = (false, false, false);

    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        let cur = cmd.as_deref();
        if let Some(v) = opt_match(a, "lang") {
            match v {
                Some(v) => lang = Some(v.to_string()),
                None if i + 1 < argv.len() => {
                    i += 1;
                    lang = Some(argv[i].clone());
                }
                None => {
                    bad = Some(a.to_string());
                    break;
                }
            }
        } else if a == "-h" || opt_match(a, "help").is_some() {
            help = true;
        } else if cur.is_none() && !a.starts_with('-') {
            cmd = Some(a.to_string());
        } else if cur == Some("log") && a == "-W" {
            follow = true;
        } else if cur == Some("log") && a == "-c" {
            clear = true;
        } else if cur == Some("export") && path.is_none() && !a.starts_with('-') {
            path = Some(a.to_string());
        } else {
            bad = Some(a.to_string());
            break;
        }
        i += 1;
    }

    // An unknown locale falls back to English silently here; the daemon is the
    // one that logs the warning.
    match &lang {
        Some(l) => {
            i18n::set(l);
        }
        None => i18n::auto(),
    }

    if let Some(b) = bad {
        eprint!("{}", fmt(Msg::UnknownArg, &[&PROG, &b]));
        eprint!("{}", msg(Msg::UsageCtl));
        process::exit(2);
    }
    let Some(cmd) = cmd else {
        if help {
            watchseal::out!("{}", msg(Msg::UsageCtl));
            return;
        }
        eprint!("{}", msg(Msg::UsageCtl));
        process::exit(2);
    };
    if help {
        watchseal::out!("{}", msg(Msg::UsageCtl));
        return;
    }

    let code = match cmd.as_str() {
        "status" => cmd_status(),
        "log" => cmd_log(follow, clear),
        "export" => match path {
            Some(p) => cmd_export(&p),
            None => {
                eprint!("{}", msg(Msg::NeedPath));
                2
            }
        },
        other => {
            eprint!("{}", fmt(Msg::UnknownArg, &[&PROG, &other]));
            eprint!("{}", msg(Msg::UsageCtl));
            2
        }
    };
    process::exit(code);
}
