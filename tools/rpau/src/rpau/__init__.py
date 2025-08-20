import functools
from concurrent.futures import ProcessPoolExecutor
from typing import TYPE_CHECKING

import tomllib

from rpau.cli import build_argparse
from rpau.logger import build_root_logger, get_logger
from rpau.logic import run

if TYPE_CHECKING:
    import pathlib
    import re
    from collections.abc import Iterator


def iter_confs(
    conf_dir: "pathlib.Path", *, include: "re.Pattern", exclude: "re.Pattern"
) -> "Iterator[pathlib.Path]":
    for conf_file in conf_dir.rglob("**/*.toml"):
        if not conf_file.is_file():
            continue

        uri = conf_file.as_uri().removeprefix("file://")
        if not include.match(uri):
            continue

        if exclude.match(uri):
            continue

        yield conf_file


def main() -> None:
    parser = build_argparse()
    args = parser.parse_args()

    logger = build_root_logger(level=args.log_level)
    logger.debug(f"{args=}")

    if args.cache:
        logger.debug(f"Ensuring {args.cache_dir} exists")
        cache_dir = args.cache_dir
        cache_dir.mkdir(parents=True, exist_ok=True)
    else:
        cache_dir = None

    conf_dir = args.conf_dir
    with ProcessPoolExecutor(args.workers) as executor:
        for conf_file in iter_confs(
            conf_dir, include=args.include, exclude=args.exclude
        ):
            try:
                conf = tomllib.loads(conf_file.read_text())
            except tomllib.TOMLDecodeError as err:
                logger.warn(f"{conf_file}: {err}")
                continue

            try:
                path = conf.pop("path")
            except KeyError:
                logger.warn(f"{conf_file}: has no 'path' key. skipping")
                continue

            version = conf.pop("version", args.default_version)
            func = functools.partial(
                run,
                path=path,
                conf=conf,
                cache_dir=cache_dir,
                version=version,
                output_dir=args.output_dir,
                base_upstream_url=args.base_upstream_url,
            )
            executor.submit(func)
