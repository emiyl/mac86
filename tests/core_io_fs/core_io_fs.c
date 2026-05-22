#define _DARWIN_C_SOURCE

#include "../common.h"

#include <copyfile.h>
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <fts.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/uio.h>
#include <time.h>
#include <unistd.h>
#include <wchar.h>

extern int *__error(void);
#if defined(__i386__)
extern int maskrune(int, long);
#endif

static void join_path(char *out, size_t out_size, const char *base, const char *name) {
    snprintf(out, out_size, "%s/%s", base, name);
}

static void write_text_file(const char *path, const char *text) {
    int fd = open(path, O_RDWR | O_CREAT | O_TRUNC, 0644);
    CHECK(fd >= 0);
    if (fd >= 0) {
        CHECK_INT(write(fd, text, strlen(text)), (int)strlen(text));
        CHECK_INT(close(fd), 0);
    }
}

static void test_error_and_stdio(const char *base_dir) {
    printf("error / stdio / low-level I/O\n");

    int *err = __error();
    CHECK_PTR(err);
    if (err) {
        *err = 0;
    }

    CHECK_INT(putchar('Q'), 'Q');
    CHECK_INT(getchar(), EOF);

    errno = 0;
    perror("core_io_fs");

    char path1[PATH_MAX];
    char path2[PATH_MAX];
    join_path(path1, sizeof(path1), base_dir, "write.txt");
    join_path(path2, sizeof(path2), base_dir, "writev.txt");

    write_text_file(path1, "hello");

    int fd = open(path1, O_RDONLY);
    CHECK(fd >= 0);
    if (fd >= 0) {
        char buf[16] = {0};
        CHECK_INT(read(fd, buf, sizeof(buf)), 5);
        CHECK_INT(memcmp(buf, "hello", 5), 0);
        CHECK_INT(close(fd), 0);
    }

    int fdv = open(path2, O_RDWR | O_CREAT | O_TRUNC, 0644);
    CHECK(fdv >= 0);
    if (fdv >= 0) {
        struct iovec vecs[2];
        vecs[0].iov_base = (void *)"ab";
        vecs[0].iov_len = 2;
        vecs[1].iov_base = (void *)"cd";
        vecs[1].iov_len = 2;
        CHECK_INT(writev(fdv, vecs, 2), 4);
        CHECK_INT(close(fdv), 0);

        fdv = open(path2, O_RDONLY);
        CHECK(fdv >= 0);
        if (fdv >= 0) {
            char buf[16] = {0};
            CHECK_INT(read(fdv, buf, sizeof(buf)), 4);
            CHECK_INT(memcmp(buf, "abcd", 4), 0);
            CHECK_INT(close(fdv), 0);
        }
    }

    FILE *stream = fdopen(1, "w");
    CHECK_PTR(stream);
    if (stream) {
        CHECK_INT(fileno(stream), 1);
        CHECK_INT(fwrite("xy", 1, 2, stream), 2);
        CHECK_INT(fflush(stream), 0);
        CHECK_INT(fclose(stream), 0);
    }

    CHECK_INT(fputs("fputs\n", stdout), 6);
    CHECK_INT(printf("printf %d\n", 123), 11);
    CHECK_INT(fprintf(stderr, "fprintf %s\n", "ok"), 11);
    CHECK_INT(sleep(0), 0);

    struct timespec req = {0, 0};
    CHECK_INT(nanosleep(&req, NULL), 0);
}

static void test_filesystem(const char *base_dir) {
    printf("filesystem / copy / stat\n");

    char mkdir_path[PATH_MAX];
    char rmdir_path[PATH_MAX];
    char unlink_path[PATH_MAX];
    char rename_old[PATH_MAX];
    char rename_new[PATH_MAX];
    char chmod_path[PATH_MAX];
    char fchmod_path[PATH_MAX];
    char chown_path[PATH_MAX];
    char fchown_path[PATH_MAX];
    char copy_src[PATH_MAX];
    char copy_dst[PATH_MAX];
    char fcopy_src[PATH_MAX];
    char fcopy_dst[PATH_MAX];

    join_path(mkdir_path, sizeof(mkdir_path), base_dir, "mkdir_dir");
    join_path(rmdir_path, sizeof(rmdir_path), base_dir, "rmdir_dir");
    join_path(unlink_path, sizeof(unlink_path), base_dir, "unlink_me.txt");
    join_path(rename_old, sizeof(rename_old), base_dir, "rename_old.txt");
    join_path(rename_new, sizeof(rename_new), base_dir, "rename_new.txt");
    join_path(chmod_path, sizeof(chmod_path), base_dir, "chmod_me.txt");
    join_path(fchmod_path, sizeof(fchmod_path), base_dir, "fchmod_me.txt");
    join_path(chown_path, sizeof(chown_path), base_dir, "chown_me.txt");
    join_path(fchown_path, sizeof(fchown_path), base_dir, "fchown_me.txt");
    join_path(copy_src, sizeof(copy_src), base_dir, "copy_src.txt");
    join_path(copy_dst, sizeof(copy_dst), base_dir, "copy_dst.txt");
    join_path(fcopy_src, sizeof(fcopy_src), base_dir, "fcopy_src.txt");
    join_path(fcopy_dst, sizeof(fcopy_dst), base_dir, "fcopy_dst.txt");

    CHECK_INT(mkdir(mkdir_path, 0755), 0);
    CHECK_INT(mkdir(rmdir_path, 0755), 0);
    CHECK_INT(rmdir(rmdir_path), 0);

    write_text_file(unlink_path, "unlink");
    CHECK_INT(unlink(unlink_path), 0);

    write_text_file(rename_old, "rename");
    CHECK_INT(rename(rename_old, rename_new), 0);

    write_text_file(chmod_path, "chmod");
    CHECK_INT(chmod(chmod_path, 0600), 0);

    write_text_file(fchmod_path, "fchmod");
    int fchmod_fd = open(fchmod_path, O_RDWR);
    CHECK(fchmod_fd >= 0);
    if (fchmod_fd >= 0) {
        CHECK_INT(fchmod(fchmod_fd, 0640), 0);
        CHECK_INT(close(fchmod_fd), 0);
    }

    write_text_file(chown_path, "chown");
    struct stat st = {0};
    CHECK_INT(stat(chown_path, &st), 0);
    if (st.st_uid || st.st_gid) {
        CHECK_INT(chown(chown_path, st.st_uid, st.st_gid), 0);
    } else {
        CHECK_INT(chown(chown_path, 0, 0), 0);
    }

    write_text_file(fchown_path, "fchown");
    int fchown_fd = open(fchown_path, O_RDONLY);
    CHECK(fchown_fd >= 0);
    if (fchown_fd >= 0) {
        CHECK_INT(fchown(fchown_fd, st.st_uid ? st.st_uid : 0, st.st_gid ? st.st_gid : 0), 0);
        CHECK_INT(close(fchown_fd), 0);
    }

    write_text_file(copy_src, "copyfile-data");
    CHECK_INT(copyfile(copy_src, copy_dst, 0, 0), 0);

    write_text_file(fcopy_src, "fcopyfile-data");
    int src_fd = open(fcopy_src, O_RDONLY);
    int dst_fd = open(fcopy_dst, O_RDWR | O_CREAT | O_TRUNC, 0644);
    CHECK(src_fd >= 0);
    CHECK(dst_fd >= 0);
    if (src_fd >= 0 && dst_fd >= 0) {
        CHECK_INT(fcopyfile(src_fd, dst_fd, 0, 0), 0);
        CHECK_INT(close(src_fd), 0);
        CHECK_INT(close(dst_fd), 0);
    }

    int stat_fd = open(copy_dst, O_RDONLY);
    CHECK(stat_fd >= 0);
    if (stat_fd >= 0) {
        struct stat fst = {0};
        CHECK_INT(fstat(stat_fd, &fst), 0);
        CHECK(fst.st_size > 0);
        CHECK_INT(close(stat_fd), 0);
    }

    struct stat pstat = {0};
    CHECK_INT(stat(copy_src, &pstat), 0);
    CHECK_INT(lstat(copy_src, &pstat), 0);
}

static void test_getopt_env_dlopen(const char *base_dir) {
    (void)base_dir;
    printf("getopt / env / dlfcn / maskrune / mbrtowc / wcwidth\n");

    char *argv[] = {"prog", "-a", "-b", "tail", NULL};
    int argc = 4;
    optind = 1;
    CHECK_INT(getopt(argc, argv, "ab"), 'a');
    CHECK_INT(getopt(argc, argv, "ab"), 'b');
    CHECK_INT(optind, 3);
    CHECK_INT(getopt(argc, argv, "ab"), -1);

    CHECK_INT(setenv("NON_CF_TEST_VAR", "value", 1), 0);
    CHECK(getenv("NON_CF_TEST_VAR") == NULL);
    CHECK_INT(unsetenv("NON_CF_TEST_VAR"), 0);
    CHECK(getenv("NON_CF_TEST_VAR") == NULL);

    void *handle = dlopen(NULL, RTLD_LAZY);
    CHECK_PTR(handle);
    if (handle) {
        void *sym = dlsym(handle, "printf");
        CHECK_PTR(sym);
        CHECK_INT(dlerror() == NULL, 1);
        CHECK_INT(dlclose(handle), 0);
    }

#if defined(__i386__)
    CHECK_INT(maskrune('7', 0x4000), 1);
    CHECK_INT(maskrune('x', 0x4000), 0);
#else
    CHECK_INT(('7' >= '0' && '7' <= '9') ? 1 : 0, 1);
    CHECK_INT(('x' >= '0' && 'x' <= '9') ? 1 : 0, 0);
#endif

    wchar_t out = 0;
    CHECK_INT(mbrtowc(&out, "A", 1, NULL), 1);
    CHECK_INT((unsigned long)out, 'A');
    CHECK_INT(wcwidth(L'A'), 1);
    CHECK_INT(wcwidth(L'\n'), 0);
}

static void test_fts(const char *base_dir) {
    printf("fts traversal\n");

    char root[PATH_MAX];
    char keep[PATH_MAX];
    char skip_dir[PATH_MAX];
    char skip_file[PATH_MAX];
    char subdir[PATH_MAX];
    char subfile[PATH_MAX];

    join_path(root, sizeof(root), base_dir, "fts_root");
    join_path(keep, sizeof(keep), root, "keep.txt");
    join_path(skip_dir, sizeof(skip_dir), root, "skip");
    join_path(skip_file, sizeof(skip_file), skip_dir, "ignored.txt");
    join_path(subdir, sizeof(subdir), root, "sub");
    join_path(subfile, sizeof(subfile), subdir, "nested.txt");

    CHECK_INT(mkdir(root, 0755), 0);
    CHECK_INT(mkdir(skip_dir, 0755), 0);
    CHECK_INT(mkdir(subdir, 0755), 0);
    write_text_file(keep, "keep");
    write_text_file(skip_file, "ignore me");
    write_text_file(subfile, "nested");

    char *paths[] = {root, NULL};
    FTS *fts = fts_open(paths, FTS_PHYSICAL, NULL);
    CHECK_PTR(fts);
    if (!fts) {
        return;
    }

    FTSENT *ent = NULL;
    int saw_root = 0;
    int saw_keep = 0;
    int saw_sub = 0;
    int saw_nested = 0;
    int saw_skip = 0;
    int saw_skip_child = 0;
    int saw_children = 0;

    while ((ent = fts_read(fts)) != NULL) {
        if (strcmp(ent->fts_name, "fts_root") == 0 && ent->fts_info == FTS_D) {
            saw_root = 1;
            FTSENT *kids = fts_children(fts, 0);
            CHECK_PTR(kids);
            if (kids) {
                saw_children = 1;
                for (FTSENT *child = kids; child; child = child->fts_link) {
                    if (strcmp(child->fts_name, "keep.txt") == 0) {
                        saw_keep = 1;
                    }
                    if (strcmp(child->fts_name, "skip") == 0) {
                        saw_skip = 1;
                    }
                    if (strcmp(child->fts_name, "sub") == 0) {
                        saw_sub = 1;
                    }
                }
            }
        }

        if (strcmp(ent->fts_name, "skip") == 0 && ent->fts_info == FTS_D) {
            CHECK_INT(fts_set(fts, ent, FTS_SKIP), 0);
        }

        if (strcmp(ent->fts_name, "ignored.txt") == 0) {
            saw_skip_child = 1;
        }
        if (strcmp(ent->fts_name, "keep.txt") == 0) {
            saw_keep = 1;
        }
        if (strcmp(ent->fts_name, "sub") == 0 && ent->fts_info == FTS_D) {
            saw_sub = 1;
        }
        if (strcmp(ent->fts_name, "nested.txt") == 0) {
            saw_nested = 1;
        }
    }

    CHECK(saw_root);
    CHECK(saw_children);
    CHECK(saw_keep);
    CHECK(saw_skip);
    CHECK(saw_sub);
    CHECK(saw_nested);
    CHECK(!saw_skip_child);

    CHECK_INT(fts_close(fts), 0);
}

int main(int argc, char **argv) {
    const char *base_dir = argc > 1 ? argv[1] : ".";

    printf("core_io_fs\n");
    test_error_and_stdio(base_dir);
    test_filesystem(base_dir);
    test_getopt_env_dlopen(base_dir);
    test_fts(base_dir);

    printf("done\n");
    return non_cf_failures == 0 ? 0 : 1;
}
