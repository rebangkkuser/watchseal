#ifndef WS_COMMON_H
#define WS_COMMON_H

#include <stddef.h>
#include <sys/types.h>

#define WS_VERSION "0.1.0-dev"

/* Paths can be overridden at build time (useful for host-side tests). */
#ifndef WS_LOG_PATH
#define WS_LOG_PATH  "/data/sealmsg"
#endif
#ifndef WS_PID_PATH
#define WS_PID_PATH  "/data/sealmsg.pid"
#endif
#ifndef WS_LOCK_PATH
#define WS_LOCK_PATH "/data/sealmsg.lock"
#endif

int ws_write_all(int fd, const void *buf, size_t len);
int ws_mkdir_p(const char *path, mode_t mode);
int ws_read_pidfile(void);

/*
 * Matches a long option written as -name or --name, optionally followed by
 * =value. Returns 1 on match; *val is the value or NULL if there is none.
 */
int ws_opt_match(const char *arg, const char *name, const char **val);

#endif
