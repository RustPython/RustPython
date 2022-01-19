import unittest
import marshal

class MarshalTests(unittest.TestCase):
    """
    Testing the (incomplete) marshal module.
    """
    
    def dump_then_load(self, data):
        return marshal.loads(marshal.dumps(data))

    def test_dump_and_load_int(self):
        self.assertEqual(self.dump_then_load(0), 0)
        self.assertEqual(self.dump_then_load(-1), -1)
        self.assertEqual(self.dump_then_load(1), 1)
        self.assertEqual(self.dump_then_load(100000000), 100000000)   

    def test_dump_and_load_int(self):
        self.assertEqual(self.dump_then_load(0.0), 0.0)
        self.assertEqual(self.dump_then_load(-10.0), -10.0)
        self.assertEqual(self.dump_then_load(10), 10)

    def test_dump_and_load_str(self):
        self.assertEqual(self.dump_then_load(""), "")
        self.assertEqual(self.dump_then_load("Hello, World"), "Hello, World")

    def test_dump_and_load_list(self):
        self.assertEqual(self.dump_then_load([]), [])
        self.assertEqual(self.dump_then_load([1, "hello", 1.0]), [1, "hello", 1.0])
        self.assertEqual(self.dump_then_load([[0], ['a','b']]),[[0], ['a','b']])

    def test_dump_and_load_tuple(self):
        self.assertEqual(self.dump_then_load(()), ())
        self.assertEqual(self.dump_then_load((1, "hello", 1.0)), (1, "hello", 1.0))

    def test_dump_and_load_dict(self):
        self.assertEqual(self.dump_then_load({}), {})
        self.assertEqual(self.dump_then_load({'a':1, 1:'a'}), {'a':1, 1:'a'})
        self.assertEqual(self.dump_then_load({'a':{'b':2}, 'c':[0.0, 4.0, 6, 9]}), {'a':{'b':2}, 'c':[0.0, 4.0, 6, 9]})

if __name__ == "__main__":
    unittest.main()