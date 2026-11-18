use rustpython::{InterpreterBuilder, InterpreterBuilderExt};

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub fn main() -> std::process::ExitCode {
    let mut config = InterpreterBuilder::new();
    #[cfg(feature = "stdlib")]
    {
        config = config.init_stdlib();
    }
    #[cfg(all(
        feature = "host_env",
        feature = "stdio",
        any(target_os = "linux", target_os = "macos")
    ))]
    {
        let closed = startup_stdio::closed();
        config = config.init_hook(move |vm| {
            rustpython::vm::common::rc::PyRc::get_mut(&mut vm.state)
                .unwrap()
                .config
                .settings
                .closed_stdio = closed;
        });
    }
    // SAFETY: the executable has not started threads or initialized the VM.
    unsafe { rustpython::run_with_locale(config) }
}

// Rust opens /dev/null for missing standard descriptors before calling main.
// Keep that safety measure, but remember which Python streams should be None.
// This initializer belongs to the executable, never an embedding library.
#[cfg(all(
    feature = "host_env",
    feature = "stdio",
    any(target_os = "linux", target_os = "macos")
))]
mod startup_stdio {
    use core::sync::atomic::{AtomicU8, Ordering};

    static CLOSED: AtomicU8 = AtomicU8::new(0);

    extern "C" fn capture() {
        // SAFETY: libc has initialized this thread-local cell before constructors.
        #[cfg(target_os = "linux")]
        let errno_ptr = unsafe { libc::__errno_location() };
        #[cfg(target_os = "macos")]
        let errno_ptr = unsafe { libc::__error() };
        // SAFETY: the pointer is valid for this thread throughout the callback.
        let saved_errno = unsafe { *errno_ptr };
        let mut mask = 0;
        for fd in 0..3 {
            loop {
                // SAFETY: F_GETFD observes an integer descriptor without touching memory.
                if unsafe { libc::fcntl(fd, libc::F_GETFD) } != -1 {
                    break;
                }
                // SAFETY: fcntl updated this thread's errno on failure.
                let errno = unsafe { *errno_ptr };
                if errno == libc::EINTR {
                    continue;
                }
                if errno == libc::EBADF {
                    mask |= 1 << fd;
                }
                break;
            }
        }
        // SAFETY: restore the same thread-local cell, leaving other constructors
        // with the errno value they would have seen without these queries.
        unsafe { *errno_ptr = saved_errno };
        CLOSED.store(mask, Ordering::Relaxed);
    }

    // SAFETY: the loader calls this before Rust's runtime initialization. capture
    // only uses libc descriptor queries and a lock-free atomic; no Rust I/O,
    // allocation, thread creation, VM access, or descriptor mutation occurs.
    #[used]
    #[cfg_attr(target_os = "linux", unsafe(link_section = ".init_array"))]
    #[cfg_attr(target_os = "macos", unsafe(link_section = "__DATA,__mod_init_func"))]
    static CAPTURE: extern "C" fn() = capture;

    pub(super) fn closed() -> [bool; 3] {
        let mask = CLOSED.load(Ordering::Relaxed);
        core::array::from_fn(|fd| mask & (1 << fd) != 0)
    }
}
