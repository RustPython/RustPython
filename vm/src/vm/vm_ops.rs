use super::{PyMethod, VirtualMachine};
use crate::{
    builtins::{PyInt, PyIntRef, PyStrInterned},
    function::PyArithmeticValue,
    object::{AsObject, PyObject, PyObjectRef, PyResult},
    protocol::PyIterReturn,
    types::PyComparisonOp,
};

/// Collection of operators
impl VirtualMachine {
    pub fn to_index_opt(&self, obj: PyObjectRef) -> Option<PyResult<PyIntRef>> {
        match obj.downcast() {
            Ok(val) => Some(Ok(val)),
            Err(obj) => self
                .get_method(obj, identifier!(self, __index__))
                .map(|index| {
                    // TODO: returning strict subclasses of int in __index__ is deprecated
                    self.invoke(&index?, ())?.downcast().map_err(|bad| {
                        self.new_type_error(format!(
                            "__index__ returned non-int (type {})",
                            bad.class().name()
                        ))
                    })
                }),
        }
    }

    pub fn to_index(&self, obj: &PyObject) -> PyResult<PyIntRef> {
        self.to_index_opt(obj.to_owned()).unwrap_or_else(|| {
            Err(self.new_type_error(format!(
                "'{}' object cannot be interpreted as an integer",
                obj.class().name()
            )))
        })
    }

    #[inline]
    pub fn bool_eq(&self, a: &PyObject, b: &PyObject) -> PyResult<bool> {
        a.rich_compare_bool(b, PyComparisonOp::Eq, self)
    }

    pub fn identical_or_equal(&self, a: &PyObject, b: &PyObject) -> PyResult<bool> {
        if a.is(b) {
            Ok(true)
        } else {
            self.bool_eq(a, b)
        }
    }

    pub fn bool_seq_lt(&self, a: &PyObject, b: &PyObject) -> PyResult<Option<bool>> {
        let value = if a.rich_compare_bool(b, PyComparisonOp::Lt, self)? {
            Some(true)
        } else if !self.bool_eq(a, b)? {
            Some(false)
        } else {
            None
        };
        Ok(value)
    }

    pub fn bool_seq_gt(&self, a: &PyObject, b: &PyObject) -> PyResult<Option<bool>> {
        let value = if a.rich_compare_bool(b, PyComparisonOp::Gt, self)? {
            Some(true)
        } else if !self.bool_eq(a, b)? {
            Some(false)
        } else {
            None
        };
        Ok(value)
    }

    pub fn length_hint_opt(&self, iter: PyObjectRef) -> PyResult<Option<usize>> {
        match iter.length(self) {
            Ok(len) => return Ok(Some(len)),
            Err(e) => {
                if !e.fast_isinstance(&self.ctx.exceptions.type_error) {
                    return Err(e);
                }
            }
        }
        let hint = match self.get_method(iter, identifier!(self, __length_hint__)) {
            Some(hint) => hint?,
            None => return Ok(None),
        };
        let result = match self.invoke(&hint, ()) {
            Ok(res) => {
                if res.is(&self.ctx.not_implemented) {
                    return Ok(None);
                }
                res
            }
            Err(e) => {
                return if e.fast_isinstance(&self.ctx.exceptions.type_error) {
                    Ok(None)
                } else {
                    Err(e)
                }
            }
        };
        let hint = result
            .payload_if_subclass::<PyInt>(self)
            .ok_or_else(|| {
                self.new_type_error(format!(
                    "'{}' object cannot be interpreted as an integer",
                    result.class().name()
                ))
            })?
            .try_to_primitive::<isize>(self)?;
        if hint.is_negative() {
            Err(self.new_value_error("__length_hint__() should return >= 0".to_owned()))
        } else {
            Ok(Some(hint as usize))
        }
    }

    /// Checks that the multiplication is able to be performed. On Ok returns the
    /// index as a usize for sequences to be able to use immediately.
    pub fn check_repeat_or_overflow_error(&self, length: usize, n: isize) -> PyResult<usize> {
        if n <= 0 {
            Ok(0)
        } else {
            let n = n as usize;
            if length > crate::stdlib::sys::MAXSIZE as usize / n {
                Err(self.new_overflow_error("repeated value are too long".to_owned()))
            } else {
                Ok(n)
            }
        }
    }

    /// Calls a method on `obj` passing `arg`, if the method exists.
    ///
    /// Otherwise, or if the result is the special `NotImplemented` built-in constant,
    /// calls `unsupported` to determine fallback value.
    pub fn call_or_unsupported<F>(
        &self,
        obj: &PyObject,
        arg: &PyObject,
        method: &'static PyStrInterned,
        unsupported: F,
    ) -> PyResult
    where
        F: Fn(&VirtualMachine, &PyObject, &PyObject) -> PyResult,
    {
        if let Some(method_or_err) = self.get_method(obj.to_owned(), method) {
            let method = method_or_err?;
            let result = self.invoke(&method, (arg.to_owned(),))?;
            if let PyArithmeticValue::Implemented(x) = PyArithmeticValue::from_object(self, result)
            {
                return Ok(x);
            }
        }
        unsupported(self, obj, arg)
    }

    /// Calls a method, falling back to its reflection with the operands
    /// reversed, and then to the value provided by `unsupported`.
    ///
    /// For example: the following:
    ///
    /// `call_or_reflection(lhs, rhs, "__and__", "__rand__", unsupported)`
    ///
    /// 1. Calls `__and__` with `lhs` and `rhs`.
    /// 2. If above is not implemented, calls `__rand__` with `rhs` and `lhs`.
    /// 3. If above is not implemented, invokes `unsupported` for the result.
    pub fn call_or_reflection(
        &self,
        lhs: &PyObject,
        rhs: &PyObject,
        default: &'static PyStrInterned,
        reflection: &'static PyStrInterned,
        unsupported: fn(&VirtualMachine, &PyObject, &PyObject) -> PyResult,
    ) -> PyResult {
        if rhs.fast_isinstance(&lhs.class()) {
            let lop = lhs.get_class_attr(reflection);
            let rop = rhs.get_class_attr(reflection);
            if let Some((lop, rop)) = lop.zip(rop) {
                if !lop.is(&rop) {
                    if let Ok(r) = self.call_or_unsupported(rhs, lhs, reflection, |vm, _, _| {
                        Err(vm.new_exception_empty(vm.ctx.exceptions.exception_type.clone()))
                    }) {
                        return Ok(r);
                    }
                }
            }
        }
        // Try to call the default method
        self.call_or_unsupported(lhs, rhs, default, move |vm, lhs, rhs| {
            // Try to call the reflection method
            // don't call reflection method if operands are of the same type
            if !lhs.class().is(&rhs.class()) {
                vm.call_or_unsupported(rhs, lhs, reflection, |_, rhs, lhs| {
                    // switch them around again
                    unsupported(vm, lhs, rhs)
                })
            } else {
                unsupported(vm, lhs, rhs)
            }
        })
    }

    pub fn _sub(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_reflection(
            a,
            b,
            identifier!(self, __sub__),
            identifier!(self, __rsub__),
            |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "-")),
        )
    }

    pub fn _isub(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_unsupported(a, b, identifier!(self, __isub__), |vm, a, b| {
            vm.call_or_reflection(
                a,
                b,
                identifier!(self, __sub__),
                identifier!(self, __rsub__),
                |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "-=")),
            )
        })
    }

    pub fn _add(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_reflection(
            a,
            b,
            identifier!(self, __add__),
            identifier!(self, __radd__),
            |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "+")),
        )
    }

    pub fn _iadd(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_unsupported(a, b, identifier!(self, __iadd__), |vm, a, b| {
            vm.call_or_reflection(
                a,
                b,
                identifier!(self, __add__),
                identifier!(self, __radd__),
                |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "+=")),
            )
        })
    }

    pub fn _mul(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_reflection(
            a,
            b,
            identifier!(self, __mul__),
            identifier!(self, __rmul__),
            |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "*")),
        )
    }

    pub fn _imul(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_unsupported(a, b, identifier!(self, __imul__), |vm, a, b| {
            vm.call_or_reflection(
                a,
                b,
                identifier!(self, __mul__),
                identifier!(self, __rmul__),
                |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "*=")),
            )
        })
    }

    pub fn _matmul(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_reflection(
            a,
            b,
            identifier!(self, __matmul__),
            identifier!(self, __rmatmul__),
            |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "@")),
        )
    }

    pub fn _imatmul(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_unsupported(a, b, identifier!(self, __imatmul__), |vm, a, b| {
            vm.call_or_reflection(
                a,
                b,
                identifier!(self, __matmul__),
                identifier!(self, __rmatmul__),
                |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "@=")),
            )
        })
    }

    pub fn _truediv(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_reflection(
            a,
            b,
            identifier!(self, __truediv__),
            identifier!(self, __rtruediv__),
            |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "/")),
        )
    }

    pub fn _itruediv(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_unsupported(a, b, identifier!(self, __itruediv__), |vm, a, b| {
            vm.call_or_reflection(
                a,
                b,
                identifier!(self, __truediv__),
                identifier!(self, __rtruediv__),
                |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "/=")),
            )
        })
    }

    pub fn _floordiv(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_reflection(
            a,
            b,
            identifier!(self, __floordiv__),
            identifier!(self, __rfloordiv__),
            |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "//")),
        )
    }

    pub fn _ifloordiv(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_unsupported(a, b, identifier!(self, __ifloordiv__), |vm, a, b| {
            vm.call_or_reflection(
                a,
                b,
                identifier!(self, __floordiv__),
                identifier!(self, __rfloordiv__),
                |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "//=")),
            )
        })
    }

    pub fn _pow(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_reflection(
            a,
            b,
            identifier!(self, __pow__),
            identifier!(self, __rpow__),
            |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "**")),
        )
    }

    pub fn _ipow(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_unsupported(a, b, identifier!(self, __ipow__), |vm, a, b| {
            vm.call_or_reflection(
                a,
                b,
                identifier!(self, __pow__),
                identifier!(self, __rpow__),
                |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "**=")),
            )
        })
    }

    pub fn _mod(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_reflection(
            a,
            b,
            identifier!(self, __mod__),
            identifier!(self, __rmod__),
            |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "%")),
        )
    }

    pub fn _imod(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_unsupported(a, b, identifier!(self, __imod__), |vm, a, b| {
            vm.call_or_reflection(
                a,
                b,
                identifier!(self, __mod__),
                identifier!(self, __rmod__),
                |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "%=")),
            )
        })
    }

    pub fn _divmod(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_reflection(
            a,
            b,
            identifier!(self, __divmod__),
            identifier!(self, __rdivmod__),
            |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "divmod")),
        )
    }

    pub fn _lshift(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_reflection(
            a,
            b,
            identifier!(self, __lshift__),
            identifier!(self, __rlshift__),
            |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "<<")),
        )
    }

    pub fn _ilshift(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_unsupported(a, b, identifier!(self, __ilshift__), |vm, a, b| {
            vm.call_or_reflection(
                a,
                b,
                identifier!(self, __lshift__),
                identifier!(self, __rlshift__),
                |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "<<=")),
            )
        })
    }

    pub fn _rshift(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_reflection(
            a,
            b,
            identifier!(self, __rshift__),
            identifier!(self, __rrshift__),
            |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, ">>")),
        )
    }

    pub fn _irshift(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_unsupported(a, b, identifier!(self, __irshift__), |vm, a, b| {
            vm.call_or_reflection(
                a,
                b,
                identifier!(self, __rshift__),
                identifier!(self, __rrshift__),
                |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, ">>=")),
            )
        })
    }

    pub fn _xor(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_reflection(
            a,
            b,
            identifier!(self, __xor__),
            identifier!(self, __rxor__),
            |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "^")),
        )
    }

    pub fn _ixor(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_unsupported(a, b, identifier!(self, __ixor__), |vm, a, b| {
            vm.call_or_reflection(
                a,
                b,
                identifier!(self, __xor__),
                identifier!(self, __rxor__),
                |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "^=")),
            )
        })
    }

    pub fn _or(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_reflection(
            a,
            b,
            identifier!(self, __or__),
            identifier!(self, __ror__),
            |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "|")),
        )
    }

    pub fn _ior(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_unsupported(a, b, identifier!(self, __ior__), |vm, a, b| {
            vm.call_or_reflection(
                a,
                b,
                identifier!(self, __or__),
                identifier!(self, __ror__),
                |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "|=")),
            )
        })
    }

    pub fn _and(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_reflection(
            a,
            b,
            identifier!(self, __and__),
            identifier!(self, __rand__),
            |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "&")),
        )
    }

    pub fn _iand(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_unsupported(a, b, identifier!(self, __iand__), |vm, a, b| {
            vm.call_or_reflection(
                a,
                b,
                identifier!(self, __and__),
                identifier!(self, __rand__),
                |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "&=")),
            )
        })
    }

    pub fn _abs(&self, a: &PyObject) -> PyResult<PyObjectRef> {
        self.get_special_method(a.to_owned(), identifier!(self, __abs__))?
            .map_err(|_| self.new_unsupported_unary_error(a, "abs()"))?
            .invoke((), self)
    }

    pub fn _pos(&self, a: &PyObject) -> PyResult {
        self.get_special_method(a.to_owned(), identifier!(self, __pos__))?
            .map_err(|_| self.new_unsupported_unary_error(a, "unary +"))?
            .invoke((), self)
    }

    pub fn _neg(&self, a: &PyObject) -> PyResult {
        self.get_special_method(a.to_owned(), identifier!(self, __neg__))?
            .map_err(|_| self.new_unsupported_unary_error(a, "unary -"))?
            .invoke((), self)
    }

    pub fn _invert(&self, a: &PyObject) -> PyResult {
        self.get_special_method(a.to_owned(), identifier!(self, __invert__))?
            .map_err(|_| self.new_unsupported_unary_error(a, "unary ~"))?
            .invoke((), self)
    }

    // https://docs.python.org/3/reference/expressions.html#membership-test-operations
    fn _membership_iter_search(
        &self,
        haystack: PyObjectRef,
        needle: PyObjectRef,
    ) -> PyResult<PyIntRef> {
        let iter = haystack.get_iter(self)?;
        loop {
            if let PyIterReturn::Return(element) = iter.next(self)? {
                if self.bool_eq(&needle, &element)? {
                    return Ok(self.ctx.new_bool(true));
                } else {
                    continue;
                }
            } else {
                return Ok(self.ctx.new_bool(false));
            }
        }
    }

    pub fn _contains(&self, haystack: PyObjectRef, needle: PyObjectRef) -> PyResult {
        match PyMethod::get_special(haystack, identifier!(self, __contains__), self)? {
            Ok(method) => method.invoke((needle,), self),
            Err(haystack) => self
                ._membership_iter_search(haystack, needle)
                .map(Into::into),
        }
    }
}
