// kfse-tap: an independent clone watcher on /dev/fsevents — the kernel's own
// event order, as fseventsd is handed it — printed for paths containing <needle>.
// build: clang -O2 -o kfse-tap kfse-tap.c     run (root): kfse-tap <needle> > log
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <unistd.h>

#pragma pack(push, 4)
typedef struct { int8_t *event_list; int32_t num_events; int32_t event_queue_depth; int32_t *fd; } fsevent_clone_args;
#pragma pack(pop)
#define FSEVENTS_CLONE _IOW('s', 1, fsevent_clone_args)
#define FSEVENTS_WANT_EXTENDED_INFO _IO('s', 102)
#define FSE_ARG_STRING 0x0002
#define FSE_ARG_INT64 0x0005
#define FSE_ARG_INO 0x0007
#define FSE_ARG_DONE 0xb33f

static const char *names[] = {"CREATE_FILE", "DELETE", "STAT_CHANGED", "RENAME", "CONTENT_MODIFIED", "EXCHANGE",
                              "FINDER_INFO", "CREATE_DIR", "CHOWN", "XATTR_MOD", "XATTR_RM", "DOCID_CREATED",
                              "DOCID_CHANGED", "UNMOUNT_PENDING", "CLONE", "ACTIVITY"};

int main(int argc, char **argv) {
    const char *needle = argc > 1 ? argv[1] : "/";
    int dev = open("/dev/fsevents", O_RDONLY);
    if (dev < 0) { perror("/dev/fsevents"); return 2; }
    int8_t list[32]; memset(list, 1, sizeof list); // FSE_REPORT (1) for every type
    int32_t fd = -1;
    fsevent_clone_args args = {list, 32, 65536, &fd};
    if (ioctl(dev, FSEVENTS_CLONE, &args) < 0) { perror("FSEVENTS_CLONE"); return 2; }
    close(dev);
    if (ioctl(fd, FSEVENTS_WANT_EXTENDED_INFO) < 0) perror("WANT_EXTENDED_INFO");
    static unsigned char buf[1 << 20];
    unsigned long long seq = 0;
    setvbuf(stdout, NULL, _IOFBF, 1 << 16);
    for (;;) {
        ssize_t n = read(fd, buf, sizeof buf);
        if (n == 0 || (n < 0 && errno == EINTR)) continue;
        if (n < 0) { perror("read"); return 2; }
        ssize_t off = 0;
        while (off + 8 <= n) {
            int32_t type, pid; memcpy(&type, buf + off, 4); memcpy(&pid, buf + off + 4, 4); off += 8;
            const char *path = NULL, *path2 = NULL; uint64_t ino = 0, abstime = 0;
            while (off + 2 <= n) {
                uint16_t at, al; memcpy(&at, buf + off, 2); off += 2;
                if (at == FSE_ARG_DONE) break;
                memcpy(&al, buf + off, 2); off += 2;
                if (at == FSE_ARG_STRING) { if (!path) path = (const char *)buf + off; else path2 = (const char *)buf + off; }
                else if (at == FSE_ARG_INO && al == 8 && !ino) memcpy(&ino, buf + off, 8);
                else if (at == FSE_ARG_INT64 && al == 8) memcpy(&abstime, buf + off, 8);
                off += al;
            }
            seq++;
            if ((type & 0xfff) == 999) { printf("k=%llu EVENTS_DROPPED\n", seq); continue; }
            if (path && (strstr(path, needle) || (path2 && strstr(path2, needle)))) {
                int t = type & 0xfff, fl = (type >> 12) & 0xf;
                printf("k=%llu t=%llu %s%s%s pid=%d ino=%llu %s%s%s\n", seq, abstime, t < 16 ? names[t] : "?",
                       fl & 1 ? "+COMBINED" : "", fl & 2 ? "+DROPPED" : "", pid, ino, path, path2 ? " -> " : "", path2 ? path2 : "");
            }
        }
        fflush(stdout);
    }
}
