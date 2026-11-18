#![allow(clippy::tests_outside_test_module)]

// cspell:ignore rlib rmeta

use core::sync::atomic::{AtomicUsize, Ordering};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct Project {
    path: PathBuf,
}

impl Project {
    fn new() -> Self {
        static NEXT_PROJECT: AtomicUsize = AtomicUsize::new(0);
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = NEXT_PROJECT.fetch_add(1, Ordering::Relaxed);
        let project = Self {
            path: std::env::temp_dir().join(format!(
                "rustpython-source-dependencies-{}-{suffix}-{id}",
                std::process::id()
            )),
        };
        project.write(
            "Cargo.toml",
            &format!(
                r#"[package]
name = "source-dependencies-fixture"
version = "0.0.0"
edition = "2024"

[dependencies]
rustpython-derive = {{ path = {:?} }}
rustpython-compiler-core = {{ path = {:?} }}

[profile.dev]
debug = false

[workspace]
"#,
                workspace.join("crates/derive"),
                workspace.join("crates/compiler-core"),
            ),
        );
        fs::copy(
            workspace.join("Cargo.lock"),
            project.path.join("Cargo.lock"),
        )
        .unwrap();
        project.write(
            "src/main.rs",
            "mod nested;\nfn main() { nested::show(); }\n",
        );
        project.write(
            "src/nested/mod.rs",
            r#"use rustpython_compiler_core::{
    bytecode::{BasicBag, ConstantData},
    frozen::{FrozenCodeObject, FrozenLib},
};
use rustpython_derive::{py_compile, py_freeze};

fn compiled() -> FrozenCodeObject<&'static [u8]> {
    py_compile!(
        file = "../compiled source.py",
        crate_name = "rustpython_compiler_core",
    )
}
fn inline() -> FrozenCodeObject<&'static [u8]> {
    py_compile!(
        source = "VALUE = 3",
        crate_name = "rustpython_compiler_core",
    )
}
const FILE: &FrozenLib = py_freeze!(
    file = "../frozen ü.py",
    module_name = "single",
    crate_name = "rustpython_compiler_core",
);
const DIRECTORY: &FrozenLib = py_freeze!(
    dir = "../python tree",
    crate_name = "rustpython_compiler_core",
);

fn show_code(name: &str, code: FrozenCodeObject<&[u8]>) {
    for constant in code.decode(BasicBag).constants.iter() {
        if let ConstantData::Integer { value } = constant {
            println!("{name}:{value}");
        }
    }
}

pub fn show() {
    show_code("compiled", compiled());
    show_code("inline", inline());
    for (name, module) in FILE {
        show_code(name, module.code);
    }
    let mut modules = DIRECTORY.into_iter().collect::<Vec<_>>();
    modules.sort_unstable_by_key(|(name, _)| *name);
    for (name, module) in modules {
        show_code(&format!("directory/{name}"), module.code);
    }
}
"#,
        );
        project.write("src/compiled source.py", "VALUE = 1\n");
        project.write("src/frozen ü.py", "VALUE = 10\n");
        project.write("src/python tree/root.py", "VALUE = 100\n");
        project.write("src/python tree/pkg/__init__.py", "VALUE = 200\n");
        project.write("src/python tree/badsyntax_skipped.py", "VALUE = (\n");
        project
    }

    fn write(&self, path: &str, contents: &str) {
        let path = self.path.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn run(&self) -> String {
        // A separate target directory avoids the parent cargo test's build lock.
        let output = Command::new(env!("CARGO"))
            .args(["run", "--offline", "--quiet"])
            .arg("--manifest-path")
            .arg(self.path.join("Cargo.toml"))
            .arg("--target-dir")
            .arg(self.path.join("target"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "fixture failed:\n{}",
            String::from_utf8_lossy(&output.stderr),
        );
        String::from_utf8(output.stdout).unwrap()
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn rebuilds_when_python_sources_change() {
    let project = Project::new();
    let mut expected =
        "compiled:1\ninline:3\nsingle:10\ndirectory/pkg:200\ndirectory/root:100\n".to_owned();
    assert_eq!(project.run(), expected);

    for (path, source, before, after) in [
        (
            "src/compiled source.py",
            "VALUE = 2\n",
            "compiled:1",
            "compiled:2",
        ),
        ("src/frozen ü.py", "VALUE = 11\n", "single:10", "single:11"),
        (
            "src/python tree/pkg/__init__.py",
            "VALUE = 201\n",
            "directory/pkg:200",
            "directory/pkg:201",
        ),
        (
            "src/python tree/root.py",
            "VALUE = 101\n",
            "directory/root:100",
            "directory/root:101",
        ),
    ] {
        project.write(path, source);
        expected = expected.replace(before, after);
        assert_eq!(project.run(), expected, "editing {path} did not rebuild");
    }

    project.write("src/python tree/badsyntax_skipped.py", "VALUE = 300\n");
    expected = expected.replace(
        "directory/pkg",
        "directory/badsyntax_skipped:300\ndirectory/pkg",
    );
    assert_eq!(project.run(), expected);

    fs::remove_file(project.path.join("src/python tree/badsyntax_skipped.py")).unwrap();
    expected = expected.replace("directory/badsyntax_skipped:300\n", "");
    assert_eq!(project.run(), expected);
}

#[test]
fn rebuilds_when_non_utf8_source_becomes_valid() {
    let project = Project::new();
    let source = "src/python tree/badsyntax_non_utf8.py";
    fs::write(project.path.join(source), [0xff]).unwrap();
    let original = project.run();

    project.write(source, "VALUE = 301\n");
    let expected = original.replace(
        "directory/pkg",
        "directory/badsyntax_non_utf8:301\ndirectory/pkg",
    );
    assert_eq!(project.run(), expected);
}

#[cfg(target_os = "linux")]
#[test]
fn compiles_from_non_utf8_directory_through_wrapper_macros() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let project = Project::new();
    project.run();
    let deps = project.path.join("target/debug/deps");
    let find_library = |prefix: &str, extension: &str| {
        fs::read_dir(&deps)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with(prefix)
                    && path.extension().and_then(|ext| ext.to_str()) == Some(extension)
            })
            .unwrap()
    };
    let derive = find_library("librustpython_derive-", std::env::consts::DLL_EXTENSION);
    let core = find_library("librustpython_compiler_core-", "rlib");
    let directory = project
        .path
        .join(OsString::from_vec(b"non-utf8-\xff".to_vec()));
    fs::create_dir(&directory).unwrap();
    fs::create_dir(directory.join("package")).unwrap();
    fs::write(directory.join("source.py"), "VALUE = 1\n").unwrap();
    fs::write(directory.join("package/module.py"), "VALUE = 2\n").unwrap();
    project.write(
        "wrappers.rs",
        r#"macro_rules! compile_file {
    ($path:literal) => {
        rustpython_derive::py_compile!(file = $path, crate_name = "rustpython_compiler_core")
    };
}
macro_rules! freeze_file {
    ($path:literal) => {
        rustpython_derive::py_freeze!(file = $path, crate_name = "rustpython_compiler_core")
    };
}
macro_rules! freeze_directory {
    ($path:literal) => {
        rustpython_derive::py_freeze!(dir = $path, crate_name = "rustpython_compiler_core")
    };
}
"#,
    );
    fs::write(
        directory.join("main.rs"),
        r#"#[macro_use]
#[path = "../wrappers.rs"]
mod wrappers;
fn main() {
    let _ = compile_file!("source.py");
    let _ = freeze_file!("source.py");
    let _ = freeze_directory!("package");
}
"#,
    )
    .unwrap();
    let output = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .current_dir(&directory)
        .args([
            "--edition=2024",
            "--crate-name=non_utf8_source_paths",
            "--emit=metadata",
            "main.rs",
        ])
        .arg("-L")
        .arg(format!("dependency={}", deps.display()))
        .arg("--extern")
        .arg(format!("rustpython_derive={}", derive.display()))
        .arg("--extern")
        .arg(format!("rustpython_compiler_core={}", core.display()))
        .arg("-o")
        .arg(project.path.join("probe.rmeta"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "non-UTF-8 source paths failed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
}
