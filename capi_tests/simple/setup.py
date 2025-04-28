from setuptools import setup, Extension

setup(
    name='my_module',
    version='1.0.0',
    ext_modules=[Extension('my_module', sources=['my_module.c'])],
)
