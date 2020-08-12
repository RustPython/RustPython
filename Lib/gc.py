#!/usr/bin/env python  # [built-in module gc]
"""
This module provides access to the garbage collector for reference cycles.

enable() -- Enable automatic garbage collection.
disable() -- Disable automatic garbage collection.
isenabled() -- Returns true if automatic collection is enabled.
collect() -- Do a full collection right now.
get_count() -- Return the current collection counts.
get_stats() -- Return list of dictionaries containing per-generation stats.
set_debug() -- Set debugging flags.
get_debug() -- Get debugging flags.
set_threshold() -- Set the collection thresholds.
get_threshold() -- Return the current the collection thresholds.
get_objects() -- Return a list of all objects tracked by the collector.
is_tracked() -- Returns true if a given object is tracked.
get_referrers() -- Return the list of objects that refer to an object.
get_referents() -- Return the list of objects that an object refers to.
"""

## DATA ##

DEBUG_COLLECTABLE = 2
# None
DEBUG_LEAK = 38
# None
DEBUG_SAVEALL = 32
# None
DEBUG_STATS = 1
# None
DEBUG_UNCOLLECTABLE = 4
# None
callbacks = []
# None
garbage = []
# None

## FUNCTIONS ##


def collect(*args, **kwargs):  # unknown args #
    """
    collect([generation]) -> n

    With no arguments, run a full collection.  The optional argument
    may be an integer specifying which generation to collect.  A ValueError
    is raised if the generation number is invalid.

    The number of unreachable objects is returned.
    """
    return 0


def disable(*args, **kwargs):  # unknown args #
    """
    disable() -> None

    Disable automatic garbage collection.
    """
    raise NotImplementedError()


def enable(*args, **kwargs):  # unknown args #
    """
    enable() -> None

    Enable automatic garbage collection.
    """
    raise NotImplementedError()


def get_count(*args, **kwargs):  # unknown args #
    """
    get_count() -> (count0, count1, count2)

    Return the current collection counts
    """
    raise NotImplementedError()


def get_debug(*args, **kwargs):  # unknown args #
    """
    get_debug() -> flags

    Get the garbage collection debugging flags.
    """
    raise NotImplementedError()


def get_objects(*args, **kwargs):  # unknown args #
    """
    get_objects() -> [...]

    Return a list of objects tracked by the collector (excluding the list
    returned).
    """
    raise NotImplementedError()


def get_referents(*args, **kwargs):  # unknown args #
    """
    get_referents(*objs) -> list
    Return the list of objects that are directly referred to by objs.
    """
    raise NotImplementedError()


def get_referrers(*args, **kwargs):  # unknown args #
    """
    get_referrers(*objs) -> list
    Return the list of objects that directly refer to any of objs.
    """
    raise NotImplementedError()


def get_stats(*args, **kwargs):  # unknown args #
    """
    get_stats() -> [...]

    Return a list of dictionaries containing per-generation statistics.
    """
    raise NotImplementedError()


def get_threshold(*args, **kwargs):  # unknown args #
    """
    get_threshold() -> (threshold0, threshold1, threshold2)

    Return the current collection thresholds
    """
    raise NotImplementedError()


def is_tracked(*args, **kwargs):  # unknown args #
    """
    is_tracked(obj) -> bool

    Returns true if the object is tracked by the garbage collector.
    Simple atomic objects will return false.
    """
    raise NotImplementedError()


def isenabled(*args, **kwargs):  # unknown args #
    """
    isenabled() -> status

    Returns true if automatic garbage collection is enabled.
    """
    raise NotImplementedError()


def set_debug(*args, **kwargs):  # unknown args #
    """
    set_debug(flags) -> None

    Set the garbage collection debugging flags. Debugging information is
    written to sys.stderr.

    flags is an integer and can have the following bits turned on:

      DEBUG_STATS - Print statistics during collection.
      DEBUG_COLLECTABLE - Print collectable objects found.
      DEBUG_UNCOLLECTABLE - Print unreachable but uncollectable objects found.
      DEBUG_SAVEALL - Save objects to gc.garbage rather than freeing them.
      DEBUG_LEAK - Debug leaking programs (everything but STATS).
    """
    raise NotImplementedError()


def set_threshold(*args, **kwargs):  # unknown args #
    """
    set_threshold(threshold0, [threshold1, threshold2]) -> None

    Sets the collection thresholds.  Setting threshold0 to zero disables
    collection.
    """
    raise NotImplementedError()
