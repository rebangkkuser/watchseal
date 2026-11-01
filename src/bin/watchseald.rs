// SPDX-License-Identifier: GPL-3.0-or-later
//! watchseald - Android event monitoring daemon.
//!
//! Implemented in this version:
//!   - single instance (flock + pid file), clean shutdown on signals
//!   - persistent append-only log
//!   - process appear/disappear detection by polling /proc
//!
//! Not implemented yet: proc connector, fanotify/inotify, kmsg/klogctl,
//! SELinux audit. Nothing is reported that was not actually observed.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::process;
use std::sync::atomic::{AtomicI32, Ordering};
use std::thread::sleep;
use std::time::Duration;

use watchseal::common::{
    args, err_string, opt_match, read_pidfile, strftime_now, LOCK_PATH, LOG_PATH, PID_PATH, VERSION,
};
use watchseal::i18n::{self, fmt, msg, Msg};
use watchseal::procscan::{self, diff, Change, Proc};

const PFX_INFO: &str = "watchseal info:";
const PFX_WARN: &str = "watchseal warn:";
const PFX_NEW: &str = "watchseal: uaahhh!!:";
const PFX_GONE: &str = "watchseal arf:";
const PFX_STOP: &str = "watchseal mrr...:";

const SCAN_INTERVAL_SECS: u64 = 1;
const TICK_MS: u64 = 100; // how often we look at the stop flag

static STOP: AtomicI32 = AtomicI32::new(0);

extern "C" fn on_signal(sig: libc::c_int) {
    STOP.store(sig, Ordering::SeqCst); // atomic store: async-signal-safe
}

struct Logger {
    file: File,
}

impl Logger {
    fn open() -> io::Result<Logger> {
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o644)
            .open(LOG_PATH)?;
        Ok(Logger { file })
    }

    /// One write() on an O_APPEND fd: lines never interleave or tear, and a
    /// `watchsealctl log -c` truncation does not break us.
    fn log(&self, prefix: &str, text: &str) {
        let line = format!("{} {} {}\n", strftime_now("%Y-%m-%d %H:%M:%S"), prefix, text);
        let _ = (&self.file).write_all(line.as_bytes());
    }

    fn report_new(&self, p: &Proc) {
        self.log(
            PFX_NEW,
            &format!("new process pid={} ppid={} uid={} name={}", p.pid, p.ppid, p.uid, p.comm),
        );
    }

    fn report_gone(&self, p: &Proc) {
        // /proc cannot tell us HOW it ended, so we do not claim to know.
        self.log(
            PFX_GONE,
            &format!(
                "process gone from /proc pid={} name={} (exit reason unknown)",
                p.pid, p.comm
            ),
        );
    }
}

struct Opts {
    foreground: bool,
    help: bool,
    version: bool,
    lang: Option<String>,
    bad: Option<String>,
}

fn parse_args(argv: &[String]) -> Opts {
    let mut o = Opts { foreground: false, help: false, version: false, lang: None, bad: None };
    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        if let Some(v) = opt_match(a, "lang") {
            match v {
                Some(v) => o.lang = Some(v.to_string()),
                None if i + 1 < argv.len() => {
                    i += 1;
                    o.lang = Some(argv[i].clone());
                }
                None => {
                    o.bad = Some(a.to_string());
                    break;
                }
            }
        } else if a == "-f" || opt_match(a, "foreground").is_some() {
            o.foreground = true;
        } else if a == "-h" || opt_match(a, "help").is_some() {
            o.help = true;
        } else if opt_match(a, "version").is_some() {
            o.version = true;
        } else {
            o.bad = Some(a.to_string());
            break;
        }
        i += 1;
    }
    o
}

fn fail_open(path: &str, e: &io::Error) {
    eprint!("{}", fmt(Msg::CannotOpen, &[&"watchseald", &path, &err_string(e)]));
    if matches!(e.kind(), io::ErrorKind::PermissionDenied) || e.raw_os_error() == Some(libc::EROFS) {
        eprint!("{}", msg(Msg::HintRoot));
    }
}

/// Detaches from the terminal. Returns in the child only.
fn daemonize() {
    // SAFETY: single-threaded at this point; the parent exits immediately and
    // the child keeps the inherited lock fd (flock belongs to the open file).
    unsafe {
        let pid = libc::fork();
        if pid < 0 {
            eprintln!("watchseald: fork: {}", io::Error::last_os_error());
            process::exit(1);
        }
        if pid > 0 {
            watchseal::out!("{}", fmt(Msg::Started, &[&pid]));
            let _ = io::stdout().flush();
            libc::_exit(0);
        }
        libc::setsid();
        let _ = libc::chdir(b"/\0".as_ptr() as *const libc::c_char);
        let nul = libc::open(b"/dev/null\0".as_ptr() as *const libc::c_char, libc::O_RDWR);
        if nul >= 0 {
            libc::dup2(nul, 0);
            libc::dup2(nul, 1);
            libc::dup2(nul, 2);
            if nul > 2 {
                libc::close(nul);
            }
        }
    }
}

fn install_signals() {
    // SAFETY: the handler only does an atomic store. No SA_RESTART is set.
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as usize;
        libc::sigemptyset(&mut sa.sa_mask);
        sa.sa_flags = 0;
        libc::sigaction(libc::SIGTERM, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}

fn main() {
    let o = parse_args(&args());

    // explicit --lang wins; otherwise auto-detect
    let lang_missing = match &o.lang {
        Some(l) => !i18n::set(l),
        None => {
            i18n::auto();
            false
        }
    };

    if o.version {
        watchseal::outln!("watchseald {}", VERSION);
        return;
    }
    if let Some(bad) = &o.bad {
        eprint!("{}", fmt(Msg::UnknownArg, &[&"watchseald", bad]));
        eprint!("{}", msg(Msg::UsageDaemon));
        process::exit(2);
    }
    if o.help {
        watchseal::out!("{}", msg(Msg::UsageDaemon));
        return;
    }

    let missing_note = if lang_missing {
        Some(format!(
            "locale '{}' not found, using 'en' (available: {})",
            o.lang.as_deref().unwrap_or(""),
            i18n::available()
        ))
    } else {
        None
    };
    if let Some(n) = &missing_note {
        eprintln!("watchseald: {}", n);
    }

    // Single instance: the kernel drops the lock if we die, so no stale locks.
    let lock = match OpenOptions::new().read(true).write(true).create(true).mode(0o644).open(LOCK_PATH) {
        Ok(f) => f,
        Err(e) => {
            fail_open(LOCK_PATH, &e);
            process::exit(1);
        }
    };
    // SAFETY: valid fd owned by `lock`, which lives until the process ends.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } < 0 {
        let e = io::Error::last_os_error();
        if e.raw_os_error() == Some(libc::EWOULDBLOCK) {
            eprint!("{}", fmt(Msg::AlreadyRunning, &[&read_pidfile()]));
        } else {
            fail_open(LOCK_PATH, &e);
        }
        process::exit(1);
    }

    let logger = match Logger::open() {
        Ok(l) => l,
        Err(e) => {
            fail_open(LOG_PATH, &e);
            process::exit(1);
        }
    };

    if !o.foreground {
        daemonize();
    }
    let _ = fs::write(PID_PATH, format!("{}\n", process::id()));
    install_signals();

    logger.log(PFX_INFO, &format!("watchseald {} started pid={}", VERSION, process::id()));
    if let Some(n) = &missing_note {
        logger.log(PFX_WARN, n);
    }
    logger.log(
        PFX_INFO,
        &format!(
            "sources: proc=polling({}s); kernel threads excluded; \
             processes shorter than the interval can be missed",
            SCAN_INTERVAL_SECS
        ),
    );

    let mut prev: Vec<Proc> = Vec::new();
    let mut scan_failed = false;
    match procscan::scan() {
        Ok(list) => {
            logger.log(
                PFX_INFO,
                &format!(
                    "baseline: {} processes (already running, not reported as new)",
                    list.len()
                ),
            );
            prev = list;
        }
        Err(e) => {
            logger.log(
                PFX_WARN,
                &format!("cannot read /proc: {}; process monitoring unavailable", err_string(&e)),
            );
            scan_failed = true;
        }
    }

    let ticks_per_scan = SCAN_INTERVAL_SECS * 1000 / TICK_MS;
    'main: loop {
        for _ in 0..ticks_per_scan {
            sleep(Duration::from_millis(TICK_MS));
            if STOP.load(Ordering::SeqCst) != 0 {
                break 'main;
            }
        }
        match procscan::scan() {
            Err(e) => {
                // incomplete snapshot: never diffed, so no false "gone" events
                if !scan_failed {
                    logger.log(PFX_WARN, &format!("cannot read /proc: {}", err_string(&e)));
                }
                scan_failed = true;
            }
            Ok(cur) => {
                if scan_failed {
                    logger.log(
                        PFX_INFO,
                        &format!("/proc readable again; new baseline: {} processes", cur.len()),
                    );
                    scan_failed = false;
                } else {
                    for change in diff(&prev, &cur) {
                        match change {
                            Change::New(p) => logger.report_new(p),
                            Change::Gone(p) => logger.report_gone(p),
                        }
                    }
                }
                prev = cur;
            }
        }
    }

    logger.log(
        PFX_STOP,
        &format!("watchseald stopping (signal {})", STOP.load(Ordering::SeqCst)),
    );
    let _ = fs::remove_file(PID_PATH);
}
