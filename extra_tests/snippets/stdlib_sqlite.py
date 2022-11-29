import sqlite3 as sqlite

rows = [(3,), (4,)]
cx = sqlite.connect(":memory:")
cx.execute("CREATE TABLE foo(key INTEGER)")
cx.executemany("INSERT INTO foo(key) VALUES (?)", rows)

cur = cx.cursor()
fetchall = cur.execute("SELECT * FROM foo").fetchall()
assert fetchall == rows
