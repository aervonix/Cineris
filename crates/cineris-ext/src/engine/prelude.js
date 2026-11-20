// The host functions as a script sees them (docs/architecture/12-extensions.md,
// "Host functions"). Evaluated once in each runtime, before the extension's
// own code. It takes the two native entry points off the global object, so
// that a script can reach the host only through `cineris`, and every value
// it passes crosses as JSON text: no function or object of the script's is
// handed to the host.
(() => {
  "use strict";
  // A script's code must not run while an error is being made: the engine
  // calls `Error.prepareStackTrace` when it builds one, the host's own
  // included. The native side holds nothing while it does, and this keeps
  // the hook from being set at all.
  Object.defineProperty(Error, "prepareStackTrace", {
    value: undefined,
    writable: false,
    enumerable: false,
    configurable: false,
  });
  const host = globalThis.__cineris_host;
  const register = globalThis.__cineris_register;
  delete globalThis.__cineris_host;
  delete globalThis.__cineris_register;

  const stringify = JSON.stringify;
  const call = (kind, payload) => host(kind, stringify(payload === undefined ? null : payload));
  const text = (parts) => parts.map((part) => (typeof part === "string" ? part : String(part))).join(" ");
  const options = (value) => (value === undefined || value === null ? {} : value);

  const fs = Object.freeze({
    read(path, opts) {
      const o = options(opts);
      return call("read", { path, offset: o.offset, limit: o.limit });
    },
    write(path, content) {
      return call("write", { path, text: content });
    },
    list(path, opts) {
      return call("list", { path, glob: options(opts).glob });
    },
    search(pattern, opts) {
      const o = options(opts);
      return call("search", { pattern, path: o.path, glob: o.glob });
    },
  });

  const state = Object.freeze({
    get(key, opts) {
      return call("state.get", { key, scope: options(opts).scope });
    },
    set(key, value, opts) {
      return call("state.set", { key, value, scope: options(opts).scope });
    },
  });

  const pairs = (headers) => {
    if (headers === undefined || headers === null) return [];
    if (Array.isArray(headers)) return headers.map(([name, value]) => [String(name), String(value)]);
    return Object.keys(headers).map((name) => [name, String(headers[name])]);
  };

  const http = Object.freeze({
    fetch(url, opts) {
      const o = options(opts);
      return call("fetch", {
        url,
        method: o.method,
        headers: pairs(o.headers),
        body: o.body,
        timeout: o.timeout,
      });
    },
  });

  const log = (...parts) => {
    call("log", text(parts));
  };

  const cineris = Object.freeze({
    tool(def) {
      register("tool", def);
    },
    command(def) {
      register("command", def);
    },
    on(event, filter, fn) {
      if (typeof filter === "function") {
        register("hook", { event }, filter);
      } else {
        register("hook", Object.assign({}, options(filter), { event }), fn);
      }
    },
    exec(program, args, opts) {
      const o = options(opts);
      return call("exec", {
        program,
        args: args === undefined ? [] : args,
        cwd: o.cwd,
        stdin: o.stdin,
        timeout: o.timeout,
      });
    },
    fs,
    http,
    ask(question, choices) {
      return call("ask", { question, choices: choices === undefined ? [] : choices });
    },
    notify(message) {
      return call("notify", String(message));
    },
    progress(message) {
      return call("progress", String(message));
    },
    state,
    log,
  });

  const console = Object.freeze({ log, info: log, warn: log, error: log, debug: log });
  for (const [name, value] of [["cineris", cineris], ["console", console]]) {
    Object.defineProperty(globalThis, name, { value, writable: false, enumerable: false, configurable: false });
  }
})();
