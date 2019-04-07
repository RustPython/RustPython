
from unittest import TestLoader, TextTestRunner


def main():
    loader = TestLoader()
    suite = loader.discover(start_dir='cpython_tests')
    with open('cpython_test_output', 'w') as fp:
        runner = TextTestRunner(stream=fp)
        runner.run(suite)


if __name__ == '__main__':
    main()
