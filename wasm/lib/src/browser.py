from _browser import *

from _js import JSValue, Promise
from _window import window


jsstr = window.new_from_str
jsclosure = window.new_closure


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

    return _prompt.call(*(jsstr(arg) for arg in [msg, default_val] if arg)).as_str()
