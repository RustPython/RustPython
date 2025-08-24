import argparse
import logging
import os
import pathlib
import re
import sys


class CustomArgumentParser(argparse.ArgumentParser):
    class _CustomHelpFormatter(argparse.ArgumentDefaultsHelpFormatter):
        def _get_help_string(self, action):
            help_msg = super()._get_help_string(action)
            if action.dest != "help":
                env_name = f"{self._prog}_{action.dest}".upper()
                env_value = os.environ.get(env_name, "")
                help_msg += f" [env: {env_name}={env_value}]"
            return help_msg

    def __init__(self, *, formatter_class=_CustomHelpFormatter, **kwargs):
        super().__init__(formatter_class=formatter_class, **kwargs)

    def _add_action(self, action):
        action.default = os.environ.get(
            f"{self.prog}_{action.dest}".upper(), action.default
        )
        return super()._add_action(action)


def get_cache_dir(prog: str) -> pathlib.Path:
    home = pathlib.Path.home()

    if sys.platform.startswith("win"):
        local_appdata = pathlib.Path(
            os.getenv("LOCALAPPDATA", home / "AppData" / "Local")
        )
        path = local_appdata / prog / prog / "Cache"
    elif sys.platform == "darwin":
        path = home / "Library" / "Caches" / prog
    else:
        cache_home = pathlib.Path(os.getenv("XDG_CACHE_HOME", home / ".cache"))
        path = cache_home / prog

    return path


def build_argparse() -> argparse.ArgumentParser:
    parser = CustomArgumentParser(
        prog="rpau", description="Automatic update code from CPython"
    )

    # Cache
    cache_group = parser.add_argument_group("Cache options")

    cache_group.add_argument(
        "--cache",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="Whether reading from or writing to the cache is allowed",
    )
    cache_group.add_argument(
        "--cache-dir",
        default=get_cache_dir(parser.prog),
        help="Path to the cache directory",
        metavar="PATH",
        type=pathlib.Path,
    )

    # Output
    output_group = parser.add_argument_group("Output option")

    output_group.add_argument(
        "-o",
        "--output-dir",
        default=pathlib.Path(__file__).parents[4],
        help="Output dir",
        metavar="PATH",
        type=pathlib.Path,
    )

    # Filter
    filter_group = parser.add_argument_group("Filter options")

    filter_group.add_argument(
        "-i",
        "--include",
        default=".*",
        help="RE pattern used to include files and/or directories",
        metavar="PATTERN",
        type=re.compile,
    )
    filter_group.add_argument(
        "-e",
        "--exclude",
        default="^$",
        help="RE pattern used to omit files and/or directories",
        metavar="PATTERN",
        type=re.compile,
    )

    # Global
    global_group = parser.add_argument_group("Global options")

    global_group.add_argument(
        "--log-level",
        choices=logging.getLevelNamesMapping(),
        default="WARNING",
        help="Log level",
    )

    global_group.add_argument(
        "-j", "--workers", default=1, help="Number of processes", type=int
    )
    global_group.add_argument(
        "-c",
        "--conf-dir",
        default=pathlib.Path(__file__).parents[2] / "confs",
        help="Path to conf dir",
        metavar="PATH",
        type=pathlib.Path,
    )
    global_group.add_argument(
        "--base-upstream-url",
        default="https://raw.githubusercontent.com/python/cpython/refs/tags",
        help="Base upstream url of CPython",
        metavar="URL",
    )
    global_group.add_argument(
        "--default-version",
        default="v3.13.7",
        help="Fallback version of cpython if none specified in conf",
        metavar="VERSION_TAG",
    )

    return parser
