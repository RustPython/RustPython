#!/usr/bin/env python3
"""Copy benchmark programs from a python/pyperformance release into
benches/pyperformance/.

The Criterion target ``benches/pyperformance.rs`` runs the upstream
``bm_<name>/run_benchmark.py`` files unmodified inside a RustPython
interpreter. This script is the only way those files should change: it
downloads the pinned pyperformance release archive, verifies its SHA-256,
and copies the requested ``bm_*`` directories verbatim. Directories under
the destination that are not in the list are removed, so the checked-in
tree always equals "this script's defaults, applied to this tag".

    python3 scripts/vendor-pyperformance.py            # re-vendor the default set
    python3 scripts/vendor-pyperformance.py --check    # CI: fail if the tree differs
    python3 scripts/vendor-pyperformance.py --tag 1.15.0 chaos go nbody

Which benchmarks are vendored, and why the others are not, is documented in
benches/pyperformance/README.md.
"""

from __future__ import annotations

import argparse
import filecmp
import hashlib
import shutil
import sys
import tarfile
import tempfile
import urllib.request
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_DEST = REPO_ROOT / "benches" / "pyperformance"
DEFAULT_CACHE = REPO_ROOT / "target" / "pyperformance-vendor"
DEFAULT_TAG = "1.14.0"

# SHA-256 of https://github.com/python/pyperformance/archive/refs/tags/<tag>.tar.gz
ARCHIVE_SHA256 = {
    "1.14.0": "2eb90812292522297bb046e589e0876c00ec5b5f49e8653eca39f8fd942840ed",
}

# Directory names (without the ``bm_`` prefix) that are vendored by default.
# Keep this sorted; README.md explains every upstream directory that is absent.
BENCHMARKS = (
    "argparse",
    "async_generators",
    "async_tree",
    "asyncio_tcp",
    "barnes_hut",
    "base64",
    "chaos",
    "comprehensions",
    "coroutines",
    "deepcopy",
    "deltablue",
    "fannkuch",
    "float",
    "gc_collect",
    "gc_traversal",
    "generators",
    "go",
    "hexiom",
    "json_dumps",
    "json_loads",
    "logging",
    "mdp",
    "meteor_contest",
    "nbody",
    "nqueens",
    "pathlib",
    "pickle",
    "pidigits",
    "pprint",
    "pyflate",
    "raytrace",
    "regex_compile",
    "regex_dna",
    "regex_effbot",
    "regex_v8",
    "richards",
    "richards_super",
    "scimark",
    "spectral_norm",
    "telco",
    "typing_runtime_protocols",
    "unpack_sequence",
    "xml_etree",
)

# Files removed after copying. README.md says why each benchmark is not run;
# its directory is still wanted for the other benchmarks it holds.
EXCLUDED_FILES = frozenset(
    {
        # The asyncio_tcp_ssl variant, and the certificate and key only it uses.
        "bm_asyncio_tcp/bm_asyncio_tcp_ssl.toml",
        "bm_asyncio_tcp/ssl_cert.pem",
        "bm_asyncio_tcp/ssl_key.pem",
    }
)

IGNORED_NAMES = {"__pycache__"}


def log(msg: str) -> None:
    print(f"[vendor-pyperformance] {msg}", file=sys.stderr, flush=True)


def fetch_archive(tag: str, cache_dir: Path) -> Path:
    try:
        expected = ARCHIVE_SHA256[tag]
    except KeyError:
        raise SystemExit(
            f"no SHA-256 recorded for pyperformance {tag}; add it to ARCHIVE_SHA256"
        )
    cache_dir.mkdir(parents=True, exist_ok=True)
    archive = cache_dir / f"pyperformance-{tag}.tar.gz"
    if not archive.exists():
        url = f"https://github.com/python/pyperformance/archive/refs/tags/{tag}.tar.gz"
        log(f"downloading {url}")
        with urllib.request.urlopen(url) as response:
            archive.write_bytes(response.read())
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    if digest != expected:
        archive.unlink()
        raise SystemExit(f"{archive}: SHA-256 {digest} does not match {expected}")
    return archive


def extract_benchmarks(archive: Path, tag: str, into: Path) -> Path:
    """Extract the archive and return its ``data-files/benchmarks`` directory."""
    with tarfile.open(archive) as tar:
        tar.extractall(into, filter="data")
    root = into / f"pyperformance-{tag}" / "pyperformance" / "data-files" / "benchmarks"
    if not root.is_dir():
        raise SystemExit(f"{archive} has no {root.relative_to(into)}")
    return root


def vendor(source_root: Path, names: list[str], dest: Path) -> None:
    dest.mkdir(parents=True, exist_ok=True)
    wanted = {f"bm_{name}" for name in names}
    for existing in dest.glob("bm_*"):
        if existing.is_dir() and existing.name not in wanted:
            log(f"removing {existing.relative_to(REPO_ROOT)}")
            shutil.rmtree(existing)
    for name in names:
        src = source_root / f"bm_{name}"
        if not src.is_dir():
            raise SystemExit(f"pyperformance has no benchmark directory bm_{name}")
        target = dest / src.name
        if target.exists():
            shutil.rmtree(target)
        shutil.copytree(src, target, ignore=shutil.ignore_patterns(*IGNORED_NAMES))
    for relative in sorted(EXCLUDED_FILES):
        variant = dest / relative
        if variant.exists():
            variant.unlink()


def differences(left: Path, right: Path) -> list[str]:
    """Paths that differ between two vendored trees, considering ``bm_*`` only."""
    found: list[str] = []

    def walk(cmp: filecmp.dircmp, prefix: str) -> None:
        for name in cmp.left_only:
            found.append(f"only in checked-in tree: {prefix}{name}")
        for name in cmp.right_only:
            found.append(f"missing from checked-in tree: {prefix}{name}")
        for name in cmp.diff_files + cmp.funny_files:
            found.append(f"differs: {prefix}{name}")
        for name, sub in cmp.subdirs.items():
            walk(sub, f"{prefix}{name}/")

    left_dirs = sorted(p.name for p in left.glob("bm_*") if p.is_dir())
    right_dirs = sorted(p.name for p in right.glob("bm_*") if p.is_dir())
    for name in left_dirs:
        if name not in right_dirs:
            found.append(f"only in checked-in tree: {name}")
    for name in right_dirs:
        if name not in left_dirs:
            found.append(f"missing from checked-in tree: {name}")
    for name in sorted(set(left_dirs) & set(right_dirs)):
        walk(
            filecmp.dircmp(left / name, right / name, ignore=list(IGNORED_NAMES)),
            f"{name}/",
        )
    return found


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("names", nargs="*", default=list(BENCHMARKS))
    parser.add_argument("--tag", default=DEFAULT_TAG)
    parser.add_argument("--dest", type=Path, default=DEFAULT_DEST)
    parser.add_argument("--cache-dir", type=Path, default=DEFAULT_CACHE)
    parser.add_argument(
        "--check",
        action="store_true",
        help="do not write; exit 1 if --dest differs from a fresh vendoring",
    )
    args = parser.parse_args(argv)

    archive = fetch_archive(args.tag, args.cache_dir)
    with tempfile.TemporaryDirectory(dir=args.cache_dir) as tmp:
        source_root = extract_benchmarks(archive, args.tag, Path(tmp))
        if args.check:
            fresh = Path(tmp) / "fresh"
            vendor(source_root, args.names, fresh)
            diffs = differences(args.dest, fresh)
            for line in diffs:
                print(line)
            if diffs:
                log(f"{args.dest} is out of date; run {Path(__file__).name}")
                return 1
            log(f"{args.dest} matches pyperformance {args.tag}")
            return 0
        vendor(source_root, args.names, args.dest)
    log(
        f"vendored {len(args.names)} benchmark directories from pyperformance {args.tag}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
