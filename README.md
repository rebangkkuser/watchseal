# Watchseal

Native Android event monitor in C: `watchseald` (daemon) and `watchsealctl` (CLI).
Target: Android 7.0+ (API 24), AArch64. Version 0.1.0-dev.

## Build

    # Termux / NDK clang
    make CC=aarch64-linux-android24-clang

    # Host test build with sandboxed paths
    make CC=gcc CPPFLAGS='-DWS_LOG_PATH=\"/tmp/ws/sealmsg\" -DWS_PID_PATH=\"/tmp/ws/sealmsg.pid\" -DWS_LOCK_PATH=\"/tmp/ws/sealmsg.lock\"'

## Usage

    watchseald [-f|--foreground] [--lang=CODE]
    watchsealctl [--lang=CODE] status
    watchsealctl log [-W | -c]
    watchsealctl export <dir>       # -> <dir>/sealmsg-YYYYMMDD-HHMMSS.log

`--lang` accepts `en`, `pt-br` (alias `pt`), or `auto`. Unknown locale: warning on
the terminal and in the log, falls back to English. Auto detection order:
`persist.sys.locale`, `ro.product.locale`, `persist.sys.language/country`,
then `LC_ALL`/`LANGUAGE`/`LANG`. The event log is always English.

To add a language: add a message array in `src/i18n.c` and one line in `locales[]`.

## Status of features

| feature                              | state        |
|--------------------------------------|--------------|
| single instance (flock + pid file)   | implemented  |
| persistent log, signal-safe shutdown | implemented  |
| new/gone process detection (/proc)   | implemented (polling, 1s) |
| ctl: status, log, log -W, log -c     | implemented  |
| ctl: export                          | implemented  |
| klogctl / kmsg, fanotify, inotify    | planned (probed by `status`) |
| SELinux audit (AVC)                  | planned      |
| honk (crash) / bzzt (reboot) events  | planned, needs a reliable source |

## Known limits (honest monitoring)

- polling misses processes that live shorter than the scan interval
- a process vanishing from /proc is reported as "gone", never as killed/crashed/normal exit
- kernel threads are excluded
- `status` probes what the *calling* process can access; run it as the same user as the daemon
