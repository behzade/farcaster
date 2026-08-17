#define _DARWIN_C_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#ifndef F_GETPATH
#define F_GETPATH 50
#endif

struct concealed_path {
    char kind;
    char *path;
};

static struct concealed_path *concealed = NULL;
static size_t concealed_count = 0;

static int hex_value(char value) {
    if (value >= '0' && value <= '9') return value - '0';
    if (value >= 'a' && value <= 'f') return value - 'a' + 10;
    if (value >= 'A' && value <= 'F') return value - 'A' + 10;
    return -1;
}

__attribute__((constructor))
static void load_concealed_paths(void) {
    const char *encoded = getenv("PI_SANDBOX_CONCEALED_PATHS");
    if (encoded == NULL || *encoded == '\0') return;

    size_t count = 1;
    for (const char *cursor = encoded; *cursor != '\0'; cursor++) {
        if (*cursor == ',') count++;
    }
    struct concealed_path *items = calloc(count, sizeof(*items));
    if (items == NULL) return;

    size_t used = 0;
    const char *cursor = encoded;
    while (*cursor != '\0' && used < count) {
        char kind = *cursor++;
        if (*cursor++ != ':') break;
        const char *end = strchr(cursor, ',');
        if (end == NULL) end = cursor + strlen(cursor);
        size_t encoded_length = (size_t)(end - cursor);
        if ((kind != 'f' && kind != 't' && kind != 'g' && kind != 'x') || encoded_length % 2 != 0) break;

        size_t length = encoded_length / 2;
        char *path = malloc(length + 1);
        if (path == NULL) break;
        bool valid = true;
        for (size_t index = 0; index < length; index++) {
            int high = hex_value(cursor[index * 2]);
            int low = hex_value(cursor[index * 2 + 1]);
            if (high < 0 || low < 0) {
                valid = false;
                break;
            }
            path[index] = (char)((high << 4) | low);
        }
        if (!valid || length == 0 || path[0] != '/') {
            free(path);
            break;
        }
        path[length] = '\0';
        items[used++] = (struct concealed_path){.kind = kind, .path = path};
        cursor = *end == ',' ? end + 1 : end;
    }
    if (*cursor != '\0') {
        for (size_t index = 0; index < used; index++) free(items[index].path);
        free(items);
        return;
    }
    concealed = items;
    concealed_count = used;
}

static bool glob_matches_with_budget(const char *pattern, const char *path, size_t *budget) {
    if (*budget == 0) return false;
    (*budget)--;
    while (*pattern != '\0') {
        if (*pattern == '*') {
            const char *end = pattern;
            while (*end == '*') end++;
            bool crosses_slashes = end - pattern >= 2;
            pattern = end;
            if (crosses_slashes && *pattern == '/') {
                pattern++;
                if (glob_matches_with_budget(pattern, path, budget)) return true;
                for (const char *cursor = path; *cursor != '\0'; cursor++) {
                    if (*cursor == '/' && glob_matches_with_budget(pattern, cursor + 1, budget)) {
                        return true;
                    }
                }
                return false;
            }
            for (const char *cursor = path;; cursor++) {
                if (glob_matches_with_budget(pattern, cursor, budget)) return true;
                if (*cursor == '\0' || (!crosses_slashes && *cursor == '/')) return false;
            }
        }
        if (*pattern == '?') {
            if (*path == '\0' || *path == '/') return false;
            pattern++;
            path++;
            continue;
        }
        if (*pattern != *path) return false;
        pattern++;
        path++;
    }
    return *path == '\0';
}

static bool glob_matches(const char *pattern, const char *path) {
    size_t budget = PATH_MAX * 8;
    return glob_matches_with_budget(pattern, path, &budget);
}

static bool normalize_path_at(int directory, const char *path, char output[PATH_MAX]) {
    if (path == NULL || *path == '\0') return false;
    char joined[PATH_MAX * 2];
    if (path[0] == '/') {
        if (strlen(path) >= sizeof(joined)) return false;
        strcpy(joined, path);
    } else {
        char base[PATH_MAX];
        if (directory == AT_FDCWD) {
            if (getcwd(base, sizeof(base)) == NULL) return false;
        } else if (fcntl(directory, F_GETPATH, base) != 0) {
            return false;
        }
        if (snprintf(joined, sizeof(joined), "%s/%s", base, path) >= (int)sizeof(joined)) {
            return false;
        }
    }

    size_t output_length = 0;
    output[output_length++] = '/';
    char *cursor = joined;
    while (*cursor != '\0') {
        while (*cursor == '/') cursor++;
        if (*cursor == '\0') break;
        char *end = strchr(cursor, '/');
        size_t length = end == NULL ? strlen(cursor) : (size_t)(end - cursor);
        if (length == 1 && cursor[0] == '.') {
            cursor = end == NULL ? cursor + length : end;
            continue;
        }
        if (length == 2 && cursor[0] == '.' && cursor[1] == '.') {
            if (output_length > 1) {
                output_length--;
                while (output_length > 1 && output[output_length - 1] != '/') output_length--;
            }
            cursor = end == NULL ? cursor + length : end;
            continue;
        }
        if (output_length > 1 && output[output_length - 1] != '/') {
            if (output_length >= PATH_MAX - 1) return false;
            output[output_length++] = '/';
        }
        if (output_length + length >= PATH_MAX) return false;
        memcpy(output + output_length, cursor, length);
        output_length += length;
        cursor = end == NULL ? cursor + length : end;
    }
    if (output_length > 1 && output[output_length - 1] == '/') output_length--;
    output[output_length] = '\0';
    return true;
}

static bool path_is_concealed_at(int directory, const char *path) {
    char normalized[PATH_MAX];
    if (!normalize_path_at(directory, path, normalized)) return false;
    for (size_t index = 0; index < concealed_count; index++) {
        if (concealed[index].kind != 'x') continue;
        const char *root = concealed[index].path;
        size_t length = strlen(root);
        if (strcmp(normalized, root) == 0 ||
            (strncmp(normalized, root, length) == 0 && normalized[length] == '/')) {
            return false;
        }
    }
    for (size_t index = 0; index < concealed_count; index++) {
        const char *hidden = concealed[index].path;
        if (concealed[index].kind == 'x') continue;
        if (concealed[index].kind == 'g' && glob_matches(hidden, normalized)) return true;
        size_t length = strlen(hidden);
        if (strcmp(normalized, hidden) == 0) return true;
        if (concealed[index].kind == 't' && strncmp(normalized, hidden, length) == 0 &&
            normalized[length] == '/') {
            return true;
        }
    }
    return false;
}

static bool flags_may_read(int flags) {
    return (flags & O_ACCMODE) != O_WRONLY;
}

static int hidden_error(void) {
    errno = ENOENT;
    return -1;
}

static int pi_open(const char *path, int flags, ...) {
    if (flags_may_read(flags) && path_is_concealed_at(AT_FDCWD, path)) return hidden_error();
    if ((flags & O_CREAT) != 0) {
        va_list arguments;
        va_start(arguments, flags);
        mode_t mode = (mode_t)va_arg(arguments, int);
        va_end(arguments);
        return open(path, flags, mode);
    }
    return open(path, flags);
}

static int pi_openat(int directory, const char *path, int flags, ...) {
    if (flags_may_read(flags) && path_is_concealed_at(directory, path)) return hidden_error();
    if ((flags & O_CREAT) != 0) {
        va_list arguments;
        va_start(arguments, flags);
        mode_t mode = (mode_t)va_arg(arguments, int);
        va_end(arguments);
        return openat(directory, path, flags, mode);
    }
    return openat(directory, path, flags);
}

extern int openat_nocancel(int, const char *, int, ...)
    __asm("_openat$NOCANCEL");

static int pi_openat_nocancel(int directory, const char *path, int flags, ...) {
    if (flags_may_read(flags) && path_is_concealed_at(directory, path)) return hidden_error();
    if ((flags & O_CREAT) != 0) {
        va_list arguments;
        va_start(arguments, flags);
        mode_t mode = (mode_t)va_arg(arguments, int);
        va_end(arguments);
        return openat_nocancel(directory, path, flags, mode);
    }
    return openat_nocancel(directory, path, flags);
}

static FILE *pi_fopen(const char *path, const char *mode) {
    if (mode != NULL && (mode[0] == 'r' || strchr(mode, '+') != NULL) &&
        path_is_concealed_at(AT_FDCWD, path)) {
        hidden_error();
        return NULL;
    }
    return fopen(path, mode);
}

static int pi_stat(const char *path, struct stat *buffer) {
    if (path_is_concealed_at(AT_FDCWD, path)) return hidden_error();
    return stat(path, buffer);
}

static int pi_lstat(const char *path, struct stat *buffer) {
    if (path_is_concealed_at(AT_FDCWD, path)) return hidden_error();
    return lstat(path, buffer);
}

static int pi_access(const char *path, int mode) {
    if (path_is_concealed_at(AT_FDCWD, path)) return hidden_error();
    return access(path, mode);
}

static int pi_faccessat(int directory, const char *path, int mode, int flags) {
    if (path_is_concealed_at(directory, path)) return hidden_error();
    return faccessat(directory, path, mode, flags);
}

static int pi_fstatat(int directory, const char *path, struct stat *buffer, int flags) {
    if (path_is_concealed_at(directory, path)) return hidden_error();
    return fstatat(directory, path, buffer, flags);
}

static ssize_t pi_readlink(const char *path, char *buffer, size_t size) {
    if (path_is_concealed_at(AT_FDCWD, path)) return hidden_error();
    return readlink(path, buffer, size);
}

static ssize_t pi_readlinkat(int directory, const char *path, char *buffer, size_t size) {
    if (path_is_concealed_at(directory, path)) return hidden_error();
    return readlinkat(directory, path, buffer, size);
}

#define PI_INTERPOSE(replacement, replacee)                                      \
    __attribute__((used)) static struct {                                        \
        const void *replacement;                                                 \
        const void *replacee;                                                    \
    } interpose_##replacee __attribute__((section("__DATA,__interpose"))) = {    \
        (const void *)(replacement), (const void *)(replacee)                    \
    }

PI_INTERPOSE(pi_open, open);
PI_INTERPOSE(pi_openat, openat);
PI_INTERPOSE(pi_openat_nocancel, openat_nocancel);
PI_INTERPOSE(pi_fopen, fopen);
PI_INTERPOSE(pi_stat, stat);
PI_INTERPOSE(pi_lstat, lstat);
PI_INTERPOSE(pi_access, access);
PI_INTERPOSE(pi_faccessat, faccessat);
PI_INTERPOSE(pi_fstatat, fstatat);
PI_INTERPOSE(pi_readlink, readlink);
PI_INTERPOSE(pi_readlinkat, readlinkat);
