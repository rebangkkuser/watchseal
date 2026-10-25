#include "i18n.h"

#include <ctype.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef __ANDROID__
#include <sys/system_properties.h>
#endif

static const char *const msgs_en[MSG_COUNT] = {
    [MSG_USAGE_DAEMON] =
        "usage: watchseald [options]\n"
        "  -f, --foreground   do not detach from the terminal\n"
        "      --lang=CODE    message language (en, pt-br, pt, auto)\n"
        "  -h, --help         show this help\n"
        "      --version      show version\n",
    [MSG_USAGE_CTL] =
        "usage: watchsealctl [--lang=CODE] <command>\n"
        "  status             daemon state and monitoring sources\n"
        "  log                print the event history\n"
        "  log -W             follow new events\n"
        "  log -c             clear the history (truncates the file)\n"
        "  export <dir>       copy the history to <dir>/sealmsg-DATE-TIME.log\n",
    [MSG_ALREADY_RUNNING]   = "watchseald: already running (pid %d)\n",
    [MSG_STARTED]           = "watchseald: started (pid %d)\n",
    [MSG_CANNOT_OPEN]       = "%s: cannot open '%s': %s\n",
    [MSG_HINT_ROOT]         = "hint: this probably needs root (e.g. su -c '...')\n",
    [MSG_UNKNOWN_ARG]       = "%s: unknown or invalid argument: %s\n",
    [MSG_NEED_PATH]         = "export: missing destination directory\n",
    [MSG_EXPORT_FAILED]     = "%s: export failed: %s\n",
    [MSG_LOG_CLEARED]       = "log cleared\n",
    [MSG_ST_DAEMON_RUNNING] = "daemon:   running (pid %d)\n",
    [MSG_ST_DAEMON_STOPPED] = "daemon:   not running\n",
    [MSG_ST_DAEMON_UNKNOWN] = "daemon:   unknown (cannot read the lock file)\n",
    [MSG_ST_LANG]           = "language: %s (available: %s)\n",
    [MSG_ST_SOURCES]        = "monitoring sources, as seen by this process:\n",
    [MSG_ST_AVAILABLE]      = "available",
    [MSG_ST_UNAVAILABLE]    = "unavailable",
    [MSG_ST_IMPLEMENTED]    = "implemented",
    [MSG_ST_PLANNED]        = "planned",
};

static const char *const msgs_pt_br[MSG_COUNT] = {
    [MSG_USAGE_DAEMON] =
        "uso: watchseald [opções]\n"
        "  -f, --foreground   não desanexa do terminal\n"
        "      --lang=COD     idioma das mensagens (en, pt-br, pt, auto)\n"
        "  -h, --help         mostra esta ajuda\n"
        "      --version      mostra a versão\n",
    [MSG_USAGE_CTL] =
        "uso: watchsealctl [--lang=COD] <comando>\n"
        "  status             estado do daemon e fontes de monitoramento\n"
        "  log                mostra o histórico de eventos\n"
        "  log -W             acompanha novos eventos\n"
        "  log -c             limpa o histórico (trunca o arquivo)\n"
        "  export <dir>       copia o histórico para <dir>/sealmsg-DATA-HORA.log\n",
    [MSG_ALREADY_RUNNING]   = "watchseald: já está rodando (pid %d)\n",
    [MSG_STARTED]           = "watchseald: iniciado (pid %d)\n",
    [MSG_CANNOT_OPEN]       = "%s: não foi possível abrir '%s': %s\n",
    [MSG_HINT_ROOT]         = "dica: provavelmente precisa de root (ex.: su -c '...')\n",
    [MSG_UNKNOWN_ARG]       = "%s: argumento desconhecido ou inválido: %s\n",
    [MSG_NEED_PATH]         = "export: falta o diretório de destino\n",
    [MSG_EXPORT_FAILED]     = "%s: falha ao exportar: %s\n",
    [MSG_LOG_CLEARED]       = "histórico limpo\n",
    [MSG_ST_DAEMON_RUNNING] = "daemon:   rodando (pid %d)\n",
    [MSG_ST_DAEMON_STOPPED] = "daemon:   parado\n",
    [MSG_ST_DAEMON_UNKNOWN] = "daemon:   desconhecido (não foi possível ler o arquivo de lock)\n",
    [MSG_ST_LANG]           = "idioma:   %s (disponíveis: %s)\n",
    [MSG_ST_SOURCES]        = "fontes de monitoramento, como este processo as enxerga:\n",
    [MSG_ST_AVAILABLE]      = "disponível",
    [MSG_ST_UNAVAILABLE]    = "indisponível",
    [MSG_ST_IMPLEMENTED]    = "implementada",
    [MSG_ST_PLANNED]        = "planejada",
};

struct ws_locale {
    const char *code;
    const char *alias;
    const char *const *msgs;
};

/* To add a language: add its message array above and one line here. */
static const struct ws_locale locales[] = {
    { "en",    NULL, msgs_en },
    { "pt-br", "pt", msgs_pt_br },
};
#define NLOCALES (sizeof locales / sizeof locales[0])

static const struct ws_locale *cur = &locales[0];

static void normalize(const char *in, char *out, size_t sz)
{
    size_t i = 0;

    for (; in[i] && in[i] != '.' && in[i] != '@' && in[i] != ':' && i + 1 < sz; i++) {
        char c = in[i] == '_' ? '-' : in[i];
        out[i] = (char)tolower((unsigned char)c);
    }
    out[i] = '\0';
}

static const struct ws_locale *find_exact(const char *norm)
{
    for (size_t i = 0; i < NLOCALES; i++)
        if (strcmp(locales[i].code, norm) == 0)
            return &locales[i];
    for (size_t i = 0; i < NLOCALES; i++)
        if (locales[i].alias && strcmp(locales[i].alias, norm) == 0)
            return &locales[i];
    return NULL;
}

static const struct ws_locale *find(const char *norm)
{
    const struct ws_locale *l;
    char base[16];
    size_t n = strcspn(norm, "-");

    if (norm[0] == '\0' || strcmp(norm, "c") == 0 || strcmp(norm, "posix") == 0)
        return &locales[0];
    l = find_exact(norm);
    if (l)
        return l;
    /* "pt-pt" -> "pt", "en-us" -> "en" */
    if (n > 0 && n < sizeof base && norm[n] == '-') {
        memcpy(base, norm, n);
        base[n] = '\0';
        return find_exact(base);
    }
    return NULL;
}

static void detect(char *out, size_t sz)
{
    const char *env;

    out[0] = '\0';
#ifdef __ANDROID__
    {
        char v[PROP_VALUE_MAX], c[PROP_VALUE_MAX];

        if (__system_property_get("persist.sys.locale", v) > 0 ||
            __system_property_get("ro.product.locale", v) > 0) {
            normalize(v, out, sz);
            return;
        }
        if (__system_property_get("persist.sys.language", v) > 0) {
            if (__system_property_get("persist.sys.country", c) > 0) {
                char both[PROP_VALUE_MAX * 2 + 2];
                size_t n = strlen(v);

                memcpy(both, v, n);
                both[n] = '-';
                strcpy(both + n + 1, c);
                normalize(both, out, sz);
            } else {
                normalize(v, out, sz);
            }
            return;
        }
    }
#endif
    env = getenv("LC_ALL");
    if (!env || !*env)
        env = getenv("LANGUAGE");
    if (!env || !*env)
        env = getenv("LANG");
    if (env && *env)
        normalize(env, out, sz);
}

void ws_lang_auto(void)
{
    char norm[64];
    const struct ws_locale *l;

    detect(norm, sizeof norm);
    l = find(norm);
    cur = l ? l : &locales[0];
}

int ws_lang_set(const char *name)
{
    char norm[64];
    const struct ws_locale *l;

    if (!name || !*name || strcmp(name, "auto") == 0) {
        ws_lang_auto();
        return 0;
    }
    normalize(name, norm, sizeof norm);
    l = find(norm);
    if (!l) {
        cur = &locales[0];
        return -1;
    }
    cur = l;
    return 0;
}

const char *ws_lang_current(void)
{
    return cur->code;
}

void ws_lang_available(char *buf, size_t sz)
{
    size_t used = 0;

    buf[0] = '\0';
    for (size_t i = 0; i < NLOCALES && used + 1 < sz; i++) {
        int n = snprintf(buf + used, sz - used, "%s%s", i ? ", " : "", locales[i].code);
        if (n < 0)
            break;
        used += (size_t)n;
    }
}

const char *ws_msg(enum ws_msg id)
{
    const char *m = cur->msgs[id];

    return m ? m : msgs_en[id];
}
