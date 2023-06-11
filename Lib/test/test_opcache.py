import unittest


class TestLoadAttrCache(unittest.TestCase):
    def test_descriptor_added_after_optimization(self):
        class Descriptor:
            pass

        class C:
            def __init__(self):
                self.x = 1
            x = Descriptor()

        def f(o):
            return o.x

        o = C()
        for i in range(1025):
            assert f(o) == 1

        Descriptor.__get__ = lambda self, instance, value: 2
        Descriptor.__set__ = lambda *args: None

        self.assertEqual(f(o), 2)

    def test_metaclass_descriptor_added_after_optimization(self):
        class Descriptor:
            pass

        class Metaclass(type):
            attribute = Descriptor()

        class Class(metaclass=Metaclass):
            attribute = True

        def __get__(self, instance, owner):
            return False

        def __set__(self, instance, value):
            return None

        def f():
            return Class.attribute

        for _ in range(1025):
            self.assertTrue(f())

        Descriptor.__get__ = __get__
        Descriptor.__set__ = __set__

        for _ in range(1025):
            self.assertFalse(f())

    def test_metaclass_descriptor_shadows_class_attribute(self):
        class Metaclass(type):
            @property
            def attribute(self):
                return True

        class Class(metaclass=Metaclass):
            attribute = False

        def f():
            return Class.attribute

        for _ in range(1025):
            self.assertTrue(f())

    def test_metaclass_set_descriptor_after_optimization(self):
        class Metaclass(type):
            pass

        class Class(metaclass=Metaclass):
            attribute = True

        @property
        def attribute(self):
            return False

        def f():
            return Class.attribute

        for _ in range(1025):
            self.assertTrue(f())

        Metaclass.attribute = attribute

        for _ in range(1025):
            self.assertFalse(f())

    def test_metaclass_del_descriptor_after_optimization(self):
        class Metaclass(type):
            @property
            def attribute(self):
                return True

        class Class(metaclass=Metaclass):
            attribute = False

        def f():
            return Class.attribute

        for _ in range(1025):
            self.assertTrue(f())

        del Metaclass.attribute

        for _ in range(1025):
            self.assertFalse(f())

    def test_type_descriptor_shadows_attribute_method(self):
        class Class:
            mro = None

        def f():
            return Class.mro

        for _ in range(1025):
            self.assertIsNone(f())

    def test_type_descriptor_shadows_attribute_member(self):
        class Class:
            __base__ = None

        def f():
            return Class.__base__

        for _ in range(1025):
            self.assertIs(f(), object)

    def test_type_descriptor_shadows_attribute_getset(self):
        class Class:
            __name__ = "Spam"

        def f():
            return Class.__name__

        for _ in range(1025):
            self.assertEqual(f(), "Class")

    def test_metaclass_getattribute(self):
        class Metaclass(type):
            def __getattribute__(self, name):
                return True

        class Class(metaclass=Metaclass):
            attribute = False

        def f():
            return Class.attribute

        for _ in range(1025):
            self.assertTrue(f())

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_metaclass_swap(self):
        class OldMetaclass(type):
            @property
            def attribute(self):
                return True

        class NewMetaclass(type):
            @property
            def attribute(self):
                return False

        class Class(metaclass=OldMetaclass):
            pass

        def f():
            return Class.attribute

        for _ in range(1025):
            self.assertTrue(f())

        Class.__class__ = NewMetaclass

        for _ in range(1025):
            self.assertFalse(f())

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_load_shadowing_slot_should_raise_type_error(self):
        class Class:
            __slots__ = ("slot",)

        class Sneaky:
            __slots__ = ("shadowed",)
            shadowing = Class.slot

        def f(o):
            o.shadowing

        o = Sneaky()
        o.shadowed = 42

        for _ in range(1025):
            with self.assertRaises(TypeError):
                f(o)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_store_shadowing_slot_should_raise_type_error(self):
        class Class:
            __slots__ = ("slot",)

        class Sneaky:
            __slots__ = ("shadowed",)
            shadowing = Class.slot

        def f(o):
            o.shadowing = 42

        o = Sneaky()

        for _ in range(1025):
            with self.assertRaises(TypeError):
                f(o)

    @unittest.skip("TODO: RUSTPYTHON")
    def test_load_borrowed_slot_should_not_crash(self):
        class Class:
            __slots__ = ("slot",)

        class Sneaky:
            borrowed = Class.slot

        def f(o):
            o.borrowed

        o = Sneaky()

        for _ in range(1025):
            with self.assertRaises(TypeError):
                f(o)

    @unittest.skip("TODO: RUSTPYTHON")
    def test_store_borrowed_slot_should_not_crash(self):
        class Class:
            __slots__ = ("slot",)

        class Sneaky:
            borrowed = Class.slot

        def f(o):
            o.borrowed = 42

        o = Sneaky()

        for _ in range(1025):
            with self.assertRaises(TypeError):
                f(o)


class TestLoadMethodCache(unittest.TestCase):
    def test_descriptor_added_after_optimization(self):
        class Descriptor:
            pass

        class Class:
            attribute = Descriptor()

        def __get__(self, instance, owner):
            return lambda: False

        def __set__(self, instance, value):
            return None

        def attribute():
            return True

        instance = Class()
        instance.attribute = attribute

        def f():
            return instance.attribute()

        for _ in range(1025):
            self.assertTrue(f())

        Descriptor.__get__ = __get__
        Descriptor.__set__ = __set__

        for _ in range(1025):
            self.assertFalse(f())

    def test_metaclass_descriptor_added_after_optimization(self):
        class Descriptor:
            pass

        class Metaclass(type):
            attribute = Descriptor()

        class Class(metaclass=Metaclass):
            def attribute():
                return True

        def __get__(self, instance, owner):
            return lambda: False

        def __set__(self, instance, value):
            return None

        def f():
            return Class.attribute()

        for _ in range(1025):
            self.assertTrue(f())

        Descriptor.__get__ = __get__
        Descriptor.__set__ = __set__

        for _ in range(1025):
            self.assertFalse(f())

    def test_metaclass_descriptor_shadows_class_attribute(self):
        class Metaclass(type):
            @property
            def attribute(self):
                return lambda: True

        class Class(metaclass=Metaclass):
            def attribute():
                return False

        def f():
            return Class.attribute()

        for _ in range(1025):
            self.assertTrue(f())

    def test_metaclass_set_descriptor_after_optimization(self):
        class Metaclass(type):
            pass

        class Class(metaclass=Metaclass):
            def attribute():
                return True

        @property
        def attribute(self):
            return lambda: False

        def f():
            return Class.attribute()

        for _ in range(1025):
            self.assertTrue(f())

        Metaclass.attribute = attribute

        for _ in range(1025):
            self.assertFalse(f())

    def test_metaclass_del_descriptor_after_optimization(self):
        class Metaclass(type):
            @property
            def attribute(self):
                return lambda: True

        class Class(metaclass=Metaclass):
            def attribute():
                return False

        def f():
            return Class.attribute()

        for _ in range(1025):
            self.assertTrue(f())

        del Metaclass.attribute

        for _ in range(1025):
            self.assertFalse(f())

    def test_type_descriptor_shadows_attribute_method(self):
        class Class:
            def mro():
                return ["Spam", "eggs"]

        def f():
            return Class.mro()

        for _ in range(1025):
            self.assertEqual(f(), ["Spam", "eggs"])

    def test_type_descriptor_shadows_attribute_member(self):
        class Class:
            def __base__():
                return "Spam"

        def f():
            return Class.__base__()

        for _ in range(1025):
            self.assertNotEqual(f(), "Spam")

    def test_metaclass_getattribute(self):
        class Metaclass(type):
            def __getattribute__(self, name):
                return lambda: True

        class Class(metaclass=Metaclass):
            def attribute():
                return False

        def f():
            return Class.attribute()

        for _ in range(1025):
            self.assertTrue(f())

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_metaclass_swap(self):
        class OldMetaclass(type):
            @property
            def attribute(self):
                return lambda: True

        class NewMetaclass(type):
            @property
            def attribute(self):
                return lambda: False

        class Class(metaclass=OldMetaclass):
            pass

        def f():
            return Class.attribute()

        for _ in range(1025):
            self.assertTrue(f())

        Class.__class__ = NewMetaclass

        for _ in range(1025):
            self.assertFalse(f())


if __name__ == "__main__":
    import unittest
    unittest.main()
