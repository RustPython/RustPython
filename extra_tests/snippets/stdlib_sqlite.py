import sqlite3 as sqlite
import unittest

rows = [(3,), (4,)]
cx = sqlite.connect(":memory:")
cx.execute(";")
cx.executescript(";")
cx.execute("CREATE TABLE foo(key INTEGER)")
cx.executemany("INSERT INTO foo(key) VALUES (?)", rows)

cur = cx.cursor()
fetchall = cur.execute("SELECT * FROM foo").fetchall()
assert fetchall == rows

cx.executescript("""
    /* CREATE TABLE foo(key INTEGER); */
    INSERT INTO foo(key) VALUES (10);
    INSERT INTO foo(key) VALUES (11);
""")

class AggrSum:
    def __init__(self):
        self.val = 0.0

    def step(self, val):
        self.val += val

    def finalize(self):
        return self.val

cx.create_aggregate("mysum", 1, AggrSum)
cur.execute("select mysum(key) from foo")
assert cur.fetchone()[0] == 28.0

# toobig = 2**64
# cur.execute("insert into foo(key) values (?)", (toobig,))

class AggrText:
    def __init__(self):
        self.txt = ""
    def step(self, txt):
        txt = str(txt)
        self.txt = self.txt + txt
    def finalize(self):
        return self.txt

cx.create_aggregate("aggtxt", 1, AggrText)
cur.execute("select aggtxt(key) from foo")
assert cur.fetchone()[0] == '341011'