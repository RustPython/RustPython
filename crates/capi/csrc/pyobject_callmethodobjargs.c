#include <stdarg.h>
#include <stddef.h>
#include <stdlib.h>

typedef struct _object PyObject;

extern PyObject *RustPython_PyObject_CallMethodObjArgsArray(
    PyObject *receiver,
    PyObject *name,
    PyObject *const *args,
    size_t nargs
);

PyObject *PyObject_CallMethodObjArgs(PyObject *receiver, PyObject *name, ...) {
    va_list ap;
    size_t nargs = 0;

    va_start(ap, name);
    while (va_arg(ap, PyObject *) != NULL) {
        nargs++;
    }
    va_end(ap);

    PyObject **args = NULL;
    if (nargs > 0) {
        args = (PyObject **)malloc(sizeof(PyObject *) * nargs);
        if (args == NULL) {
            return NULL;
        }
        va_start(ap, name);
        for (size_t i = 0; i < nargs; i++) {
            args[i] = va_arg(ap, PyObject *);
        }
        (void)va_arg(ap, PyObject *);
        va_end(ap);
    }

    PyObject *result = RustPython_PyObject_CallMethodObjArgsArray(receiver, name, args, nargs);
    free(args);
    return result;
}

void *RustPython_Keep_PyObject_CallMethodObjArgs(void) {
    return (void *)&PyObject_CallMethodObjArgs;
}
