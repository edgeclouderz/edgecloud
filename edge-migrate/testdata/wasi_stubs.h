// Minimal C declarations for every WASI symbol the C `Transformer`
// emits. These stubs are NOT linked — they exist solely so that
// `clang -fsyntax-only -Werror` can type-check the transformer's
// output without the wasi-sdk toolchain.
//
// The intent is regression-net only: the transformer should produce
// code that references only symbols declared here. Any symbol the
// transformer emits that is NOT declared here (e.g. the
// `wasi_poll_pollable_block` and `pollable` reference from the
// pre-fix Accept emit) will surface as an implicit-function-declaration
// or undeclared-identifier error, and the e2e test will fail.
//
// Keep this file in lockstep with `edge-migrate-lib/src/transformer.rs`
// — every new symbol emitted by the transformer needs a matching
// declaration here. The end-to-end test
// (`test_transform_e2e_wasi_stubs_compile` in `transformer.rs`) is
// what catches drift.
#pragma once

// ---------------------------------------------------------------------------
// wasi/sockets.h
// ---------------------------------------------------------------------------

#define IP_ADDRESS_FAMILY_IPV4 0

// Minimal POSIX socket-API stubs so the test fixture
// (`testdata/http_client.c`) compiles. The fixture uses
// `AF_INET` / `SOCK_STREAM` / `struct sockaddr_in` as the POSIX
// types that get rewritten to the WASI equivalents below. These
// declarations are NEVER linked — they're here so `clang
// -fsyntax-only` can type-check the input source before the
// transformer's emit replaces the POSIX calls with WASI calls.
#define AF_INET 2
#define SOCK_STREAM 1
#define SOCK_DGRAM 2

struct sockaddr {
  unsigned short sa_family;
  char sa_data[14];
};

struct sockaddr_in {
  unsigned short sin_family;
  unsigned short sin_port;
  struct in_addr {
    unsigned int s_addr;
  } sin_addr;
  char sin_zero[8];
};

typedef struct wasi_socket_tcp_t wasi_socket_tcp_t;
typedef struct wasi_socket_udp_t wasi_socket_udp_t;

wasi_socket_tcp_t *wasi_socket_tcp_create(int family);
wasi_socket_udp_t *wasi_socket_udp_create(int family);

void wasi_socket_tcp_start_bind(wasi_socket_tcp_t *fd, const void *addr);
void wasi_socket_tcp_finish_bind(wasi_socket_tcp_t *fd);
void wasi_socket_tcp_start_listen(wasi_socket_tcp_t *fd, int backlog);
void wasi_socket_tcp_finish_listen(wasi_socket_tcp_t *fd);
void wasi_socket_tcp_start_connect(wasi_socket_tcp_t *fd, const void *addr);
void wasi_socket_tcp_finish_connect(wasi_socket_tcp_t *fd);
void wasi_socket_tcp_close(wasi_socket_tcp_t *fd);
void wasi_socket_udp_close(wasi_socket_udp_t *fd);

#define WASI_SOCKET_TCP_ACCEPT_ERROR_WOULD_BLOCK 1

typedef struct {
  int tag;
  void *val; /* accepted socket in result.val */
} wasi_socket_tcp_accept_result_t;

wasi_socket_tcp_accept_result_t wasi_socket_tcp_accept(wasi_socket_tcp_t *fd);

// wasi_socket_close dispatches to the typed close based on the call site.
// The transformer always emits the untyped form.
void wasi_socket_close(void *fd);

// ---------------------------------------------------------------------------
// wasi/io/streams.h
// ---------------------------------------------------------------------------

typedef struct wasi_input_stream_t wasi_input_stream_t;
typedef struct wasi_output_stream_t wasi_output_stream_t;

int wasi_input_stream_read(wasi_input_stream_t *fd, void *buf, int len);
int wasi_output_stream_write(wasi_output_stream_t *fd, const void *buf,
                             int len);

// ---------------------------------------------------------------------------
// wasi/filesystem.h
// ---------------------------------------------------------------------------

typedef struct wasi_filesystem_file_t wasi_filesystem_file_t;

wasi_filesystem_file_t *wasi_filesystem_open(const char *path, const char *mode);
int wasi_filesystem_read(wasi_filesystem_file_t *fd, void *buf, int len);
int wasi_filesystem_write(wasi_filesystem_file_t *fd, const void *buf, int len);
void wasi_filesystem_close(wasi_filesystem_file_t *fd);

// ---------------------------------------------------------------------------
// wasi/ip-name-lookup.h
// ---------------------------------------------------------------------------

// Declared for completeness. In MVP this emit is suppressed
// (gethostbyname → NotTransformable; see `edge-migrate/docs/design.md`),
// but the include is still emitted by the transformer, so the header
// must exist as a no-op shim.
typedef struct wasi_ip_name_lookup_t wasi_ip_name_lookup_t;
