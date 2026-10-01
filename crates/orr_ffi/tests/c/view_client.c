/*
 * A view client written in plain C against orrery.h: opens a host on a scene,
 * reads the schema, drives two players with inputs laid out as the schema
 * says, steps 120 ticks, reads every view frame and prints a checksum of all
 * entity records. The Rust test runs the same scenario through the Rust
 * bridge and compares the line printed at the end.
 *
 * Usage: view_client [scene.yaml]
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
#define RECORD_LEN 48
#define FLAG_DISCONTINUITY 2
#define FLAG_PAUSED 4

static uint64_t rd_u64(const uint8_t* p) {
    uint64_t v = 0;
    for (int i = 7; i >= 0; i--) v = (v << 8) | p[i];
    return v;
}
static uint32_t rd_u32(const uint8_t* p) {
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}
static uint16_t rd_u16(const uint8_t* p) { return (uint16_t)(p[0] | (p[1] << 8)); }
static float rd_f32(const uint8_t* p) {
    uint32_t u = rd_u32(p);
    float f;
    memcpy(&f, &u, 4);
    return f;
}
static void wr_le(uint8_t* p, uint64_t v, int bytes) {
    for (int i = 0; i < bytes; i++) p[i] = (uint8_t)(v >> (8 * i));
}

static uint64_t fnv(uint64_t h, const uint8_t* p, size_t n) {
    for (size_t i = 0; i < n; i++) {
        h ^= p[i];
        h *= 0x100000001b3ULL;
    }
    return h;
}

/* ---- a tiny schema reader: finds "name":"<field>" and the next "offset":N.
 * A real client uses a JSON library; the schema is ordinary JSON. ---- */
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

/* The scenario: a pure function of (player, tick), identical in the Rust test. */
static void scenario_input(uint8_t* buf, size_t size, long off_x, long off_y, long off_spin, long off_buttons, int p, int t) {
    int64_t ax = (int64_t)((t / 20 + p) % 3) - 1;
    int64_t ay = (int64_t)((t / 30 + 2 * p) % 3) - 1;
    int32_t spin = (int32_t)((t / 25 + p) % 3) - 1;
    uint32_t buttons = (p == 0 && t % 40 < 5) ? 1u : 0u;
    memset(buf, 0, size);
    wr_le(buf + off_x, (uint64_t)(ax * 65536), 8);   /* "fixed": int64 = value * 65536 */
    wr_le(buf + off_y, (uint64_t)(ay * 65536), 8);
    wr_le(buf + off_spin, (uint64_t)(int64_t)spin, 4);
    wr_le(buf + off_buttons, buttons, 4);
}

/* Waits until a frame with tick >= want arrives; returns its pointer and length (valid until the next poll). */
static const uint8_t* wait_frame(OrrHost* h, uint64_t want, size_t* len) {
    for (int waited = 0; waited < 10000; waited++) {
        const uint8_t* data = NULL;
        int rc = orr_view_poll_ptr(h, &data, len);
        if (rc == ORR_OK) {
            CHECK(*len >= HEADER_LEN);
            if (rd_u64(data + 8) >= want) return data;
        } else {
            CHECK(rc == ORR_NO_FRAME);
            sleep_ms(1);
        }
    }
    fprintf(stderr, "timed out waiting for the frame of tick %" PRIu64 "\n", want);
    exit(1);
}

int main(int argc, char** argv) {
    const char* scene = argc > 1 ? argv[1] : NULL;

    CHECK(orr_abi_version() == ORR_ABI_VERSION);

    /* ---- error paths before any host exists ---- */
    CHECK(orr_view_poll(NULL, NULL, 0, NULL) == ORR_ERR_NULL);
    CHECK(strlen(orr_last_error()) > 0);
    CHECK(orr_control(NULL, ORR_CTL_PAUSE, 0) == ORR_ERR_NULL);
    CHECK(orr_schema_json(NULL, NULL, 0) == 0);
    orr_host_close(NULL); /* ignored */
    CHECK(orr_host_open("/no/such/scene.yaml", NULL) == NULL);
    CHECK(strstr(orr_last_error(), "cannot read scene") != NULL);
    OrrHostConfig bad_cfg = {0, 0, 0}; /* struct_size not set */
    CHECK(orr_host_open(NULL, &bad_cfg) == NULL);

    OrrHostConfig cfg;
    memset(&cfg, 0, sizeof cfg);
    cfg.struct_size = sizeof cfg;
    OrrHost* h = orr_host_open(scene, &cfg);
    CHECK(h != NULL);

    /* ---- the schema: ask for the size, then read it ---- */
    size_t need = orr_schema_json(h, NULL, 0);
    CHECK(need > 100);
    char tiny[8];
    memset(tiny, 'x', sizeof tiny);
    CHECK(orr_schema_json(h, tiny, sizeof tiny) == need && tiny[0] == '\0'); /* too small: size reported, empty string */
    char* schema = (char*)malloc(need);
    CHECK(schema != NULL);
    CHECK(orr_schema_json(h, schema, need) == need && strlen(schema) == need - 1);
    CHECK(strstr(schema, "\"format\":\"orrery.viewstream\"") != NULL);
    long input_size = schema_number_after(schema, "\"input\":{", "\"size\":");
    long players = schema_number_after(schema, "\"player_count\":", "\"player_count\":");
    long off_x = field_offset(schema, "axis_x");
    long off_y = field_offset(schema, "axis_y");
    long off_spin = field_offset(schema, "spin");
    long off_buttons = field_offset(schema, "buttons");
    CHECK(input_size == 24 && players == 2);
    CHECK(off_x == 0 && off_y == 8 && off_spin == 16 && off_buttons == 20);
    char url[64];
    CHECK(orr_host_url(h, url, sizeof url) == 0); /* not listening */

    /* ---- more error paths on a live handle ---- */
    uint8_t input[64];
    scenario_input(input, (size_t)input_size, off_x, off_y, off_spin, off_buttons, 0, 1);
    CHECK(orr_set_input(h, 0, NULL, 24) == ORR_ERR_NULL);
    CHECK(orr_set_input(h, 0, input, 7) == ORR_ERR_ARG);            /* wrong size */
    CHECK(strstr(orr_last_error(), "24 bytes") != NULL);
    CHECK(orr_set_input(h, 9, input, 24) == ORR_ERR_ARG);           /* no such player */
    CHECK(orr_control(h, 99, 0) == ORR_ERR_ARG);                    /* unknown op */
    CHECK(orr_control(h, ORR_CTL_STEP, 0) == ORR_ERR_ARG);          /* needs n >= 1 */
    CHECK(orr_control(h, ORR_CTL_SEEK, 100000) == ORR_ERR_RPC);     /* outside the recorded range */
    size_t w = 123;
    CHECK(orr_view_poll(h, NULL, 0, NULL) == ORR_ERR_NULL);
    CHECK(orr_view_poll(h, NULL, 5, &w) == ORR_ERR_NULL);

    /* ---- the initial frame (tick 0, paused): too-small buffer reports the size and keeps the frame ---- */
    for (int i = 0; i < 5000; i++) {
        int rc = orr_view_poll(h, NULL, 0, &w);
        if (rc == ORR_ERR_BUFFER) break;
        CHECK(rc == ORR_NO_FRAME);
        sleep_ms(1);
    }
    CHECK(w >= HEADER_LEN);
    size_t first_size = w;
    uint8_t short_buf[16]; /* not `small`: windows.h defines it as a macro */
    CHECK(orr_view_poll(h, short_buf, sizeof short_buf, &w) == ORR_ERR_BUFFER && w == first_size);
    uint8_t* copy = (uint8_t*)malloc(first_size);
    CHECK(orr_view_poll(h, copy, first_size, &w) == ORR_OK && w == first_size);
    CHECK(memcmp(copy, "OVS1", 4) == 0 && rd_u16(copy + 4) == 1 && copy[6] == 1);
    CHECK(rd_u64(copy + 8) == 0 && (copy[7] & FLAG_PAUSED));
    uint32_t count0 = rd_u32(copy + 48);
    CHECK(orr_view_poll(h, copy, first_size, &w) == ORR_NO_FRAME && w == 0);
    free(copy);

    /* ---- the scenario: 120 ticks, both players, every frame read ---- */
    uint64_t hash = 0xcbf29ce484222325ULL;
    uint32_t count = 0;
    int frames = 0;
    for (int t = 1; t <= 120; t++) {
        for (int p = 0; p < 2; p++) {
            scenario_input(input, (size_t)input_size, off_x, off_y, off_spin, off_buttons, p, t);
            CHECK(orr_set_input(h, (uint8_t)p, input, (size_t)input_size) == ORR_OK);
        }
        CHECK(orr_control(h, ORR_CTL_STEP, 1) == ORR_OK);
        size_t len = 0;
        const uint8_t* f = wait_frame(h, (uint64_t)t, &len);
        CHECK(rd_u64(f + 8) == (uint64_t)t);
        count = rd_u32(f + 48);
        CHECK(len == (size_t)HEADER_LEN + (size_t)count * RECORD_LEN + rd_u32(f + 52));
        CHECK(!(f[7] & FLAG_DISCONTINUITY)); /* stepping is smooth: prev is the tick before */
        hash = fnv(hash, f + 8, 8);                                         /* the tick */
        hash = fnv(hash, f + HEADER_LEN, (size_t)count * RECORD_LEN);       /* the entity records */
        frames++;
    }
    CHECK(count >= count0); /* the paddle's shots add bodies */

    /* ---- the last frame's records, read field by field ---- */
    {
        size_t len = 0;
        CHECK(orr_control(h, ORR_CTL_STEP, 1) == ORR_OK);
        const uint8_t* f = wait_frame(h, 121, &len);
        const uint8_t* rec = f + HEADER_LEN;
        int finite = 1;
        for (uint32_t i = 0; i < count; i++, rec += RECORD_LEN) {
            float x = rd_f32(rec + 36), y = rd_f32(rec + 40);
            if (!(x == x) || !(y == y)) finite = 0;
        }
        CHECK(finite);
    }

    /* ---- ERP in process: the whole editor method set ---- */
    char answer[4096];
    size_t needed = 0;
    CHECK(orr_erp_call(h, "{\"method\":\"sim.state\"}", answer, sizeof answer, &needed) == ORR_OK);
    CHECK(strstr(answer, "\"head_tick\":121") != NULL);
    CHECK(orr_erp_call(h, "{\"method\":\"sim.state\"}", answer, 10, &needed) == ORR_ERR_BUFFER && needed > 10);
    CHECK(orr_erp_call(h, "{\"method\":\"no.such.method\"}", answer, sizeof answer, &needed) == ORR_ERR_RPC);
    CHECK(strstr(answer, "\"error\"") != NULL);
    CHECK(orr_erp_call(h, "not json", answer, sizeof answer, &needed) == ORR_ERR_ARG);
    CHECK(orr_erp_call(h, "{\"method\":\"watch.subscribe\"}", answer, sizeof answer, &needed) == ORR_ERR_ARG);

    /* ---- events: player 0 shoots during the scenario ("shot", event type 1). A local session has
     * no rollback, so every event is final: state 1 (verified), never predicted or canceled ---- */
    {
        uint8_t ev[8192];
        int shots = 0;
        for (;;) {
            int rc = orr_events_poll(h, ev, sizeof ev, &w);
            if (rc == ORR_NO_FRAME) break;
            CHECK(rc == ORR_OK && w >= 16 && memcmp(ev, "OVS1", 4) == 0 && ev[6] == 2);
            uint32_t n = rd_u32(ev + 8);
            const uint8_t* r = ev + 16;
            for (uint32_t i = 0; i < n; i++) {
                uint32_t plen = rd_u32(r + 20);
                CHECK(r[16] == 1); /* verified */
                if (rd_u16(r + 18) == 1) shots++;
                r += 24 + ((plen + 7u) & ~7u);
            }
        }
        CHECK(shots > 0);
    }

    /* ---- a seek is a jump: flagged, paused, no blend ---- */
    CHECK(orr_control(h, ORR_CTL_SEEK, 60) == ORR_OK);
    {
        size_t len = 0;
        const uint8_t* f = NULL;
        for (int i = 0; i < 10000; i++) {
            int rc = orr_view_poll_ptr(h, &f, &len);
            if (rc == ORR_OK && rd_u64(f + 8) == 60) break;
            f = NULL;
            sleep_ms(1);
        }
        CHECK(f != NULL);
        CHECK((f[7] & FLAG_DISCONTINUITY) && (f[7] & FLAG_PAUSED));
        const uint8_t* rec = f + HEADER_LEN;
        CHECK(memcmp(rec + 24, rec + 36, 12) == 0); /* prev == cur after a jump */
    }
    CHECK(orr_control(h, ORR_CTL_RESTART, 0) == ORR_OK);

    orr_host_close(h);
    free(schema);
    printf("RESULT entities=%u frames=%d fnv=0x%016" PRIx64 "\n", count, frames, hash);
    return 0;
}
