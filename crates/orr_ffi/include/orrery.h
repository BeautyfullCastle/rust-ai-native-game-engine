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
#define ORR_ABI_VERSION 2u  /* 2: client sessions (orr_client_open, orr_session_status) */

/* Return codes. */
#define ORR_OK 0
#define ORR_NO_FRAME 1        /* nothing new to read */
#define ORR_ERR_NULL (-1)     /* a required pointer was null */
#define ORR_ERR_ARG (-2)      /* invalid argument (size, op, JSON, path) */
#define ORR_ERR_BUFFER (-3)   /* buffer too small; the size needed was reported */
#define ORR_ERR_HOST (-4)     /* the host thread is gone */
#define ORR_ERR_RPC (-5)      /* the host refused the request */
#define ORR_ERR_PANIC (-6)    /* a panic was caught inside the library (a bug) */
#define ORR_ERR_NOT_READY (-7) /* a client handle is still joining the room (see orr_session_status) */

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

/* OrrClientConfig.flags */
#define ORR_CLIENT_WAIT 1u      /* orr_client_open returns only when the room has started (or failed) */
#define ORR_CLIENT_INSECURE 2u  /* QUIC: accept any server certificate (development only) */

/* OrrClientConfig.transport */
#define ORR_TRANSPORT_QUIC 0u   /* needs fingerprint or ORR_CLIENT_INSECURE */
#define ORR_TRANSPORT_WS 1u     /* plain WebSocket (put TLS in a reverse proxy) */

/* OrrSessionStatus.mode / .state / .flags */
#define ORR_MODE_LOCAL 0u
#define ORR_MODE_CLIENT 1u
#define ORR_STATE_CONNECTING 0u   /* joining: connecting, handshake, waiting for the other players */
#define ORR_STATE_PLAYING 1u
#define ORR_STATE_DISCONNECTED 2u /* the connection to the server is gone */
#define ORR_STATE_FAILED 3u       /* joining failed (refused, unreachable, timed out) */
#define ORR_STATUS_DESYNC 1u      /* a desync was detected by the server's room */

typedef struct OrrHost OrrHost; /* opaque */

typedef struct OrrHostConfig {
    uint32_t struct_size;  /* sizeof(OrrHostConfig): set it */
    uint32_t flags;        /* ORR_HOST_* */
    uint32_t listen_port;  /* with ORR_HOST_LISTEN: port, 0 = any free port (see orr_host_url) */
} OrrHostConfig;

/* Settings of orr_client_open. Zero the struct, then set struct_size and server. */
typedef struct OrrClientConfig {
    uint32_t struct_size;         /* sizeof(OrrClientConfig): set it */
    uint32_t flags;               /* ORR_CLIENT_* */
    uint32_t transport;           /* ORR_TRANSPORT_* */
    int32_t slot;                 /* slot to ask for; negative = any free slot */
    uint64_t room;                /* room id; 0 = room 1 (the default room of orr_server) */
    uint64_t sim_seed;            /* seed of the simulated loss/jitter; 0 = fresh per client */
    uint32_t sim_latency_ms;      /* simulated one-way delay in each direction (testing) */
    uint32_t sim_jitter_ms;       /* simulated random extra delay, up to this much */
    uint32_t sim_loss_permille;   /* simulated loss of unreliable messages, each way: 20 = 2 % */
    uint32_t connect_timeout_ms;  /* wait for handshake + room start; 0 = 60 s */
    const char* server;           /* "host:port" (UTF-8), required */
    const char* fingerprint;      /* QUIC: server certificate SHA-256 as hex (printed by orr_server); NULL = none */
    const char* desync_dir;       /* where desync dumps (.orrd) go; NULL = a folder in the temp directory */
} OrrClientConfig;

/* What orr_session_status fills. Zero the struct and set struct_size; on return it holds the
 * number of bytes written (an older or newer library: the part both know). */
typedef struct OrrSessionStatus {
    uint32_t struct_size;         /* sizeof(OrrSessionStatus): set it */
    uint32_t mode;                /* ORR_MODE_* */
    uint32_t state;               /* ORR_STATE_* */
    uint32_t flags;               /* ORR_STATUS_* */
    uint32_t slot;                /* the joined slot (client) */
    uint32_t player_count;
    uint32_t rtt_ms;              /* smoothed round trip time to the server (client) */
    uint32_t input_delay;         /* input delay in ticks (client) */
    uint64_t head_tick;           /* predicted head tick */
    uint64_t verified_tick;       /* newest fully confirmed tick */
    uint64_t rollbacks;           /* rollbacks so far */
    uint64_t resim_ticks;         /* ticks resimulated by all rollbacks */
    uint64_t last_rollback_from;  /* the latest rollback resimulated from..to (0, 0 = none yet) */
    uint64_t last_rollback_to;
    uint64_t desyncs;             /* desyncs the room reported */
    uint64_t stall_episodes;      /* times the prediction limit stopped the sim */
    uint64_t stalled_ms;          /* total time stalled */
    uint64_t repeated_inputs;     /* ticks confirmed with a repeat of this client's input (it was late) */
} OrrSessionStatus;

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

/* Opens a handle that PLAYS on a relay server (orr_server --game physics) as a client:
 * it joins a room, predicts, rolls back, and publishes the same view stream and events as a
 * local host, but frames carry the rolled_back flag and range, and events arrive as predicted,
 * then verified or canceled (docs/view-stream.md). Returns at once with the handle in state
 * ORR_STATE_CONNECTING (poll orr_session_status; until it is PLAYING the view and schema calls
 * return ORR_NO_FRAME / ORR_ERR_NOT_READY), or waits for the room to start with ORR_CLIENT_WAIT.
 * orr_set_input works for the joined slot only; timeline controls, ERP calls and listening
 * belong to a local host and return ORR_ERR_ARG. Returns NULL on failure (see orr_last_error). */
ORR_API OrrHost* orr_client_open(const OrrClientConfig* cfg);

/* Fills *out with the state of the session (works on both kinds of handle). Set out->struct_size. */
ORR_API int orr_session_status(OrrHost* host, OrrSessionStatus* out);

/* The checksum of the confirmed (verified) state of a client session at `tick`: the value the
 * client also reports to the server, recorded at ticks that are multiples of the room's checksum
 * interval (30 by default). Peers that agree on a tick have the same state there. tick 0 = the
 * newest checkpoint. ORR_OK sets *found_tick and *checksum; ORR_NO_FRAME: not confirmed yet or
 * not a checkpoint tick; ORR_ERR_ARG on a local host. */
ORR_API int orr_confirmed_checksum(OrrHost* host, uint64_t tick, uint64_t* found_tick, uint64_t* checksum);

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
 *   ORR_ERR_NULL    buf is NULL with nonzero cap, even if nothing is ready;
 *                   *written = 0, no frame/event is consumed
 * NULL with cap 0 is a size probe: ORR_ERR_BUFFER if data exists, otherwise
 * ORR_NO_FRAME. Neither case consumes data.
 * Newer frames normally replace older unread ones: poll as often as you draw.
 * A frame carrying view-stream FLAG_EVENTS_RESET (bit 3; see docs/view-stream.md)
 * is a recovery baseline: pending event batches are discarded and later batches
 * are suppressed until this frame is successfully copied or taken with
 * orr_view_poll_ptr. If newer snapshots replace an unread baseline, the returned
 * frame retains FLAG_EVENTS_RESET | FLAG_DISCONTINUITY and prev == cur. A
 * too-small buffer does not acknowledge the baseline. After acknowledgement,
 * event records at or before the baseline tick are ignored to prevent late old
 * timeline events from resurfacing; a later ordinary discontinuity clears that
 * cutoff. */
ORR_API int orr_view_poll(OrrHost* host, uint8_t* buf, size_t cap, size_t* written);

/* Zero-copy variant: *data / *len point at the newest unread frame, valid
 * until the next orr_view_poll or orr_view_poll_ptr on this handle, or close.
 * ORR_NO_FRAME: nothing new (*data = NULL). */
ORR_API int orr_view_poll_ptr(OrrHost* host, const uint8_t** data, size_t* len);

/* Takes the oldest queued event batch message. Same buffer rules as
 * orr_view_poll. Events are queued, never replaced, except that a received
 * FLAG_EVENTS_RESET baseline discards queued batches and suppresses new batches
 * until the baseline is acknowledged; afterward records at or before the
 * baseline tick are ignored until an ordinary discontinuity. The FFI's own
 * 4096-batch safety cap still drops its oldest batch without synthesizing a
 * reset. The local-host ERP path does not gain a new upstream bounded-queue
 * recovery guarantee here. */
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
