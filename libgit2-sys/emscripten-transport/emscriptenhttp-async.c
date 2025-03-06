#ifdef __EMSCRIPTEN__

#include "common.h"
#include "emscripten.h"
#include "git2/transport.h"
#include "smart.h"

#define DEFAULT_BUFSIZE 65536

static const char *upload_pack_ls_service_url =
    "/info/refs?service=git-upload-pack";
static const char *upload_pack_service_url = "/git-upload-pack";
static const char *receive_pack_ls_service_url =
    "/info/refs?service=git-receive-pack";
static const char *receive_pack_service_url = "/git-receive-pack";

typedef struct {
  git_smart_subtransport_stream parent;
  const char *service_url;
  int connectionNo;
} emscriptenhttp_stream;

typedef struct {
  git_smart_subtransport parent;
  transport_smart *owner;

} emscriptenhttp_subtransport;

// Asyncified functions wrap async transports for browser when not using a
// webworker

EM_JS(int, emscriptenhttp_do_get, (const char *url, size_t buf_size), {
  return Asyncify.handleAsync(async() => {
    const urlString = UTF8ToString(url);
    return await Module.emscriptenhttpconnect(urlString, buf_size);
  });
});

EM_JS(int, emscriptenhttp_do_post, (const char *url, size_t buf_size), {
  return Asyncify.handleAsync(async() => {
    const urlString = UTF8ToString(url);
    return await Module.emscriptenhttpconnect(urlString, buf_size, 'POST', {
      'Content-Type' : urlString.indexOf('git-upload-pack') > 0
          ? 'application/x-git-upload-pack-request'
          : 'application/x-git-receive-pack-request'
    });
  });
});

EM_JS(size_t, emscriptenhttp_do_read,
      (int connectionNo, char *buffer, size_t buf_size), {
        return Asyncify.handleAsync(async() => {
          return await Module.emscriptenhttpread(connectionNo, buffer,
                                                 buf_size);
        });
      });

static void emscriptenhttp_stream_free(git_smart_subtransport_stream *stream) {
  emscriptenhttp_stream *s = (emscriptenhttp_stream *)stream;

  git__free(s);
}

static int emscriptenhttp_stream_read(git_smart_subtransport_stream *stream,
                                      char *buffer, size_t buf_size,
                                      size_t *bytes_read) {
  emscriptenhttp_stream *s = (emscriptenhttp_stream *)stream;

  if (s->connectionNo == -1) {
    s->connectionNo = emscriptenhttp_do_get(s->service_url, DEFAULT_BUFSIZE);
  }

  int read = emscriptenhttp_do_read(s->connectionNo, buffer, buf_size);

  if (read < 0) {
    git_error_set(0, "request aborted by user");
    return -1;
  } else {
    *bytes_read = (size_t)read;
  }

  return 0;
}

static int
emscriptenhttp_stream_write_single(git_smart_subtransport_stream *stream,
                                   const char *buffer, size_t len) {
  emscriptenhttp_stream *s = (emscriptenhttp_stream *)stream;

  if (s->connectionNo == -1) {
    s->connectionNo = emscriptenhttp_do_post(s->service_url, DEFAULT_BUFSIZE);
  }

  EM_ASM(
      { return Module.emscriptenhttpwrite($0, $1, $2); }, s->connectionNo,
      buffer, len);

  return 0;
}

static int emscriptenhttp_stream_alloc(emscriptenhttp_subtransport *t,
                                       emscriptenhttp_stream **stream) {
  emscriptenhttp_stream *s;

  if (!stream)
    return -1;

  s = git__calloc(1, sizeof(emscriptenhttp_stream));
  GIT_ERROR_CHECK_ALLOC(s);

  s->parent.subtransport = &t->parent;
  s->parent.read = emscriptenhttp_stream_read;
  s->parent.write = emscriptenhttp_stream_write_single;
  s->parent.free = emscriptenhttp_stream_free;
  s->connectionNo = -1;

  *stream = s;

  return 0;
}

static int emscriptenhttp_action(git_smart_subtransport_stream **stream,
                                 git_smart_subtransport *subtransport,
                                 const char *url, git_smart_service_t action) {
  emscriptenhttp_subtransport *t = (emscriptenhttp_subtransport *)subtransport;
  emscriptenhttp_stream *s;

  if (emscriptenhttp_stream_alloc(t, &s) < 0)
    return -1;

  git_str buf = GIT_STR_INIT;

  switch (action) {
  case GIT_SERVICE_UPLOADPACK_LS:
    git_str_printf(&buf, "%s%s", url, upload_pack_ls_service_url);

    break;
  case GIT_SERVICE_UPLOADPACK:
    git_str_printf(&buf, "%s%s", url, upload_pack_service_url);
    break;
  case GIT_SERVICE_RECEIVEPACK_LS:
    git_str_printf(&buf, "%s%s", url, receive_pack_ls_service_url);
    break;
  case GIT_SERVICE_RECEIVEPACK:
    git_str_printf(&buf, "%s%s", url, receive_pack_service_url);
    break;
  }

  s->service_url = git_str_cstr(&buf);
  *stream = &s->parent;

  return 0;
}

static int emscriptenhttp_close(git_smart_subtransport *subtransport) {
  return 0;
}

static void emscriptenhttp_free(git_smart_subtransport *subtransport) {
  emscriptenhttp_subtransport *t = (emscriptenhttp_subtransport *)subtransport;

  emscriptenhttp_close(subtransport);

  git__free(t);
}

int git_smart_subtransport_http(git_smart_subtransport **out,
                                git_transport *owner, void *param) {
  emscriptenhttp_subtransport *t;

  GIT_UNUSED(param);

  if (!out)
    return -1;

  t = git__calloc(1, sizeof(emscriptenhttp_subtransport));
  GIT_ERROR_CHECK_ALLOC(t);

  t->owner = (transport_smart *)owner;
  t->parent.action = emscriptenhttp_action;
  t->parent.close = emscriptenhttp_close;
  t->parent.free = emscriptenhttp_free;

  *out = (git_smart_subtransport *)t;

  return 0;
}

#endif /* __EMSCRIPTEN__ */
