#define _DARWIN_C_SOURCE

#include "../common.h"

#include <pthread.h>
#include <stdint.h>

static pthread_once_t once_token = PTHREAD_ONCE_INIT;
static int once_count = 0;

static void once_init(void) {
    once_count++;
}

static pthread_key_t tls_key;

static void *thread_main(void *arg) {
    intptr_t value = (intptr_t)arg;
    pthread_setspecific(tls_key, (void *)(value + 1));
    pthread_once(&once_token, once_init);
    return (void *)(value + 7);
}

int main(void) {
    printf("pthread_tls\n");

    pthread_t thread;
    void *joined = NULL;
    pthread_mutex_t mutex = PTHREAD_MUTEX_INITIALIZER;
    pthread_cond_t cond = PTHREAD_COND_INITIALIZER;
    pthread_rwlock_t rwlock = PTHREAD_RWLOCK_INITIALIZER;
    pthread_attr_t attr;

    CHECK_INT(pthread_key_create(&tls_key, NULL), 0);
    CHECK_INT(pthread_setspecific(tls_key, (void *)0x11), 0);
    CHECK_PTR(pthread_getspecific(tls_key));
    CHECK_INT((intptr_t)pthread_getspecific(tls_key), 0x11);

    CHECK_INT(pthread_once(&once_token, once_init), 0);
    CHECK_INT(once_count, 1);

    CHECK_INT(pthread_self() != 0, 1);

    CHECK_INT(pthread_mutex_init(&mutex, NULL), 0);
    CHECK_INT(pthread_mutex_lock(&mutex), 0);
    CHECK_INT(pthread_mutex_trylock(&mutex), 0);
    CHECK_INT(pthread_mutex_unlock(&mutex), 0);
    CHECK_INT(pthread_mutex_destroy(&mutex), 0);

    CHECK_INT(pthread_rwlock_init(&rwlock, NULL), 0);
    CHECK_INT(pthread_rwlock_rdlock(&rwlock), 0);
    CHECK_INT(pthread_rwlock_unlock(&rwlock), 0);
    CHECK_INT(pthread_rwlock_wrlock(&rwlock), 0);
    CHECK_INT(pthread_rwlock_unlock(&rwlock), 0);
    CHECK_INT(pthread_rwlock_destroy(&rwlock), 0);

    CHECK_INT(pthread_cond_init(&cond, NULL), 0);
    CHECK_INT(pthread_cond_signal(&cond), 0);
    CHECK_INT(pthread_cond_broadcast(&cond), 0);
    CHECK_INT(pthread_cond_destroy(&cond), 0);

    CHECK_INT(pthread_attr_init(&attr), 0);
    CHECK_INT(pthread_attr_setdetachstate(&attr, PTHREAD_CREATE_JOINABLE), 0);
    CHECK_INT(pthread_attr_setstacksize(&attr, 0x10000), 0);
    CHECK_INT(pthread_attr_destroy(&attr), 0);

    CHECK_INT(pthread_create(&thread, NULL, thread_main, (void *)5), 0);
    CHECK_INT(pthread_join(thread, &joined), 0);
    CHECK_PTR(joined);
    CHECK_INT((intptr_t)joined, 12);
    CHECK_PTR(pthread_getspecific(tls_key));
    CHECK_INT((intptr_t)pthread_getspecific(tls_key), 6);

    CHECK_INT(pthread_cancel(thread), 0);
    pthread_testcancel();
    CHECK_INT(pthread_key_delete(tls_key), 0);

    printf("done\n");
    return non_cf_failures == 0 ? 0 : 1;
}
