#ifndef TUYA_RULE_ENGINE_H
#define TUYA_RULE_ENGINE_H
#include <stddef.h>
#ifdef __cplusplus
extern "C" {
#endif
/* Borrowed UTF-8 NUL-terminated request, owned UTF-8 JSON response.
   Release every response exactly once with tuya_engine_free from this library.
   Null input returns an error; free(NULL) is allowed. Invalid pointers are UB. */
char *tuya_engine_eval(const char *request_json);
void tuya_engine_free(char *response_json);
/* For hosts that must write the request into this library's memory (WebAssembly):
   a zeroed buffer of len bytes (NULL for 0), released with tuya_engine_dealloc(buffer, len). */
unsigned char *tuya_engine_alloc(size_t len);
void tuya_engine_dealloc(unsigned char *buffer, size_t len);
#ifdef __cplusplus
}
#endif
#endif
