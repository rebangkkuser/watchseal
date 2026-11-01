/* watchsealctl - query and control watchseald. */
#include "common.h"
#include "i18n.h"
#include "sources.h"

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <string.h>
#include <sys/file.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#define PROG "watchsealctl"

static void fail_open(const char *path)
{
    fprintf(stderr, ws_msg(MSG_CANNOT_OPEN), PROG, path, strerror(errno));
    if (errno == EACCES || errno == EPERM)
        fputs(ws_msg(MSG_HINT_ROOT), stderr);
}

/* ---- status ------------------------------------------------------------- */

static int cmd_status(void)
{
    char avail[64];
    int fd = open(WS_LOCK_PATH, O_RDONLY | O_CLOEXEC);

    if (fd < 0) {
        /* no lock file at all means the daemon never ran; EACCES means unknown */
        if (errno == ENOENT)
            printf("%s", ws_msg(MSG_ST_DAEMON_STOPPED));
        else
            printf("%s", ws_msg(MSG_ST_DAEMON_UNKNOWN));
    } else {
        if (flock(fd, LOCK_SH | LOCK_NB) == 0) {
            flock(fd, LOCK_UN);
            printf("%s", ws_msg(MSG_ST_DAEMON_STOPPED));
        } else if (errno == EWOULDBLOCK) {
            printf(ws_msg(MSG_ST_DAEMON_RUNNING), ws_read_pidfile());
        } else {
            printf("%s", ws_msg(MSG_ST_DAEMON_UNKNOWN));
        }
        close(fd);
    }

    ws_lang_available(avail, sizeof avail);
    printf(ws_msg(MSG_ST_LANG), ws_lang_current(), avail);

    printf("%s", ws_msg(MSG_ST_SOURCES));
    for (size_t i = 0; i < ws_nsources; i++) {
        char why[96] = "";
        int ok = ws_source_probe(&ws_sources[i], why, sizeof why) == 0;

        printf("  %-9s %-13s %-12s%s%s\n", ws_sources[i].name,
               ws_msg(ok ? MSG_ST_AVAILABLE : MSG_ST_UNAVAILABLE),
               ws_msg(ws_sources[i].implemented ? MSG_ST_IMPLEMENTED : MSG_ST_PLANNED),
               why[0] ? " - " : "", why);
    }
    return 0;
}

/* ---- log ---------------------------------------------------------------- */

static int cmd_log(int follow, int clear)
{
    char buf[8192];
    off_t pos = 0;
    int fd;

    if (clear) {
        /* truncate in place; the daemon appends with O_APPEND so it keeps working */
        fd = open(WS_LOG_PATH, O_WRONLY | O_TRUNC | O_CLOEXEC);
        if (fd < 0) {
            fail_open(WS_LOG_PATH);
            return 1;
        }
        close(fd);
        printf("%s", ws_msg(MSG_LOG_CLEARED));
        return 0;
    }

    fd = open(WS_LOG_PATH, O_RDONLY | O_CLOEXEC);
    if (fd < 0) {
        fail_open(WS_LOG_PATH);
        return 1;
    }
    for (;;) {
        ssize_t n = read(fd, buf, sizeof buf);

        if (n > 0) {
            if (ws_write_all(STDOUT_FILENO, buf, (size_t)n) < 0)
                break;
            pos += n;
            continue;
        }
        if (n < 0 && errno != EINTR) {
            perror(PROG ": read");
            break;
        }
        if (!follow)
            break;
        {
            struct stat st;
            struct timespec ts = { 0, 200 * 1000 * 1000 };

            if (fstat(fd, &st) == 0 && st.st_size < pos) { /* log was cleared */
                lseek(fd, 0, SEEK_SET);
                pos = 0;
                continue;
            }
            nanosleep(&ts, NULL);
        }
    }
    close(fd);
    return 0;
}

/* ---- export ------------------------------------------------------------- */

static int cmd_export(const char *dir)
{
    char path[PATH_MAX], stamp[32], buf[8192];
    time_t t = time(NULL);
    struct tm tm;
    struct stat st;
    off_t left, copied = 0, last_nl_end = 0;
    int in, out = -1;

    if (ws_mkdir_p(dir, 0755) < 0) {
        fprintf(stderr, ws_msg(MSG_EXPORT_FAILED), PROG, strerror(errno));
        return 1;
    }
    in = open(WS_LOG_PATH, O_RDONLY | O_CLOEXEC);
    if (in < 0) {
        fail_open(WS_LOG_PATH);
        return 1;
    }
    if (fstat(in, &st) < 0) {
        fprintf(stderr, ws_msg(MSG_EXPORT_FAILED), PROG, strerror(errno));
        close(in);
        return 1;
    }

    localtime_r(&t, &tm);
    strftime(stamp, sizeof stamp, "%Y%m%d-%H%M%S", &tm);
    /* never overwrite an earlier export: O_EXCL, with a numeric suffix on clash */
    for (int i = 0; i < 100 && out < 0; i++) {
        if (i == 0)
            snprintf(path, sizeof path, "%s/sealmsg-%s.log", dir, stamp);
        else
            snprintf(path, sizeof path, "%s/sealmsg-%s-%d.log", dir, stamp, i + 1);
        out = open(path, O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, 0644);
        if (out < 0 && errno != EEXIST) {
            fprintf(stderr, ws_msg(MSG_CANNOT_OPEN), PROG, path, strerror(errno));
            close(in);
            return 1;
        }
    }
    if (out < 0) {
        fprintf(stderr, ws_msg(MSG_EXPORT_FAILED), PROG, strerror(EEXIST));
        close(in);
        return 1;
    }

    /* the daemon keeps appending: copy only the size seen now, and drop any
     * trailing half-written line so the export ends on a complete record */
    left = st.st_size;
    while (left > 0) {
        size_t want = left < (off_t)sizeof buf ? (size_t)left : sizeof buf;
        ssize_t n = read(in, buf, want);

        if (n < 0 && errno == EINTR)
            continue;
        if (n <= 0)
            break;
        if (ws_write_all(out, buf, (size_t)n) < 0) {
            fprintf(stderr, ws_msg(MSG_EXPORT_FAILED), PROG, strerror(errno));
            close(in);
            close(out);
            unlink(path);
            return 1;
        }
        for (ssize_t k = 0; k < n; k++)
            if (buf[k] == '\n')
                last_nl_end = copied + k + 1;
        copied += n;
        left -= n;
    }
    if (copied > last_nl_end && ftruncate(out, last_nl_end) < 0)
        perror(PROG ": ftruncate");
    fsync(out);
    close(out);
    close(in);
    puts(path);
    return 0;
}

/* ---- main --------------------------------------------------------------- */

int main(int argc, char **argv)
{
    const char *cmd = NULL, *path = NULL, *lang = NULL, *v;
    const char *bad = NULL;
    int follow = 0, clear = 0, help = 0;

    for (int i = 1; i < argc; i++) {
        const char *a = argv[i];

        if (ws_opt_match(a, "lang", &v)) {
            if (!v) {
                if (i + 1 >= argc) {
                    bad = a;
                    break;
                }
                v = argv[++i];
            }
            lang = v;
        } else if (strcmp(a, "-h") == 0 || ws_opt_match(a, "help", &v)) {
            help = 1;
        } else if (!cmd && a[0] != '-') {
            cmd = a;
        } else if (cmd && strcmp(cmd, "log") == 0 && strcmp(a, "-W") == 0) {
            follow = 1;
        } else if (cmd && strcmp(cmd, "log") == 0 && strcmp(a, "-c") == 0) {
            clear = 1;
        } else if (cmd && strcmp(cmd, "export") == 0 && !path && a[0] != '-') {
            path = a;
        } else {
            bad = a;
            break;
        }
    }

    if (lang)
        ws_lang_set(lang); /* unknown locale: falls back to English silently here;
                              the daemon is the one that logs the warning */
    else
        ws_lang_auto();

    if (bad) {
        fprintf(stderr, ws_msg(MSG_UNKNOWN_ARG), PROG, bad);
        fputs(ws_msg(MSG_USAGE_CTL), stderr);
        return 2;
    }
    if (help || !cmd) {
        fputs(ws_msg(MSG_USAGE_CTL), help ? stdout : stderr);
        return help ? 0 : 2;
    }

    if (strcmp(cmd, "status") == 0)
        return cmd_status();
    if (strcmp(cmd, "log") == 0)
        return cmd_log(follow, clear);
    if (strcmp(cmd, "export") == 0) {
        if (!path) {
            fputs(ws_msg(MSG_NEED_PATH), stderr);
            return 2;
        }
        return cmd_export(path);
    }
    fprintf(stderr, ws_msg(MSG_UNKNOWN_ARG), PROG, cmd);
    fputs(ws_msg(MSG_USAGE_CTL), stderr);
    return 2;
}
