// Thread L: open(O_CLOEXEC)+flock(EX|NB), close, reopen, flock(EX|NB) again.
// Thread S (optional): posix_spawn /usr/bin/true in a loop.
// Counts how often the re-lock fails with EWOULDBLOCK although this
// process closed its only descriptor of the file.
#include <fcntl.h>
#include <pthread.h>
#include <spawn.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/file.h>
#include <sys/wait.h>
#include <unistd.h>
#include <errno.h>
extern char **environ;
static atomic_int stop;
static const char *path;
static void *spawner(void *arg) {
    (void)arg;
    char *argv[] = {"/usr/bin/true", NULL};
    while (!atomic_load(&stop)) {
        pid_t pid;
        if (posix_spawn(&pid, "/usr/bin/true", NULL, NULL, argv, environ) == 0)
            waitpid(pid, NULL, 0);
    }
    return NULL;
}
int main(int argc, char **argv) {
    path = argv[1];
    int spawn = atoi(argv[2]);
    int iters = atoi(argv[3]);
    pthread_t t[4];
    for (int i = 0; i < spawn; i++) pthread_create(&t[i], NULL, spawner, NULL);
    int fails = 0;
    for (int i = 0; i < iters; i++) {
        int fd = open(path, O_RDWR | O_CREAT | O_CLOEXEC, 0600);
        if (flock(fd, LOCK_EX | LOCK_NB) != 0) { fails++; close(fd); continue; }
        close(fd);
        fd = open(path, O_RDWR | O_CREAT | O_CLOEXEC, 0600);
        if (flock(fd, LOCK_EX | LOCK_NB) != 0 && errno == EWOULDBLOCK) fails++;
        close(fd);
    }
    atomic_store(&stop, 1);
    for (int i = 0; i < spawn; i++) pthread_join(t[i], NULL);
    printf("spawner_threads=%d iterations=%d relock_failures=%d\n", spawn, iters, fails);
    return 0;
}
