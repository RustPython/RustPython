# Modified from code from the PyPy project:
# https://bitbucket.org/pypy/pypy/src/default/pypy/objspace/std/objectobject.py

# The MIT License

# Permission is hereby granted, free of charge, to any person
# obtaining a copy of this software and associated documentation
# files (the "Software"), to deal in the Software without
# restriction, including without limitation the rights to use,
# copy, modify, merge, publish, distribute, sublicense, and/or
# sell copies of the Software, and to permit persons to whom the
# Software is furnished to do so, subject to the following conditions:

# The above copyright notice and this permission notice shall be included
# in all copies or substantial portions of the Software.

# THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
# OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
# FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL
# THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
# LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
# FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
# DEALINGS IN THE SOFTWARE.

import copyreg


def _abstract_method_error(typ):
    methods = ", ".join(sorted(typ.__abstractmethods__))
    err = "Can't instantiate abstract class %s with abstract methods %s"
    raise TypeError(err % (typ.__name__, methods))


def reduce_2(obj):
    cls = obj.__class__

    try:
        getnewargs = obj.__getnewargs__
    except AttributeError:
        args = ()
    else:
        args = getnewargs()
        if not isinstance(args, tuple):
            raise TypeError("__getnewargs__ should return a tuple")

    try:
        getstate = obj.__getstate__
    except AttributeError:
        state = getattr(obj, "__dict__", None)
        names = slotnames(cls)  # not checking for list
        if names is not None:
            slots = {}
            for name in names:
                try:
                    value = getattr(obj, name)
                except AttributeError:
                    pass
                else:
                    slots[name] = value
            if slots:
                state = state, slots
    else:
        state = getstate()

    listitems = iter(obj) if isinstance(obj, list) else None
    dictitems = iter(obj.items()) if isinstance(obj, dict) else None

    newobj = copyreg.__newobj__

    args2 = (cls,) + args
    return newobj, args2, state, listitems, dictitems


def slotnames(cls):
    if not isinstance(cls, type):
        return None

    try:
        return cls.__dict__["__slotnames__"]
    except KeyError:
        pass

    slotnames = copyreg._slotnames(cls)
    if not isinstance(slotnames, list) and slotnames is not None:
        raise TypeError("copyreg._slotnames didn't return a list or None")
    return slotnames
