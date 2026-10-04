/* Public-header consumer for the explicit Yard3D C ABI entry. */
#ifndef _WIN32
#define _POSIX_C_SOURCE 200809L
#endif
#include "orrery.h"

#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef _WIN32
#include <windows.h>
static void sleep_ms(unsigned ms) { Sleep(ms); }
#else
#include <time.h>
static void sleep_ms(unsigned ms) {
    struct timespec ts;
    ts.tv_sec = (time_t)(ms / 1000u);
    ts.tv_nsec = (long)(ms % 1000u) * 1000000L;
    nanosleep(&ts, NULL);
}
#endif

#define CHECK(cond) do { \
    if (!(cond)) { \
        fprintf(stderr, "CHECK FAILED %s:%d: %s\n  last error: %s\n", __FILE__, __LINE__, #cond, orr_last_error()); \
        exit(1); \
    } \
} while (0)

#define HEADER_LEN 56u
#define RECORD3D_LEN 88u
#define FLAG_DISCONTINUITY 2u
#define FLAG_PAUSED 4u

static uint16_t rd_u16(const uint8_t* p) {
    return (uint16_t)((uint16_t)p[0] | ((uint16_t)p[1] << 8));
}
static uint32_t rd_u32(const uint8_t* p) {
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}
static uint64_t rd_u64(const uint8_t* p) {
    uint64_t v = 0;
    int i;
    for (i = 7; i >= 0; --i) v = (v << 8) | p[i];
    return v;
}
static void wr_le(uint8_t* p, uint32_t v) {
    p[0] = (uint8_t)v;
    p[1] = (uint8_t)(v >> 8);
    p[2] = (uint8_t)(v >> 16);
    p[3] = (uint8_t)(v >> 24);
}
static uint64_t fnv(uint64_t h, const uint8_t* p, size_t n) {
    size_t i;
    for (i = 0; i < n; ++i) {
        h ^= p[i];
        h *= UINT64_C(0x100000001b3);
    }
    return h;
}

static long number_after(const char* text, const char* key) {
    const char* at = strstr(text, key);
    if (at == NULL) return -1;
    at += strlen(key);
    return strtol(at, NULL, 10);
}

static long field_offset(const char* schema, const char* name) {
    char key[96];
    const char* at;
    snprintf(key, sizeof key, "\"name\":\"%s\"", name);
    at = strstr(schema, key);
    if (at == NULL) return -1;
    return number_after(at, "\"offset\":");
}

static long schema_input_size(const char* schema) {
    const char* input = strstr(schema, "\"input\":{");
    const char* at;
    unsigned depth = 0;
    CHECK(input != NULL);
    /* Read the input object's own size, not a nested field's size. JSON object
     * member order is unspecified and the fields array may precede size. */
    at = strchr(input, '{');
    for (; *at != '\0'; ++at) {
        if (*at == '"') {
            const char* start = ++at;
            while (*at != '\0' && *at != '"') {
                if (*at == '\\' && at[1] != '\0') ++at;
                ++at;
            }
            CHECK(*at == '"');
            if (depth == 1u && at - start == 4 && memcmp(start, "size", 4) == 0) {
                const char* value = at + 1;
                while (*value == ' ' || *value == '\t' || *value == '\n') ++value;
                CHECK(*value == ':');
                return strtol(value + 1, NULL, 10);
            }
        } else if (*at == '{' || *at == '[') {
            ++depth;
        } else if (*at == '}' || *at == ']') {
            CHECK(depth > 0u);
            if (--depth == 0u) break;
        }
    }
    return -1;
}

static void make_input(uint8_t out[32], unsigned player, unsigned tick) {
    uint32_t buttons = 0;
    memset(out, 0, 32);
    if (player == 0 && tick % 4u == 0u) buttons = 4u; /* Yard3D SPAWN_BALL */
    if (player == 1 && tick % 4u == 1u) buttons = 1u; /* Yard3D SHOOT */
    wr_le(out + 0, buttons);
    /* Camera ray from (0, 10, 20) toward the floor center. */
    wr_le(out + 8, 0u);
    wr_le(out + 12, 1000u);
    wr_le(out + 16, 2000u);
    wr_le(out + 20, 0u);
    wr_le(out + 24, (uint32_t)-447);
    wr_le(out + 28, (uint32_t)-894);
}

static const uint8_t* wait_frame(OrrHost* host, uint64_t want_tick, size_t* len) {
    unsigned waited;
    for (waited = 0; waited < 20000u; ++waited) {
        const uint8_t* data = NULL;
        int rc = orr_view_poll_ptr(host, &data, len);
        if (rc == ORR_OK) {
            CHECK(*len >= HEADER_LEN);
            if (rd_u64(data + 8) >= want_tick) return data;
        } else {
            CHECK(rc == ORR_NO_FRAME);
            sleep_ms(1);
        }
    }
    fprintf(stderr, "timed out waiting for Yard3D tick %" PRIu64 "\n", want_tick);
    exit(1);
}

static const uint8_t* wait_frame_exact(OrrHost* host, uint64_t want_tick, size_t* len) {
    unsigned waited;
    for (waited = 0; waited < 20000u; ++waited) {
        const uint8_t* data = NULL;
        int rc = orr_view_poll_ptr(host, &data, len);
        if (rc == ORR_OK) {
            CHECK(*len >= HEADER_LEN);
            if (rd_u64(data + 8) == want_tick) return data;
        } else {
            CHECK(rc == ORR_NO_FRAME);
            sleep_ms(1);
        }
    }
    fprintf(stderr, "timed out waiting for exact Yard3D tick %" PRIu64 "\n", want_tick);
    exit(1);
}

static void check_frame3(const uint8_t* frame, size_t len, uint64_t tick) {
    uint32_t count;
    uint32_t props;
    uint32_t i;
    CHECK(memcmp(frame, "OVS1", 4) == 0);
    CHECK(rd_u16(frame + 4) == 2u && frame[6] == 3u);
    CHECK(rd_u64(frame + 8) == tick);
    count = rd_u32(frame + 48);
    props = rd_u32(frame + 52);
    CHECK(count > 0u);
    CHECK(len == HEADER_LEN + (size_t)count * RECORD3D_LEN + props);
    for (i = 0; i < count; ++i) {
        const uint8_t* record = frame + HEADER_LEN + (size_t)i * RECORD3D_LEN;
        CHECK(record[31] == 0u); /* reserved byte in every v2 record */
    }
}

static void erp_call(OrrHost* host, const char* request, char out[4096]) {
    size_t needed = 0;
    int rc = orr_erp_call(host, request, out, 4096, &needed);
    CHECK(rc == ORR_OK && needed > 1u && needed <= 4096u);
}

static uint64_t state_tick(OrrHost* host, char checksum[32]) {
    char answer[4096];
    const char* at;
    erp_call(host, "{\"method\":\"sim.state\"}", answer);
    at = strstr(answer, "\"checksum\":\"");
    CHECK(at != NULL);
    at += strlen("\"checksum\":\"");
    CHECK(strlen(at) >= 18u);
    memcpy(checksum, at, 18u);
    checksum[18] = '\0';
    return (uint64_t)number_after(answer, "\"head_tick\":");
}

int main(void) {
    OrrHostConfig cfg;
    OrrHostConfig bad;
    OrrHost* host;
    size_t schema_size;
    char* schema;
    uint8_t raw[32];
    size_t written = 0;
    const uint8_t* first;
    size_t first_len = 0;
    uint8_t first_copy[HEADER_LEN];
    uint64_t hash = UINT64_C(0xcbf29ce484222325);
    uint32_t final_count = 0;
    unsigned tick;
    char checksum_before[32];
    char checksum_after[32];
    char request[96];
    char answer[4096];

    CHECK(orr_abi_version() == ORR_ABI_VERSION);
    memset(&bad, 0, sizeof bad);
    bad.struct_size = sizeof bad;
    bad.listen_port = 70000u;
    CHECK(orr_yard3d_host_open_v1(1u, &bad) == NULL);
    CHECK(strstr(orr_last_error(), "version 2") != NULL);
    memset(&bad, 0, sizeof bad);
    CHECK(orr_yard3d_host_open_v1(2u, &bad) == NULL);
    CHECK(strstr(orr_last_error(), "struct_size") != NULL);
    bad.struct_size = sizeof bad;
    bad.listen_port = 70000u;
    CHECK(orr_yard3d_host_open_v1(2u, &bad) == NULL);
    CHECK(strstr(orr_last_error(), "listen_port") != NULL);

    memset(&cfg, 0, sizeof cfg);
    cfg.struct_size = sizeof cfg;
    host = orr_yard3d_host_open_v1(2u, &cfg);
    CHECK(host != NULL);

    schema_size = orr_schema_json(host, NULL, 0);
    CHECK(schema_size > 100u);
    {
        char tiny[16];
        memset(tiny, 'x', sizeof tiny);
        CHECK(orr_schema_json(host, tiny, sizeof tiny) == schema_size);
        CHECK(tiny[0] == '\0');
    }
    schema = (char*)malloc(schema_size);
    CHECK(schema != NULL);
    CHECK(orr_schema_json(host, schema, schema_size) == schema_size);
    CHECK(strstr(schema, "\"game\":\"Yard3D\"") != NULL);
    CHECK(strstr(schema, "\"version\":2") != NULL);
    CHECK(strstr(schema, "\"frame3d\":{") != NULL);
    CHECK(strstr(schema, "\"record_len\":88") != NULL);
    CHECK(strstr(schema, "\"message_type\":3") != NULL);
    CHECK(schema_input_size(schema) == 32);
    CHECK(field_offset(schema, "buttons") == 0);
    CHECK(field_offset(schema, "_pad") == 4);
    CHECK(field_offset(schema, "origin") == 8);
    CHECK(field_offset(schema, "dir") == 20);

    /* The initial frame is nonempty and a NULL/short-buffer probe retains it. */
    for (tick = 0; tick < 10000u; ++tick) {
        int rc = orr_view_poll(host, NULL, 0, &written);
        if (rc == ORR_ERR_BUFFER) break;
        CHECK(rc == ORR_NO_FRAME);
        sleep_ms(1);
    }
    CHECK(written >= HEADER_LEN + RECORD3D_LEN);
    {
        uint8_t short_buf[8];
        CHECK(orr_view_poll(host, short_buf, sizeof short_buf, &written) == ORR_ERR_BUFFER);
        CHECK(written >= HEADER_LEN + RECORD3D_LEN);
    }
    first = wait_frame(host, 0, &first_len);
    check_frame3(first, first_len, 0);
    memcpy(first_copy, first, sizeof first_copy);
    CHECK((first[7] & FLAG_PAUSED) != 0u);
    /* Calls other than view polling leave the zero-copy bytes valid. */
    CHECK(orr_schema_json(host, schema, schema_size) == schema_size);
    CHECK(memcmp(first, first_copy, sizeof first_copy) == 0);

    make_input(raw, 0, 1);
    CHECK(rd_u32(raw + 4) == 0u);
    CHECK(orr_set_input(host, 0, raw, 31) == ORR_ERR_ARG);
    CHECK(orr_set_input(host, 2, raw, sizeof raw) == ORR_ERR_ARG);

    for (tick = 1; tick <= 16u; ++tick) {
        unsigned player;
        const uint8_t* frame;
        size_t len = 0;
        for (player = 0; player < 2u; ++player) {
            make_input(raw, player, tick);
            CHECK(orr_set_input(host, (uint8_t)player, raw, sizeof raw) == ORR_OK);
        }
        CHECK(orr_control(host, ORR_CTL_STEP, 1) == ORR_OK);
        frame = wait_frame(host, tick, &len);
        check_frame3(frame, len, tick);
        CHECK(!(frame[7] & FLAG_DISCONTINUITY));
        final_count = rd_u32(frame + 48);
        hash = fnv(hash, frame + 8, 8);
        hash = fnv(hash, frame + HEADER_LEN, len - HEADER_LEN);
    }

    /* The local host's ERP controller records deterministic steps. Seeking
     * back and replaying to tick 16 must reproduce the exact simulation checksum. */
    CHECK(state_tick(host, checksum_before) == 16u);
    CHECK(orr_control(host, ORR_CTL_PLAY, 0) == ORR_OK);
    {
        unsigned tries;
        uint64_t now = 16;
        for (tries = 0; tries < 10000u && now <= 16u; ++tries) {
            sleep_ms(1);
            now = state_tick(host, checksum_after);
        }
        CHECK(now > 16u);
    }
    CHECK(orr_control(host, ORR_CTL_PAUSE, 0) == ORR_OK);
    {
        uint64_t paused_tick = state_tick(host, checksum_after);
        CHECK(paused_tick > 16u);
        CHECK(orr_control(host, ORR_CTL_SEEK, 16) == ORR_OK);
    }
    CHECK(state_tick(host, checksum_after) == 16u);
    CHECK(strcmp(checksum_before, checksum_after) == 0);
    {
        size_t len = 0;
        const uint8_t* frame = wait_frame_exact(host, 16, &len);
        uint32_t count;
        uint32_t i;
        check_frame3(frame, len, 16);
        CHECK((frame[7] & (FLAG_DISCONTINUITY | FLAG_PAUSED)) == (FLAG_DISCONTINUITY | FLAG_PAUSED));
        count = rd_u32(frame + 48);
        for (i = 0; i < count; ++i) {
            const uint8_t* record = frame + HEADER_LEN + (size_t)i * RECORD3D_LEN;
            CHECK(memcmp(record + 32, record + 60, 28) == 0);
        }
    }

    /* Also exercise the ERP seek path directly, then restart to a fresh paused
     * tick-zero baseline. The raw direct-control replay check above is enough
     * to compare the checksum without confusing seek semantics with rollback. */
    snprintf(request, sizeof request, "{\"method\":\"sim.seek\",\"params\":{\"tick\":8}}");
    erp_call(host, request, answer);
    CHECK(state_tick(host, checksum_after) == 8u);
    CHECK(orr_control(host, ORR_CTL_RESTART, 0) == ORR_OK);
    {
        size_t len = 0;
        const uint8_t* frame = wait_frame_exact(host, 0, &len);
        check_frame3(frame, len, 0);
        CHECK((frame[7] & (FLAG_DISCONTINUITY | FLAG_PAUSED)) == (FLAG_DISCONTINUITY | FLAG_PAUSED));
    }

    orr_host_close(host);
    free(schema);
    printf("RESULT records=%u fnv=0x%016" PRIx64 " checksum=%s\n", final_count, hash, checksum_before);
    return 0;
}
