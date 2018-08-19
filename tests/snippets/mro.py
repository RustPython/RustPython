#
# This doesn't work because equality is broken for classes
#

# assert object.__mro__ == (object,)
# assert type.__mro__ == (type, object,)

# class A:
#     pass

# assert A.__mro__ == (A, object)
