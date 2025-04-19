#include <Python.h>

// Function to be called from Python
static PyObject* my_function(PyObject* self, PyObject* args) {
    int num;

    // Parse arguments from Python
    if (!PyArg_ParseTuple(args, "i", &num)) {
        return NULL; // Return NULL if arguments are invalid
    }

    // Perform some operation
    int result = num * 2;

    // Return the result as a Python object
    return Py_BuildValue("i", result);
}

// Method definition table
static PyMethodDef MyModuleMethods[] = {
    {"my_function", my_function, METH_VARARGS, "Doubles the input number."},
    {NULL, NULL, 0, NULL} // Sentinel value ending the table
};

// Module definition structure
static struct PyModuleDef mymodule = {
    PyModuleDef_HEAD_INIT,
    "my_module", // Module name
    NULL, // Module documentation (optional)
    -1, // Module state size, -1 for modules that don't maintain state
    MyModuleMethods
};

// Module initialization function
PyMODINIT_FUNC PyInit_my_module(void) {
    return PyModule_Create(&mymodule);
}