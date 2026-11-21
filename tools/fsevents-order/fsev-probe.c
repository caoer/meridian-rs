// fsev-probe: does an FSEvents stream, configured as notify 8.2.2 configures it
// (SinceNow, latency 0.0, FileEvents|NoDefer, a run loop on its own thread),
// deliver a sentinel's event before the event of a write that completed
// before the sentinel was written?
//
// One iteration models one §6.4 cookie barrier:
//   1. the writer completes a member write (open/write/close, or create),
//   2. writes a fresh sentinel  <root>/.meridian/cookies/tok-<serial>,
//   3. waits for the stream to deliver an event on that sentinel,
//   4. and reads "is the member dirty" the moment it wakes.
// The callback locks the shared state once per event, as notify's handler
// lock and the feed's state mutex do, so the waiter can wake between two
// events of one callback batch.
//
// Counters on each progress line:
//   early_at_sighting  a sentinel entry arrived while a member entry of this
//                      iteration was still undelivered (stream order broken)
//   stale_at_wake      ... and still undelivered when the waiter woke
//   early_created      the same, for a sentinel entry carrying ItemCreated
//   same_batch / later_batch / never   where the member entry then landed
//   nonmono_*          event ids that went backwards
// Each early sighting prints an ANOMALY block with the surrounding entries.
//
// build: clang -O2 -o fsev-probe fsev-probe.c -framework CoreServices
// run:   fsev-probe <root-dir> <iterations> [mode] [gap_us=2000] [verbose=0] [empty]
//   mode   rewrite | create | trio (rewrite one member, remove one, create one),
//          each also as flush-<mode>: FSEventStreamFlushSync on the run-loop
//          thread after the sentinel write, counted as missing_after_flush
//   empty  create the sentinel with no content write
// The anomaly rate rises with the number of streams: run several at once.

#include <CoreServices/CoreServices.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#define NMEMBERS 8
#define RING (1u << 16)
#define FLAG_DROP (kFSEventStreamEventFlagMustScanSubDirs | kFSEventStreamEventFlagUserDropped | kFSEventStreamEventFlagKernelDropped)

typedef struct {
    uint64_t batch, id, t_ns;
    uint32_t idx, flags;
    char kind;      // 'm' member, 's' sentinel, 'o' other
    uint64_t n;     // member index or sentinel serial
} Rec;

static char root[PATH_MAX], cookies[PATH_MAX];
static const char *base;                 // "/<root basename>", matched in event paths
static const char *mode = "rewrite";

static pthread_mutex_t mu = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t cv = PTHREAD_COND_INITIALIZER;
static Rec ring[RING];
static uint64_t nrec;                    // records written, ever
static uint64_t batches, events, drops;
static uint64_t nonmono_in_batch, nonmono_across;
static uint64_t max_id_prev_batches;
static uint64_t member_max_id[NMEMBERS]; // highest event id seen per member
static uint64_t member_last_rec[NMEMBERS];
// the barrier in flight
static uint64_t cur_serial;              // 0 = none
static uint64_t cur_floor;               // ids above this belong to this iteration
static unsigned cur_mask;                // members this iteration wrote
static int sighted;                      // sentinel cur_serial delivered
static uint64_t sight_id, sight_batch, sight_rec, sight_t;
static unsigned sight_missing;           // members not yet delivered at the sighting
static int c_sighted;                    // sentinel cur_serial delivered with ItemCreated
static unsigned c_missing;               // members not yet delivered at that entry
static uint64_t c_rec;
static uint64_t stream_started;
static FSEventStreamRef g_stream;
static CFRunLoopRef g_rl;
static uint64_t flush_done;           // serial whose run-loop-thread flush has returned
static uint64_t cb_in_flush, in_flush; // callback batches that ran inside FlushSync
static pthread_t writer_thread;
static uint64_t cb_on_writer;         // callback batches run on the writer thread (inside FlushSync)

static uint64_t now_ns(void) { return clock_gettime_nsec_np(CLOCK_MONOTONIC_RAW); }

static unsigned missing_now(void) {
    unsigned miss = 0;
    for (unsigned k = 0; k < NMEMBERS; k++)
        if ((cur_mask >> k & 1) && member_max_id[k] <= cur_floor) miss |= 1u << k;
    return miss;
}

static void callback(ConstFSEventStreamRef s, void *info, size_t n, void *paths_v,
                     const FSEventStreamEventFlags flags[], const FSEventStreamEventId ids[]) {
    (void)s; (void)info;
    char **paths = paths_v;
    uint64_t t = now_ns(), batch, batch_max = 0, prev = 0;
    pthread_mutex_lock(&mu);
    batch = ++batches;
    if (pthread_equal(pthread_self(), writer_thread)) cb_on_writer++;
    if (in_flush) cb_in_flush++;
    pthread_mutex_unlock(&mu);
    for (size_t i = 0; i < n; i++) {
        const char *p = paths[i];
        const char *rel = strstr(p, base);
        Rec r = {.batch = batch, .id = ids[i], .t_ns = t, .idx = (uint32_t)i, .flags = flags[i], .kind = 'o'};
        if (rel) {
            rel += strlen(base);
            unsigned k; unsigned long long ser;
            if (sscanf(rel, "/.meridian/cookies/tok-%llu", &ser) == 1) { r.kind = 's'; r.n = ser; }
            else if (sscanf(rel, "/m%u.md", &k) == 1 && k < NMEMBERS) { r.kind = 'm'; r.n = k; }
        }
        // one lock per event: the waiter may run between two events of a batch
        pthread_mutex_lock(&mu);
        events++;
        if (flags[i] & FLAG_DROP) drops++;
        if (i && ids[i] < prev) nonmono_in_batch++;
        if (ids[i] < max_id_prev_batches) nonmono_across++;
        ring[nrec % RING] = r;
        if (r.kind == 'm') {
            if (r.id > member_max_id[r.n]) member_max_id[r.n] = r.id;
            member_last_rec[r.n] = nrec;
            pthread_cond_broadcast(&cv);
        } else if (r.kind == 's' && r.n == cur_serial && !sighted &&
                   (flags[i] & (kFSEventStreamEventFlagItemCreated | kFSEventStreamEventFlagItemModified))) {
            sighted = 1; sight_id = r.id; sight_batch = batch; sight_rec = nrec; sight_t = t;
            sight_missing = missing_now();
            pthread_cond_broadcast(&cv);
        }
        if (r.kind == 's' && r.n == cur_serial && !c_sighted && (flags[i] & kFSEventStreamEventFlagItemCreated)) {
            c_sighted = 1; c_missing = missing_now(); c_rec = nrec;
            pthread_cond_broadcast(&cv);
        }
        nrec++;
        pthread_mutex_unlock(&mu);
        prev = ids[i];
        if (ids[i] > batch_max) batch_max = ids[i];
    }
    pthread_mutex_lock(&mu);
    if (batch_max > max_id_prev_batches) max_id_prev_batches = batch_max;
    pthread_mutex_unlock(&mu);
}

static void *loop(void *arg) {
    (void)arg;
    CFStringRef path = CFStringCreateWithCString(NULL, root, kCFStringEncodingUTF8);
    CFArrayRef arr = CFArrayCreate(NULL, (const void **)&path, 1, &kCFTypeArrayCallBacks);
    FSEventStreamRef st = FSEventStreamCreate(NULL, callback, NULL, arr, kFSEventStreamEventIdSinceNow, 0.0,
                                              kFSEventStreamCreateFlagFileEvents | kFSEventStreamCreateFlagNoDefer);
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
    FSEventStreamScheduleWithRunLoop(st, CFRunLoopGetCurrent(), kCFRunLoopDefaultMode);
#pragma clang diagnostic pop
    if (!FSEventStreamStart(st)) { fprintf(stderr, "stream start failed\n"); exit(2); }
    g_stream = st; g_rl = CFRunLoopGetCurrent();
    pthread_mutex_lock(&mu); stream_started = 1; pthread_cond_broadcast(&cv); pthread_mutex_unlock(&mu);
    CFRunLoopRun();
    return NULL;
}

static void put(const char *path, const char *body) {
    int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd < 0) { perror(path); exit(2); }
    if (write(fd, body, strlen(body)) < 0) { perror("write"); exit(2); }
    close(fd);
}

static void dump(FILE *f, uint64_t from, uint64_t to) {
    for (uint64_t i = from; i < to; i++) {
        Rec *r = &ring[i % RING];
        fprintf(f, "    rec=%llu batch=%llu idx=%u id=%llu flags=0x%05x %c%llu\n", i, r->batch, r->idx, r->id, r->flags,
                r->kind, r->n);
    }
}

int main(int argc, char **argv) {
    if (argc < 3) { fprintf(stderr, "usage: %s <root> <iterations> [rewrite|create|trio] [gap_us] [verbose]\n", argv[0]); return 2; }
    if (!realpath(argv[1], root)) { perror(argv[1]); return 2; }
    base = strrchr(root, '/');
    uint64_t iters = strtoull(argv[2], NULL, 10);
    if (argc > 3) mode = argv[3];
    useconds_t gap = argc > 4 ? (useconds_t)atoi(argv[4]) : 2000;
    int verbose = argc > 5 ? atoi(argv[5]) : 0;
    int empty = argc > 6 && !strcmp(argv[6], "empty");   // create-only sentinel: no content write
    int flush = !strncmp(mode, "flush-", 6);
    if (flush) mode += 6;
    int create = !strcmp(mode, "create"), trio = !strcmp(mode, "trio");
    writer_thread = pthread_self();

    char p[PATH_MAX], body[64];
    snprintf(cookies, sizeof cookies, "%s/.meridian/cookies", root);
    snprintf(p, sizeof p, "%s/.meridian", root); mkdir(p, 0755); mkdir(cookies, 0755);
    for (unsigned k = 0; k < NMEMBERS; k++) {
        snprintf(p, sizeof p, "%s/m%u.md", root, k);
        if (create) unlink(p); else put(p, "# seed\n");
    }
    pthread_t th; pthread_create(&th, NULL, loop, NULL);
    pthread_mutex_lock(&mu); while (!stream_started) pthread_cond_wait(&cv, &mu); pthread_mutex_unlock(&mu);
    usleep(300000);

    uint64_t lat_sum = 0, lat_max = 0, fl_sum = 0, fl_max = 0, unsighted_after_flush = 0, missing_after_flush = 0, slow_flush = 0, no_created = 0, early_created = 0;
    uint64_t early = 0, stale = 0, in_batch = 0, later_batch = 0, never = 0, timeouts = 0, floor = 0;
    uint64_t t0 = now_ns(); srandom(42);
    for (uint64_t i = 1; i <= iters; i++) {
        unsigned k = (unsigned)(i % NMEMBERS), mask = 1u << k;
        if (trio) mask = 1u << 0 | 1u << 5 | 1u << 7;
        pthread_mutex_lock(&mu);
        cur_serial = i; cur_floor = floor; cur_mask = mask; sighted = 0; c_sighted = 0;
        pthread_mutex_unlock(&mu);
        // 1. the member write, complete before the sentinel exists
        snprintf(body, sizeof body, "# write %llu\n", i);
        if (trio) {
            snprintf(p, sizeof p, "%s/m0.md", root); put(p, body);
            snprintf(p, sizeof p, "%s/m5.md", root); if (i & 1) unlink(p); else put(p, body);
            snprintf(p, sizeof p, "%s/m7.md", root); if (i & 1) put(p, body); else unlink(p);
        } else {
            snprintf(p, sizeof p, "%s/m%u.md", root, k);
            if (create) unlink(p);
            put(p, body);
        }
        // 2. the sentinel
        char sp[PATH_MAX]; snprintf(sp, sizeof sp, "%s/tok-%llu", cookies, i);
        snprintf(body, sizeof body, "%llu", i);
        if (empty) { int fd = open(sp, O_WRONLY | O_CREAT | O_EXCL, 0644); if (fd < 0) { perror(sp); exit(2); } close(fd); }
        else put(sp, body);
        uint64_t t_sent = now_ns();
        if (flush) {
            // the macOS barrier under test: flush after the sentinel write, then require the sighting
            // on the stream's own run-loop thread, so its callbacks run inline
            uint64_t ser = i;
            CFRunLoopPerformBlock(g_rl, kCFRunLoopDefaultMode, ^{
                pthread_mutex_lock(&mu); in_flush = 1; pthread_mutex_unlock(&mu);
                FSEventStreamFlushSync(g_stream);
                pthread_mutex_lock(&mu); in_flush = 0; flush_done = ser; pthread_cond_broadcast(&cv); pthread_mutex_unlock(&mu);
            });
            CFRunLoopWakeUp(g_rl);
            struct timespec fdl; clock_gettime(CLOCK_REALTIME, &fdl); fdl.tv_sec += 5;
            pthread_mutex_lock(&mu);
            int frc = 0;
            while (flush_done != ser && frc != ETIMEDOUT) frc = pthread_cond_timedwait(&cv, &mu, &fdl);
            if (flush_done != ser) { printf("FLUSH-HANG iter=%llu\n", i); fflush(stdout); }
            uint64_t fl = now_ns() - t_sent; fl_sum += fl; if (fl > fl_max) fl_max = fl;
            if (fl > 100000000ull) { slow_flush++; printf("FLUSH-SLOW iter=%llu ms=%.1f sighted=%d missing=0x%x\n", i, fl / 1e6, sighted, missing_now()); fflush(stdout); }
            if (!sighted) unsighted_after_flush++;
            if (missing_now()) {
                missing_after_flush++;
                printf("FLUSH-MISS iter=%llu floor=%llu sighted=%d missing=0x%x\n", i, cur_floor, sighted, missing_now());
                dump(stdout, nrec > 14 ? nrec - 14 : 0, nrec); fflush(stdout);
            }
            pthread_mutex_unlock(&mu);
        }
        // 3. wait for the sighting, 4. read the dirty state on wake
        struct timespec dl; clock_gettime(CLOCK_REALTIME, &dl); dl.tv_sec += 2;
        pthread_mutex_lock(&mu);
        int rc = 0;
        while (!(sighted && c_sighted) && rc != ETIMEDOUT) rc = pthread_cond_timedwait(&cv, &mu, &dl);
        if (sighted && !c_sighted) no_created++;
        if (c_sighted && c_missing) {
            early_created++;
            printf("CREATED-EARLY iter=%llu missing=0x%x\n", i, c_missing);
            dump(stdout, c_rec > 12 ? c_rec - 12 : 0, nrec); fflush(stdout);
        }
        if (!sighted) {
            timeouts++; pthread_mutex_unlock(&mu); unlink(sp);
            if (verbose) printf("iter %llu TIMEOUT (no sighting in 2s)\n", i);
            usleep(50000);
            pthread_mutex_lock(&mu); floor = max_id_prev_batches; pthread_mutex_unlock(&mu);
            continue;
        }
        unsigned miss_at_sight = sight_missing, miss_at_wake = missing_now();
        uint64_t lat = sight_t > t_sent ? sight_t - t_sent : 0; lat_sum += lat; if (lat > lat_max) lat_max = lat;
        uint64_t sid = sight_id, sbatch = sight_batch, srec = sight_rec;
        if (verbose) printf("iter %llu sighted id=%llu batch=%llu early=%x stale=%x\n", i, sid, sbatch, miss_at_sight, miss_at_wake);
        if (miss_at_sight) {
            early++; if (miss_at_wake) stale++;
            // where does the missing member event land?
            clock_gettime(CLOCK_REALTIME, &dl); dl.tv_sec += 2; rc = 0;
            while (missing_now() && rc != ETIMEDOUT) rc = pthread_cond_timedwait(&cv, &mu, &dl);
            unsigned still = missing_now();
            const char *where = "never(2s)";
            if (!still) {
                uint64_t worst = 0;
                for (unsigned m = 0; m < NMEMBERS; m++)
                    if ((miss_at_sight >> m & 1) && ring[member_last_rec[m] % RING].batch > worst) worst = ring[member_last_rec[m] % RING].batch;
                if (worst == sbatch) { where = "same-batch-after-sentinel"; in_batch++; }
                else { where = "later-batch"; later_batch++; }
            } else never++;
            printf("ANOMALY iter=%llu serial=%llu floor=%llu sentinel_id=%llu sentinel_batch=%llu missing_at_sighting=0x%x "
                   "missing_at_wake=0x%x member_event=%s\n", i, i, cur_floor, sid, sbatch, miss_at_sight, miss_at_wake, where);
            dump(stdout, srec > 12 ? srec - 12 : 0, nrec);
            fflush(stdout);
        }
        floor = sid;
        pthread_mutex_unlock(&mu);
        unlink(sp);
        if (gap) usleep((useconds_t)(random() % (gap + 1)));
        if (i % 10000 == 0 || i == iters) {
            pthread_mutex_lock(&mu);
            printf("progress iters=%llu secs=%.1f batches=%llu events=%llu drops=%llu timeouts=%llu early_at_sighting=%llu "
                   "stale_at_wake=%llu same_batch=%llu later_batch=%llu never=%llu nonmono_in_batch=%llu nonmono_across=%llu "
                   "lat_avg_ms=%.2f lat_max_ms=%.1f flush=%d flush_avg_ms=%.2f flush_max_ms=%.1f unsighted_after_flush=%llu "
                   "missing_after_flush=%llu slow_flush=%llu cb_in_flush=%llu empty_sentinel=%d early_created=%llu no_created=%llu\n",
                   i, (now_ns() - t0) / 1e9, batches, events, drops, timeouts, early, stale, in_batch, later_batch, never,
                   nonmono_in_batch, nonmono_across, lat_sum / 1e6 / (double)(i - timeouts ? i - timeouts : 1), lat_max / 1e6,
                   flush, fl_sum / 1e6 / (double)i, fl_max / 1e6, unsighted_after_flush, missing_after_flush, slow_flush, cb_in_flush, empty, early_created, no_created);
            pthread_mutex_unlock(&mu);
            fflush(stdout);
        }
    }
    return 0;
}
