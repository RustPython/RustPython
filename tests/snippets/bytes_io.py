
from io import BytesIO

def test_01():
    bytes_string =  b'Test String 1'

    f = BytesIO()
    f.write(bytes_string)

    assert f.getvalue() == bytes_string

def test_02():
    bytes_string =  b'Test String 2'

    f = BytesIO()
    f.write(bytes_string)

    assert f.read() == bytes_string
    assert f.read() == b''
    assert f.getvalue() == b''

if __name__ == "__main__":
    test_01()
    test_02()
