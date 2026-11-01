#include "sources.h"

#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <linux/netlink.h>
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/inotify.h>
#include <sys/klog.h>
#include <sys/socket.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

static int fail(char *why, size_t sz)
{
    snprintf(why, sz, "%s", strerror(errno));
    return -1;
}

static int probe_proc(char *why, size_t sz)
{
    DIR *d = opendir("/proc");

    if (!d)
        return fail(why, sz);
    closedir(d);
    return 0;
}

static int probe_inotify(char *why, size_t sz)
{
    int fd = inotify_init1(IN_CLOEXEC);

    if (fd < 0)
        return fail(why, sz);
    close(fd);
    return 0;
}

static int probe_fanotify(char *why, size_t sz)
{
#ifdef __NR_fanotify_init
    /* FAN_CLOEXEC | FAN_CLASS_NOTIF; raw syscall avoids depending on headers */
    long fd = syscall(__NR_fanotify_init, 1u, O_RDONLY);

    if (fd < 0)
        return fail(why, sz);
    close((int)fd);
    return 0;
#else
    snprintf(why, sz, "syscall not known at build time");
    return -1;
#endif
}

static int probe_kmsg_dev(char *why, size_t sz)
{
    int fd = open("/dev/kmsg", O_RDONLY | O_NONBLOCK | O_CLOEXEC);

    if (fd < 0)
        return fail(why, sz);
    close(fd);
    return 0;
}

static int probe_klogctl(char *why, size_t sz)
{
    char c;

    /* SYSLOG_ACTION_READ_ALL (3): reads without consuming the ring buffer */
    if (klogctl(3, &c, 1) < 0)
        return fail(why, sz);
    return 0;
}

static int probe_audit(char *why, size_t sz)
{
    int fd = socket(AF_NETLINK, SOCK_RAW | SOCK_CLOEXEC, 9 /* NETLINK_AUDIT */);

    if (fd < 0)
        return fail(why, sz);
    close(fd);
    snprintf(why, sz, "socket opens; reading events not verified");
    return 0;
}

static int probe_logcat(char *why, size_t sz)
{
    if (access("/system/bin/logcat", X_OK) < 0)
        return fail(why, sz);
    return 0;
}

const struct ws_source ws_sources[] = {
    { "proc",     1, probe_proc },
    { "inotify",  0, probe_inotify },
    { "fanotify", 0, probe_fanotify },
    { "klogctl",  0, probe_klogctl },
    { "kmsg",     0, probe_kmsg_dev },
    { "audit",    0, probe_audit },
    { "logcat",   0, probe_logcat },
};
const size_t ws_nsources = sizeof ws_sources / sizeof ws_sources[0];

int ws_source_probe(const struct ws_source *s, char *why, size_t sz)
{
    int pfd[2], status = 0;
    pid_t pid;
    ssize_t n;

    why[0] = '\0';
    if (pipe(pfd) < 0)
        return fail(why, sz);
    fflush(NULL);
    pid = fork();
    if (pid < 0) {
        close(pfd[0]);
        close(pfd[1]);
        return fail(why, sz);
    }
    if (pid == 0) {
        char buf[96] = "";
        int rc;

        close(pfd[0]);
        rc = s->probe(buf, sizeof buf);
        if (write(pfd[1], buf, strlen(buf)) < 0) { /* parent just sees no reason */ }
        _exit(rc == 0 ? 0 : 1);
    }
    close(pfd[1]);
    n = read(pfd[0], why, sz - 1);
    why[n > 0 ? n : 0] = '\0';
    close(pfd[0]);
    while (waitpid(pid, &status, 0) < 0 && errno == EINTR)
        ;
    if (WIFSIGNALED(status)) {
        int sig = WTERMSIG(status);

        snprintf(why, sz, "blocked by the system (signal %d%s), likely seccomp",
                 sig, sig == SIGSYS ? "/SIGSYS" : "");
        return -1;
    }
    return WIFEXITED(status) && WEXITSTATUS(status) == 0 ? 0 : -1;
}
