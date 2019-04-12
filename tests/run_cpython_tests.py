import io
import sys
from unittest import TestLoader, TextTestRunner


def main():
    sys.path.insert(0, 'tests/cpython_tests')

    loader = TestLoader()
    # doesn't work yet without os.path support
    # suite = loader.discover(start_dir='cpython_tests')

    import test_bool
    suite = loader.loadTestsFromModule(test_bool)

    stream = io.StringIO()

    runner = TextTestRunner(stream=stream)
    runner.run(suite)

    print(stream.getvalue())


if __name__ == '__main__':
    main()
