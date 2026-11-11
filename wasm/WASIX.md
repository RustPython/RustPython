# WASIX sockets

The optional `wasix` Cargo feature enables real IPv4/IPv6 TCP and UDP sockets
when compiling for `wasm32-wasip1`. It imports the `wasix_32v1` socket and DNS
functions in addition to standard `wasi_snapshot_preview1` functions. These
extensions are **not portable WASI preview1**. Wasmer 7.5.0 is the tested host.

```sh
cargo build --locked --profile wasm-release --target wasm32-wasip1 \
  --no-default-features \
  --features freeze-stdlib,stdlib,stdio,importlib,host_env,wasix

wasmer run --net --volume "$PWD" \
  --env "RUSTPYTHONPATH=$PWD/Lib" \
  target/wasm32-wasip1/wasm-release/rustpython.wasm -- \
  "$PWD/extra_tests/wasix_socket.py" -v
```

`--net` grants the guest access to the host network. Only use it for trusted
Python code, or use Wasmer's `--net=<rules>` filters to restrict access. Mount
only directories the guest should be able to read or write. Without networking
permission, operations fail with an OS error. The tests use loopback sockets
and resolve `localhost`; no public service is needed. Check the denied case by
omitting `--net` and passing `--denied` to the test script.

## Supported behavior

- TCP bind, listen, connect, accept, send, receive, shutdown, and endpoint names.
- UDP bind, connect, send/receive, sendto/recvfrom, including empty datagrams.
- `sendall`, `recv_into`, `recvfrom_into`, `makefile`, and `create_connection`.
- Blocking, nonblocking and finite-timeout socket operations using host polling.
- Descriptor ownership, close, detach, duplication and cleanup on destruction.
- `MSG_PEEK` and the socket options supported by the WASIX host.
- Forward DNS, numeric IPv4/IPv6 addresses and numeric service ports through
  `getaddrinfo`/`gethostbyname`. DNS resolution itself follows the host's timing,
  not the socket timeout.

## Boundaries

WASI preview1 has no standard socket-open, connect or DNS APIs. Builds without
`wasix`, browser WebAssembly and WASI preview2 retain address conversion and
other host-independent helpers but reject socket construction with `ENOTSUP`.
They do not invent descriptors or report successful network operations. A
WASIX build cannot be instantiated on a host missing its required imports.

This backend does not provide Unix-domain/raw sockets, ancillary messages,
multicast configuration, interface enumeration, reverse DNS, system hostname,
service-name lookup or IPv6 flowinfo/scope IDs. Unsupported flags and options
raise errors instead of being silently ignored. `getaddrinfo` accepts
`AI_PASSIVE`, `AI_NUMERICHOST` and `AI_NUMERICSERV`; other flags are rejected.

Wasmer's descriptor metadata identifies connected TCP and unbound sockets,
but not TCP listeners or bound UDP sockets. Pass explicit `family` and `type`
when wrapping one of those descriptors with `fileno`. `accept`, `dup` and
`socket.fromfd` preserve the Python attributes. When the host cannot identify
a rewrapped descriptor, `getsockopt(SO_TYPE)` raises `ENOTSUP` rather than
claiming that caller-supplied metadata is a host result. Descriptor inheritance
across subprocesses is not provided by this backend.

TLS is separate and is not enabled or validated by this feature. The normal
native Unix/Windows socket implementation is unchanged.
