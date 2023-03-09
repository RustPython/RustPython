use super::{PyMethod, VirtualMachine};
use crate::{
    builtins::{PyInt, PyIntRef, PyStr, PyStrRef},
    object::{AsObject, PyObject, PyObjectRef, PyResult},
    protocol::{PyIterReturn, PyNumberBinaryOpSlot, PySequence},
    types::PyComparisonOp,
};
use num_traits::ToPrimitive;

macro_rules! binary_func {
    ($fn:ident, $op_slot:ident, $op:expr) => {
        pub fn $fn(&self, a: &PyObject, b: &PyObject) -> PyResult {
            self.binary_op(a, b, &PyNumberBinaryOpSlot::$op_slot, $op)
        }
    };
}

macro_rules! inplace_binary_func {
    ($fn:ident, $iop_slot:ident, $op_slot:ident, $op:expr) => {
        pub fn $fn(&self, a: &PyObject, b: &PyObject) -> PyResult {
            self.binary_iop(
                a,
                b,
                &PyNumberBinaryOpSlot::$iop_slot,
                &PyNumberBinaryOpSlot::$op_slot,
                $op,
            )
        }
    };
}

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
        let result = match hint.call((), self) {
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

    /// Calling scheme used for binary operations:
    ///
    /// Order operations are tried until either a valid result or error:
    ///   b.rop(b,a)[*], a.op(a,b), b.rop(b,a)
    ///
    /// [*] only when Py_TYPE(a) != Py_TYPE(b) && Py_TYPE(b) is a subclass of Py_TYPE(a)
    pub fn binary_op1(
        &self,
        a: &PyObject,
        b: &PyObject,
        op_slot: &PyNumberBinaryOpSlot,
    ) -> PyResult {
        let slot_a = a.class().slots.number.get_left_binary_op(op_slot)?;
        let mut slot_b = if b.class().is(a.class()) {
            None
        } else {
            match b.class().slots.number.get_right_binary_op(op_slot)? {
                Some(slot_b)
                    if slot_b as usize == slot_a.map(|s| s as usize).unwrap_or_default() =>
                {
                    None
                }
                slot_b => slot_b,
            }
        };

        if let Some(slot_a) = slot_a {
            if let Some(slot_bb) = slot_b {
                if b.fast_isinstance(a.class()) {
                    let x = slot_bb(b.to_number(), a, self)?;
                    if !x.is(&self.ctx.not_implemented) {
                        return Ok(x);
                    }
                    slot_b = None;
                }
            }
            let x = slot_a(a.to_number(), b, self)?;
            if !x.is(&self.ctx.not_implemented) {
                return Ok(x);
            }
        }

        if let Some(slot_b) = slot_b {
            let x = slot_b(b.to_number(), a, self)?;
            if !x.is(&self.ctx.not_implemented) {
                return Ok(x);
            }
        }

        Ok(self.ctx.not_implemented())
    }

    pub fn binary_op(
        &self,
        a: &PyObject,
        b: &PyObject,
        op_slot: &PyNumberBinaryOpSlot,
        op: &str,
    ) -> PyResult {
        let result = self.binary_op1(a, b, op_slot)?;
        if !result.is(&self.ctx.not_implemented) {
            return Ok(result);
        }
        Err(self.new_unsupported_binop_error(a, b, op))
    }

    /// Binary in-place operators
    ///
    /// The in-place operators are defined to fall back to the 'normal',
    /// non in-place operations, if the in-place methods are not in place.
    ///
    /// - If the left hand object has the appropriate struct members, and
    ///   they are filled, call the appropriate function and return the
    ///   result.  No coercion is done on the arguments; the left-hand object
    ///   is the one the operation is performed on, and it's up to the
    ///   function to deal with the right-hand object.
    ///
    /// - Otherwise, in-place modification is not supported. Handle it exactly as
    ///   a non in-place operation of the same kind.
    fn binary_iop1(
        &self,
        a: &PyObject,
        b: &PyObject,
        iop_slot: &PyNumberBinaryOpSlot,
        op_slot: &PyNumberBinaryOpSlot,
    ) -> PyResult {
        if let Some(slot) = a.class().slots.number.get_left_binary_op(iop_slot)? {
            let x = slot(a.to_number(), b, self)?;
            if !x.is(&self.ctx.not_implemented) {
                return Ok(x);
            }
        }
        self.binary_op1(a, b, op_slot)
    }

    fn binary_iop(
        &self,
        a: &PyObject,
        b: &PyObject,
        iop_slot: &PyNumberBinaryOpSlot,
        op_slot: &PyNumberBinaryOpSlot,
        op: &str,
    ) -> PyResult {
        let result = self.binary_iop1(a, b, iop_slot, op_slot)?;
        if !result.is(&self.ctx.not_implemented) {
            return Ok(result);
        }
        Err(self.new_unsupported_binop_error(a, b, op))
    }

    binary_func!(_sub, Subtract, "-");
    binary_func!(_mod, Remainder, "%");
    binary_func!(_divmod, Divmod, "divmod");
    binary_func!(_pow, Power, "**");
    binary_func!(_lshift, Lshift, "<<");
    binary_func!(_rshift, Rshift, ">>");
    binary_func!(_and, And, "&");
    binary_func!(_xor, Xor, "^");
    binary_func!(_or, Or, "|");
    binary_func!(_floordiv, FloorDivide, "//");
    binary_func!(_truediv, TrueDivide, "/");
    binary_func!(_matmul, MatrixMultiply, "@");

    inplace_binary_func!(_isub, InplaceSubtract, Subtract, "-=");
    inplace_binary_func!(_imod, InplaceRemainder, Remainder, "%=");
    inplace_binary_func!(_ipow, InplacePower, Power, "**=");
    inplace_binary_func!(_ilshift, InplaceLshift, Lshift, "<<=");
    inplace_binary_func!(_irshift, InplaceRshift, Rshift, ">>=");
    inplace_binary_func!(_iand, InplaceAnd, And, "&=");
    inplace_binary_func!(_ixor, InplaceXor, Xor, "^=");
    inplace_binary_func!(_ior, InplaceOr, Or, "|=");
    inplace_binary_func!(_ifloordiv, InplaceFloorDivide, FloorDivide, "//=");
    inplace_binary_func!(_itruediv, InplaceTrueDivide, TrueDivide, "/=");
    inplace_binary_func!(_imatmul, InplaceMatrixMultiply, MatrixMultiply, "@=");

    pub fn _add(&self, a: &PyObject, b: &PyObject) -> PyResult {
        let result = self.binary_op1(a, b, &PyNumberBinaryOpSlot::Add)?;
        if !result.is(&self.ctx.not_implemented) {
            return Ok(result);
        }
        if let Ok(seq_a) = PySequence::try_protocol(a, self) {
            let result = seq_a.concat(b, self)?;
            if !result.is(&self.ctx.not_implemented) {
                return Ok(result);
            }
        }
        Err(self.new_unsupported_binop_error(a, b, "+"))
    }

    pub fn _iadd(&self, a: &PyObject, b: &PyObject) -> PyResult {
        let result = self.binary_iop1(
            a,
            b,
            &PyNumberBinaryOpSlot::InplaceAdd,
            &PyNumberBinaryOpSlot::Add,
        )?;
        if !result.is(&self.ctx.not_implemented) {
            return Ok(result);
        }
        if let Ok(seq_a) = PySequence::try_protocol(a, self) {
            let result = seq_a.inplace_concat(b, self)?;
            if !result.is(&self.ctx.not_implemented) {
                return Ok(result);
            }
        }
        Err(self.new_unsupported_binop_error(a, b, "+="))
    }

    pub fn _mul(&self, a: &PyObject, b: &PyObject) -> PyResult {
        let result = self.binary_op1(a, b, &PyNumberBinaryOpSlot::Multiply)?;
        if !result.is(&self.ctx.not_implemented) {
            return Ok(result);
        }
        if let Ok(seq_a) = PySequence::try_protocol(a, self) {
            let n =
                b.try_index(self)?.as_bigint().to_isize().ok_or_else(|| {
                    self.new_overflow_error("repeated bytes are too long".to_owned())
                })?;
            return seq_a.repeat(n, self);
        } else if let Ok(seq_b) = PySequence::try_protocol(b, self) {
            let n =
                a.try_index(self)?.as_bigint().to_isize().ok_or_else(|| {
                    self.new_overflow_error("repeated bytes are too long".to_owned())
                })?;
            return seq_b.repeat(n, self);
        }
        Err(self.new_unsupported_binop_error(a, b, "*"))
    }

    pub fn _imul(&self, a: &PyObject, b: &PyObject) -> PyResult {
        let result = self.binary_iop1(
            a,
            b,
            &PyNumberBinaryOpSlot::InplaceMultiply,
            &PyNumberBinaryOpSlot::Multiply,
        )?;
        if !result.is(&self.ctx.not_implemented) {
            return Ok(result);
        }
        if let Ok(seq_a) = PySequence::try_protocol(a, self) {
            let n =
                b.try_index(self)?.as_bigint().to_isize().ok_or_else(|| {
                    self.new_overflow_error("repeated bytes are too long".to_owned())
                })?;
            return seq_a.inplace_repeat(n, self);
        } else if let Ok(seq_b) = PySequence::try_protocol(b, self) {
            let n =
                a.try_index(self)?.as_bigint().to_isize().ok_or_else(|| {
                    self.new_overflow_error("repeated bytes are too long".to_owned())
                })?;
            /* Note that the right hand operand should not be
             * mutated in this case so inplace_repeat is not
             * used. */
            return seq_b.repeat(n, self);
        }
        Err(self.new_unsupported_binop_error(a, b, "*="))
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

    // PyObject_Format
    pub fn format(&self, obj: &PyObject, format_spec: PyStrRef) -> PyResult<PyStrRef> {
        if format_spec.is_empty() {
            let obj = match obj.to_owned().downcast_exact::<PyStr>(self) {
                Ok(s) => return Ok(s.into_pyref()),
                Err(obj) => obj,
            };
            if obj.class().is(self.ctx.types.int_type) {
                return obj.str(self);
            }
        }
        let bound_format = self
            .get_special_method(obj.to_owned(), identifier!(self, __format__))?
            .map_err(|_| {
                self.new_type_error(format!(
                    "Type {} doesn't define __format__",
                    obj.class().name()
                ))
            })?;
        let formatted = bound_format.invoke((format_spec,), self)?;
        formatted.downcast().map_err(|result| {
            self.new_type_error(format!(
                "__format__ must return a str, not {}",
                &result.class().name()
            ))
        })
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
