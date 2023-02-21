use super::{PyMethod, VirtualMachine};
use crate::{
    builtins::{PyInt, PyIntRef, PyStrInterned},
    function::PyArithmeticValue,
    object::{AsObject, PyObject, PyObjectRef, PyResult},
    protocol::{PyIterReturn, PyNumberMethodsOffset, PySequence},
    types::PyComparisonOp,
};
use num_traits::ToPrimitive;

/// Collection of operators
impl VirtualMachine {
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
                if !e.fast_isinstance(self.ctx.exceptions.type_error) {
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
                return if e.fast_isinstance(self.ctx.exceptions.type_error) {
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

    // TODO: Should be deleted after transplanting complete number protocol
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

    // TODO: Should be deleted after transplanting complete number protocol
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
        if rhs.fast_isinstance(lhs.class()) {
            let lop = lhs.get_class_attr(reflection);
            let rop = rhs.get_class_attr(reflection);
            if let Some((lop, rop)) = lop.zip(rop) {
                if !lop.is(&rop) {
                    if let Ok(r) = self.call_or_unsupported(rhs, lhs, reflection, |vm, _, _| {
                        Err(vm.new_exception_empty(vm.ctx.exceptions.exception_type.to_owned()))
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
            if !lhs.class().is(rhs.class()) {
                vm.call_or_unsupported(rhs, lhs, reflection, |_, rhs, lhs| {
                    // switch them around again
                    unsupported(vm, lhs, rhs)
                })
            } else {
                unsupported(vm, lhs, rhs)
            }
        })
    }

    fn binary_op1<F>(
        &self,
        a: &PyObject,
        b: &PyObject,
        op_slot: PyNumberMethodsOffset,
        unsupported: F,
    ) -> PyResult
    where
        F: Fn(&VirtualMachine, &PyObject, &PyObject) -> PyResult,
    {
        let num_a = a.to_number();
        let num_b = b.to_number();

        let slot_a = num_a.methods(&op_slot, self)?.load();
        let slot_b = num_b.methods(&op_slot, self)?.load();

        if let Some(slot_a) = slot_a {
            if let Some(slot_b) = slot_b {
                // Check if `a` is subclass of `b`
                if b.fast_isinstance(a.class()) {
                    let ret = slot_b(num_a, b, self)?;
                    if ret.rich_compare_bool(
                        self.ctx.not_implemented.as_object(),
                        PyComparisonOp::Ne,
                        self,
                    )? {
                        return Ok(ret);
                    }
                }
            }

            let ret = slot_a(num_a, b, self)?;
            if ret.rich_compare_bool(
                self.ctx.not_implemented.as_object(),
                PyComparisonOp::Ne,
                self,
            )? {
                return Ok(ret);
            }
        }

        // No slot_a or Not implemented
        if let Some(slot_b) = slot_b {
            let ret = slot_b(num_a, b, self)?;
            if ret.rich_compare_bool(
                self.ctx.not_implemented.as_object(),
                PyComparisonOp::Ne,
                self,
            )? {
                return Ok(ret);
            }
        }

        // Both slot_a & slot_b don't exist or are not implemented.
        unsupported(self, a, b)
    }

    /// `binary_op()` can work only with [`PyNumberMethods::BinaryFunc`].
    pub fn binary_op<F>(
        &self,
        a: &PyObject,
        b: &PyObject,
        op_slot: PyNumberMethodsOffset,
        op: &str,
        unsupported: F,
    ) -> PyResult
    where
        F: Fn(&VirtualMachine, &PyObject, &PyObject) -> PyResult,
    {
        let result = self.binary_op1(a, b, op_slot, unsupported)?;

        if result.rich_compare_bool(
            self.ctx.not_implemented.as_object(),
            PyComparisonOp::Eq,
            self,
        )? {
            Err(self.new_unsupported_binop_error(a, b, op))
        } else {
            Ok(result)
        }
    }

    /// ### Binary in-place operators
    ///
    /// The in-place operators are defined to fall back to the 'normal',
    /// non in-place operations, if the in-place methods are not in place.
    ///
    /// - If the left hand object has the appropriate struct members, and
    ///     they are filled, call the appropriate function and return the
    ///     result.  No coercion is done on the arguments; the left-hand object
    ///     is the one the operation is performed on, and it's up to the
    ///     function to deal with the right-hand object.
    ///
    /// - Otherwise, in-place modification is not supported. Handle it exactly as
    ///     a non in-place operation of the same kind.
    fn binary_iop1<F>(
        &self,
        a: &PyObject,
        b: &PyObject,
        iop_slot: PyNumberMethodsOffset,
        op_slot: PyNumberMethodsOffset,
        unsupported: F,
    ) -> PyResult
    where
        F: Fn(&VirtualMachine, &PyObject, &PyObject) -> PyResult,
    {
        let num_a = a.to_number();
        let slot_a = num_a.methods(&iop_slot, self)?.load();

        if let Some(slot_a) = slot_a {
            let ret = slot_a(num_a, b, self)?;
            if ret.rich_compare_bool(
                self.ctx.not_implemented.as_object(),
                PyComparisonOp::Ne,
                self,
            )? {
                return Ok(ret);
            }
        }

        self.binary_op1(a, b, op_slot, unsupported)
    }

    /// `binary_iop()` can work only with [`PyNumberMethods::BinaryFunc`].
    fn binary_iop<F>(
        &self,
        a: &PyObject,
        b: &PyObject,
        iop_slot: PyNumberMethodsOffset,
        op_slot: PyNumberMethodsOffset,
        op: &str,
        unsupported: F,
    ) -> PyResult
    where
        F: Fn(&VirtualMachine, &PyObject, &PyObject) -> PyResult,
    {
        let result = self.binary_iop1(a, b, iop_slot, op_slot, unsupported)?;

        if result.rich_compare_bool(
            self.ctx.not_implemented.as_object(),
            PyComparisonOp::Eq,
            self,
        )? {
            Err(self.new_unsupported_binop_error(a, b, op))
        } else {
            Ok(result)
        }
    }

    pub fn _add(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.binary_op(a, b, PyNumberMethodsOffset::Add, "+", |vm, a, b| {
            let seq_a = PySequence::try_protocol(a, vm);

            if let Ok(seq_a) = seq_a {
                let ret = seq_a.concat(b, vm)?;
                if ret.rich_compare_bool(
                    vm.ctx.not_implemented.as_object(),
                    PyComparisonOp::Ne,
                    vm,
                )? {
                    return Ok(ret);
                }
            }

            Ok(vm.ctx.not_implemented())
        })
    }

    pub fn _iadd(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.binary_iop(
            a,
            b,
            PyNumberMethodsOffset::InplaceAdd,
            PyNumberMethodsOffset::Add,
            "+=",
            |vm, a, b| {
                let seq_a = PySequence::try_protocol(a, vm);

                if let Ok(seq_a) = seq_a {
                    return seq_a.inplace_concat(b, vm);
                }

                Ok(vm.ctx.not_implemented())
            },
        )
    }

    pub fn _sub(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.binary_op(a, b, PyNumberMethodsOffset::Subtract, "-", |vm, _, _| {
            Ok(vm.ctx.not_implemented())
        })
    }

    pub fn _isub(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.binary_iop(
            a,
            b,
            PyNumberMethodsOffset::InplaceSubtract,
            PyNumberMethodsOffset::Subtract,
            "-=",
            |vm, _, _| Ok(vm.ctx.not_implemented()),
        )
    }

    pub fn _mul(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.binary_op(a, b, PyNumberMethodsOffset::Multiply, "*", |vm, a, b| {
            // TODO: check if PySequence::with_methods can replace try_protocol
            let seq_a = PySequence::try_protocol(a, vm);
            let seq_b = PySequence::try_protocol(b, vm);

            // TODO: I think converting to isize process should be handled in repeat function.
            // TODO: This can be helpful to unify the sequence protocol's closure.

            if let Ok(seq_a) = seq_a {
                let n = b.try_int(vm)?.as_bigint().to_isize().ok_or_else(|| {
                    vm.new_overflow_error("repeated bytes are too long".to_owned())
                })?;

                return seq_a.repeat(n, vm);
            } else if let Ok(seq_b) = seq_b {
                let n = a.try_int(vm)?.as_bigint().to_isize().ok_or_else(|| {
                    vm.new_overflow_error("repeated bytes are too long".to_owned())
                })?;

                return seq_b.repeat(n, vm);
            }

            Ok(vm.ctx.not_implemented())
        })
    }

    pub fn _imul(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.binary_iop(
            a,
            b,
            PyNumberMethodsOffset::InplaceMultiply,
            PyNumberMethodsOffset::Multiply,
            "*=",
            |vm, a, b| {
                // TODO: check if PySequence::with_methods can replace try_protocol
                let seq_a = PySequence::try_protocol(a, vm);
                let seq_b = PySequence::try_protocol(b, vm);

                if let Ok(seq_a) = seq_a {
                    let n = b.try_int(vm)?.as_bigint().to_isize().ok_or_else(|| {
                        vm.new_overflow_error("repeated bytes are too long".to_owned())
                    })?;

                    return seq_a.inplace_repeat(n, vm);
                } else if let Ok(seq_b) = seq_b {
                    let n = a.try_int(vm)?.as_bigint().to_isize().ok_or_else(|| {
                        vm.new_overflow_error("repeated bytes are too long".to_owned())
                    })?;

                    /* Note that the right hand operand should not be
                     * mutated in this case so sq_inplace_repeat is not
                     * used. */
                    return seq_b.repeat(n, vm);
                }

                Ok(vm.ctx.not_implemented())
            },
        )
    }

    pub fn _matmul(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.binary_op(
            a,
            b,
            PyNumberMethodsOffset::MatrixMultiply,
            "@",
            |vm, _, _| Ok(vm.ctx.not_implemented()),
        )
    }

    pub fn _imatmul(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.binary_iop(
            a,
            b,
            PyNumberMethodsOffset::InplaceMatrixMultiply,
            PyNumberMethodsOffset::MatrixMultiply,
            "@=",
            |vm, _, _| Ok(vm.ctx.not_implemented()),
        )
    }

    pub fn _truediv(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.binary_op(a, b, PyNumberMethodsOffset::TrueDivide, "/", |vm, _, _| {
            Ok(vm.ctx.not_implemented())
        })
    }

    pub fn _itruediv(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.binary_iop(
            a,
            b,
            PyNumberMethodsOffset::InplaceTrueDivide,
            PyNumberMethodsOffset::TrueDivide,
            "/=",
            |vm, _, _| Ok(vm.ctx.not_implemented()),
        )
    }

    pub fn _floordiv(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.binary_op(
            a,
            b,
            PyNumberMethodsOffset::FloorDivide,
            "//",
            |vm, _, _| Ok(vm.ctx.not_implemented()),
        )
    }

    pub fn _ifloordiv(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.binary_iop(
            a,
            b,
            PyNumberMethodsOffset::InplaceFloorDivide,
            PyNumberMethodsOffset::FloorDivide,
            "//=",
            |vm, _, _| Ok(vm.ctx.not_implemented()),
        )
    }

    pub fn _pow(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.binary_op(a, b, PyNumberMethodsOffset::Power, "**", |vm, _, _| {
            Ok(vm.ctx.not_implemented())
        })
    }

    pub fn _ipow(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.binary_iop(
            a,
            b,
            PyNumberMethodsOffset::InplacePower,
            PyNumberMethodsOffset::Power,
            "**=",
            |vm, _, _| Ok(vm.ctx.not_implemented()),
        )
    }

    // TODO: `str` modular opertation(mod, imod) is not supported now. Should implement it.
    pub fn _mod(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_reflection(
            a,
            b,
            identifier!(self, __mod__),
            identifier!(self, __rmod__),
            |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "%")),
        )

        // self.binary_op(a, b, PyNumberMethodsOffset::Remainder, "%", |vm, _, _| {
        //     Ok(vm.ctx.not_implemented())
        // })
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

        // self.binary_iop(
        //     a,
        //     b,
        //     PyNumberMethodsOffset::InplaceRemainder,
        //     PyNumberMethodsOffset::Remainder,
        //     "%=",
        //     |vm, _, _| Ok(vm.ctx.not_implemented()),
        // )
    }

    pub fn _divmod(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.binary_op(a, b, PyNumberMethodsOffset::Divmod, "divmod", |vm, _, _| {
            Ok(vm.ctx.not_implemented())
        })
    }

    pub fn _lshift(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.binary_op(a, b, PyNumberMethodsOffset::Lshift, "<<", |vm, _, _| {
            Ok(vm.ctx.not_implemented())
        })
    }

    pub fn _ilshift(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.binary_iop(
            a,
            b,
            PyNumberMethodsOffset::InplaceLshift,
            PyNumberMethodsOffset::Lshift,
            "<<=",
            |vm, _, _| Ok(vm.ctx.not_implemented()),
        )
    }

    pub fn _rshift(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.binary_op(a, b, PyNumberMethodsOffset::Rshift, ">>", |vm, _, _| {
            Ok(vm.ctx.not_implemented())
        })
    }

    pub fn _irshift(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.binary_iop(
            a,
            b,
            PyNumberMethodsOffset::InplaceRshift,
            PyNumberMethodsOffset::Rshift,
            ">>=",
            |vm, _, _| Ok(vm.ctx.not_implemented()),
        )
    }

    pub fn _xor(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.binary_op(a, b, PyNumberMethodsOffset::Xor, "^", |vm, _, _| {
            Ok(vm.ctx.not_implemented())
        })
    }

    pub fn _ixor(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.binary_iop(
            a,
            b,
            PyNumberMethodsOffset::InplaceXor,
            PyNumberMethodsOffset::Xor,
            "^=",
            |vm, _, _| Ok(vm.ctx.not_implemented()),
        )
    }

    // TODO: `or` method doesn't work because of structure of `type_::or_()`.
    // TODO: It should be changed by adjusting with AsNumber.
    pub fn _or(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_reflection(
            a,
            b,
            identifier!(self, __or__),
            identifier!(self, __ror__),
            |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "|")),
        )

        // self.binary_op(a, b, PyNumberMethodsOffset::Or, "|", |vm, _, _| {
        //     Ok(vm.ctx.not_implemented())
        // })
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

        // self.binary_iop(
        //     a,
        //     b,
        //     PyNumberMethodsOffset::InplaceOr,
        //     PyNumberMethodsOffset::Or,
        //     "|=",
        //     |vm, _, _| Ok(vm.ctx.not_implemented()),
        // )
    }

    // TODO: `and` method doesn't work because of structure of `set`.
    // TODO: It should be changed by adjusting with AsNumber.
    pub fn _and(&self, a: &PyObject, b: &PyObject) -> PyResult {
        self.call_or_reflection(
            a,
            b,
            identifier!(self, __and__),
            identifier!(self, __rand__),
            |vm, a, b| Err(vm.new_unsupported_binop_error(a, b, "&")),
        )

        // self.binary_op(a, b, PyNumberMethodsOffset::And, "&", |vm, _, _| {
        //     Ok(vm.ctx.not_implemented())
        // })
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

        // self.binary_iop(
        //     a,
        //     b,
        //     PyNumberMethodsOffset::InplaceAnd,
        //     PyNumberMethodsOffset::And,
        //     "&=",
        //     |vm, _, _| Ok(vm.ctx.not_implemented()),
        // )
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
                if self.bool_eq(&element, &needle)? {
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
