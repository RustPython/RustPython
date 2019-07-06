
from io import BytesIO

def test_01():
    bytes_string =  b'Test String 1'

    f = BytesIO()
    f.write(bytes_string)

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
    assert f.seek(0) == 0
    assert f.read(4) == b'Test'

if __name__ == "__main__":
    test_01()
    test_02()
    test_03()
    test_04()

