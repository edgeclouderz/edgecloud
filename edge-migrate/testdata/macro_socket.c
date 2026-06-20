/* Fixture: POSIX socket call hidden behind a macro.
 * Without C preprocessor expansion, edge-migrate cannot see the
 * `socket(AF_INET, SOCK_STREAM, 0)` call because it lives inside a
 * `#define`. With clang -E expansion, the macro is expanded and
 * tree-sitter sees the real call.
 *
 * NB: no #include <stdio.h> — we run with -nostdinc and stdio.h is
 * not available.
 */
#define socket(family, type, proto) make_socket(family, type, proto)
#define SOCK_STREAM 1
#define AF_INET 2

/* Forward declaration so clang does not warn on the expanded call.
 * The point of this fixture is the macro expansion, not the call. */
int make_socket(int family, int type, int proto);

int main(void) {
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    (void)fd;
    return 0;
}
