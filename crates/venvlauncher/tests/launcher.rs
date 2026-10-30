#![cfg(windows)]

#[cfg(test)]
mod tests {
    use core::sync::atomic::{AtomicUsize, Ordering};
    use std::ffi::OsString;
    use std::fs;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;
    use std::process::Command;

    struct TestVenv {
        root: PathBuf,
    }

    impl TestVenv {
        fn new() -> Self {
            static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
            let root = loop {
                let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "rustpython-venvlauncher-{}-{id}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => break path,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("cannot create test environment: {error}"),
                }
            };
            let fixture = Self { root };
            let home = fixture.root.join("home");
            let scripts = fixture.root.join("Scripts");
            fs::create_dir(&home).unwrap();
            fs::create_dir(&scripts).unwrap();
            let system_root = std::env::var_os("SystemRoot").unwrap();
            fs::copy(
                PathBuf::from(system_root).join("System32/cmd.exe"),
                home.join("python.exe"),
            )
            .unwrap();
            fs::copy(
                env!("CARGO_BIN_EXE_venvlaunchert"),
                scripts.join("python.exe"),
            )
            .unwrap();
            fs::write(
                fixture.root.join("pyvenv.cfg"),
                format!("home = {}\n", home.display()),
            )
            .unwrap();
            fixture
        }

        fn command(&self) -> Command {
            Command::new(self.root.join("Scripts/python.exe"))
        }
    }

    impl Drop for TestVenv {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[test]
    fn preserves_windows_exit_codes() {
        let fixture = TestVenv::new();
        for code in [0_u32, 255, 256, 4660, 0xC0000005, u32::MAX] {
            let output = fixture
                .command()
                .args(["/d", "/c", "exit", &code.to_string()])
                .output()
                .unwrap();
            assert_eq!(output.status.code().map(|code| code as u32), Some(code));
        }
    }

    #[test]
    fn forwards_windows_wide_arguments() {
        let fixture = TestVenv::new();
        for units in [&[0xd800][..], &[0xdc00][..], &[0xd83d, 0xde00][..]] {
            let output = fixture
                .command()
                .args(["/d", "/u", "/c", "echo"])
                .arg(OsString::from_wide(units))
                .output()
                .unwrap();
            assert!(output.status.success(), "{:?}", output.stderr);
            let expected: Vec<_> = units
                .iter()
                .copied()
                .chain([13, 10])
                .flat_map(u16::to_le_bytes)
                .collect();
            assert_eq!(output.stdout, expected);
        }
    }
}
