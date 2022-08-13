from .. import abc
from .. import util

machinery = util.import_importlib('importlib.machinery')

import unittest
import warnings


class FinderTests(abc.FinderTests):

    """Test the finder for extension modules."""

    def find_spec(self, fullname):
        importer = self.machinery.FileFinder(util.EXTENSIONS.path,
                                            (self.machinery.ExtensionFileLoader,
                                             self.machinery.EXTENSION_SUFFIXES))

        return importer.find_spec(fullname)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_module(self):
        self.assertTrue(self.find_spec(util.EXTENSIONS.name))

    # No extension module as an __init__ available for testing.
    test_package = test_package_in_package = None

    # No extension module in a package available for testing.
    test_module_in_package = None

    # Extension modules cannot be an __init__ for a package.
    test_package_over_module = None

    def test_failure(self):
        self.assertIsNone(self.find_spec('asdfjkl;'))


(Frozen_FinderTests,
 Source_FinderTests
 ) = util.test_both(FinderTests, machinery=machinery)


if __name__ == '__main__':
    unittest.main()
