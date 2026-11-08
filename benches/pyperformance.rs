//! Run the upstream pyperformance benchmark programs vendored under
//! `benches/pyperformance/` inside a RustPython interpreter.
//!
//! The `bm_*/run_benchmark.py` files are byte-for-byte copies of
//! python/pyperformance (see `scripts/vendor-pyperformance.py`). Their
//! `import pyperf` resolves to the shim in `benches/pyperformance/pyperf/`,
//! which runs each benchmark body once and does no timing of its own; this
//! harness repeats the whole script and Criterion (or CodSpeed, through the
//! `codspeed-criterion-compat` alias) measures it.
//!
//! This target deliberately does not link CPython through `pyo3`, so the built
//! binary can be moved from the machine that compiled it to the CodSpeed Macro
//! runner that executes it.

// cspell:ignore pyperformance pyperf

#[path = "pyperformance/support.rs"]
mod support;

use criterion::{Bencher, Criterion, criterion_group, criterion_main};
use std::fs;
use std::path::Path;
use support::{BENCHMARKS_DIR, Bench, discover, interpreter_for, run_once};

fn bench_in_rustpython(b: &mut Bencher<'_>, bench: &Bench) {
    let script = bench.script();
    let source =
        fs::read_to_string(&script).unwrap_or_else(|e| panic!("{}: {e}", script.display()));
    // One interpreter per benchmark: module imports stay cached across
    // iterations, so what is measured is the benchmark body plus compiling
    // the script, as `benches/execution.rs` measures too.
    interpreter_for(bench).enter(|vm| b.iter(|| run_once(vm, bench, &source)))
}

fn criterion_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("pyperformance");
    for bench in discover(Path::new(BENCHMARKS_DIR)) {
        group.bench_function(&bench.name, |b| bench_in_rustpython(b, &bench));
    }
    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
