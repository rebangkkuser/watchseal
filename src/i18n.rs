// SPDX-License-Identifier: GPL-3.0-or-later
//! Terminal messages. The event log itself is always English.
//!
//! To add a language: write one more `[&str; COUNT]` array and add one entry
//! to `LOCALES`. The compiler rejects an array with a missing message.

use std::env;
use std::fmt::Display;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Copy)]
#[repr(usize)]
pub enum Msg {
    UsageDaemon,
    UsageCtl,
    AlreadyRunning,
    Started,
    CannotOpen,
    HintRoot,
    UnknownArg,
    NeedPath,
    ExportFailed,
    LogCleared,
    StDaemonRunning,
    StDaemonStopped,
    StDaemonUnknown,
    StLang,
    StSources,
    StAvailable,
    StUnavailable,
    StImplemented,
    StPlanned,
}
const COUNT: usize = 19;

// "{}" marks where an argument goes (see `fmt`). Same order as `Msg`.
static EN: [&str; COUNT] = [
    "usage: watchseald [options]\n\
     \x20 -f, --foreground   do not detach from the terminal\n\
     \x20     --lang=CODE    message language (en, pt-br, pt, auto)\n\
     \x20 -h, --help         show this help\n\
     \x20     --version      show version\n",
    "usage: watchsealctl [--lang=CODE] <command>\n\
     \x20 status             daemon state and monitoring sources\n\
     \x20 log                print the event history\n\
     \x20 log -W             follow new events\n\
     \x20 log -c             clear the history (truncates the file)\n\
     \x20 export <dir>       copy the history to <dir>/sealmsg-DATE-TIME.log\n",
    "watchseald: already running (pid {})\n",
    "watchseald: started (pid {})\n",
    "{}: cannot open '{}': {}\n",
    "hint: this probably needs root (e.g. su -c '...')\n",
    "{}: unknown or invalid argument: {}\n",
    "export: missing destination directory\n",
    "{}: export failed: {}\n",
    "log cleared\n",
    "daemon:   running (pid {})\n",
    "daemon:   not running\n",
    "daemon:   unknown (cannot read the lock file)\n",
    "language: {} (available: {})\n",
    "monitoring sources, as seen by this process:\n",
    "available",
    "unavailable",
    "implemented",
    "planned",
];

static PT_BR: [&str; COUNT] = [
    "uso: watchseald [opções]\n\
     \x20 -f, --foreground   não desanexa do terminal\n\
     \x20     --lang=COD     idioma das mensagens (en, pt-br, pt, auto)\n\
     \x20 -h, --help         mostra esta ajuda\n\
     \x20     --version      mostra a versão\n",
    "uso: watchsealctl [--lang=COD] <comando>\n\
     \x20 status             estado do daemon e fontes de monitoramento\n\
     \x20 log                mostra o histórico de eventos\n\
     \x20 log -W             acompanha novos eventos\n\
     \x20 log -c             limpa o histórico (trunca o arquivo)\n\
     \x20 export <dir>       copia o histórico para <dir>/sealmsg-DATA-HORA.log\n",
    "watchseald: já está rodando (pid {})\n",
    "watchseald: iniciado (pid {})\n",
    "{}: não foi possível abrir '{}': {}\n",
    "dica: provavelmente precisa de root (ex.: su -c '...')\n",
    "{}: argumento desconhecido ou inválido: {}\n",
    "export: falta o diretório de destino\n",
    "{}: falha ao exportar: {}\n",
    "histórico limpo\n",
    "daemon:   rodando (pid {})\n",
    "daemon:   parado\n",
    "daemon:   desconhecido (não foi possível ler o arquivo de lock)\n",
    "idioma:   {} (disponíveis: {})\n",
    "fontes de monitoramento, como este processo as enxerga:\n",
    "disponível",
    "indisponível",
    "implementada",
    "planejada",
];

struct Locale {
    code: &'static str,
    alias: Option<&'static str>,
    msgs: &'static [&'static str; COUNT],
}

static LOCALES: [Locale; 2] = [
    Locale { code: "en", alias: None, msgs: &EN },
    Locale { code: "pt-br", alias: Some("pt"), msgs: &PT_BR },
];

static CUR: AtomicUsize = AtomicUsize::new(0);

/// "pt_BR.UTF-8" -> "pt-br"; cuts encoding/modifier/list suffixes.
fn normalize(s: &str) -> String {
    s.chars()
        .take_while(|&c| c != '.' && c != '@' && c != ':')
        .map(|c| if c == '_' { '-' } else { c.to_ascii_lowercase() })
        .collect()
}

fn find_exact(norm: &str) -> Option<usize> {
    LOCALES
        .iter()
        .position(|l| l.code == norm)
        .or_else(|| LOCALES.iter().position(|l| l.alias == Some(norm)))
}

fn find(norm: &str) -> Option<usize> {
    if norm.is_empty() || norm == "c" || norm == "posix" {
        return Some(0);
    }
    if let Some(i) = find_exact(norm) {
        return Some(i);
    }
    // "pt-pt" -> "pt", "en-us" -> "en"
    let base = norm.split('-').next().unwrap_or("");
    if base != norm && !base.is_empty() {
        return find_exact(base);
    }
    None
}

#[cfg(target_os = "android")]
fn prop(name: &str) -> Option<String> {
    use std::ffi::{CStr, CString};
    use std::os::raw::{c_char, c_int};

    extern "C" {
        fn __system_property_get(name: *const c_char, value: *mut c_char) -> c_int;
    }
    let cname = CString::new(name).ok()?;
    let mut buf = [0 as c_char; 92]; // PROP_VALUE_MAX
    // SAFETY: buf is PROP_VALUE_MAX bytes and cname is NUL-terminated.
    let n = unsafe { __system_property_get(cname.as_ptr(), buf.as_mut_ptr()) };
    if n <= 0 {
        return None;
    }
    // SAFETY: the call NUL-terminates the value.
    let s = unsafe { CStr::from_ptr(buf.as_ptr()) };
    Some(s.to_string_lossy().into_owned())
}

#[cfg(not(target_os = "android"))]
fn prop(_name: &str) -> Option<String> {
    None
}

fn detect() -> String {
    if let Some(v) = prop("persist.sys.locale").or_else(|| prop("ro.product.locale")) {
        return normalize(&v);
    }
    if let Some(l) = prop("persist.sys.language") {
        return match prop("persist.sys.country") {
            Some(c) => normalize(&format!("{}-{}", l, c)),
            None => normalize(&l),
        };
    }
    for var in ["LC_ALL", "LANGUAGE", "LANG"] {
        if let Ok(v) = env::var(var) {
            if !v.is_empty() {
                return normalize(&v);
            }
        }
    }
    String::new()
}

/// Detects the language from Android properties, then LC_ALL/LANGUAGE/LANG.
pub fn auto() {
    CUR.store(find(&detect()).unwrap_or(0), Ordering::Relaxed);
}

/// Selects a locale by code or alias. "" and "auto" run auto-detection.
/// Returns false if the locale does not exist (English is used then).
pub fn set(name: &str) -> bool {
    if name.is_empty() || name == "auto" {
        auto();
        return true;
    }
    match find(&normalize(name)) {
        Some(i) => {
            CUR.store(i, Ordering::Relaxed);
            true
        }
        None => {
            CUR.store(0, Ordering::Relaxed);
            false
        }
    }
}

pub fn current() -> &'static str {
    LOCALES[CUR.load(Ordering::Relaxed)].code
}

pub fn available() -> String {
    LOCALES.iter().map(|l| l.code).collect::<Vec<_>>().join(", ")
}

pub fn msg(m: Msg) -> &'static str {
    LOCALES[CUR.load(Ordering::Relaxed)].msgs[m as usize]
}

/// Message with each "{}" replaced by the next argument.
pub fn fmt(m: Msg, args: &[&dyn Display]) -> String {
    let mut out = String::new();
    let mut it = args.iter();
    for (i, part) in msg(m).split("{}").enumerate() {
        if i > 0 {
            if let Some(a) = it.next() {
                out.push_str(&a.to_string());
            }
        }
        out.push_str(part);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_rules() {
        assert_eq!(find("pt"), Some(1));
        assert_eq!(find("pt-br"), Some(1));
        assert_eq!(find("pt-pt"), Some(1));
        assert_eq!(find("en-us"), Some(0));
        assert_eq!(find("c"), Some(0));
        assert_eq!(find(""), Some(0));
        assert_eq!(find("xx"), None);
        assert_eq!(find("xx-yy"), None);
    }

    #[test]
    fn normalize_forms() {
        assert_eq!(normalize("pt_BR.UTF-8"), "pt-br");
        assert_eq!(normalize("pt_BR:pt:en"), "pt-br");
        assert_eq!(normalize("en_US@euro"), "en-us");
    }

    #[test]
    fn format_args() {
        set("pt");
        assert_eq!(fmt(Msg::Started, &[&42]), "watchseald: iniciado (pid 42)\n");
        set("en");
        assert_eq!(
            fmt(Msg::CannotOpen, &[&"p", &"/x", &"boom"]),
            "p: cannot open '/x': boom\n"
        );
    }
}
