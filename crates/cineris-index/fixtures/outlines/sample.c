#include <stdio.h>
#include "list.h"

#ifndef SAMPLE_H
#define SAMPLE_H
#define MAX_ITEMS 64
#define SQUARE(x) ((x) * (x))

/* A plain comment. */
typedef struct {
    int x;
    int y;
} point_t;

typedef struct node node_t;

typedef void (*handler_t)(int signal);

struct list {
    node_t *head;
    size_t length;
};

enum color { RED, GREEN, BLUE };

static int helper(int a);

/** Adds two points. */
point_t point_add(point_t a, point_t b)
{
    const char *s = "int fake(void) {";
    point_t out = { a.x + b.x, a.y + b.y };
    return out;
}

static int
helper (int a)
{
    if (a > 0) {
        return a;
    }
    return -a;
}

struct list *list_new(void) {
    return NULL;
}

int main(int argc, char **argv) {
    for (int i = 0; i < argc; i++) {
        printf("%s\n", argv[i]);
    }
    return 0;
}
#endif
