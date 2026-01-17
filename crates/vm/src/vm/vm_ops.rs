use super::VirtualMachine;
use crate::stdlib::warnings;
use crate::{
    PyRef,
    builtins::{PyInt, PyStr, PyStrRef, PyUtf8Str},
    object::{AsObject, PyObject, PyObjectRef, PyResult},
    protocol::{PyNumberBinaryOp, PyNumberTernaryOp},
    types::PyComparisonOp,
};
use num_traits::ToPrimitive;

macro_rules! binary_func {
    ($fn:ident, $op_slot:ident, $op:expr) => {
        pub fn $fn(&self, a: &PyObject, b: &PyObject) -> PyResult {
            self.binary_op(a, b, PyNumberBinaryOp::$op_slot, $op)
        }
    };
}

macro_rules! ternary_func {
    ($fn:ident, $op_slot:ident, $op:expr) => {
        pub fn $fn(&self, a: &PyObject, b: &PyObject, c: &PyObject) -> PyResult {
            self.ternary_op(a, b, c, PyNumberTernaryOp::$op_slot, $op)
        }
    };
}

macro_rules! inplace_binary_func {
    ($fn:ident, $iop_slot:ident, $op_slot:ident, $op:expr) => {
        pub fn $fn(&self, a: &PyObject, b: &PyObject) -> PyResult {
            self.binary_iop(
                a,
                b,
                PyNumberBinaryOp::$iop_slot,
                PyNumberBinaryOp::$op_slot,
                $op,
            )
        }
    };
}

macro_rules! inplace_ternary_func {
    ($fn:ident, $iop_slot:ident, $op_slot:ident, $op:expr) => {
        pub fn $fn(&self, a: &PyObject, b: &PyObject, c: &PyObject) -> PyResult {
            self.ternary_iop(
                a,
                b,
                c,
                PyNumberTernaryOp::$iop_slot,
                PyNumberTernaryOp::$op_slot,
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
                };
            }
        };
        let hint = result
            .downcast_ref::<PyInt>()
            .ok_or_else(|| {
                self.new_type_error(format!(
                    "'{}' object cannot be interpreted as an integer",
                    result.class().name()
                ))
            })?
            .try_to_primitive::<isize>(self)?;
        if hint.is_negative() {
            Err(self.new_value_error("__length_hint__() should return >= 0"))
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
                Err(self.new_overflow_error("repeated value are too long"))
            } else {
                Ok(n)
            }
        }
    }

    /// Calling scheme used for binary operations:
    ///
    /// Order operations are tried until either a valid result or error:
    ///   `b.rop(b,a)[*], a.op(a,b), b.rop(b,a)`
    ///
    /// `[*]` - only when Py_TYPE(a) != Py_TYPE(b) && Py_TYPE(b) is a subclass of Py_TYPE(a)
    pub fn binary_op1(&self, a: &PyObject, b: &PyObject, op_slot: PyNumberBinaryOp) -> PyResult {
        let class_a = a.class();
        let class_b = b.class();

        // Number slots are inherited, direct access is O(1)
        let slot_a = class_a.slots.as_number.left_binary_op(op_slot);
        let mut slot_b = None;

        if !class_a.is(class_b) {
            let slot_bb = class_b.slots.as_number.right_binary_op(op_slot);
            if slot_bb.map(|x| x as usize) != slot_a.map(|x| x as usize) {
                slot_b = slot_bb;
            }
        }

        if let Some(slot_a) = slot_a {
            if let Some(slot_bb) = slot_b
                && class_b.fast_issubclass(class_a)
            {
                let ret = slot_bb(a, b, self)?;
                if !ret.is(&self.ctx.not_implemented) {
                    return Ok(ret);
                }
                slot_b = None;
            }
            let ret = slot_a(a, b, self)?;
            if !ret.is(&self.ctx.not_implemented) {
                return Ok(ret);
            }
        }

        if let Some(slot_b) = slot_b {
            let ret = slot_b(a, b, self)?;
            if !ret.is(&self.ctx.not_implemented) {
                return Ok(ret);
            }
        }

        Ok(self.ctx.not_implemented())
    }

    pub fn binary_op(
        &self,
        a: &PyObject,
        b: &PyObject,
        op_slot: PyNumberBinaryOp,
        op: &str,
    ) -> PyResult {
        let result = self.binary_op1(a, b, op_slot)?;
        if !result.is(&self.ctx.not_implemented) {
            return Ok(result);
        }
        Err(self.new_unsupported_bin_op_error(a, b, op))
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
        iop_slot: PyNumberBinaryOp,
        op_slot: PyNumberBinaryOp,
    ) -> PyResult {
        if let Some(slot) = a.class().slots.as_number.left_binary_op(iop_slot) {
            let x = slot(a, b, self)?;
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
        iop_slot: PyNumberBinaryOp,
        op_slot: PyNumberBinaryOp,
        op: &str,
    ) -> PyResult {
        let result = self.binary_iop1(a, b, iop_slot, op_slot)?;
        if !result.is(&self.ctx.not_implemented) {
            return Ok(result);
        }
        Err(self.new_unsupported_bin_op_error(a, b, op))
    }

    fn ternary_op(
        &self,
        a: &PyObject,
        b: &PyObject,
        c: &PyObject,
        op_slot: PyNumberTernaryOp,
        op_str: &str,
    ) -> PyResult {
        let class_a = a.class();
        let class_b = b.class();
        let class_c = c.class();

        // Number slots are inherited, direct access is O(1)
        let slot_a = class_a.slots.as_number.left_ternary_op(op_slot);
        let mut slot_b = None;

        if !class_a.is(class_b) {
            let slot_bb = class_b.slots.as_number.right_ternary_op(op_slot);
            if slot_bb.map(|x| x as usize) != slot_a.map(|x| x as usize) {
                slot_b = slot_bb;
            }
        }

        if let Some(slot_a) = slot_a {
            if let Some(slot_bb) = slot_b
                && class_b.fast_issubclass(class_a)
            {
                let ret = slot_bb(a, b, c, self)?;
                if !ret.is(&self.ctx.not_implemented) {
                    return Ok(ret);
                }
                slot_b = None;
            }
            let ret = slot_a(a, b, c, self)?;
            if !ret.is(&self.ctx.not_implemented) {
                return Ok(ret);
            }
        }

        if let Some(slot_b) = slot_b {
            let ret = slot_b(a, b, c, self)?;
            if !ret.is(&self.ctx.not_implemented) {
                return Ok(ret);
            }
        }

        if let Some(slot_c) = class_c.slots.as_number.left_ternary_op(op_slot)
            && slot_a.is_some_and(|slot_a| !core::ptr::fn_addr_eq(slot_a, slot_c))
            && slot_b.is_some_and(|slot_b| !core::ptr::fn_addr_eq(slot_b, slot_c))
        {
            let ret = slot_c(a, b, c, self)?;
            if !ret.is(&self.ctx.not_implemented) {
                return Ok(ret);
            }
        }

        Err(if self.is_none(c) {
            self.new_type_error(format!(
                "unsupported operand type(s) for {}: \
                '{}' and '{}'",
                op_str,
                a.class(),
                b.class()
            ))
        } else {
            self.new_type_error(format!(
                "unsupported operand type(s) for {}: \
                '{}' and '{}', '{}'",
                op_str,
                a.class(),
                b.class(),
                c.class()
            ))
        })
    }

    fn ternary_iop(
        &self,
        a: &PyObject,
        b: &PyObject,
        c: &PyObject,
        iop_slot: PyNumberTernaryOp,
        op_slot: PyNumberTernaryOp,
        op_str: &str,
    ) -> PyResult {
        if let Some(slot) = a.class().slots.as_number.left_ternary_op(iop_slot) {
            let x = slot(a, b, c, self)?;
            if !x.is(&self.ctx.not_implemented) {
                return Ok(x);
            }
        }
        self.ternary_op(a, b, c, op_slot, op_str)
    }

    binary_func!(_sub, Subtract, "-");
    binary_func!(_mod, Remainder, "%");
    binary_func!(_divmod, Divmod, "divmod");
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
    inplace_binary_func!(_ilshift, InplaceLshift, Lshift, "<<=");
    inplace_binary_func!(_irshift, InplaceRshift, Rshift, ">>=");
    inplace_binary_func!(_iand, InplaceAnd, And, "&=");
    inplace_binary_func!(_ixor, InplaceXor, Xor, "^=");
    inplace_binary_func!(_ior, InplaceOr, Or, "|=");
    inplace_binary_func!(_ifloordiv, InplaceFloorDivide, FloorDivide, "//=");
    inplace_binary_func!(_itruediv, InplaceTrueDivide, TrueDivide, "/=");
    inplace_binary_func!(_imatmul, InplaceMatrixMultiply, MatrixMultiply, "@=");

    ternary_func!(_pow, Power, "** or pow()");
    inplace_ternary_func!(_ipow, InplacePower, Power, "**=");

    pub fn _add(&self, a: &PyObject, b: &PyObject) -> PyResult {
        let result = self.binary_op1(a, b, PyNumberBinaryOp::Add)?;
        if !result.is(&self.ctx.not_implemented) {
            return Ok(result);
        }
        // Check if concat slot is available directly, matching PyNumber_Add behavior
        let seq = a.sequence_unchecked();
        if let Some(f) = seq.slots().concat.load() {
            let result = f(seq, b, self)?;
            if !result.is(&self.ctx.not_implemented) {
                return Ok(result);
            }
        }
        Err(self.new_unsupported_bin_op_error(a, b, "+"))
    }

    pub fn _iadd(&self, a: &PyObject, b: &PyObject) -> PyResult {
        let result = self.binary_iop1(a, b, PyNumberBinaryOp::InplaceAdd, PyNumberBinaryOp::Add)?;
        if !result.is(&self.ctx.not_implemented) {
            return Ok(result);
        }
        // Check inplace_concat or concat slot directly, matching PyNumber_InPlaceAdd behavior
        let seq = a.sequence_unchecked();
        let slots = seq.slots();
        if let Some(f) = slots.inplace_concat.load().or_else(|| slots.concat.load()) {
            let result = f(seq, b, self)?;
            if !result.is(&self.ctx.not_implemented) {
                return Ok(result);
            }
        }
        Err(self.new_unsupported_bin_op_error(a, b, "+="))
    }

    pub fn _mul(&self, a: &PyObject, b: &PyObject) -> PyResult {
        let result = self.binary_op1(a, b, PyNumberBinaryOp::Multiply)?;
        if !result.is(&self.ctx.not_implemented) {
            return Ok(result);
        }
        if let Ok(seq_a) = a.try_sequence(self) {
            let n = b
                .try_index(self)?
                .as_bigint()
                .to_isize()
                .ok_or_else(|| self.new_overflow_error("repeated bytes are too long"))?;
            return seq_a.repeat(n, self);
        } else if let Ok(seq_b) = b.try_sequence(self) {
            let n = a
                .try_index(self)?
                .as_bigint()
                .to_isize()
                .ok_or_else(|| self.new_overflow_error("repeated bytes are too long"))?;
            return seq_b.repeat(n, self);
        }
        Err(self.new_unsupported_bin_op_error(a, b, "*"))
    }

    pub fn _imul(&self, a: &PyObject, b: &PyObject) -> PyResult {
        let result = self.binary_iop1(
            a,
            b,
            PyNumberBinaryOp::InplaceMultiply,
            PyNumberBinaryOp::Multiply,
        )?;
        if !result.is(&self.ctx.not_implemented) {
            return Ok(result);
        }
        if let Ok(seq_a) = a.try_sequence(self) {
            let n = b
                .try_index(self)?
                .as_bigint()
                .to_isize()
                .ok_or_else(|| self.new_overflow_error("repeated bytes are too long"))?;
            return seq_a.inplace_repeat(n, self);
        } else if let Ok(seq_b) = b.try_sequence(self) {
            let n = a
                .try_index(self)?
                .as_bigint()
                .to_isize()
                .ok_or_else(|| self.new_overflow_error("repeated bytes are too long"))?;
            /* Note that the right hand operand should not be
             * mutated in this case so inplace_repeat is not
             * used. */
            return seq_b.repeat(n, self);
        }
        Err(self.new_unsupported_bin_op_error(a, b, "*="))
    }

    pub fn _abs(&self, a: &PyObject) -> PyResult<PyObjectRef> {
        self.get_special_method(a, identifier!(self, __abs__))?
            .ok_or_else(|| self.new_unsupported_unary_error(a, "abs()"))?
            .invoke((), self)
    }

    pub fn _pos(&self, a: &PyObject) -> PyResult {
        self.get_special_method(a, identifier!(self, __pos__))?
            .ok_or_else(|| self.new_unsupported_unary_error(a, "unary +"))?
            .invoke((), self)
    }

    pub fn _neg(&self, a: &PyObject) -> PyResult {
        self.get_special_method(a, identifier!(self, __neg__))?
            .ok_or_else(|| self.new_unsupported_unary_error(a, "unary -"))?
            .invoke((), self)
    }

    pub fn _invert(&self, a: &PyObject) -> PyResult {
        const STR: &str = "Bitwise inversion '~' on bool is deprecated and will be removed in Python 3.16. \
            This returns the bitwise inversion of the underlying int object and is usually not what you expect from negating a bool. \
            Use the 'not' operator for boolean negation or ~int(x) if you really want the bitwise inversion of the underlying int.";
        if a.fast_isinstance(self.ctx.types.bool_type) {
            warnings::warn(
                self.ctx.exceptions.deprecation_warning,
                STR.to_owned(),
                1,
                self,
            )?;
        }
        self.get_special_method(a, identifier!(self, __invert__))?
            .ok_or_else(|| self.new_unsupported_unary_error(a, "unary ~"))?
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
            .get_special_method(obj, identifier!(self, __format__))?
            .ok_or_else(|| {
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
    pub fn format_utf8(&self, obj: &PyObject, format_spec: PyStrRef) -> PyResult<PyRef<PyUtf8Str>> {
        self.format(obj, format_spec)?.try_into_utf8(self)
    }

    pub fn _contains(&self, haystack: &PyObject, needle: &PyObject) -> PyResult<bool> {
        let seq = haystack.sequence_unchecked();
        seq.contains(needle, self)
    }
}
