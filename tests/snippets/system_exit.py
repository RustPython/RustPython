try:
    exit(1)
except SystemExit as e:
    assert e.code == 1


try:
    exit()
except SystemExit as e:
    assert e.code is None, "Exit code is " + str(e.code)
