from browser import window as Window # type: ignore
from .util import jsint, jsstr, UNDEFINED

__all__ = [
    "Window",
    "alert",
    "atob",
    "btoa",
    "cancel_animation_frame",
    "close",
    "confirm",
    "fetch",
    "focus",
    "print",
    "prompt",
    "request_animation_frame",
    "resize_by",
    "resize_to",
]

_alert = Window.get_prop("alert")

def alert(msg = None):
    if msg is None:
        return _alert.call()
    if type(msg) != str:
        raise TypeError("msg must be a string")
    _alert.call(jsstr(msg))

_atob = Window.get_prop("atob")

def atob(data):
    if type(data) != str:
        raise TypeError("data must be a string")
    return _atob.call(jsstr(data)).as_str()

_btoa = Window.get_prop("btoa")
def btoa(data):
    if type(data) != str:
        raise TypeError("data must be a string")
    return _btoa.call(jsstr(data)).as_str()


from _browser import cancel_animation_frame

_close = Window.get_prop("close")
def close():
    return _close.call()

_confirm = Window.get_prop("confirm")
def confirm(msg):
    if type(msg) != str:
        raise TypeError("msg must be a string")
    return _confirm.call(jsstr(msg)).as_bool()

from _browser import fetch

_focus = Window.get_prop("focus")
def focus():
    return _focus.call()

_print = Window.get_prop("print")
def print():
    return _print.call()

_prompt = Window.get_prop("prompt")
def prompt(msg, default_val=None):
    if type(msg) != str:
        raise TypeError("msg must be a string")
    if default_val is not None and type(default_val) != str:
        raise TypeError("default_val must be a string")

    return _prompt.call(
        jsstr(msg), jsstr(default_val) if default_val else UNDEFINED
    ).as_str()

from _browser import request_animation_frame

_resize_by = Window.get_prop("resizeBy")

def resize_by(x, y):
    if type(x) != int:
        raise TypeError("x must be an int")
    if type(y) != int:
        raise TypeError("y must be an int")
    _resize_by.call(jsint(x), jsint(y))

_resize_to = Window.get_prop("resizeTo")

def resize_to(x, y):
    if type(x) != int:
        raise TypeError("x must be an int")
    if type(y) != int:
        raise TypeError("y must be an int")
    _resize_to.call(jsint(x), jsint(y))

_stop = Window.get_prop("stop")
def stop():
    return _stop.call()
