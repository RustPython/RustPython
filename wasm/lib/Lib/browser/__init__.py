from _browser import (
    Document,
    Element,
    load_module,
)

from _js import JSValue, Promise
from .window import alert, atob, btoa, confirm, prompt, request_animation_frame, cancel_animation_frame

from .util import jsstr, jsclosure, jsclosure_once, jsfloat, NULL, UNDEFINED

__all__ = [
    "jsstr",
    "jsclosure",
    "jsclosure_once",
    "jsfloat",
    "NULL",
    "UNDEFINED",
    "alert",
    "atob",
    "btoa",
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


