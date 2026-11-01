#include "common.h"

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

int ws_write_all(int fd, const void *buf, size_t len)
{
    const char *p = buf;
    while (len > 0) {
        ssize_t n = write(fd, p, len);
        if (n < 0) {
            if (errno == EINTR)
                continue;
            return -1;
        }
        p += n;
        len -= (size_t)n;
    }
    return 0;
}

int ws_mkdir_p(const char *path, mode_t mode)
{
    char tmp[PATH_MAX];
    size_t len = strlen(path);

    if (len == 0 || len >= sizeof tmp) {
        errno = ENAMETOOLONG;
        return -1;
    }
    memcpy(tmp, path, len + 1);
    for (char *p = tmp + 1; *p; p++) {
        if (*p != '/')
            continue;
        *p = '\0';
        if (mkdir(tmp, mode) < 0 && errno != EEXIST)
            return -1;
        *p = '/';
    }
    if (mkdir(tmp, mode) < 0 && errno != EEXIST)
        return -1;
    return 0;
}

int ws_read_pidfile(void)
{
    char buf[32];
    int fd = open(WS_PID_PATH, O_RDONLY | O_CLOEXEC);
    ssize_t n;

    if (fd < 0)
        return 0;
    n = read(fd, buf, sizeof buf - 1);
    close(fd);
    if (n <= 0)
        return 0;
    buf[n] = '\0';
    return atoi(buf);
}

int ws_opt_match(const char *arg, const char *name, const char **val)
{
    size_t n = strlen(name);

    if (arg[0] != '-')
        return 0;
    arg += (arg[1] == '-') ? 2 : 1;
    if (strncmp(arg, name, n) != 0)
        return 0;
    if (arg[n] == '\0') {
        *val = NULL;
        return 1;
    }
    if (arg[n] == '=') {
        *val = arg + n + 1;
        return 1;
    }
    return 0;
}
