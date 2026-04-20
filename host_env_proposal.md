# Plan: Create `rustpython-host_env` crate

## Context

RustPython controls host OS access via the `host_env` feature flag, enforced by `#[cfg(feature = "host_env")]` scattered across hundreds of locations. If a `cfg` is forgotten, host code leaks into sandbox builds silently.

By isolating host OS API wrappers into a dedicated crate, **the crate boundary itself becomes the sandbox guarantee**. Key constraint: this crate has **zero Python runtime dependency**. All Python-level bindings must be added by the consumer (vm/stdlib).

## Current State

### Already Python-free host abstractions in `crates/common/src/`:
- `os.rs` — errno handling, exit_code, winerror_to_errno, OsStr ffi conversions
- `crt_fd.rs` — CRT file descriptor abstraction (Owned/Borrowed types, open/read/write/close)
- `fileutils.rs` — fstat, fopen, Windows StatStruct
- `windows.rs` — ToWideString, FromWideString traits
- `macros.rs` — `suppress_iph!` macro (MSVC invalid parameter handler suppression)

### Pure host functions embedded in vm/stdlib modules:

These files mix Python bindings with pure host API calls. The host parts should be extracted:

**`vm/src/stdlib/posix.rs`** (2908 lines):
- `set_inheritable(fd, inheritable)` — pure nix fcntl wrapper
- `getgroups_impl()` — pure libc/nix wrapper
- `get_right_permission()`, `get_permissions()` — pure permission logic
- 400+ libc constant re-exports (`#[pyattr] use libc::*`)

**`vm/src/stdlib/nt.rs`** (2301 lines):
- `win32_hchmod()`, `win32_lchmod()`, `fchmod_impl()` — pure Windows API calls (currently return PyResult, should return io::Result)
- Spawn mode constants, `O_*` flags

**`vm/src/stdlib/_signal.rs`** (729 lines):
- `timeval_to_double()`, `double_to_timeval()`, `itimerval_to_tuple()` — pure math
- 30+ signal/timer constants

**`vm/src/stdlib/time.rs`** (1616 lines):
- `asctime_from_tm()` — pure string formatting
- `get_tz_info()` — pure Windows API
- Time unit constants (`SEC_TO_MS`, `MS_TO_US`, etc.)
- `duration_since_system_now()` — host clock access (currently takes vm, can return io::Result instead)

**`vm/src/stdlib/msvcrt.rs`**:
- `getch()`, `getwch()`, `getche()`, `getwche()`, `kbhit()`, `setmode_binary()` — all pure host
- Locking constants (`LK_UNLCK`, `LK_LOCK`, etc.)

**`vm/src/stdlib/_winapi.rs`** (2180 lines):
- `GetACP()`, `GetCurrentProcess()`, `GetLastError()`, `GetVersion()` — pure host
- 100+ Windows API constants

**`vm/src/stdlib/os.rs`** (2395 lines):
- `fs_metadata()` — pure `std::fs` wrapper
- libc flag constants (`O_APPEND`, `O_CREAT`, etc.)

## Dependency Graph (After)

```
rustpython-host_env  (NEW — zero Python dep, independent of common)
├── Dependencies: libc, nix (unix), windows-sys (win), widestring (win), rustpython-wtf8
├── From common: os, crt_fd, fileutils, windows, macros
└── Extracted from vm/stdlib: posix, nt, signal, time, msvcrt, winapi, socket, mmap, ...

rustpython-common  (NO host_env dependency — pure algorithmic code only)
└── cformat, float_ops, hash, int, str, encodings, etc.

rustpython-vm
├── rustpython-common
├── rustpython-host_env (optional, feature = "host_env")
├── libc (retained for type definitions & constants used inline in #[pyattr])
└── Python bindings call host_env for actual OS operations

rustpython-stdlib
├── rustpython-vm, rustpython-common
├── rustpython-host_env (optional, feature = "host_env")
└── libc, nix, socket2, memmap2 (retained for now — future migration target)
```

`common` and `host_env` are fully independent — no dependency in either direction.

## Phase 1: Create the crate and move modules from common

Create `crates/host_env/`, **move** host modules from common, and update common to re-export.

### New files:

**`crates/host_env/Cargo.toml`:**
```toml
[package]
name = "rustpython-host_env"
description = "Host OS API abstractions for RustPython (zero Python dependency)"
version.workspace = true
edition.workspace = true

[dependencies]
rustpython-wtf8 = { workspace = true }
libc = { workspace = true }
num-traits = { workspace = true }
cfg-if = { workspace = true }

[target.'cfg(unix)'.dependencies]
nix = { workspace = true }

[target.'cfg(windows)'.dependencies]
widestring = { workspace = true }
windows-sys = { workspace = true, features = [
    "Win32_Foundation",
    "Win32_Globalization",
    "Win32_Networking_WinSock",
    "Win32_Storage_FileSystem",
    "Win32_System_Console",
    "Win32_System_Ioctl",
    "Win32_System_LibraryLoader",
    "Win32_System_SystemServices",
    "Win32_System_Time",
] }
```

**`crates/host_env/src/lib.rs`:**
```rust
#[macro_use]
mod macros;
pub use macros::*;

pub mod os;

#[cfg(any(unix, windows, target_os = "wasi"))]
pub mod crt_fd;

#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub mod fileutils;

#[cfg(windows)]
pub mod windows;

// New modules — extracted from vm/stdlib (Phase 2)
#[cfg(unix)]
pub mod posix;
#[cfg(windows)]
pub mod nt;
pub mod signal;
pub mod time;
#[cfg(windows)]
pub mod msvcrt;
#[cfg(windows)]
pub mod winapi;
```

**Modules moved from common**: `os.rs`, `crt_fd.rs`, `fileutils.rs`, `windows.rs`, `macros.rs`

### Modified files:

**`Cargo.toml` (workspace root):**
- Add `"crates/host_env"` to `[workspace.members]`
- Add `rustpython-host_env = { path = "crates/host_env" }` to `[workspace.dependencies]`

**`crates/common/Cargo.toml`:**
- Remove `nix`, `windows-sys`, `widestring` from direct dependencies
- Keep `libc` for type definitions (`wchar_t` in `str.rs`)
- No `host_env` feature or dependency — common stays purely algorithmic

**`crates/common/src/lib.rs`:**
- Remove `pub mod os`, `pub mod crt_fd`, `pub mod fileutils`, `pub mod windows` declarations
- Remove `#[macro_use] mod macros` and `suppress_iph!` macro (moved to host_env)
- Delete the source files: `os.rs`, `crt_fd.rs`, `fileutils.rs`, `windows.rs`, `macros.rs`

**`crates/vm/Cargo.toml`:**
```toml
[features]
host_env = ["rustpython-host_env"]

[dependencies]
rustpython-host_env = { workspace = true, optional = true }
```

**`crates/stdlib/Cargo.toml`:**
```toml
[features]
host_env = ["rustpython-vm/host_env", "rustpython-host_env"]

[dependencies]
rustpython-host_env = { workspace = true, optional = true }
```

### Verification:
```bash
cargo check -p rustpython-host_env
cargo test
cargo check -p rustpython-vm --no-default-features --features compiler,gc   # sandbox build
```

## Phase 2: Extract host functions from vm/stdlib modules

Extract pure host API functions and constants from vm's stdlib modules into new modules within `host_env`.

### New modules in `crates/host_env/src/`:

**`posix.rs`** — extracted from `vm/src/stdlib/posix.rs`:
```rust
use std::os::fd::BorrowedFd;

pub fn set_inheritable(fd: BorrowedFd<'_>, inheritable: bool) -> nix::Result<()> {
    use nix::fcntl;
    let flags = fcntl::FdFlag::from_bits_truncate(fcntl::fcntl(fd, fcntl::FcntlArg::F_GETFD)?);
    let mut new_flags = flags;
    new_flags.set(fcntl::FdFlag::FD_CLOEXEC, !inheritable);
    if flags != new_flags {
        fcntl::fcntl(fd, fcntl::FcntlArg::F_SETFD(new_flags))?;
    }
    Ok(())
}

pub fn getgroups() -> nix::Result<Vec<nix::unistd::Gid>> { ... }
pub fn get_right_permission(mode: u32, file_owner: Uid, file_group: Gid) -> nix::Result<Permissions> { ... }
```

**`nt.rs`** — extracted from `vm/src/stdlib/nt.rs`:
```rust
pub fn win32_hchmod(handle: HANDLE, mode: u32) -> io::Result<()> { ... }
pub fn win32_lchmod(path: &OsStr, mode: u32) -> io::Result<()> { ... }
```

**`signal.rs`** — extracted from `vm/src/stdlib/_signal.rs`:
```rust
pub fn timeval_to_double(tv: &libc::timeval) -> f64 { ... }
pub fn double_to_timeval(val: f64) -> libc::timeval { ... }
pub fn itimerval_to_tuple(it: &libc::itimerval) -> (f64, f64) { ... }
```

**`time.rs`** — extracted from `vm/src/stdlib/time.rs`:
```rust
pub const SEC_TO_MS: i64 = 1000;
pub const MS_TO_US: i64 = 1000;
// ...

pub fn asctime_from_tm(tm: &libc::tm) -> String { ... }
pub fn duration_since_system_now() -> io::Result<Duration> { ... }

#[cfg(windows)]
pub fn get_tz_info() -> TIME_ZONE_INFORMATION { ... }
```

**`msvcrt.rs`** — extracted from `vm/src/stdlib/msvcrt.rs`:
```rust
pub fn getch() -> Vec<u8> { ... }
pub fn getwch() -> String { ... }
pub fn kbhit() -> i32 { ... }
pub fn setmode_binary(fd: crt_fd::Borrowed<'_>) { ... }

pub const LK_UNLCK: i32 = 0;
pub const LK_LOCK: i32 = 1;
// ...
```

**`winapi.rs`** — extracted from `vm/src/stdlib/_winapi.rs`:
```rust
pub fn get_acp() -> u32 { ... }
pub fn get_current_process() -> HANDLE { ... }
pub fn get_last_error() -> u32 { ... }
pub fn get_version() -> u32 { ... }
// + Windows API constants
```

### Modified vm/stdlib files:

Each file is updated to call `rustpython_host_env::` instead of inlining the host calls:

```rust
// BEFORE (vm/src/stdlib/posix.rs)
pub fn set_inheritable(fd: BorrowedFd<'_>, inheritable: bool) -> nix::Result<()> {
    use nix::fcntl;
    // ... 10 lines of nix API calls
}

// AFTER (vm/src/stdlib/posix.rs)
pub use rustpython_host_env::posix::set_inheritable;
```

## Phase 3: vm/stdlib import migration

All `common::os`, `common::crt_fd`, `common::fileutils`, `common::windows` imports must be updated to `rustpython_host_env::`.

### Import migration targets (vm) — ~20 files:

| File | Current | New |
|------|---------|-----|
| `ospath.rs` | `rustpython_common::crt_fd` | `rustpython_host_env::crt_fd` |
| `stdlib/os.rs` | `common::crt_fd`, `common::os::*` | `rustpython_host_env::` |
| `stdlib/nt.rs` | `common::windows::*`, `common::crt_fd::*` | `rustpython_host_env::` |
| `stdlib/_io.rs` | `common::crt_fd::Offset`, `common::fileutils::fstat` | `rustpython_host_env::` |
| `stdlib/_signal.rs` | `common::crt_fd::*`, `common::fileutils::fstat` | `rustpython_host_env::` |
| `stdlib/posix.rs` | `common::os::*`, `common::crt_fd::Offset` | `rustpython_host_env::` |
| `stdlib/_ctypes/function.rs` | `rustpython_common::os::get_errno` | `rustpython_host_env::os::` |
| `stdlib/_codecs.rs` | `common::windows::ToWideString` | `rustpython_host_env::windows::` |
| `stdlib/sys.rs`, `winreg.rs`, `winsound.rs` | `common::windows::ToWideString` | `rustpython_host_env::windows::` |
| `windows.rs` | `rustpython_common::windows::ToWideString` | `rustpython_host_env::windows::` |
| `exceptions.rs` | `common::os::ErrorExt`, `common::os::winerror_to_errno` | `rustpython_host_env::os::` |

### Import migration targets (stdlib) — ~7 files:

| File | Current | New |
|------|---------|-----|
| `socket.rs` | `common::os::ErrorExt`, `common::os::errno_io_error` | `rustpython_host_env::os::` |
| `mmap.rs` | `rustpython_common::crt_fd` | `rustpython_host_env::crt_fd` |
| `faulthandler.rs` | `rustpython_common::os::{get_errno, set_errno}` | `rustpython_host_env::os::` |
| `posixshmem.rs` | `common::os::errno_io_error` | `rustpython_host_env::os::` |
| `termios.rs` | `common::os::ErrorExt` | `rustpython_host_env::os::` |
| `overlapped.rs` | `crate::vm::common::os::winerror_to_errno` | `rustpython_host_env::os::` |
| `openssl.rs` | `rustpython_common::fileutils::fopen` | `rustpython_host_env::fileutils::` |

### External consumers:

| File | Current | New |
|------|---------|-----|
| `src/lib.rs` | `rustpython_vm::common::os::exit_code` | `rustpython_host_env::os::exit_code` |
| `examples/*.rs` | `vm::common::os::exit_code` | Keep via re-export |

## Phase 4 (Future): Extract host functions from stdlib modules

Same pattern as Phase 2, but for `crates/stdlib/src/` modules. These modules heavily use `libc`, `nix`, `socket2`, `memmap2` directly. Extract the pure host layer into `host_env`.

**Target modules and what goes into host_env:**

| stdlib module | host_env module | What to extract |
|---------------|----------------|-----------------|
| `socket.rs` (3498 lines) | `host_env::socket` | Socket creation, bind, connect, address conversion, cmsg helpers, poll wrappers. Re-export `socket2` types. |
| `mmap.rs` (1625 lines) | `host_env::mmap` | mmap/munmap wrappers, madvise, msync. Re-export `memmap2` types. |
| `select.rs` (745 lines) | `host_env::select` | select/poll/epoll/kqueue wrappers via libc/nix. |
| `posixsubprocess.rs` (537 lines) | `host_env::subprocess` | fork_exec, pipe, dup2, close-on-exec logic. |
| `multiprocessing.rs` (1152 lines) | `host_env::multiprocessing` | Semaphore operations (sem_open/wait/post/unlink via libc). |
| `fcntl.rs` (220 lines) | `host_env::fcntl` | fcntl, ioctl, flock wrappers. |
| `faulthandler.rs` (1333 lines) | `host_env::faulthandler` | Signal handler registration, stack dump via libc write. |
| `locale.rs` (332 lines) | `host_env::locale` | strcoll, strxfrm, setlocale wrappers. |
| `resource.rs` (194 lines) | `host_env::resource` | getrusage, getrlimit, setrlimit wrappers. |
| `grp.rs` (103 lines) | `host_env::grp` | getgrent/setgrent/endgrent, Group lookup via nix. |
| `syslog.rs` (148 lines) | `host_env::syslog` | openlog, syslog, closelog, setlogmask wrappers. |
| `posixshmem.rs` (52 lines) | `host_env::shm` | shm_open, shm_unlink wrappers. |
| `termios.rs` (280 lines) | `host_env::termios` | Terminal attribute get/set via termios crate. |

After this, `nix`, `socket2`, `memmap2`, `rustix` are removed from stdlib's direct dependencies. Only `host_env` provides them.

## Phase 5: Lint enforcement

Three layers of enforcement, from strongest to lightest:

### Layer 1: Crate boundary (compile-time, absolute)

The strongest guarantee. If a crate doesn't list `rustpython-host_env` in its `[dependencies]`, it physically cannot call any host_env function. This is already enforced by Rust's module system.

**Pure crates (no host_env dependency allowed):**
- `rustpython-common`
- `rustpython-compiler`, `rustpython-compiler-core`, `rustpython-compiler-source`
- `rustpython-codegen`
- `rustpython-literal`
- `rustpython-sre_engine`
- `rustpython-wtf8`
- `rustpython-derive`, `rustpython-derive-impl`

CI check:
```bash
# Verify pure crates don't depend on host_env
for crate in common compiler compiler-core compiler-source codegen literal sre_engine wtf8 derive derive-impl; do
  if rg 'rustpython-host_env' "crates/$crate/Cargo.toml"; then
    echo "ERROR: $crate should not depend on host_env"
    exit 1
  fi
done
```

### Layer 2: clippy disallowed_methods (compile-time, configurable)

Block direct host API usage in vm/stdlib. Force all host access through `host_env`.

**Workspace-level `clippy.toml`** (project root):
```toml
disallowed-methods = [
    # Filesystem
    { path = "std::fs::read", reason = "use rustpython_host_env for host filesystem access" },
    { path = "std::fs::write", reason = "use rustpython_host_env" },
    { path = "std::fs::read_to_string", reason = "use rustpython_host_env" },
    { path = "std::fs::read_dir", reason = "use rustpython_host_env" },
    { path = "std::fs::create_dir", reason = "use rustpython_host_env" },
    { path = "std::fs::create_dir_all", reason = "use rustpython_host_env" },
    { path = "std::fs::remove_file", reason = "use rustpython_host_env" },
    { path = "std::fs::remove_dir", reason = "use rustpython_host_env" },
    { path = "std::fs::metadata", reason = "use rustpython_host_env" },
    { path = "std::fs::symlink_metadata", reason = "use rustpython_host_env" },
    { path = "std::fs::canonicalize", reason = "use rustpython_host_env" },
    { path = "std::fs::File::open", reason = "use rustpython_host_env" },
    { path = "std::fs::File::create", reason = "use rustpython_host_env" },
    { path = "std::fs::OpenOptions::open", reason = "use rustpython_host_env" },

    # Environment
    { path = "std::env::var", reason = "use rustpython_host_env" },
    { path = "std::env::var_os", reason = "use rustpython_host_env" },
    { path = "std::env::set_var", reason = "use rustpython_host_env" },
    { path = "std::env::remove_var", reason = "use rustpython_host_env" },
    { path = "std::env::vars", reason = "use rustpython_host_env" },
    { path = "std::env::vars_os", reason = "use rustpython_host_env" },
    { path = "std::env::current_dir", reason = "use rustpython_host_env" },
    { path = "std::env::set_current_dir", reason = "use rustpython_host_env" },
    { path = "std::env::temp_dir", reason = "use rustpython_host_env" },

    # Process
    { path = "std::process::Command::new", reason = "use rustpython_host_env" },
    { path = "std::process::exit", reason = "use rustpython_host_env" },
    { path = "std::process::abort", reason = "use rustpython_host_env" },
    { path = "std::process::id", reason = "use rustpython_host_env" },

    # Network
    { path = "std::net::TcpStream::connect", reason = "use rustpython_host_env" },
    { path = "std::net::TcpListener::bind", reason = "use rustpython_host_env" },
    { path = "std::net::UdpSocket::bind", reason = "use rustpython_host_env" },
]
```

**`crates/host_env/clippy.toml`** (overrides — host_env is allowed to use everything):
```toml
disallowed-methods = []
```

Clippy resolves `clippy.toml` by walking up from the crate directory, so `host_env`'s local config takes precedence over the workspace root.

**Workspace `Cargo.toml`:**
```toml
[workspace.lints.clippy]
disallowed_methods = "deny"
```

### Layer 3: Sandbox build verification (CI)

Build without `host_env` feature to catch any code that accidentally compiles without the feature gate:

```bash
cargo check -p rustpython-vm --no-default-features --features compiler,gc
cargo check -p rustpython-stdlib --no-default-features --features compiler
```

### Layer 4: Whitelist-based module audit (CI script)

Maintain a whitelist of modules in vm/stdlib that are known to NOT use host_env. Any change that adds a `rustpython_host_env` import to a whitelisted module triggers CI failure.

```bash
# .ci/host_env_whitelist.txt — modules that must stay host-free
# vm modules:
crates/vm/src/stdlib/_abc.rs
crates/vm/src/stdlib/_collections.rs
crates/vm/src/stdlib/_functools.rs
crates/vm/src/stdlib/_operator.rs
crates/vm/src/stdlib/_sre.rs
crates/vm/src/stdlib/_stat.rs
crates/vm/src/stdlib/_string.rs
crates/vm/src/stdlib/errno.rs
crates/vm/src/stdlib/gc.rs
crates/vm/src/stdlib/itertools.rs
crates/vm/src/stdlib/marshal.rs

# Check:
while IFS= read -r file; do
  if rg 'rustpython_host_env' "$file" 2>/dev/null; then
    echo "ERROR: $file is whitelisted as host-free but imports host_env"
    exit 1
  fi
done < .ci/host_env_whitelist.txt
```

The inverse is also useful — list all files that ARE allowed to use host_env, and reject any new file that uses it without being on the list. This catches accidental host API usage in new modules.

### Layer 5: `#![no_std]` for pure crates

After removing host modules from `common`, it could potentially become `#![no_std]` unconditionally (it already has `#![cfg_attr(not(feature = "std"), no_std)]`). This is the strongest possible guarantee — no `std::fs`, `std::env`, `std::net`, `std::process` available at all.

Candidate crates for unconditional `#![no_std]`:
- `rustpython-literal`
- `rustpython-wtf8`
- `rustpython-compiler-source`

### Summary of enforcement layers

| Layer | What it catches | Strength | Cost |
|-------|----------------|----------|------|
| Crate boundary | Missing host_env dependency | Absolute — compile error | Zero — automatic |
| clippy disallowed_methods | Direct std::fs/env/net usage | Strong — clippy deny | Low — clippy.toml config |
| Sandbox build | Missing `#[cfg(feature = "host_env")]` | Strong — compile error | Low — CI job |
| Module whitelist | Unintended host_env usage in pure modules | Medium — CI script | Low — maintain whitelist |
| `#![no_std]` | Any std usage in pure crates | Absolute — compile error | Medium — may need refactoring |

## Risk Assessment

| Risk | Level | Mitigation |
|------|-------|------------|
| Target modules have Python type dependencies | **Low** | Verified: only `libc`, `nix`, `windows-sys`, `rustpython-wtf8` |
| Internal cross-references break on move | **Low** | `crt_fd`, `os`, `fileutils`, `windows` all move together; `crate::` paths stay valid |
| `suppress_iph!` macro `$crate` resolution | **Medium** | `$crate` automatically resolves to new crate; `__macro_private` moves alongside |
| Breaking external consumers | **Medium** | Clean break — consumers must update `common::os` to `host_env::os`. No re-export shim. |
| Scope of Phase 2 extraction | **Medium** | Start with clearly pure functions; mixed functions can be migrated incrementally |
