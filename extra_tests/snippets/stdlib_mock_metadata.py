from unittest.mock import Mock

assert Mock.return_value.__doc__ == "The value to be returned when the mock is called."
assert Mock.return_value.fdel is None
