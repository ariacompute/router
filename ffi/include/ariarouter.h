#ifndef ARIAROUTER_H
#define ARIAROUTER_H

#ifdef __cplusplus
extern "C" {
#endif

#include <stddef.h>

typedef struct Ariarouter Ariarouter;

const char *ariarouter_last_error(void);

Ariarouter *ariarouter_init(const char *config_path);
Ariarouter *ariarouter_connect(const char *base_url);
void ariarouter_destroy(Ariarouter *router);

int ariarouter_complete(
    Ariarouter *router,
    const char *messages_json,
    const char *options_json,
    char *out,
    size_t out_len
);

int ariarouter_complete_stream(
    Ariarouter *router,
    const char *messages_json,
    const char *options_json,
    char *out,
    size_t out_len,
    void (*callback)(const char *chunk, void *user_data),
    void *user_data
);

int ariarouter_models(Ariarouter *router, char *out, size_t out_len);
int ariarouter_last_route(Ariarouter *router, char *out, size_t out_len);

#ifdef __cplusplus
}
#endif

#endif /* ARIAROUTER_H */
