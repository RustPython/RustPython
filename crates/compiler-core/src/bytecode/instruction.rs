use core::{fmt, marker::PhantomData};

use crate::marshal::MarshalError;

use super::{Instruction, OpArg, OpArgByte, OpArgType, Opcode, PseudoInstruction, PseudoOpcode};

impl Opcode {
    /// Map a specialized or instrumented opcode back to its adaptive (base) variant.
    #[must_use]
    pub const fn deoptimize(self) -> Self {
        match self.deopt() {
            Some(v) => v,
            None => {
                // Instrumented opcodes map back to their base
                match self.to_base() {
                    Some(v) => v,
                    None => self,
                }
            }
        }
    }

    /// Returns `true` if this is any instrumented opcode
    /// (regular INSTRUMENTED_*, INSTRUMENTED_LINE, or INSTRUMENTED_INSTRUCTION).
    #[must_use]
    pub const fn is_instrumented(self) -> bool {
        self.to_base().is_some()
            || matches!(self, Self::InstrumentedLine | Self::InstrumentedInstruction)
    }

    #[must_use]
    pub const fn is_unconditional_jump(&self) -> bool {
        matches!(
            self,
            Self::JumpForward | Self::JumpBackward | Self::JumpBackwardNoInterrupt
        )
    }

    #[must_use]
    pub const fn is_scope_exit(&self) -> bool {
        matches!(self, Self::ReturnValue | Self::RaiseVarargs | Self::Reraise)
    }
}

impl Instruction {
    /// Returns `true` if this is any instrumented opcode
    /// (regular INSTRUMENTED_*, INSTRUMENTED_LINE, or INSTRUMENTED_INSTRUCTION).
    #[must_use]
    pub const fn is_instrumented(self) -> bool {
        self.as_opcode().is_instrumented()
    }

    #[must_use]
    pub const fn is_unconditional_jump(&self) -> bool {
        self.as_opcode().is_unconditional_jump()
    }

    #[must_use]
    pub const fn is_scope_exit(&self) -> bool {
        self.as_opcode().is_scope_exit()
    }

    /// Map a specialized or instrumented opcode back to its adaptive (base) variant.
    #[must_use]
    pub const fn deoptimize(self) -> Self {
        self.as_opcode().deoptimize().as_instruction()
    }

    /// Stack effect when the instruction takes its branch (jump=true).
    ///
    /// CPython equivalent: `stack_effect(opcode, oparg, jump=True)`.
    /// For most instructions this equals the fallthrough effect.
    /// Override for instructions where branch and fallthrough differ
    /// (e.g. [`Self::ForIter`]: fallthrough = +1, branch = −1).
    pub fn stack_effect_jump(&self, oparg: u32) -> i32 {
        self.stack_effect(oparg)
    }
}

impl PseudoInstruction {
    /// Returns true if self is one of:
    /// - [`PseudoInstruction::SetupCleanup`]
    /// - [`PseudoInstruction::SetupFinally`]
    /// - [`PseudoInstruction::SetupWith`]
    #[must_use]
    pub const fn is_block_push(&self) -> bool {
        matches!(
            self.as_opcode(),
            PseudoOpcode::SetupCleanup | PseudoOpcode::SetupFinally | PseudoOpcode::SetupWith
        )
    }

    #[must_use]
    pub const fn is_unconditional_jump(&self) -> bool {
        matches!(
            self.as_opcode(),
            PseudoOpcode::Jump | PseudoOpcode::JumpNoInterrupt
        )
    }

    #[must_use]
    pub const fn is_scope_exit(&self) -> bool {
        false
    }

    /// Handler entry effect for SETUP_* pseudo ops.
    ///
    /// Fallthrough effect is 0 (NOPs), but when the branch is taken the
    /// handler block starts with extra values on the stack:
    ///   SETUP_FINALLY:  +1  (exc)
    ///   SETUP_CLEANUP:  +2  (lasti + exc)
    ///   SETUP_WITH:     +1  (pops __enter__ result, pushes lasti + exc)
    pub fn stack_effect_jump(&self, oparg: u32) -> i32 {
        match self {
            Self::SetupFinally { .. } | Self::SetupWith { .. } => 1,
            Self::SetupCleanup { .. } => 2,
            _ => self.stack_effect(oparg),
        }
    }
}

macro_rules! either_real_pseudo {
    // Const
    (
        $(#[$meta:meta])*
        $vis:vis const fn $name:ident(&self $(, $arg:ident : $arg_ty:ty)*) -> $ret:ty
    ) => {
        $(#[$meta])*
        $vis const fn $name(&self $(, $arg: $arg_ty)*) -> $ret {
            match self {
                Self::Real(v) => v.$name($($arg),*),
                Self::Pseudo(v) => v.$name($($arg),*),
            }
        }
    };

    // Not const
    (
        $(#[$meta:meta])*
        $vis:vis fn $name:ident(&self $(, $arg:ident : $arg_ty:ty)*) -> $ret:ty
    ) => {
        $(#[$meta])*
        $vis fn $name(&self $(, $arg: $arg_ty)*) -> $ret {
            match self {
                Self::Real(v) => v.$name($($arg),*),
                Self::Pseudo(v) => v.$name($($arg),*),
            }
        }
    };
}

#[derive(Clone, Copy, Debug)]
pub enum AnyInstruction {
    Real(Instruction),
    Pseudo(PseudoInstruction),
}

impl AnyInstruction {
    either_real_pseudo!(
    #[must_use]
    pub const fn is_unconditional_jump(&self) -> bool
    );

    either_real_pseudo!(
    #[must_use]
    pub const fn is_scope_exit(&self) -> bool
    );

    either_real_pseudo!(
    #[must_use]
    pub fn stack_effect(&self, oparg: u32) -> i32
    );

    either_real_pseudo!(
    #[must_use]
    pub fn stack_effect_jump(&self, oparg: u32) -> i32
    );

    either_real_pseudo!(
    #[must_use]
    pub fn stack_effect_info(&self, oparg: u32) -> StackEffect
    );
}

impl From<Instruction> for AnyInstruction {
    fn from(value: Instruction) -> Self {
        Self::Real(value)
    }
}

impl From<PseudoInstruction> for AnyInstruction {
    fn from(value: PseudoInstruction) -> Self {
        Self::Pseudo(value)
    }
}

impl TryFrom<u8> for AnyInstruction {
    type Error = MarshalError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(Instruction::try_from(value)?.into())
    }
}

impl TryFrom<u16> for AnyInstruction {
    type Error = MarshalError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match u8::try_from(value) {
            Ok(v) => v.try_into(),
            Err(_) => Ok(PseudoInstruction::try_from(value)?.into()),
        }
    }
}

impl From<Opcode> for AnyInstruction {
    fn from(value: Opcode) -> Self {
        Self::Real(value.into())
    }
}

impl From<PseudoOpcode> for AnyInstruction {
    fn from(value: PseudoOpcode) -> Self {
        Self::Pseudo(value.into())
    }
}

impl From<AnyOpcode> for AnyInstruction {
    fn from(value: AnyOpcode) -> Self {
        match value {
            AnyOpcode::Real(op) => op.into(),
            AnyOpcode::Pseudo(op) => op.into(),
        }
    }
}

impl AnyInstruction {
    /// Inner value of [`Self::Real`].
    #[must_use]
    pub const fn real(self) -> Option<Instruction> {
        match self {
            Self::Real(ins) => Some(ins),
            _ => None,
        }
    }

    /// Inner value of [`Self::Pseudo`].
    #[must_use]
    pub const fn pseudo(self) -> Option<PseudoInstruction> {
        match self {
            Self::Pseudo(ins) => Some(ins),
            _ => None,
        }
    }

    /// Get [`Self::Real`] as [`Opcode`].
    #[must_use]
    pub const fn real_opcode(self) -> Option<Opcode> {
        match self.real() {
            Some(ins) => Some(ins.as_opcode()),
            _ => None,
        }
    }

    /// Get [`Self::Pseudo`] as [`PseudoOpcode`].
    #[must_use]
    pub const fn pseudo_opcode(self) -> Option<PseudoOpcode> {
        match self.pseudo() {
            Some(ins) => Some(ins.as_opcode()),
            _ => None,
        }
    }

    /// Same as [`Self::real`] but panics if wasn't called on [`Self::Real`].
    ///
    /// # Panics
    ///
    /// If was called on something else other than [`Self::Real`].
    #[must_use]
    pub const fn expect_real(self) -> Instruction {
        self.real()
            .expect("Expected AnyInstruction::Real, found AnyInstruction::Pseudo")
    }

    /// Same as [`Self::pseudo`] but panics if wasn't called on [`Self::Pseudo`].
    ///
    /// # Panics
    ///
    /// If was called on something else other than [`Self::Pseudo`].
    #[must_use]
    pub const fn expect_pseudo(self) -> PseudoInstruction {
        self.pseudo()
            .expect("Expected AnyInstruction::Pseudo, found AnyInstruction::Real")
    }

    /// Returns true if this is a [`PseudoInstruction::PopBlock`].
    #[must_use]
    pub const fn is_pop_block(self) -> bool {
        matches!(self, Self::Pseudo(PseudoInstruction::PopBlock))
    }

    /// See [`PseudoInstruction::is_block_push`].
    #[must_use]
    pub const fn is_block_push(self) -> bool {
        matches!(self, Self::Pseudo(p) if p.is_block_push())
    }
}

#[derive(Clone, Copy, Debug)]
pub enum AnyOpcode {
    Real(Opcode),
    Pseudo(PseudoOpcode),
}

impl From<Opcode> for AnyOpcode {
    fn from(value: Opcode) -> Self {
        Self::Real(value)
    }
}

impl From<PseudoOpcode> for AnyOpcode {
    fn from(value: PseudoOpcode) -> Self {
        Self::Pseudo(value)
    }
}

impl TryFrom<u8> for AnyOpcode {
    type Error = MarshalError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(Opcode::try_from(value)?.into())
    }
}

impl TryFrom<u16> for AnyOpcode {
    type Error = MarshalError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match u8::try_from(value) {
            Ok(v) => v.try_into(),
            Err(_) => Ok(PseudoOpcode::try_from(value)?.into()),
        }
    }
}

impl From<AnyInstruction> for AnyOpcode {
    fn from(value: AnyInstruction) -> Self {
        match value {
            AnyInstruction::Real(instr) => Self::Real(instr.into()),
            AnyInstruction::Pseudo(instr) => Self::Pseudo(instr.into()),
        }
    }
}

impl AnyOpcode {
    /// Gets the inner value of [`Self::Real`].
    #[must_use]
    pub const fn real(self) -> Option<Opcode> {
        match self {
            Self::Real(op) => Some(op),
            _ => None,
        }
    }

    /// Gets the inner value of [`Self::Pseudo`].
    #[must_use]
    pub const fn pseudo(self) -> Option<PseudoOpcode> {
        match self {
            Self::Pseudo(op) => Some(op),
            _ => None,
        }
    }

    /// Same as [`Self::real`] but panics if wasn't called on [`Self::Real`].
    ///
    /// # Panics
    ///
    /// If was called on something else other than [`Self::Real`].
    #[must_use]
    pub const fn expect_real(self) -> Opcode {
        self.real()
            .expect("Expected AnyOpcode::Real, found AnyOpcode::Pseudo")
    }

    /// Same as [`Self::pseudo`] but panics if wasn't called on [`Self::Pseudo`].
    ///
    /// # Panics
    ///
    /// If was called on something else other than [`Self::Pseudo`].
    #[must_use]
    pub const fn expect_pseudo(self) -> PseudoOpcode {
        self.pseudo()
            .expect("Expected AnyOpcode::Pseudo, found AnyOpcode::Real")
    }
}

/// What effect the instruction has on the stack.
#[derive(Clone, Copy)]
pub struct StackEffect {
    /// How many items the instruction is pushing on the stack.
    pushed: u32,
    /// How many items the instruction is popping from the stack.
    popped: u32,
}

impl StackEffect {
    /// Creates a new [`Self`].
    #[must_use]
    pub const fn new(pushed: u32, popped: u32) -> Self {
        Self { pushed, popped }
    }

    /// Get the calculated stack effect as [`i32`].
    #[must_use]
    pub fn effect(self) -> i32 {
        self.into()
    }

    /// Get the pushed count.
    #[must_use]
    pub const fn pushed(self) -> u32 {
        self.pushed
    }

    /// Get the popped count.
    #[must_use]
    pub const fn popped(self) -> u32 {
        self.popped
    }
}

impl From<StackEffect> for i32 {
    fn from(effect: StackEffect) -> Self {
        (effect.pushed() as i32) - (effect.popped() as i32)
    }
}

#[derive(Copy, Clone)]
pub struct Arg<T: OpArgType>(PhantomData<T>);

impl<T: OpArgType> Arg<T> {
    #[inline]
    #[must_use]
    pub const fn marker() -> Self {
        Self(PhantomData)
    }

    #[inline]
    pub fn new(arg: T) -> (Self, OpArg) {
        (Self(PhantomData), OpArg::new(arg.into()))
    }

    #[inline]
    pub fn new_single(arg: T) -> (Self, OpArgByte)
    where
        T: Into<u8>,
    {
        (Self(PhantomData), OpArgByte::new(arg.into()))
    }

    #[inline(always)]
    #[must_use]
    pub fn get(self, arg: OpArg) -> T {
        self.try_get(arg).unwrap()
    }

    #[inline(always)]
    pub fn try_get(self, arg: OpArg) -> Result<T, MarshalError> {
        T::try_from(u32::from(arg)).map_err(|_| MarshalError::InvalidBytecode)
    }

    /// # Safety
    /// T::from_op_arg(self) must succeed
    #[inline(always)]
    #[must_use]
    pub unsafe fn get_unchecked(self, arg: OpArg) -> T {
        // SAFETY: requirements forwarded from caller
        unsafe { T::try_from(u32::from(arg)).unwrap_unchecked() }
    }
}

impl<T: OpArgType> PartialEq for Arg<T> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<T: OpArgType> Eq for Arg<T> {}

impl<T: OpArgType> fmt::Debug for Arg<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Arg<{}>", core::any::type_name::<T>())
    }
}

// TODO: Can probably remove these asserts and remove the `repr($typ)` from the macro. but this
// breaks the VM:/
const _: () = assert!(core::mem::size_of::<Instruction>() == 1);
const _: () = assert!(core::mem::size_of::<PseudoInstruction>() == 2);
