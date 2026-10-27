// moonlight-common-c's log callback is printf-style variadic, which stable Rust
// can't implement. Format here and hand Rust a finished line instead.
#include <stdarg.h>
#include <stdio.h>
#include <string.h>

typedef void (*mlw_log_sink_t)(const char* line);

static mlw_log_sink_t g_sink = NULL;

void mlw_set_log_sink(mlw_log_sink_t sink) {
    g_sink = sink;
}

void mlw_log_message(const char* format, ...) {
    char buf[1024];
    va_list ap;
    va_start(ap, format);
    vsnprintf(buf, sizeof(buf), format, ap);
    va_end(ap);

    // Messages usually end in '\n'; the Rust side logs whole lines.
    size_t len = strlen(buf);
    while (len > 0 && (buf[len - 1] == '\n' || buf[len - 1] == '\r')) {
        buf[--len] = '\0';
    }
    if (g_sink != NULL) {
        g_sink(buf);
    }
}
