#ifndef WS_I18N_H
#define WS_I18N_H

#include <stddef.h>

/* User-facing terminal text only. The event log itself is always English. */
enum ws_msg {
    MSG_USAGE_DAEMON,
    MSG_USAGE_CTL,
    MSG_ALREADY_RUNNING,
    MSG_STARTED,
    MSG_CANNOT_OPEN,
    MSG_HINT_ROOT,
    MSG_UNKNOWN_ARG,
    MSG_NEED_PATH,
    MSG_EXPORT_FAILED,
    MSG_LOG_CLEARED,
    MSG_ST_DAEMON_RUNNING,
    MSG_ST_DAEMON_STOPPED,
    MSG_ST_DAEMON_UNKNOWN,
    MSG_ST_LANG,
    MSG_ST_SOURCES,
    MSG_ST_AVAILABLE,
    MSG_ST_UNAVAILABLE,
    MSG_ST_IMPLEMENTED,
    MSG_ST_PLANNED,
    MSG_COUNT
};

/* Selects a locale by code or alias ("pt-br", "pt", "pt_BR.UTF-8", ...).
 * "" and "auto" run auto-detection. Returns 0 if found, -1 if not found
 * (in which case English is used). */
int ws_lang_set(const char *name);

/* Detects the language from Android properties, then LC_ALL/LANGUAGE/LANG. */
void ws_lang_auto(void);

const char *ws_lang_current(void);
void ws_lang_available(char *buf, size_t sz);
const char *ws_msg(enum ws_msg id);

#endif
