
from io import BytesIO

def test_01():
    bytes_string =  b'Test String 1'

    f = BytesIO()
    f.write(bytes_string)

    assert f.tell() == len(bytes_string)
    assert f.getvalue() == bytes_string

def test_02():
    bytes_string =  b'Test String 2'
    f = BytesIO(bytes_string)

    assert f.read() == bytes_string
    assert f.read() == b''

def test_03():
    """
        Tests that the read method (integer arg)
        returns the expected value
    """
    string =  b'Test String 3'
    f = BytesIO(string)

    assert f.read(1) == b'T'
    assert f.read(1) == b'e'
    assert f.read(1) == b's'
    assert f.read(1) == b't'

def test_04():
    """
        Tests that the read method increments the 
        cursor position and the seek method moves 
        the cursor to the appropriate position
    """
    string =  b'Test String 4'
    f = BytesIO(string)

    assert f.read(4) == b'Test'
    assert f.tell() == 4
    assert f.seek(0) == 0
    assert f.read(4) == b'Test'

def test_05():
    """
        Tests that the write method accpets bytearray
    """
    bytes_string =  b'Test String 5'

    f = BytesIO()
    f.write(bytearray(bytes_string))

    assert f.getvalue() == bytes_string


def test_06():
    """
        Tests readline
    """
    bytes_string =  b'Test String 6\nnew line is here\nfinished'

    f = BytesIO(bytes_string)

    assert f.readline() == b'Test String 6\n'
    assert f.readline() == b'new line is here\n'
    assert f.readline() == b'finished'
    assert f.readline() == b''


if __name__ == "__main__":
    test_01()
    test_02()
    test_03()
    test_04()
    test_05()
    test_06()

