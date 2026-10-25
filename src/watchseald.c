/*
 * watchseald - Android event monitoring daemon.
 *
 * Implemented in this version:
 *   - single instance (flock + pid file), clean shutdown on signals
 *   - persistent append-only log
 *   - process appear/disappear detection by polling /proc
 *
 * Not implemented yet: proc connector, fanotify/inotify, kmsg/klogctl,
 * SELinux audit. Nothing is reported that was not actually observed.
 */
#include "common.h"
#include "i18n.h"

#include <ctype.h>
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/file.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#define PFX_INFO "watchseal info:"
#define PFX_WARN "watchseal warn:"
#define PFX_NEW  "watchseal: uaahhh!!:"
#define PFX_GONE "watchseal arf:"
#define PFX_STOP "watchseal mrr...:"

#define SCAN_INTERVAL_SEC 1

static volatile sig_atomic_t g_stop;
static int g_log_fd = -1;

static void on_signal(int sig)
{
    g_stop = sig;
}

static void ws_log(const char *prefix, const char *fmt, ...)
    __attribute__((format(printf, 2, 3)));

static void ws_log(const char *prefix, const char *fmt, ...)
{
    char line[1024], ts[32];
    time_t t = time(NULL);
    struct tm tm;
    va_list ap;
    int n, m;
    size_t len;

    if (g_log_fd < 0)
        return;
    localtime_r(&t, &tm);
    strftime(ts, sizeof ts, "%Y-%m-%d %H:%M:%S", &tm);
    n = snprintf(line, sizeof line, "%s %s ", ts, prefix);
    va_start(ap, fmt);
    m = vsnprintf(line + n, sizeof line - (size_t)n, fmt, ap);
    va_end(ap);
    len = (size_t)n + (m < 0 ? 0 : (size_t)m);
    if (len > sizeof line - 2)
        len = sizeof line - 2;
    line[len++] = '\n';
    /* one write() on an O_APPEND fd: lines never interleave or tear */
    (void)ws_write_all(g_log_fd, line, len);
}

/* ---- process snapshots -------------------------------------------------- */

struct proc {
    int pid;
    int ppid;
    unsigned uid;
    unsigned long long start; /* starttime in clock ticks: tells pid reuse apart */
    char comm[32];
};

struct plist {
    struct proc *v;
    size_t n, cap;
};

static int read_proc(int pid, struct proc *p)
{
    char path[64], buf[1024];
    struct stat st;
    char *lp, *rp, *s;
    size_t cl;
    int fd, field = 3;
    ssize_t n;

    snprintf(path, sizeof path, "/proc/%d/stat", pid);
    fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0)
        return -1;
    n = read(fd, buf, sizeof buf - 1);
    close(fd);
    if (n <= 0)
        return -1;
    buf[n] = '\0';

    lp = strchr(buf, '(');
    rp = strrchr(buf, ')'); /* comm may itself contain ')' */
    if (!lp || !rp || rp < lp)
        return -1;
    cl = (size_t)(rp - lp - 1);
    if (cl >= sizeof p->comm)
        cl = sizeof p->comm - 1;
    memcpy(p->comm, lp + 1, cl);
    p->comm[cl] = '\0';
    for (size_t i = 0; i < cl; i++)
        if ((unsigned char)p->comm[i] < 0x20 || p->comm[i] == 0x7f)
            p->comm[i] = '?';

    /* fields after ')': state is field 3, ppid 4, starttime 22 */
    p->pid = pid;
    p->ppid = 0;
    p->start = 0;
    s = rp + 1;
    while (*s) {
        char *e;

        while (*s == ' ')
            s++;
        if (!*s)
            break;
        e = s;
        while (*e && *e != ' ')
            e++;
        if (field == 4)
            p->ppid = atoi(s);
        else if (field == 22) {
            p->start = strtoull(s, NULL, 10);
            break;
        }
        field++;
        s = e;
    }
    if (field != 22)
        return -1;

    snprintf(path, sizeof path, "/proc/%d", pid);
    p->uid = stat(path, &st) == 0 ? (unsigned)st.st_uid : (unsigned)-1;
    return 0;
}

static int cmp_pid(const void *a, const void *b)
{
    const struct proc *x = a, *y = b;

    return (x->pid > y->pid) - (x->pid < y->pid);
}

static int plist_add(struct plist *l, const struct proc *p)
{
    if (l->n == l->cap) {
        size_t ncap = l->cap ? l->cap * 2 : 256;
        struct proc *nv = realloc(l->v, ncap * sizeof *nv);

        if (!nv)
            return -1;
        l->v = nv;
        l->cap = ncap;
    }
    l->v[l->n++] = *p;
    return 0;
}

/* Kernel threads (pid 2 and its children) churn constantly and are not
 * "processes" in the sense users care about, so they are left out. */
static int is_kernel_thread(const struct proc *p)
{
    return p->pid == 2 || p->ppid == 2;
}

static int scan(struct plist *out)
{
    DIR *d = opendir("/proc");
    struct dirent *e;

    if (!d)
        return -1;
    out->n = 0;
    while ((e = readdir(d)) != NULL) {
        struct proc p;
        char *end;
        long pid;

        if (!isdigit((unsigned char)e->d_name[0]))
            continue;
        pid = strtol(e->d_name, &end, 10);
        if (*end != '\0' || pid <= 0)
            continue;
        if (read_proc((int)pid, &p) < 0)
            continue; /* vanished while scanning */
        if (is_kernel_thread(&p))
            continue;
        if (plist_add(out, &p) < 0)
            break;
    }
    closedir(d);
    if (out->n)
        qsort(out->v, out->n, sizeof *out->v, cmp_pid);
    return 0;
}

static void report_new(const struct proc *p)
{
    ws_log(PFX_NEW, "new process pid=%d ppid=%d uid=%u name=%s",
           p->pid, p->ppid, p->uid, p->comm);
}

static void report_gone(const struct proc *p)
{
    /* /proc cannot tell us HOW it ended, so we do not claim to know. */
    ws_log(PFX_GONE, "process gone from /proc pid=%d name=%s (exit reason unknown)",
           p->pid, p->comm);
}

static void diff(const struct plist *a, const struct plist *b)
{
    size_t i = 0, j = 0;

    while (i < a->n || j < b->n) {
        if (j >= b->n || (i < a->n && a->v[i].pid < b->v[j].pid)) {
            report_gone(&a->v[i++]);
        } else if (i >= a->n || b->v[j].pid < a->v[i].pid) {
            report_new(&b->v[j++]);
        } else {
            if (a->v[i].start != b->v[j].start) { /* pid was reused */
                report_gone(&a->v[i]);
                report_new(&b->v[j]);
            }
            i++;
            j++;
        }
    }
}

/* ---- main --------------------------------------------------------------- */

static void write_pidfile(void)
{
    char buf[32];
    int fd = open(WS_PID_PATH, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0644);

    if (fd < 0)
        return;
    ws_write_all(fd, buf, (size_t)snprintf(buf, sizeof buf, "%d\n", getpid()));
    close(fd);
}

int main(int argc, char **argv)
{
    const char *lang_req = NULL, *v;
    int foreground = 0, want_help = 0, want_version = 0, bad = 0;
    const char *bad_arg = NULL;
    int lang_missing;
    int lock_fd;
    struct sigaction sa;
    struct plist prev = {0}, cur = {0};
    int scan_failed = 0;

    for (int i = 1; i < argc; i++) {
        const char *a = argv[i];

        if (ws_opt_match(a, "lang", &v)) {
            if (!v) {
                if (i + 1 < argc) {
                    v = argv[++i];
                } else {
                    bad = 1;
                    bad_arg = a;
                    break;
                }
            }
            lang_req = v;
        } else if (strcmp(a, "-f") == 0 || ws_opt_match(a, "foreground", &v)) {
            foreground = 1;
        } else if (strcmp(a, "-h") == 0 || ws_opt_match(a, "help", &v)) {
            want_help = 1;
        } else if (ws_opt_match(a, "version", &v)) {
            want_version = 1;
        } else {
            bad = 1;
            bad_arg = a;
            break;
        }
    }

    /* explicit --lang wins; otherwise auto-detect */
    if (lang_req)
        lang_missing = ws_lang_set(lang_req) < 0;
    else {
        ws_lang_auto();
        lang_missing = 0;
    }

    if (want_version) {
        puts("watchseald " WS_VERSION);
        return 0;
    }
    if (bad) {
        fprintf(stderr, ws_msg(MSG_UNKNOWN_ARG), "watchseald", bad_arg);
        fputs(ws_msg(MSG_USAGE_DAEMON), stderr);
        return 2;
    }
    if (want_help) {
        fputs(ws_msg(MSG_USAGE_DAEMON), stdout);
        return 0;
    }

    if (lang_missing) {
        char avail[64];

        ws_lang_available(avail, sizeof avail);
        fprintf(stderr, "watchseald: locale '%s' not found, using 'en' (available: %s)\n",
                lang_req, avail);
    }

    /* single instance: the kernel drops the lock if we die, so no stale locks */
    lock_fd = open(WS_LOCK_PATH, O_RDWR | O_CREAT | O_CLOEXEC, 0644);
    if (lock_fd < 0) {
        fprintf(stderr, ws_msg(MSG_CANNOT_OPEN), "watchseald", WS_LOCK_PATH, strerror(errno));
        if (errno == EACCES || errno == EPERM || errno == EROFS)
            fputs(ws_msg(MSG_HINT_ROOT), stderr);
        return 1;
    }
    if (flock(lock_fd, LOCK_EX | LOCK_NB) < 0) {
        if (errno == EWOULDBLOCK)
            fprintf(stderr, ws_msg(MSG_ALREADY_RUNNING), ws_read_pidfile());
        else
            fprintf(stderr, ws_msg(MSG_CANNOT_OPEN), "watchseald", WS_LOCK_PATH, strerror(errno));
        return 1;
    }

    g_log_fd = open(WS_LOG_PATH, O_WRONLY | O_APPEND | O_CREAT | O_CLOEXEC, 0644);
    if (g_log_fd < 0) {
        fprintf(stderr, ws_msg(MSG_CANNOT_OPEN), "watchseald", WS_LOG_PATH, strerror(errno));
        return 1;
    }

    if (!foreground) {
        pid_t pid = fork();

        if (pid < 0) {
            perror("fork");
            return 1;
        }
        if (pid > 0) { /* parent: the child inherits the lock and keeps it */
            printf(ws_msg(MSG_STARTED), (int)pid);
            fflush(stdout);
            _exit(0);
        }
        setsid();
        if (chdir("/") < 0) { /* not fatal */ }
        int nul = open("/dev/null", O_RDWR);
        if (nul >= 0) {
            dup2(nul, 0);
            dup2(nul, 1);
            dup2(nul, 2);
            if (nul > 2)
                close(nul);
        }
    }

    write_pidfile();

    memset(&sa, 0, sizeof sa);
    sa.sa_handler = on_signal; /* no SA_RESTART: nanosleep returns on a signal */
    sigaction(SIGTERM, &sa, NULL);
    sigaction(SIGINT, &sa, NULL);
    signal(SIGHUP, SIG_IGN);
    signal(SIGPIPE, SIG_IGN);

    ws_log(PFX_INFO, "watchseald %s started pid=%d", WS_VERSION, (int)getpid());
    if (lang_missing) {
        char avail[64];

        ws_lang_available(avail, sizeof avail);
        ws_log(PFX_WARN, "locale '%s' not found, using 'en' (available: %s)", lang_req, avail);
    }
    ws_log(PFX_INFO, "sources: proc=polling(%ds); kernel threads excluded; "
           "processes shorter than the interval can be missed", SCAN_INTERVAL_SEC);

    if (scan(&prev) < 0) {
        ws_log(PFX_WARN, "cannot read /proc: %s; process monitoring unavailable", strerror(errno));
        scan_failed = 1;
    } else {
        ws_log(PFX_INFO, "baseline: %zu processes (already running, not reported as new)", prev.n);
    }

    while (!g_stop) {
        struct timespec ts = { SCAN_INTERVAL_SEC, 0 };
        struct plist tmp;

        nanosleep(&ts, NULL);
        if (g_stop)
            break;
        if (scan(&cur) < 0) {
            if (!scan_failed)
                ws_log(PFX_WARN, "cannot read /proc: %s", strerror(errno));
            scan_failed = 1;
            continue;
        }
        if (scan_failed) {
            ws_log(PFX_INFO, "/proc readable again; new baseline: %zu processes", cur.n);
            scan_failed = 0;
        } else {
            diff(&prev, &cur);
        }
        tmp = prev;
        prev = cur;
        cur = tmp;
    }

    ws_log(PFX_STOP, "watchseald stopping (signal %d)", (int)g_stop);
    unlink(WS_PID_PATH);
    free(prev.v);
    free(cur.v);
    return 0;
}
