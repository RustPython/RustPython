
from io import StringIO

def test_01():
    """
        Test that the constructor and getvalue
        method return expected values
    """
    string =  'Test String 1'
    f = StringIO()
    f.write(string)

    assert f.getvalue() == string

def test_02():
    """
        Test that the read method (no arg)
        results the expected value
    """
    string =  'Test String 2'
    f = StringIO(string)

    assert f.read() == string
    assert f.read() == ''

def test_03():
    """
        Tests that the read method (integer arg)
        returns the expected value
    """
    string =  'Test String 3'
    f = StringIO(string)

    assert f.read(1) == 'T'
    assert f.read(1) == 'e'
    assert f.read(1) == 's'
    assert f.read(1) == 't'

def test_04():
    """
        Tests that the read method increments the 
        cursor position and the seek method moves 
        the cursor to the appropriate position
    """
    string =  'Test String 4'
    f = StringIO(string)

    assert f.read(4) == 'Test'
    assert f.seek(0) == 0
    assert f.read(4) == 'Test'

if __name__ == "__main__":
    test_01()
    test_02()
    test_03()
    test_04()
