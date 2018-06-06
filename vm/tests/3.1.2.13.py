word = "Python"

assert "Python" == word[:2] + word[2:]
assert "Python" == word[:4] + word[4:]

assert "Py" == word[:2]
assert "on" == word[4:]
assert "on" == word[-2:]
assert "Py" == word[:-4]
# assert "Py" == word[::2]
