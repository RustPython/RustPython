"""
A shim of the os module containing only simple path-related utilities
"""

try:
    from os import *
except ImportError:
    import abc, sys

    def __getattr__(name):
        if name in {"_path_normpath", "__path__"}:
            raise AttributeError(name)
        if name.isupper():
            return 0
        def dummy(*args, **kwargs):
            import io
            return io.UnsupportedOperation(f"{name}: no os specific module found")
        dummy.__name__ = f"dummy_{name}"
        return dummy

    sys.modules['os'] = sys.modules['posix'] = sys.modules[__name__]

    import posixpath as path
    sys.modules['os.path'] = path
    del sys

    sep = path.sep
    supports_dir_fd = set()
    supports_effective_ids = set()
    supports_fd = set()
    supports_follow_symlinks = set()


    def fspath(path):
        """Return the path representation of a path-like object.

        If str or bytes is passed in, it is returned unchanged. Otherwise the
        os.PathLike interface is used to get the path representation. If the
        path representation is not str or bytes, TypeError is raised. If the
        provided path is not str, bytes, or os.PathLike, TypeError is raised.
        """
        if isinstance(path, (str, bytes)):
            return path

        # Work from the object's type to match method resolution of other magic
        # methods.
        path_type = type(path)
        try:
            path_repr = path_type.__fspath__(path)
        except AttributeError:
            if hasattr(path_type, '__fspath__'):
                raise
            else:
                raise TypeError("expected str, bytes or os.PathLike object, "
                                "not " + path_type.__name__)
        if isinstance(path_repr, (str, bytes)):
            return path_repr
        else:
            raise TypeError("expected {}.__fspath__() to return str or bytes, "
                            "not {}".format(path_type.__name__,
                                            type(path_repr).__name__))

    class PathLike(abc.ABC):

        """Abstract base class for implementing the file system path protocol."""

        @abc.abstractmethod
        def __fspath__(self):
            """Return the file system path representation of the object."""
            raise NotImplementedError

        @classmethod
        def __subclasshook__(cls, subclass):
            return hasattr(subclass, '__fspath__')
