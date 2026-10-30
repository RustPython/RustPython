"""Drop-in replacement for the thread module.

Meant to be used as a brain-dead substitute so that threaded code does
not need to be rewritten for when the thread module is not present.

Suggested usage is::

    try:
        import _thread
    except ImportError:
        import _dummy_thread as _thread

"""

# Exports only things specified by thread documentation;
# skipping obsolete synonyms allocate(), start_new(), exit_thread().
__all__ = [
    "error",
    "start_new_thread",
    "exit",
    "get_ident",
    "allocate_lock",
    "interrupt_main",
    "LockType",
    "RLock",
    "_count",
    "start_joinable_thread",
    "daemon_threads_allowed",
    "_shutdown",
    "_make_thread_handle",
    "_ThreadHandle",
    "_get_main_thread_ident",
    "_is_main_interpreter",
    "_local",
]

# A dummy value
TIMEOUT_MAX = 2**31

# Main thread ident for dummy implementation
_MAIN_THREAD_IDENT = -1

# NOTE: this module can be imported early in the extension building process,
# and so top level imports of other modules should be avoided.  Instead, all
# imports are done when needed on a function-by-function basis.  Since threads
# are disabled, the import lock should not be an issue anyway (??).

error = RuntimeError


def start_new_thread(function, args, kwargs={}):
    """Dummy implementation of _thread.start_new_thread().

    Compatibility is maintained by making sure that ``args`` is a
    tuple and ``kwargs`` is a dictionary.  If an exception is raised
    and it is SystemExit (which can be done by _thread.exit()) it is
    caught and nothing is done; all other exceptions are printed out
    by using traceback.print_exc().

    If the executed function calls interrupt_main the KeyboardInterrupt will be
    raised when the function returns.

    """
    if type(args) != type(tuple()):
        raise TypeError("2nd arg must be a tuple")
    if type(kwargs) != type(dict()):
        raise TypeError("3rd arg must be a dict")
    global _main
    _main = False
    try:
        function(*args, **kwargs)
    except SystemExit:
        pass
    except:
        import traceback

        traceback.print_exc()
    _main = True
    global _interrupt
    if _interrupt:
        _interrupt = False
        raise KeyboardInterrupt


def start_joinable_thread(function, handle=None, daemon=True):
    """Dummy implementation of _thread.start_joinable_thread().

    In dummy thread, we just run the function synchronously.
    """
    if handle is None:
        handle = _ThreadHandle()
    try:
        function()
    except SystemExit:
        pass
    except:
        import traceback

        traceback.print_exc()
    handle._set_done()
    return handle


def daemon_threads_allowed():
    """Dummy implementation of _thread.daemon_threads_allowed()."""
    return True


def _shutdown():
    """Dummy implementation of _thread._shutdown()."""
    pass


def _make_thread_handle(ident):
    """Dummy implementation of _thread._make_thread_handle()."""
    handle = _ThreadHandle()
    handle._ident = ident
    return handle


def _get_main_thread_ident():
    """Dummy implementation of _thread._get_main_thread_ident()."""
    return _MAIN_THREAD_IDENT


def _is_main_interpreter():
    """Dummy implementation of _thread._is_main_interpreter()."""
    return True


def exit():
    """Dummy implementation of _thread.exit()."""
    raise SystemExit


def get_ident():
    """Dummy implementation of _thread.get_ident().

    Since this module should only be used when _threadmodule is not
    available, it is safe to assume that the current process is the
    only thread.  Thus a constant can be safely returned.
    """
    return _MAIN_THREAD_IDENT


def allocate_lock():
    """Dummy implementation of _thread.allocate_lock()."""
    return LockType()


def stack_size(size=None):
    """Dummy implementation of _thread.stack_size()."""
    if size is not None:
        raise error("setting thread stack size not supported")
    return 0


def _set_sentinel():
    """Dummy implementation of _thread._set_sentinel()."""
    return LockType()


def _count():
    """Dummy implementation of _thread._count()."""
    return 0


class LockType(object):
    """Class implementing dummy implementation of _thread.LockType.

    Compatibility is maintained by maintaining self.locked_status
    which is a boolean that stores the state of the lock.  Pickling of
    the lock, though, should not be done since if the _thread module is
    then used with an unpickled ``lock()`` from here problems could
    occur from this class not having atomic methods.

    """

    def __init__(self):
        self.locked_status = False

    def acquire(self, waitflag=None, timeout=-1):
        """Dummy implementation of acquire().

        For blocking calls, self.locked_status is automatically set to
        True and returned appropriately based on value of
        ``waitflag``.  If it is non-blocking, then the value is
        actually checked and not set if it is already acquired.  This
        is all done so that threading.Condition's assert statements
        aren't triggered and throw a little fit.

        """
        if waitflag is None or waitflag:
            self.locked_status = True
            return True
        else:
            if not self.locked_status:
                self.locked_status = True
                return True
            else:
                if timeout > 0:
                    import time

                    time.sleep(timeout)
                return False

    __enter__ = acquire

    def __exit__(self, typ, val, tb):
        self.release()

    def release(self):
        """Release the dummy lock."""
        # XXX Perhaps shouldn't actually bother to test?  Could lead
        #     to problems for complex, threaded code.
        if not self.locked_status:
            raise error
        self.locked_status = False
        return True

    def locked(self):
        return self.locked_status

    def _at_fork_reinit(self):
        self.locked_status = False

    def __repr__(self):
        return "<%s %s.%s object at %s>" % (
            "locked" if self.locked_status else "unlocked",
            self.__class__.__module__,
            self.__class__.__qualname__,
            hex(id(self)),
        )


class _ThreadHandle:
    """Dummy implementation of _thread._ThreadHandle."""

    def __init__(self):
        self._ident = _MAIN_THREAD_IDENT
        self._done = False

    @property
    def ident(self):
        return self._ident

    def _set_done(self):
        self._done = True

    def is_done(self):
        return self._done

    def join(self, timeout=None):
        # In dummy thread, thread is always done
        return

    def __repr__(self):
        return f"<_ThreadHandle ident={self._ident}>"


# Used to signal that interrupt_main was called in a "thread"
_interrupt = False
# True when not executing in a "thread"
_main = True


def interrupt_main():
    """Set _interrupt flag to True to have start_new_thread raise
    KeyboardInterrupt upon exiting."""
    if _main:
        raise KeyboardInterrupt
    else:
        global _interrupt
        _interrupt = True


class RLock:
    def __init__(self):
        self.locked_count = 0

    def acquire(self, waitflag=None, timeout=-1):
        self.locked_count += 1
        return True

    __enter__ = acquire

    def __exit__(self, typ, val, tb):
        self.release()

    def release(self):
        if not self.locked_count:
            raise error
        self.locked_count -= 1
        return True

    def locked(self):
        return self.locked_count != 0

    def __repr__(self):
        return "<%s %s.%s object owner=%s count=%s at %s>" % (
            "locked" if self.locked_count else "unlocked",
            self.__class__.__module__,
            self.__class__.__qualname__,
            get_ident() if self.locked_count else 0,
            self.locked_count,
            hex(id(self)),
        )


class _local:
    """Dummy implementation of _thread._local (thread-local storage)."""

    def __init__(self):
        object.__setattr__(self, "_local__impl", {})

    def __getattribute__(self, name):
        if name.startswith("_local__"):
            return object.__getattribute__(self, name)
        impl = object.__getattribute__(self, "_local__impl")
        try:
            return impl[name]
        except KeyError:
            raise AttributeError(name)

    def __setattr__(self, name, value):
        if name.startswith("_local__"):
            return object.__setattr__(self, name, value)
        impl = object.__getattribute__(self, "_local__impl")
        impl[name] = value

    def __delattr__(self, name):
        if name.startswith("_local__"):
            return object.__delattr__(self, name)
        impl = object.__getattribute__(self, "_local__impl")
        try:
            del impl[name]
        except KeyError:
            raise AttributeError(name)
