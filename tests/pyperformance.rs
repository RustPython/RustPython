//! Checks for the vendored pyperformance suite in `benches/pyperformance/`:
//! the Criterion target there has `harness = false`, so its support code is
//! tested from here.

// cspell:ignore pyperformance nqueens

#![allow(clippy::tests_outside_test_module)]

extern crate alloc;

#[path = "../benches/pyperformance/support.rs"]
mod support;

use alloc::collections::BTreeSet;
use std::fs;
use std::path::Path;
use support::{BENCHMARKS_DIR, Bench, discover, interpreter_for, run_once};

fn all() -> Vec<Bench> {
    discover(Path::new(BENCHMARKS_DIR))
}

fn find(name: &str) -> Bench {
    all()
        .into_iter()
        .find(|b| b.name == name)
        .unwrap_or_else(|| panic!("no vendored benchmark named {name}"))
}

#[test]
fn pickle_directory_yields_base_and_variants() {
    let pickle_dir = Path::new(BENCHMARKS_DIR).join("bm_pickle");
    let names: Vec<String> = all()
        .into_iter()
        .filter(|b| b.dir == pickle_dir)
        .map(|b| b.name)
        .collect();
    assert_eq!(
        names,
        [
            "pickle",
            "pickle_dict",
            "pickle_list",
            "pickle_pure_python",
            "unpickle",
            "unpickle_list",
            "unpickle_pure_python",
        ]
    );
    assert_eq!(find("pickle").extra_opts, ["pickle"]);
    assert_eq!(find("unpickle_list").extra_opts, ["unpickle_list"]);
}

#[test]
fn directory_without_variants_yields_one_entry() {
    let chaos: Vec<Bench> = all()
        .into_iter()
        .filter(|b| b.dir.ends_with("bm_chaos"))
        .collect();
    assert_eq!(chaos.len(), 1);
    assert_eq!(chaos[0].name, "chaos");
    assert!(chaos[0].extra_opts.is_empty());
}

#[test]
fn names_are_unique_and_every_entry_has_a_script() {
    let benches = all();
    assert!(!benches.is_empty());
    let mut seen = BTreeSet::new();
    for bench in &benches {
        assert!(seen.insert(&bench.name), "duplicate name {}", bench.name);
        assert!(bench.script().is_file(), "{}", bench.script().display());
    }
}

/// The shim, `sys.path`, `sys.argv` and `__file__` wiring end to end: a plain
/// benchmark and a variant that only exists through `extra_opts`.
#[test]
fn vendored_benchmarks_run_once() {
    for name in ["nqueens", "pickle_dict"] {
        let bench = find(name);
        let source = fs::read_to_string(bench.script()).unwrap();
        interpreter_for(&bench).enter(|vm| run_once(vm, &bench, &source));
    }
}
