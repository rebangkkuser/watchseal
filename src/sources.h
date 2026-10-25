#ifndef WS_SOURCES_H
#define WS_SOURCES_H

#include <stddef.h>

struct ws_source {
    const char *name;
    int implemented; /* 1 = the daemon really uses it, 0 = only planned */
    /* returns 0 if usable by this process; otherwise -1 with a reason in why */
    int (*probe)(char *why, size_t sz);
};

extern const struct ws_source ws_sources[];
extern const size_t ws_nsources;

/* Runs the probe in a forked child, so a syscall killed by seccomp (SIGSYS,
 * common for app processes such as Termux) only kills the child. */
int ws_source_probe(const struct ws_source *s, char *why, size_t sz);

#endif
