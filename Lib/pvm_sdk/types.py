class SoftFloat(str):
    def __new__(cls, value):
        return str.__new__(cls, str(value))
