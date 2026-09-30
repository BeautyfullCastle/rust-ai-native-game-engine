/*
 * orrery.h - the C ABI of an Orrery simulation host (crate orr_ffi).
 *
 * A view written in C, C++, C#, GDScript (GDExtension) or any language that
 * loads a C library can host a deterministic simulation, read what to draw
 * as a stream of bytes and feed it inputs. No Rust type crosses this
 * boundary. Byte formats: docs/view-stream.md in the Orrery repository.
 *
 * The game is chosen when the library is built (this build hosts the physics
 * demo, "PhysGame"); the header is the same for every game.
 *
 * Conventions
 *  - Functions return int codes: ORR_OK (0), ORR_NO_FRAME (1, nothing new,
 *    not an error), negative = error. orr_last_error() describes the last
 *    error of the calling thread.
 *  - Output buffers: if the buffer is too small, the function reports the
 *    size needed and writes / consumes nothing.
 *  - All numbers in the streams are little-endian.
 *  - Threads: a handle is internally locked; calls from several threads run
 *    one after another. orr_host_close must not run concurrently with any
 *    other call on the same handle.
 */
#ifndef ORRERY_H
#define ORRERY_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#if defined(_WIN32) && defined(ORR_FFI_DLL_IMPORT)
#define ORR_API __declspec(dllimport)
#else
#define ORR_API
#endif

/* Version of this ABI. Compare with orr_abi_version() after loading the library. */
#define ORR_ABI_VERSION 1u

/* Return codes. */
#define ORR_OK 0
#define ORR_NO_FRAME 1        /* nothing new to read */
#define ORR_ERR_NULL (-1)     /* a required pointer was null */
#define ORR_ERR_ARG (-2)      /* invalid argument (size, op, JSON, path) */
#define ORR_ERR_BUFFER (-3)   /* buffer too small; the size needed was reported */
#define ORR_ERR_HOST (-4)     /* the host thread is gone */
#define ORR_ERR_RPC (-5)      /* the host refused the request */
#define ORR_ERR_PANIC (-6)    /* a panic was caught inside the library (a bug) */

/* orr_control ops. */
#define ORR_CTL_PLAY 0        /* run by the wall clock */
#define ORR_CTL_PAUSE 1
#define ORR_CTL_STEP 2        /* arg = ticks to run now (>= 1) */
#define ORR_CTL_SEEK 3        /* arg = recorded tick to go to (pauses) */
#define ORR_CTL_SPEED 4       /* arg = speed in thousandths, 1000 = 1x */
#define ORR_CTL_BRANCH 5      /* drop the recorded future after the head */
#define ORR_CTL_RESTART 6     /* fresh session from the scene: tick 0, paused */

/* OrrHostConfig.flags */
/* also serve ERP on 127.0.0.1:listen_port (WebSocket + TCP). The socket has NO
 * authentication (loopback only, any local process can drive the host): for
 * development and devkits, not for shipping. */
#define ORR_HOST_LISTEN 1u
#define ORR_HOST_RUN 2u       /* start running by the wall clock (default: paused, use ORR_CTL_STEP) */

typedef struct OrrHost OrrHost; /* opaque */

typedef struct OrrHostConfig {
    uint32_t struct_size;  /* sizeof(OrrHostConfig): set it */
    uint32_t flags;        /* ORR_HOST_* */
    uint32_t listen_port;  /* with ORR_HOST_LISTEN: port, 0 = any free port (see orr_host_url) */
} OrrHostConfig;

/* ---- library ---- */

ORR_API uint32_t orr_abi_version(void);

/* Text of the last error of the calling thread; never null; valid until the
 * next call of this library on the same thread. */
ORR_API const char* orr_last_error(void);

/* ---- host ---- */

/* Starts a host thread of the compiled-in game on the scene YAML at
 * scene_path (UTF-8; NULL = the built-in demo scene) and opens a paused play
 * session. cfg may be NULL. Returns NULL on failure (see orr_last_error). */
ORR_API OrrHost* orr_host_open(const char* scene_path, const OrrHostConfig* cfg);

/* Stops the host and frees the handle. NULL is ignored. */
ORR_API void orr_host_close(OrrHost* host);

/* Writes the schema JSON (NUL-terminated) into buf if it fits. Returns the
 * size needed including the NUL, 0 on error. orr_schema_json(h, NULL, 0) asks
 * for the size. The schema lists the entity kinds, the byte layout of the
 * input (size and fields with offsets and types), the command size and the
 * event types. */
ORR_API size_t orr_schema_json(OrrHost* host, char* buf, size_t cap);

/* The ws:// URL of the host's ERP socket, same rules as orr_schema_json;
 * returns 0 if the host does not listen. Out-of-process views connect there
 * and subscribe to the "viewstream" topic (docs/view-stream.md). */
ORR_API size_t orr_host_url(OrrHost* host, char* buf, size_t cap);

/* ---- view stream ---- */

/* Copies the newest view frame not read yet into buf; *written = its size.
 *   ORR_OK          copied
 *   ORR_NO_FRAME    nothing new (*written = 0)
 *   ORR_ERR_BUFFER  cap too small: *written = size needed, the frame stays
 * Newer frames replace older unread ones: poll as often as you draw. */
ORR_API int orr_view_poll(OrrHost* host, uint8_t* buf, size_t cap, size_t* written);

/* Zero-copy variant: *data / *len point at the newest unread frame, valid
 * until the next orr_view_poll or orr_view_poll_ptr on this handle, or close.
 * ORR_NO_FRAME: nothing new (*data = NULL). */
ORR_API int orr_view_poll_ptr(OrrHost* host, const uint8_t** data, size_t* len);

/* Takes the oldest queued event batch message. Same buffer rules as
 * orr_view_poll. Events are queued, never replaced. */
ORR_API int orr_events_poll(OrrHost* host, uint8_t* buf, size_t cap, size_t* written);

/* ---- driving the simulation ---- */

/* Sets the held input of a player: exactly input.size bytes, laid out as the
 * schema's input.fields say. A "fixed" field is an int64 holding value*65536. */
ORR_API int orr_set_input(OrrHost* host, uint8_t player, const uint8_t* bytes, size_t len);

/* Queues a command (the game's command encoding, schema "command.size") for the next tick. */
ORR_API int orr_send_command(OrrHost* host, uint8_t player, const uint8_t* bytes, size_t len);

/* Timeline control (ORR_CTL_*). */
ORR_API int orr_control(OrrHost* host, int op, int64_t arg);

/* Calls any ERP method in process (the editor's whole method set).
 * request_json: {"method":"world.query","params":{...}}.
 * The answer is written to out as NUL-terminated JSON: {"result":...} or
 * {"error":{"code":..,"message":..}} (return ORR_ERR_RPC). *needed = answer
 * size including the NUL. If cap is too small nothing is written and
 * ORR_ERR_BUFFER is returned (the call was already made). */
ORR_API int orr_erp_call(OrrHost* host, const char* request_json, char* out, size_t cap, size_t* needed);

#ifdef __cplusplus
}
#endif

#endif /* ORRERY_H */
