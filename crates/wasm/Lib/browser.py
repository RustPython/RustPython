from _browser import (
    fetch,
    request_animation_frame,
    cancel_animation_frame,
    Document,
    Element,
    load_module,
)

from _js import JSValue, Promise
from _window import window

__all__ = [
    "jsstr",
    "jsclosure",
    "jsclosure_once",
    "jsfloat",
    "NULL",
    "UNDEFINED",
    "alert",
    "confirm",
    "prompt",
    "fetch",
    "request_animation_frame",
    "cancel_animation_frame",
    "Document",
    "Element",
    "load_module",
    "JSValue",
    "Promise",
]


jsstr = window.new_from_str
jsclosure = window.new_closure
jsclosure_once = window.new_closure_once
_jsfloat = window.new_from_float

UNDEFINED = window.undefined()
NULL = window.null()


def jsfloat(n):
    return _jsfloat(float(n))


_alert = window.get_prop("alert")


def alert(msg):
    if type(msg) != str:
        raise TypeError("msg must be a string")
    _alert.call(jsstr(msg))


_confirm = window.get_prop("confirm")


def confirm(msg):
    if type(msg) != str:
        raise TypeError("msg must be a string")
    return _confirm.call(jsstr(msg)).as_bool()


_prompt = window.get_prop("prompt")


def prompt(msg, default_val=None):
    if type(msg) != str:
        raise TypeError("msg must be a string")
    if default_val is not None and type(default_val) != str:
        raise TypeError("default_val must be a string")

    return _prompt.call(
        jsstr(msg), jsstr(default_val) if default_val else UNDEFINED
    ).as_str()
