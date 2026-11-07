# pyperformance benchmarks

<!-- cspell:ignore pyperformance pyperf -->

The `bm_*` directories here are copied **unmodified** from
[python/pyperformance 1.14.0](https://github.com/python/pyperformance/tree/1.14.0/pyperformance/data-files/benchmarks)
by `scripts/vendor-pyperformance.py`. Do not edit them by hand: re-run the
script to update them, and change the script's `BENCHMARKS` / `EXCLUDED_FILES`
to add or drop one. CI runs the script with `--check` and fails when this tree
differs from a fresh vendoring.

Everything else in this directory is ours:

- `pyperf/` is a stand-in for the `pyperf` package the benchmarks import. It
  runs each benchmark body once and does no timing (see its docstring).
- `support.rs` is shared by the Criterion target `benches/pyperformance.rs` and
  the tests in `tests/pyperformance.rs`.
- `test_pyperf_shim.py` tests the shim under CPython and RustPython:

  ```shell
  python3 -m unittest benches/pyperformance/test_pyperf_shim.py
  target/release/rustpython -m unittest benches/pyperformance/test_pyperf_shim.py
  ```

## How a benchmark is run

`benches/pyperformance.rs` builds one interpreter per benchmark with the
benchmark's directory and this directory on `sys.path`, puts the variant's
`extra_opts` (from `pyproject.toml` or a `bm_<variant>.toml` beside it) into
`sys.argv`, and executes `run_benchmark.py` as `__main__`. The shim's
`Runner.bench_func` / `bench_time_func` / `bench_async_func` call the benchmark
function once (`bench_time_func` with `loops=1`), so one Criterion iteration is
one pyperf loop. Imports stay cached across iterations; compiling the script is
part of each iteration, as in `benches/execution.rs`.

Run them locally, once each (the smoke test) or measured:

```shell
cargo bench --bench pyperformance -- --test
cargo bench --bench pyperformance
cargo bench --bench pyperformance -- pickle_dict
```

## Which benchmarks are included

Every directory of pyperformance 1.14.0 is accounted for below. A benchmark is
vendored when it needs nothing beyond the standard library, runs to completion
under RustPython, and one iteration takes at most 10 seconds on a development
machine: the CodSpeed Macro runner repeats each benchmark many times, so a slow
single iteration multiplies.

### Vendored (43 directories, 65 benchmarks)

argparse (`argparse_many_optionals`, `argparse_subparsers`), async_generators,
async_tree (and its 15 `async_tree_*` variants), asyncio_tcp (without the
`asyncio_tcp_ssl` variant, see below), barnes_hut, base64, chaos, comprehensions,
coroutines, deepcopy, deltablue, fannkuch, float, gc_collect,
gc_traversal, generators, go, hexiom, json_dumps, json_loads, logging, mdp,
meteor_contest, nbody, nqueens, pathlib, pickle (`pickle`, `pickle_dict`,
`pickle_list`, `pickle_pure_python`, `unpickle`, `unpickle_list`,
`unpickle_pure_python`), pidigits, pprint, pyflate, raytrace, regex_compile,
regex_dna, regex_effbot, regex_v8, richards, richards_super, scimark,
spectral_norm, telco, typing_runtime_protocols, unpack_sequence, xml_etree.

### Not vendored

| Benchmark | Reason |
|---|---|
| `2to3`, `python_startup` (+ `python_startup_no_site`) | Measure interpreter start-up through `Runner.bench_command`; not meaningful in-process. |
| `asyncio_websockets` | Needs `websockets`. |
| `chameleon` | Needs `Chameleon`. |
| `coverage` | Needs `coverage` (C tracer). |
| `crypto_pyaes` | Needs `pyaes`. |
| `dask` | Needs `dask[distributed]`. |
| `django_template` | Needs `django`. |
| `docutils` | Needs `docutils`. |
| `dulwich_log` | Needs `dulwich`. |
| `fastapi` | Needs `fastapi`, `uvicorn`, `httpx`. |
| `genshi` | Needs `Genshi`. |
| `hg_startup` | Needs `mercurial`; a start-up benchmark. |
| `html5lib` | Needs `html5lib`. |
| `mako` | Needs `Mako`. |
| `networkx` (+ 2 variants) | Needs `networkx`. |
| `sphinx` | Needs `sphinx`. |
| `sqlalchemy_declarative`, `sqlalchemy_imperative` | Need `SQLAlchemy` and `greenlet` (C extension). |
| `sqlglot_v2` (+ 3 variants) | Needs `sqlglot`. |
| `sympy` | Needs `sympy`. |
| `tomli_loads` | Needs `tomli`. |
| `tornado_http` | Needs `tornado`. |
| `xdsl` | Needs `xdsl`. |
| `yaml` | Needs `PyYAML`. |
| `concurrent_imap` | Uses `multiprocessing.Pool`, which starts child interpreters through `sys.executable` and `sys.modules["__main__"]`; neither exists when the script runs inside the benchmark executable. |
| `sqlite_synth` | `ModuleNotFoundError: _sqlite3` in the default build. |
| `decimal_factorial` | `MemoryError` under RustPython. |
| `decimal_pi` | 24 s per iteration. |
| `bpe_tokeniser` | 13 s per iteration. |
| `asyncio_tcp_ssl` (variant of `asyncio_tcp`) | 46 s per iteration. Its `.toml` and the certificate and key only it uses are deleted after vendoring (`EXCLUDED_FILES`). |

The three "per iteration" figures were measured with a release build on an
Apple M-series laptop and include about 0.4 s of interpreter start-up.
