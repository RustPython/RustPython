
from io import StringIO

def test_01():
    string =  'Test String 1'
    f = StringIO()
    f.write(string)

    assert f.getvalue() == string

def test_02():
    string =  'Test String 2'
    f = StringIO()
    f.write(string)

    assert f.read() == string
    assert f.read() == ''
    assert f.getvalue() == ''

if __name__ == "__main__":
    test_01()
    test_02()
