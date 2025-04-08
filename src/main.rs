#[cfg(all(
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(any(target_env = "musl", target_env = "sgx"))
))]
mod _alloc {
    use mimalloc::MiMalloc;

    #[global_allocator]
    static GLOBAL: MiMalloc = MiMalloc;
}

pub fn main() -> std::process::ExitCode {
    rustpython::run(|_vm| {})
}
