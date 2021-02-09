import browser
import zipfile
import asyncweb
import io
import _frozen_importlib as _bootstrap

_IS_SETUP = False
def setup(*, log=print):
    global _IS_SETUP, LOG_FUNC

    if not _IS_SETUP:
        import sys
        sys.meta_path.insert(0, WheelFinder)
        _IS_SETUP = True

    if not log:
        def LOG_FUNC(log):
            pass
    else:
        LOG_FUNC = log

async def load_package(*args):
    await asyncweb.wait_all(_load_package(pkg) for pkg in args)

_loaded_packages = {}

LOG_FUNC = print

async def _load_package(pkg):
    # TODO: support pkg==X.Y semver specifiers as well as arbitrary URLs
    info = await browser.fetch(f'https://pypi.org/pypi/{pkg}/json', response_format="json")
    name = info['info']['name']
    ver = info['info']['version']
    ver_downloads = info['releases'][ver]
    try:
        dl = next(dl for dl in ver_downloads if dl['packagetype'] == 'bdist_wheel')
    except StopIteration:
        raise ValueError(f"no wheel available for package {Name!r} {ver}")
    if name in _loaded_packages:
        return
    fname = dl['filename']
    LOG_FUNC(f"Downloading {fname} ({format_size(dl['size'])})...")
    zip_data = io.BytesIO(await browser.fetch(dl['url'], response_format="array_buffer"))
    size = len(zip_data.getbuffer())
    LOG_FUNC(f"{fname} done!")
    _loaded_packages[name] = zipfile.ZipFile(zip_data)

def format_size(bytes):
    # type: (float) -> str
    if bytes > 1000 * 1000:
        return '{:.1f} MB'.format(bytes / 1000.0 / 1000)
    elif bytes > 10 * 1000:
        return '{} kB'.format(int(bytes / 1000))
    elif bytes > 1000:
        return '{:.1f} kB'.format(bytes / 1000.0)
    else:
        return '{} bytes'.format(int(bytes))

class WheelFinder:
    _packages = _loaded_packages
    
    @classmethod
    def find_spec(cls, fullname, path=None, target=None):
        path = fullname.replace('.', '/')
        for zname, z in cls._packages.items():
            mi, fullpath = _get_module_info(z, path)
            if mi is not None:
                return _bootstrap.spec_from_loader(fullname, cls, origin=f'wheel:{zname}/{fullpath}', is_package=mi)
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
            raise ImportError('cannot find source for module', name=fullname)

    @classmethod
    def _get_source(cls, spec):
        origin = spec.origin
        if not origin or not origin.startswith("wheel:"):
            raise ImportError(f'{module.__spec__.name!r} is not a zip module')

        zipname, slash, path = origin[len('wheel:'):].partition('/')
        return cls._packages[zipname].read(path).decode()

    @classmethod
    def exec_module(cls, module):
        spec = module.__spec__
        source = cls._get_source(spec)
        code = _bootstrap._call_with_frames_removed(compile, source, spec.origin, 'exec', dont_inherit=True)
        _bootstrap._call_with_frames_removed(exec, code, module.__dict__)


_zip_searchorder = (
    # (path_sep + '__init__.pyc', True, True),
    ('/__init__.py', False, True),
    # ('.pyc', True, False),
    ('.py', False, False),
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
