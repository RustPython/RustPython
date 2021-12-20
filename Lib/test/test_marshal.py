import unittest
import marshal

class MarshalTests(unittest.TestCase):
    """
    Testing each data type is done with two tests
        Test dumps data == expected_bytes
        Test load(dumped data) == data
    """
    
    def dump_then_load(self, data):
        return marshal.loads(marshal.dumps(data))
    
    def test_dumps_int(self):
        self.assertEqual(marshal.dumps(0), b'i0\x00')
        self.assertEqual(marshal.dumps(-1), b'i-\x01')
        self.assertEqual(marshal.dumps(1), b'i+\x01')
        self.assertEqual(marshal.dumps(100000000), b'i+\x00\xe1\xf5\x05')

    def test_dump_and_load_int(self):
        self.assertEqual(self.dump_then_load(0), 0)
        self.assertEqual(self.dump_then_load(-1), -1)
        self.assertEqual(self.dump_then_load(1), 1)
        self.assertEqual(self.dump_then_load(100000000), 100000000)   

    def test_dumps_float(self):
        self.assertEqual(marshal.dumps(0.0), b'f\x00\x00\x00\x00\x00\x00\x00\x00')
        self.assertEqual(marshal.dumps(-10.0), b'f\x00\x00\x00\x00\x00\x00$\xc0')
        self.assertEqual(marshal.dumps(10.0), b'f\x00\x00\x00\x00\x00\x00$@')

    def test_dump_and_load_int(self):
        self.assertEqual(self.dump_then_load(0.0), 0.0)
        self.assertEqual(self.dump_then_load(-10.0), -10.0)
        self.assertEqual(self.dump_then_load(10), 10)

    def test_dumps_str(self):
        self.assertEqual(marshal.dumps(""), b's')
        self.assertEqual(marshal.dumps("Hello, World"), b'sHello, World')

    def test_dump_and_load_str(self):
        self.assertEqual(self.dump_then_load(""), "")
        self.assertEqual(self.dump_then_load("Hello, World"), "Hello, World")

    def test_dumps_list(self):
        # Lists have to print the length of every element
        # so when marshelling and unmarshelling we know how many bytes to search
        # all usize values are converted to u32 to handle different architecture sizes.
        self.assertEqual(marshal.dumps([]), b'[\x00\x00\x00\x00')
        self.assertEqual(
            marshal.dumps([1, "hello", 1.0]), 
            b'[\x03\x00\x00\x00\x03\x00\x00\x00i+\x01\x06\x00\x00\x00shello\t\x00\x00\x00f\x00\x00\x00\x00\x00\x00\xf0?',
        )
        self.assertEqual(
            marshal.dumps([[0], ['a','b']]),
            b'[\x02\x00\x00\x00\x0c\x00\x00\x00[\x01\x00\x00\x00\x03\x00\x00\x00i0\x00\x11\x00\x00\x00[\x02\x00\x00\x00\x02\x00\x00\x00sa\x02\x00\x00\x00sb',
        )

    def test_dump_and_load_list(self):
        self.assertEqual(self.dump_then_load([]), [])
        self.assertEqual(self.dump_then_load([1, "hello", 1.0]), [1, "hello", 1.0])
        self.assertEqual(self.dump_then_load([[0], ['a','b']]),[[0], ['a','b']])

    def test_dumps_tuple(self):
        self.assertEqual(marshal.dumps(()), b'(\x00\x00\x00\x00')
        self.assertEqual(
            marshal.dumps((1, "hello", 1.0)), 
            b'(\x03\x00\x00\x00\x03\x00\x00\x00i+\x01\x06\x00\x00\x00shello\t\x00\x00\x00f\x00\x00\x00\x00\x00\x00\xf0?'
        )

    def test_dump_and_load_tuple(self):
        self.assertEqual(self.dump_then_load(()), ())
        self.assertEqual(self.dump_then_load((1, "hello", 1.0)), (1, "hello", 1.0))

    def test_dumps_dict(self):
        self.assertEqual(marshal.dumps({}), b',[\x00\x00\x00\x00')
        self.assertEqual(
            marshal.dumps({'a':1, 1:'a'}), 
            b',[\x02\x00\x00\x00\x12\x00\x00\x00(\x02\x00\x00\x00\x02\x00\x00\x00sa\x03\x00\x00\x00i+\x01\x12\x00\x00\x00(\x02\x00\x00\x00\x03\x00\x00\x00i+\x01\x02\x00\x00\x00sa'
        )
        self.assertEqual(
            marshal.dumps({'a':{'b':2}, 'c':[0.0, 4.0, 6, 9]}), 
            b',[\x02\x00\x00\x00+\x00\x00\x00(\x02\x00\x00\x00\x02\x00\x00\x00sa\x1c\x00\x00\x00,[\x01\x00\x00\x00\x12\x00\x00\x00(\x02\x00\x00\x00\x02\x00\x00\x00sb\x03\x00\x00\x00i+\x02<\x00\x00\x00(\x02\x00\x00\x00\x02\x00\x00\x00sc-\x00\x00\x00[\x04\x00\x00\x00\t\x00\x00\x00f\x00\x00\x00\x00\x00\x00\x00\x00\t\x00\x00\x00f\x00\x00\x00\x00\x00\x00\x10@\x03\x00\x00\x00i+\x06\x03\x00\x00\x00i+\t'
        )

    def test_dump_and_load_dict(self):
        self.assertEqual(self.dump_then_load({}), {})
        self.assertEqual(self.dump_then_load({'a':1, 1:'a'}), {'a':1, 1:'a'})
        self.assertEqual(self.dump_then_load({'a':{'b':2}, 'c':[0.0, 4.0, 6, 9]}), {'a':{'b':2}, 'c':[0.0, 4.0, 6, 9]})

if __name__ == "__main__":
    unittest.main()