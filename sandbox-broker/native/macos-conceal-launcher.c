#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static int read_barrier(void) {
    char byte = 0;
    do {
        ssize_t count = read(STDIN_FILENO, &byte, 1);
        if (count == 0) {
            return 125;
        }
        if (count < 0) {
            if (errno == EINTR) {
                continue;
            }
            return 125;
        }
    } while (byte != '\n');
    return 0;
}

int main(int argc, char **argv) {
    if (argc < 5 || strcmp(argv[3], "--") != 0) {
        fputs("invalid conceal launcher arguments\n", stderr);
        return 125;
    }
    int barrier = read_barrier();
    if (barrier != 0) {
        return barrier;
    }
    if (setenv("DYLD_INSERT_LIBRARIES", argv[1], 1) != 0 ||
        setenv("PI_SANDBOX_CONCEALED_PATHS", argv[2], 1) != 0) {
        perror("cannot set conceal environment");
        return 125;
    }
    execv(argv[4], &argv[4]);
    perror("cannot start sandbox command");
    return 125;
}
