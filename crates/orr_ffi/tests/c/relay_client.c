/*
 * A multiplayer view client written in plain C against orrery.h: joins a relay game on an
 * orr_server (QUIC) through orr_client_open, plays 300 ticks with a scripted player, reads every
 * view frame (rolled_back flag and range) and every event batch (predicted / verified /
 * canceled), prints the session status, and ends with a RESULT line whose last two parts are the
 * confirmed state at a fixed verified tick (equal for every player of the room).
 *
 * Usage: relay_client HOST:PORT FINGERPRINT_HEX [LATENCY_MS [LOSS_PERMILLE]]
 * Exit code 0 = every check passed. The last line printed starts with "RESULT".
 */
#ifndef _WIN32
#define _POSIX_C_SOURCE 200809L /* nanosleep under -std=c99 */
#endif
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <inttypes.h>
#include "orrery.h"

#ifdef _WIN32
#include <windows.h>
static void sleep_ms(unsigned ms) { Sleep(ms); }
#else
#include <time.h>
static void sleep_ms(unsigned ms) {
    struct timespec ts;
    ts.tv_sec = ms / 1000;
    ts.tv_nsec = (long)(ms % 1000) * 1000000L;
    nanosleep(&ts, NULL);
}
#endif

#define CHECK(cond)                                                                   \
    do {                                                                              \
        if (!(cond)) {                                                                \
            fprintf(stderr, "CHECK FAILED %s:%d: %s\n  last error: %s\n", __FILE__,   \
                    __LINE__, #cond, orr_last_error());                               \
            exit(1);                                                                  \
        }                                                                             \
    } while (0)

/* ---- the view stream format (docs/view-stream.md), little-endian ---- */
#define HEADER_LEN 56
#define FLAG_ROLLED_BACK 1
#define PLAY_TICKS 300
#define CHECK_TICK 120 /* a multiple of the room's checksum interval (30) */

static uint64_t rd_u64(const uint8_t* p) {
    uint64_t v = 0;
    for (int i = 7; i >= 0; i--) v = (v << 8) | p[i];
    return v;
}
static uint32_t rd_u32(const uint8_t* p) {
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}
static uint16_t rd_u16(const uint8_t* p) { return (uint16_t)(p[0] | (p[1] << 8)); }
static void wr_le(uint8_t* p, uint64_t v, int bytes) {
    for (int i = 0; i < bytes; i++) p[i] = (uint8_t)(v >> (8 * i));
}

/* A tiny schema reader (a real client uses a JSON library). */
static long schema_number_after(const char* schema, const char* from_key, const char* number_key) {
    const char* at = strstr(schema, from_key);
    if (!at) return -1;
    at = strstr(at, number_key);
    if (!at) return -1;
    return strtol(at + strlen(number_key), NULL, 10);
}
static long field_offset(const char* schema, const char* name) {
    char key[96];
    snprintf(key, sizeof key, "\"name\":\"%s\"", name);
    return schema_number_after(schema, key, "\"offset\":");
}

/* The scripted player: a pure function of (slot, tick). Changes direction every 9 ticks, shoots in bursts. */
static void scripted_input(uint8_t* buf, size_t size, long off_x, long off_y, long off_spin, long off_buttons, int slot, uint64_t tick) {
    int k = (int)((tick / 9 + (uint64_t)slot * 5) % 7);
    int shoot = ((tick / 12 + (uint64_t)slot) % 3) == 0;
    memset(buf, 0, size);
    wr_le(buf + off_x, (uint64_t)((int64_t)(k % 3 - 1) * 65536), 8); /* "fixed": int64 = value * 65536 */
    wr_le(buf + off_y, (uint64_t)((int64_t)((k / 2) % 3 - 1) * 65536), 8);
    wr_le(buf + off_spin, (uint64_t)(int64_t)(k % 2), 4);
    wr_le(buf + off_buttons, shoot ? 1u : 0u, 4);
}

static void print_status(const char* label, const OrrSessionStatus* s) {
    printf("%s: state=%u slot=%u/%u rtt=%u ms delay=%u ticks head=%" PRIu64 " verified=%" PRIu64 " rollbacks=%" PRIu64
           " resim=%" PRIu64 " last_rollback=%" PRIu64 "..%" PRIu64 " desyncs=%" PRIu64 " stalls=%" PRIu64 "\n",
           label, s->state, s->slot, s->player_count, s->rtt_ms, s->input_delay, s->head_tick, s->verified_tick, s->rollbacks,
           s->resim_ticks, s->last_rollback_from, s->last_rollback_to, s->desyncs, s->stall_episodes);
}

int main(int argc, char** argv) {
    if (argc < 3) {
        fprintf(stderr, "usage: relay_client HOST:PORT FINGERPRINT_HEX [LATENCY_MS [LOSS_PERMILLE]]\n");
        return 2;
    }
    CHECK(orr_abi_version() == ORR_ABI_VERSION);

    /* ---- error paths of the client calls ---- */
    CHECK(orr_client_open(NULL) == NULL);
    OrrClientConfig bad;
    memset(&bad, 0, sizeof bad);
    CHECK(orr_client_open(&bad) == NULL); /* struct_size not set */
    bad.struct_size = sizeof bad;
    CHECK(orr_client_open(&bad) == NULL); /* no server */
    CHECK(strstr(orr_last_error(), "server") != NULL);
    CHECK(orr_session_status(NULL, NULL) == ORR_ERR_NULL);

    /* ---- join: with ORR_CLIENT_WAIT the call returns when the room has started ---- */
    OrrClientConfig cfg;
    memset(&cfg, 0, sizeof cfg);
    cfg.struct_size = sizeof cfg;
    cfg.flags = ORR_CLIENT_WAIT;
    cfg.transport = ORR_TRANSPORT_QUIC;
    cfg.slot = -1;
    cfg.server = argv[1];
    cfg.fingerprint = argv[2];
    cfg.sim_latency_ms = argc > 3 ? (uint32_t)atoi(argv[3]) : 0;
    cfg.sim_jitter_ms = cfg.sim_latency_ms ? 5 : 0;
    cfg.sim_loss_permille = argc > 4 ? (uint32_t)atoi(argv[4]) : 0;
    cfg.connect_timeout_ms = 60000;
    OrrHost* h = orr_client_open(&cfg);
    CHECK(h != NULL);

    OrrSessionStatus st;
    memset(&st, 0, sizeof st);
    st.struct_size = sizeof st;
    CHECK(orr_session_status(h, &st) == ORR_OK);
    CHECK(st.struct_size == sizeof st && st.mode == ORR_MODE_CLIENT && st.state == ORR_STATE_PLAYING);
    CHECK(st.player_count == 2 && st.slot < 2);
    print_status("joined", &st);
    const int slot = (int)st.slot;

    /* Timeline controls belong to a local host. */
    CHECK(orr_control(h, ORR_CTL_PLAY, 0) == ORR_ERR_ARG);
    CHECK(strstr(orr_last_error(), "client session") != NULL);

    /* ---- the schema is the same as a local host's ---- */
    size_t need = orr_schema_json(h, NULL, 0);
    CHECK(need > 100);
    char* schema = (char*)malloc(need);
    CHECK(schema != NULL && orr_schema_json(h, schema, need) == need);
    CHECK(strstr(schema, "\"format\":\"orrery.viewstream\"") != NULL);
    long input_size = schema_number_after(schema, "\"input\":{", "\"size\":");
    long off_x = field_offset(schema, "axis_x");
    long off_y = field_offset(schema, "axis_y");
    long off_spin = field_offset(schema, "spin");
    long off_buttons = field_offset(schema, "buttons");
    CHECK(input_size == 24 && off_x == 0 && off_y == 8 && off_spin == 16 && off_buttons == 20);
    uint8_t input[64];
    scripted_input(input, (size_t)input_size, off_x, off_y, off_spin, off_buttons, slot, 0);
    CHECK(orr_set_input(h, (uint8_t)(1 - slot), input, (size_t)input_size) == ORR_ERR_ARG); /* only the own slot */
    CHECK(strstr(orr_last_error(), "slot") != NULL);

    /* ---- play: every frame read, every event batch read ---- */
    uint64_t last_input_tick = (uint64_t)-1, tick = 0, verified = 0;
    unsigned frames = 0, rolled = 0, max_depth = 0;
    unsigned ev_predicted = 0, ev_verified = 0, ev_canceled = 0;
    static uint8_t evbuf[1 << 16];
    for (int spins = 0; spins < 60000 && !(tick >= PLAY_TICKS && verified >= CHECK_TICK + 30); spins++) {
        const uint8_t* f = NULL;
        size_t len = 0;
        int rc = orr_view_poll_ptr(h, &f, &len);
        CHECK(rc == ORR_OK || rc == ORR_NO_FRAME);
        if (rc == ORR_OK) {
            CHECK(len >= HEADER_LEN);
            frames++;
            tick = rd_u64(f + 8);
            verified = rd_u64(f + 16);
            CHECK(verified <= tick);
            if (f[7] & FLAG_ROLLED_BACK) {
                uint64_t from = rd_u64(f + 32), to = rd_u64(f + 40);
                CHECK(from >= 1 && from <= to && to <= tick && to - from < 64);
                rolled++;
                if ((unsigned)(to - from + 1) > max_depth) max_depth = (unsigned)(to - from + 1);
            } else {
                CHECK(rd_u64(f + 32) == 0 && rd_u64(f + 40) == 0);
            }
            if (tick != last_input_tick) {
                last_input_tick = tick;
                scripted_input(input, (size_t)input_size, off_x, off_y, off_spin, off_buttons, slot, tick);
                CHECK(orr_set_input(h, (uint8_t)slot, input, (size_t)input_size) == ORR_OK);
            }
        } else {
            sleep_ms(2);
        }
        size_t w = 0;
        while ((rc = orr_events_poll(h, evbuf, sizeof evbuf, &w)) == ORR_OK) {
            CHECK(w >= 16 && memcmp(evbuf, "OVS1", 4) == 0 && evbuf[6] == 2);
            uint32_t n = rd_u32(evbuf + 8);
            const uint8_t* r = evbuf + 16;
            for (uint32_t i = 0; i < n; i++) {
                uint32_t plen = rd_u32(r + 20);
                CHECK(rd_u16(r + 18) <= 1);
                if (r[16] == 0) ev_predicted++;
                else if (r[16] == 1) ev_verified++;
                else if (r[16] == 2) ev_canceled++;
                else CHECK(0);
                r += 24 + ((plen + 7u) & ~7u);
            }
        }
        CHECK(rc == ORR_NO_FRAME);
    }
    CHECK(tick >= PLAY_TICKS && verified >= CHECK_TICK + 30);

    /* ---- the status, and the confirmed state at a fixed tick ---- */
    st.struct_size = sizeof st;
    CHECK(orr_session_status(h, &st) == ORR_OK);
    print_status("end", &st);
    CHECK(st.state == ORR_STATE_PLAYING && st.desyncs == 0 && (st.flags & ORR_STATUS_DESYNC) == 0);
    CHECK(st.rollbacks >= rolled && st.resim_ticks >= st.rollbacks);
    CHECK(st.input_delay >= 1 && st.verified_tick >= CHECK_TICK);
    uint64_t found = 0, sum = 0;
    for (int i = 0; i < 5000; i++) {
        int rc = orr_confirmed_checksum(h, CHECK_TICK, &found, &sum);
        if (rc == ORR_OK) break;
        CHECK(rc == ORR_NO_FRAME);
        sleep_ms(2);
    }
    CHECK(found == CHECK_TICK && sum != 0);
    /* A tick that is no checkpoint has no checksum. */
    CHECK(orr_confirmed_checksum(h, CHECK_TICK + 1, &found, &sum) == ORR_NO_FRAME);

    orr_host_close(h);
    free(schema);
    printf("RESULT relay slot=%d frames=%u rolled_back=%u max_depth=%u predicted=%u verified=%u canceled=%u rollbacks=%" PRIu64
           " checkpoint=%d checksum=0x%016" PRIx64 "\n",
           slot, frames, rolled, max_depth, ev_predicted, ev_verified, ev_canceled, st.rollbacks, CHECK_TICK, sum);
    return 0;
}
