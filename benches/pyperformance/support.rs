//! Shared by the `pyperformance` Criterion target and `tests/pyperformance.rs`:
//! finding the vendored pyperformance benchmarks and running one of them once
//! inside a RustPython interpreter.
//!
//! Included from both with `#[path = ...] mod support;`, because a bench target
//! with `harness = false` cannot host `#[test]` functions of its own.

// cspell:ignore pyperformance pyperf

use rustpython::{InterpreterBuilder, InterpreterBuilderExt};
use rustpython_vm::{Interpreter, Settings, VirtualMachine, compiler::Mode};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

/// Where the vendored `bm_*` directories and the `pyperf` shim live,
/// relative to the package root `cargo bench` and `cargo test` run in.
pub(crate) const BENCHMARKS_DIR: &str = "benches/pyperformance";

/// One entry to measure: a `run_benchmark.py` plus the command-line arguments
/// that select a variant (`extra_opts` in pyperformance's metadata).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Bench {
    /// pyperformance's name for it, e.g. `pickle_dict`; reported as
    /// `pyperformance/<name>`.
    pub(crate) name: String,
    /// The `bm_*` directory holding `run_benchmark.py`.
    pub(crate) dir: PathBuf,
    pub(crate) extra_opts: Vec<String>,
}

impl Bench {
    pub(crate) fn script(&self) -> PathBuf {
        self.dir.join("run_benchmark.py")
    }
}

/// Read `[tool.pyperformance]` from a `pyproject.toml` or a variant `bm_*.toml`.
fn read_metadata(path: &Path, dir: &Path) -> Bench {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let table: toml::Table = text
        .parse()
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let tool = table
        .get("tool")
        .and_then(|t| t.get("pyperformance"))
        .and_then(|t| t.as_table())
        .unwrap_or_else(|| panic!("{}: no [tool.pyperformance]", path.display()));
    let name = tool
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or_else(|| panic!("{}: [tool.pyperformance] has no name", path.display()))
        .to_owned();
    let extra_opts = tool
        .get("extra_opts")
        .map(|opts| {
            opts.as_array()
                .unwrap_or_else(|| panic!("{}: extra_opts is not an array", path.display()))
                .iter()
                .map(|opt| {
                    opt.as_str()
                        .unwrap_or_else(|| {
                            panic!("{}: extra_opts entry is not a string", path.display())
                        })
                        .to_owned()
                })
                .collect()
        })
        .unwrap_or_default();
    Bench {
        name,
        dir: dir.to_owned(),
        extra_opts,
    }
}

fn sorted_entries(dir: &Path, keep: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| keep(path))
        .collect();
    // `read_dir` order is not stable; sorting keeps `cargo bench` output and
    // local reproductions in the same order on every machine.
    paths.sort();
    paths
}

fn has_bm_prefix(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("bm_"))
}

/// Every benchmark and variant under `root`, in the order they are measured.
///
/// Each `bm_<name>/` directory contributes the entry described by its
/// `pyproject.toml` and one entry per variant file `bm_<variant>.toml` beside
/// it (pyperformance's `<local:name>` manifest entries).
pub(crate) fn discover(root: &Path) -> Vec<Bench> {
    let mut benches = Vec::new();
    for dir in sorted_entries(root, |p| p.is_dir() && has_bm_prefix(p)) {
        // A `bm_*` directory without metadata is not something the vendoring
        // script produces; fail loudly rather than silently skip a benchmark.
        let pyproject = dir.join("pyproject.toml");
        assert!(
            pyproject.is_file(),
            "{}: no pyproject.toml; re-run scripts/vendor-pyperformance.py",
            dir.display()
        );
        benches.push(read_metadata(&pyproject, &dir));
        let variants = sorted_entries(&dir, |p| {
            p.is_file() && p.extension().is_some_and(|ext| ext == "toml") && has_bm_prefix(p)
        });
        benches.extend(variants.iter().map(|path| read_metadata(path, &dir)));
    }
    benches
}

/// An interpreter set up to run `bench`: its directory and the `pyperf` shim
/// on `sys.path`, its `extra_opts` in `sys.argv`, and a fixed hash seed.
pub(crate) fn interpreter_for(bench: &Bench) -> Interpreter {
    let mut settings = Settings::default();
    settings.path_list = vec![
        bench.dir.to_str().unwrap().to_owned(),
        BENCHMARKS_DIR.to_owned(),
    ];
    settings.write_bytecode = false;
    settings.user_site_directory = false;
    // Pin the str/bytes hash secret so dict/set probe sequences (and thus
    // timings) are reproducible across runs.
    settings.hash_seed = Some(0);
    // The shim's `Runner.parse_args` reads `sys.argv[1:]`, so the variant's
    // `extra_opts` take the same argparse path they do under pyperformance.
    settings.argv = core::iter::once(OsString::from("run_benchmark.py"))
        .chain(bench.extra_opts.iter().map(OsString::from))
        .collect();
    // `init_stdlib` adds the stdlib, the `Lib/` path and the TLS provider the
    // `ssl` module needs at import, exactly as the `rustpython` binary does.
    InterpreterBuilder::new()
        .settings(settings)
        .init_stdlib()
        .build()
}

/// Run `source` (the contents of `bench.script()`) once as `__main__`.
///
/// A Python exception prints its traceback and panics, so a broken benchmark
/// fails the run instead of reporting a near-zero time.
pub(crate) fn run_once(vm: &VirtualMachine, bench: &Bench, source: &str) {
    let script = bench.script();
    let script_path = script.to_str().unwrap();
    let code = vm.compile(source, Mode::Exec, script_path).unwrap();
    let scope = vm.new_scope_with_builtins();
    scope
        .globals
        .set_item("__name__", vm.ctx.new_str("__main__").into(), vm)
        .unwrap();
    // Some benchmarks locate their `data/` directory from `__file__`.
    scope
        .globals
        .set_item("__file__", vm.ctx.new_str(script_path).into(), vm)
        .unwrap();
    let res = vm.run_code_obj(code, scope);
    vm.unwrap_pyresult(res);
}
