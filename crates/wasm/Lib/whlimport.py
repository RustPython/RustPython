import browser
import zipfile
import asyncweb
import io
import re
import posixpath
from urllib.parse import urlparse
import _frozen_importlib as _bootstrap
import _microdistlib

_IS_SETUP = False


def setup(*, log=print):
    global _IS_SETUP, LOG_FUNC

    if not _IS_SETUP:
        import sys

        sys.meta_path.insert(0, ZipFinder)
        _IS_SETUP = True

    if log:

        LOG_FUNC = log
    else:

        def LOG_FUNC(log):
            pass


async def load_package(*args):
    await asyncweb.wait_all(_load_package(pkg) for pkg in args)


_loaded_packages = {}

LOG_FUNC = print

_http_url = re.compile("^http[s]?://")


async def _load_package(pkg):
    if isinstance(pkg, str) and _http_url.match(pkg):
        urlobj = urlparse(pkg)
        fname = posixpath.basename(urlobj.path)
        name, url, size, deps = fname, pkg, None, []
    else:
        # TODO: load dependencies as well
        name, fname, url, size, deps = await _load_info_pypi(pkg)
    if name in _loaded_packages:
        return
    deps = asyncweb.spawn(asyncweb.wait_all(_load_package for dep in deps))
    size_str = format_size(size) if size is not None else "unknown size"
    LOG_FUNC(f"Downloading {fname} ({size_str})...")
    zip_data = io.BytesIO(await browser.fetch(url, response_format="array_buffer"))
    size = len(zip_data.getbuffer())
    LOG_FUNC(f"{fname} done!")
    _loaded_packages[name] = zipfile.ZipFile(zip_data)
    await deps


async def _load_info_pypi(pkg):
    pkg = _microdistlib.parse_requirement(pkg)
    # TODO: use VersionMatcher from distlib
    api_url = (
        f"https://pypi.org/pypi/{pkg.name}/json"
        if not pkg.constraints
        else f"https://pypi.org/pypi/{pkg.name}/{pkg.constraints[0][1]}/json"
    )
    info = await browser.fetch(api_url, response_format="json")
    name = info["info"]["name"]
    ver = info["info"]["version"]
    ver_downloads = info["releases"][ver]
    try:
        dl = next(dl for dl in ver_downloads if dl["packagetype"] == "bdist_wheel")
    except StopIteration:
        raise ValueError(f"no wheel available for package {name!r} {ver}")
    return (
        name,
        dl["filename"],
        dl["url"],
        dl["size"],
        info["info"]["requires_dist"] or [],
    )


def format_size(bytes):
    # type: (float) -> str
    if bytes > 1000 * 1000:
        return "{:.1f} MB".format(bytes / 1000.0 / 1000)
    elif bytes > 10 * 1000:
        return "{} kB".format(int(bytes / 1000))
    elif bytes > 1000:
        return "{:.1f} kB".format(bytes / 1000.0)
    else:
        return "{} bytes".format(int(bytes))


class ZipFinder:
    _packages = _loaded_packages

    @classmethod
    def find_spec(cls, fullname, path=None, target=None):
        path = fullname.replace(".", "/")
        for zname, z in cls._packages.items():
            mi, fullpath = _get_module_info(z, path)
            if mi is not None:
                return _bootstrap.spec_from_loader(
                    fullname, cls, origin=f"zip:{zname}/{fullpath}", is_package=mi
                )
        return None

    @classmethod
    def create_module(cls, spec):
        return None

    @classmethod
    def get_source(cls, fullname):
        spec = cls.find_spec(fullname)
        if spec:
            return cls._get_source(spec)
        else:
            raise ImportError("cannot find source for module", name=fullname)

    @classmethod
    def _get_source(cls, spec):
        origin = spec.origin and remove_prefix(spec.origin, "zip:")
        if not origin:
            raise ImportError(f"{spec.name!r} is not a zip module")

        zipname, slash, path = origin.partition("/")
        return cls._packages[zipname].read(path).decode()

    @classmethod
    def exec_module(cls, module):
        spec = module.__spec__
        source = cls._get_source(spec)
        code = _bootstrap._call_with_frames_removed(
            compile, source, spec.origin, "exec", dont_inherit=True
        )
        _bootstrap._call_with_frames_removed(exec, code, module.__dict__)


def remove_prefix(s, prefix):
    if s.startswith(prefix):
        return s[len(prefix) :]  # noqa: E203
    else:
        return None


_zip_searchorder = (
    ("/__init__.pyc", True, True),
    ("/__init__.py", False, True),
    (".pyc", True, False),
    (".py", False, False),
)


def _get_module_info(zf, path):
    for suffix, isbytecode, ispackage in _zip_searchorder:
        fullpath = path + suffix
        try:
            zf.getinfo(fullpath)
        except KeyError:
            continue
        return ispackage, fullpath
    return None, None
