#ifndef ARIA_ROUTER_H
#define ARIA_ROUTER_H

#ifdef __cplusplus
extern "C" {
#endif

#include <stddef.h>

typedef struct AriaRouter AriaRouter;

const char *aria_router_last_error(void);

AriaRouter *aria_router_init(const char *config_path);
AriaRouter *aria_router_connect(const char *base_url);
void aria_router_destroy(AriaRouter *router);

int aria_router_complete(
    AriaRouter *router,
    const char *messages_json,
    const char *options_json,
    char *out,
    size_t out_len
);

int aria_router_complete_stream(
    AriaRouter *router,
    const char *messages_json,
    const char *options_json,
    char *out,
    size_t out_len,
    void (*callback)(const char *chunk, void *user_data),
    void *user_data
);

int aria_router_models(AriaRouter *router, char *out, size_t out_len);
int aria_router_last_route(AriaRouter *router, char *out, size_t out_len);

#ifdef __cplusplus
}
#endif

#endif /* ARIA_ROUTER_H */
