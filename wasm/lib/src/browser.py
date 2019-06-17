from _browser import *
from _js import JsValue
from _window import window


_alert = window.get_prop("alert")


def alert(msg):
    if type(msg) != str:
        raise TypeError("msg must be a string")
    _alert.call(JsValue.fromstr(msg))
