//! Implement python as a virtual machine with bytecodes. This module
//! implements bytecode structure.

#![doc(html_logo_url = "https://raw.githubusercontent.com/RustPython/RustPython/main/logo.png")]
#![doc(html_root_url = "https://docs.rs/rustpython-bytecode/")]

use bincode::{Decode, Encode};
use bitflags::bitflags;
use bstr::ByteSlice;
use itertools::Itertools;
use num_bigint::BigInt;
use num_complex::Complex64;
use std::collections::BTreeSet;
use std::{fmt, hash};

/// Sourcecode location.
#[derive(Clone, Copy, Debug, Default, PartialEq, Encode, Decode)]
pub struct Location {
    row: usize,
    column: usize,
}

impl Location {
    /// Creates a new Location object at the given row and column.
    ///
    /// # Example
    /// ```
    /// use rustpython_bytecode::Location;
    /// let loc = Location::new(10, 10);
    /// ```
    pub fn new(row: usize, column: usize) -> Self {
        Location { row, column }
    }

    /// Current row
    pub fn row(&self) -> usize {
        self.row
    }

    /// Current column
    pub fn column(&self) -> usize {
        self.column
    }
}

pub trait Constant: Sized {
    type Name: AsRef<str>;

    /// Transforms the given Constant to a BorrowedConstant
    fn borrow_constant(&self) -> BorrowedConstant<Self>;
    /// Get the data this Constant holds.
    fn into_data(self) -> ConstantData {
        self.borrow_constant().into_data()
    }
    /// Map this Constant to a Bag's constant
    fn map_constant<Bag: ConstantBag>(self, bag: &Bag) -> Bag::Constant {
        bag.make_constant(self.into_data())
    }

    /// Maps the name for the given Bag.
    fn map_name<Bag: ConstantBag>(
        name: Self::Name,
        bag: &Bag,
    ) -> <Bag::Constant as Constant>::Name {
        bag.make_name_ref(name.as_ref())
    }
}

impl Constant for ConstantData {
    type Name = String;
    fn borrow_constant(&self) -> BorrowedConstant<Self> {
        use BorrowedConstant::*;
        match self {
            ConstantData::Integer { value } => Integer { value },
            ConstantData::Float { value } => Float { value: *value },
            ConstantData::Complex { value } => Complex { value: *value },
            ConstantData::Boolean { value } => Boolean { value: *value },
            ConstantData::Str { value } => Str { value },
            ConstantData::Bytes { value } => Bytes { value },
            ConstantData::Code { code } => Code { code },
            ConstantData::Tuple { elements } => Tuple {
                elements: Box::new(elements.iter().map(|e| e.borrow_constant())),
            },
            ConstantData::None => None,
            ConstantData::Ellipsis => Ellipsis,
        }
    }
    fn into_data(self) -> ConstantData {
        self
    }
    fn map_name<Bag: ConstantBag>(name: String, bag: &Bag) -> <Bag::Constant as Constant>::Name {
        bag.make_name(name)
    }
}

/// A Constant Bag
pub trait ConstantBag: Sized {
    type Constant: Constant;
    fn make_constant(&self, constant: ConstantData) -> Self::Constant;
    fn make_constant_borrowed<C: Constant>(&self, constant: BorrowedConstant<C>) -> Self::Constant {
        self.make_constant(constant.into_data())
    }
    fn make_name(&self, name: String) -> <Self::Constant as Constant>::Name;
    fn make_name_ref(&self, name: &str) -> <Self::Constant as Constant>::Name {
        self.make_name(name.to_owned())
    }
}

#[derive(Clone)]
pub struct BasicBag;

impl ConstantBag for BasicBag {
    type Constant = ConstantData;
    fn make_constant(&self, constant: ConstantData) -> Self::Constant {
        constant
    }
    fn make_name(&self, name: String) -> <Self::Constant as Constant>::Name {
        name
    }
}

/// Primary container of a single code object. Each python function has
/// a codeobject. Also a module has a codeobject.
#[derive(Clone)] //, Encode, Decode)]
pub struct CodeObject<C: Constant = ConstantData> {
    pub instructions: Box<[Instruction]>,
    pub locations: Box<[Location]>,
    pub flags: CodeFlags,
    pub posonlyarg_count: usize,
    // Number of positional-only arguments
    pub arg_count: usize,
    pub kwonlyarg_count: usize,
    pub source_path: C::Name,
    pub first_line_number: usize,
    pub max_stacksize: u32,
    pub obj_name: C::Name,
    // Name of the object that created this code object
    pub cell2arg: Option<Box<[isize]>>,
    pub constants: Box<[C]>,
    // #[bincode(bound(
    //     decode = "C::Name: Decode",
    //     encode = "C::Name: Encode"
    // ))]
    pub names: Box<[C::Name]>,
    pub varnames: Box<[C::Name]>,
    pub cellvars: Box<[C::Name]>,
    pub freevars: Box<[C::Name]>,
}

// TODO: this should be derived, https://github.com/bincode-org/bincode/issues/511
impl<C: Constant> ::bincode::Decode for CodeObject<C>
where
    C: ::bincode::Decode,
    C::Name: ::bincode::Decode,
{
    fn decode<D: ::bincode::de::Decoder>(
        decoder: &mut D,
    ) -> core::result::Result<Self, ::bincode::error::DecodeError> {
        Ok(Self {
            instructions: ::bincode::Decode::decode(decoder)?,
            locations: ::bincode::Decode::decode(decoder)?,
            flags: ::bincode::Decode::decode(decoder)?,
            posonlyarg_count: ::bincode::Decode::decode(decoder)?,
            arg_count: ::bincode::Decode::decode(decoder)?,
            kwonlyarg_count: ::bincode::Decode::decode(decoder)?,
            source_path: ::bincode::Decode::decode(decoder)?,
            first_line_number: ::bincode::Decode::decode(decoder)?,
            max_stacksize: ::bincode::Decode::decode(decoder)?,
            obj_name: ::bincode::Decode::decode(decoder)?,
            cell2arg: ::bincode::Decode::decode(decoder)?,
            constants: ::bincode::Decode::decode(decoder)?,
            names: ::bincode::Decode::decode(decoder)?,
            varnames: ::bincode::Decode::decode(decoder)?,
            cellvars: ::bincode::Decode::decode(decoder)?,
            freevars: ::bincode::Decode::decode(decoder)?,
        })
    }
}
impl<C: Constant> ::bincode::Encode for CodeObject<C>
where
    C: ::bincode::Encode,
    C::Name: ::bincode::Encode,
{
    fn encode<E: ::bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> core::result::Result<(), ::bincode::error::EncodeError> {
        ::bincode::Encode::encode(&self.instructions, encoder)?;
        ::bincode::Encode::encode(&self.locations, encoder)?;
        ::bincode::Encode::encode(&self.flags, encoder)?;
        ::bincode::Encode::encode(&self.posonlyarg_count, encoder)?;
        ::bincode::Encode::encode(&self.arg_count, encoder)?;
        ::bincode::Encode::encode(&self.kwonlyarg_count, encoder)?;
        ::bincode::Encode::encode(&self.source_path, encoder)?;
        ::bincode::Encode::encode(&self.first_line_number, encoder)?;
        ::bincode::Encode::encode(&self.max_stacksize, encoder)?;
        ::bincode::Encode::encode(&self.obj_name, encoder)?;
        ::bincode::Encode::encode(&self.cell2arg, encoder)?;
        ::bincode::Encode::encode(&self.constants, encoder)?;
        ::bincode::Encode::encode(&self.names, encoder)?;
        ::bincode::Encode::encode(&self.varnames, encoder)?;
        ::bincode::Encode::encode(&self.cellvars, encoder)?;
        ::bincode::Encode::encode(&self.freevars, encoder)?;
        Ok(())
    }
}

bitflags! {
    // #[derive(Encode, Decode)]
    pub struct CodeFlags: u16 {
        const NEW_LOCALS = 0x01;
        const IS_GENERATOR = 0x02;
        const IS_COROUTINE = 0x04;
        const HAS_VARARGS = 0x08;
        const HAS_VARKEYWORDS = 0x10;
        const IS_OPTIMIZED = 0x20;
    }
}
impl Encode for CodeFlags {
    fn encode<E: ::bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> core::result::Result<(), ::bincode::error::EncodeError> {
        self.bits.encode(encoder)
    }
}
impl Decode for CodeFlags {
    fn decode<D: ::bincode::de::Decoder>(
        decoder: &mut D,
    ) -> core::result::Result<Self, ::bincode::error::DecodeError> {
        Decode::decode(decoder).map(Self::from_bits_truncate)
    }
}

impl CodeFlags {
    pub const NAME_MAPPING: &'static [(&'static str, CodeFlags)] = &[
        ("GENERATOR", CodeFlags::IS_GENERATOR),
        ("COROUTINE", CodeFlags::IS_COROUTINE),
        (
            "ASYNC_GENERATOR",
            Self::from_bits_truncate(Self::IS_GENERATOR.bits | Self::IS_COROUTINE.bits),
        ),
        ("VARARGS", CodeFlags::HAS_VARARGS),
        ("VARKEYWORDS", CodeFlags::HAS_VARKEYWORDS),
    ];
}

#[derive(Encode, Debug, Decode, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
#[repr(transparent)]
// XXX: if you add a new instruction that stores a Label, make sure to add it in
// Instruction::label_arg{,_mut}
pub struct Label(pub u32);

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Transforms a value prior to formatting it.
#[derive(Copy, Clone, Debug, PartialEq, Encode, Decode)]
pub enum ConversionFlag {
    /// No conversion
    None,
    /// Converts by calling `str(<value>)`.
    Str,
    /// Converts by calling `ascii(<value>)`.
    Ascii,
    /// Converts by calling `repr(<value>)`.
    Repr,
}

/// The kind of Raise that occurred.
#[derive(Copy, Clone, Debug, PartialEq, Encode, Decode)]
pub enum RaiseKind {
    Reraise,
    Raise,
    RaiseCause,
}

pub type NameIdx = u32;

/// A Single bytecode instruction.
#[derive(Debug, Clone, PartialEq, Encode, Decode)]
pub enum Instruction {
    /// Importing by name
    ImportName {
        idx: NameIdx,
    },
    /// Importing without name
    ImportNameless,
    /// Import *
    ImportStar,
    /// from ... import ...
    ImportFrom {
        idx: NameIdx,
    },
    LoadFast(NameIdx),
    LoadNameAny(NameIdx),
    LoadGlobal(NameIdx),
    LoadDeref(NameIdx),
    LoadClassDeref(NameIdx),
    StoreFast(NameIdx),
    StoreLocal(NameIdx),
    StoreGlobal(NameIdx),
    StoreDeref(NameIdx),
    DeleteFast(NameIdx),
    DeleteLocal(NameIdx),
    DeleteGlobal(NameIdx),
    DeleteDeref(NameIdx),
    LoadClosure(NameIdx),
    Subscript,
    StoreSubscript,
    DeleteSubscript,
    StoreAttr {
        idx: NameIdx,
    },
    DeleteAttr {
        idx: NameIdx,
    },
    LoadConst {
        /// index into constants vec
        idx: u32,
    },
    UnaryOperation {
        op: UnaryOperator,
    },
    BinaryOperation {
        op: BinaryOperator,
    },
    BinaryOperationInplace {
        op: BinaryOperator,
    },
    LoadAttr {
        idx: NameIdx,
    },
    CompareOperation {
        op: ComparisonOperator,
    },
    Pop,
    Rotate2,
    Rotate3,
    Duplicate,
    Duplicate2,
    GetIter,
    Continue {
        target: Label,
    },
    Break,
    Jump {
        target: Label,
    },
    /// Pop the top of the stack, and jump if this value is true.
    JumpIfTrue {
        target: Label,
    },
    /// Pop the top of the stack, and jump if this value is false.
    JumpIfFalse {
        target: Label,
    },
    /// Peek at the top of the stack, and jump if this value is true.
    /// Otherwise, pop top of stack.
    JumpIfTrueOrPop {
        target: Label,
    },
    /// Peek at the top of the stack, and jump if this value is false.
    /// Otherwise, pop top of stack.
    JumpIfFalseOrPop {
        target: Label,
    },
    MakeFunction(MakeFunctionFlags),
    CallFunctionPositional {
        nargs: u32,
    },
    CallFunctionKeyword {
        nargs: u32,
    },
    CallFunctionEx {
        has_kwargs: bool,
    },
    LoadMethod {
        idx: NameIdx,
    },
    CallMethodPositional {
        nargs: u32,
    },
    CallMethodKeyword {
        nargs: u32,
    },
    CallMethodEx {
        has_kwargs: bool,
    },
    ForIter {
        target: Label,
    },
    ReturnValue,
    YieldValue,
    YieldFrom,
    SetupAnnotation,
    SetupLoop {
        break_target: Label,
    },

    /// Setup a finally handler, which will be called whenever one of this events occurs:
    /// - the block is popped
    /// - the function returns
    /// - an exception is returned
    SetupFinally {
        handler: Label,
    },

    /// Enter a finally block, without returning, excepting, just because we are there.
    EnterFinally,

    /// Marker bytecode for the end of a finally sequence.
    /// When this bytecode is executed, the eval loop does one of those things:
    /// - Continue at a certain bytecode position
    /// - Propagate the exception
    /// - Return from a function
    /// - Do nothing at all, just continue
    EndFinally,

    SetupExcept {
        handler: Label,
    },
    SetupWith {
        end: Label,
    },
    WithCleanupStart,
    WithCleanupFinish,
    PopBlock,
    Raise {
        kind: RaiseKind,
    },
    BuildString {
        size: u32,
    },
    BuildTuple {
        unpack: bool,
        size: u32,
    },
    BuildList {
        unpack: bool,
        size: u32,
    },
    BuildSet {
        unpack: bool,
        size: u32,
    },
    BuildMap {
        unpack: bool,
        for_call: bool,
        size: u32,
    },
    BuildSlice {
        /// whether build a slice with a third step argument
        step: bool,
    },
    ListAppend {
        i: u32,
    },
    SetAdd {
        i: u32,
    },
    MapAdd {
        i: u32,
    },

    PrintExpr,
    LoadBuildClass,
    UnpackSequence {
        size: u32,
    },
    UnpackEx {
        before: u8,
        after: u8,
    },
    FormatValue {
        conversion: ConversionFlag,
    },
    PopException,
    Reverse {
        amount: u32,
    },
    GetAwaitable,
    BeforeAsyncWith,
    SetupAsyncWith {
        end: Label,
    },
    GetAIter,
    GetANext,
    EndAsyncFor,

    /// Reverse order evaluation in MapAdd
    /// required to support named expressions of Python 3.8 in dict comprehension
    /// today (including Py3.9) only required in dict comprehension.
    MapAddRev {
        i: u32,
    },
}

use self::Instruction::*;

bitflags! {
    // #[derive(Encode, Decode)]
    pub struct MakeFunctionFlags: u8 {
        const CLOSURE = 0x01;
        const ANNOTATIONS = 0x02;
        const KW_ONLY_DEFAULTS = 0x04;
        const DEFAULTS = 0x08;
    }
}
impl Encode for MakeFunctionFlags {
    fn encode<E: ::bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> core::result::Result<(), ::bincode::error::EncodeError> {
        self.bits.encode(encoder)
    }
}
impl Decode for MakeFunctionFlags {
    fn decode<D: ::bincode::de::Decoder>(
        decoder: &mut D,
    ) -> core::result::Result<Self, ::bincode::error::DecodeError> {
        Decode::decode(decoder).map(Self::from_bits_truncate)
    }
}

/// A Constant (which usually encapsulates data within it)
///
/// # Examples
/// ```
/// use rustpython_bytecode::ConstantData;
/// let a = ConstantData::Float {value: 120f64};
/// let b = ConstantData::Boolean {value: false};
/// assert_ne!(a, b);
/// ```
#[derive(Debug, Clone)] //, Encode, Decode)]
pub enum ConstantData {
    Tuple { elements: Vec<ConstantData> },
    Integer { value: BigInt },
    Float { value: f64 },
    Complex { value: Complex64 },
    Boolean { value: bool },
    Str { value: String },
    Bytes { value: Vec<u8> },
    Code { code: Box<CodeObject> },
    None,
    Ellipsis,
}

impl ::bincode::Decode for ConstantData {
    fn decode<D: ::bincode::de::Decoder>(
        decoder: &mut D,
    ) -> core::result::Result<Self, ::bincode::error::DecodeError> {
        let variant_index = <u32 as ::bincode::Decode>::decode(decoder)?;
        match variant_index {
            0u32 => Ok(Self::Tuple {
                elements: ::bincode::Decode::decode(decoder)?,
            }),
            1u32 => {
                use num_bigint::Sign;
                let (sign, digits) = Decode::decode(decoder)?;
                let sign = match sign {
                    -1i8 => Sign::Minus,
                    0 => Sign::NoSign,
                    1 => Sign::Plus,
                    _ => {
                        return Err(bincode::error::DecodeError::OtherString(
                            "invalid sign".into(),
                        ))
                    }
                };
                Ok(Self::Integer {
                    value: BigInt::new(sign, digits),
                })
            }
            2u32 => Ok(Self::Float {
                value: ::bincode::Decode::decode(decoder)?,
            }),
            3u32 => {
                let (re, im) = ::bincode::Decode::decode(decoder)?;
                Ok(Self::Complex {
                    value: Complex64 { re, im },
                })
            }
            4u32 => Ok(Self::Boolean {
                value: ::bincode::Decode::decode(decoder)?,
            }),
            5u32 => Ok(Self::Str {
                value: ::bincode::Decode::decode(decoder)?,
            }),
            6u32 => Ok(Self::Bytes {
                value: ::bincode::Decode::decode(decoder)?,
            }),
            7u32 => Ok(Self::Code {
                code: ::bincode::Decode::decode(decoder)?,
            }),
            8u32 => Ok(Self::None {}),
            9u32 => Ok(Self::Ellipsis {}),
            variant => Err(::bincode::error::DecodeError::UnexpectedVariant {
                found: variant,
                type_name: "ConstantData",
                allowed: ::bincode::error::AllowedEnumVariants::Range { min: 0, max: 9 },
            }),
        }
    }
}
impl ::bincode::Encode for ConstantData {
    fn encode<E: ::bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> core::result::Result<(), ::bincode::error::EncodeError> {
        match self {
            Self::Tuple { elements } => {
                <u32 as ::bincode::Encode>::encode(&(0u32), encoder)?;
                ::bincode::Encode::encode(elements, encoder)?;
                Ok(())
            }
            Self::Integer { value } => {
                <u32 as ::bincode::Encode>::encode(&(1u32), encoder)?;
                let (sign, digits) = value.to_u32_digits();
                use num_bigint::Sign;
                let sign = match sign {
                    Sign::Minus => -1i8,
                    Sign::NoSign => 0,
                    Sign::Plus => 1,
                };
                (sign, digits).encode(encoder)?;
                Ok(())
            }
            Self::Float { value } => {
                <u32 as ::bincode::Encode>::encode(&(2u32), encoder)?;
                ::bincode::Encode::encode(value, encoder)?;
                Ok(())
            }
            Self::Complex { value } => {
                <u32 as ::bincode::Encode>::encode(&(3u32), encoder)?;
                (value.re, value.im).encode(encoder)?;
                Ok(())
            }
            Self::Boolean { value } => {
                <u32 as ::bincode::Encode>::encode(&(4u32), encoder)?;
                ::bincode::Encode::encode(value, encoder)?;
                Ok(())
            }
            Self::Str { value } => {
                <u32 as ::bincode::Encode>::encode(&(5u32), encoder)?;
                ::bincode::Encode::encode(value, encoder)?;
                Ok(())
            }
            Self::Bytes { value } => {
                <u32 as ::bincode::Encode>::encode(&(6u32), encoder)?;
                ::bincode::Encode::encode(value, encoder)?;
                Ok(())
            }
            Self::Code { code } => {
                <u32 as ::bincode::Encode>::encode(&(7u32), encoder)?;
                ::bincode::Encode::encode(code, encoder)?;
                Ok(())
            }
            Self::None => {
                <u32 as ::bincode::Encode>::encode(&(8u32), encoder)?;
                Ok(())
            }
            Self::Ellipsis => {
                <u32 as ::bincode::Encode>::encode(&(9u32), encoder)?;
                Ok(())
            }
        }
    }
}

impl PartialEq for ConstantData {
    fn eq(&self, other: &Self) -> bool {
        use ConstantData::*;
        match (self, other) {
            (Integer { value: a }, Integer { value: b }) => a == b,
            // we want to compare floats *by actual value* - if we have the *exact same* float
            // already in a constant cache, we want to use that
            (Float { value: a }, Float { value: b }) => a.to_bits() == b.to_bits(),
            (Complex { value: a }, Complex { value: b }) => {
                a.re.to_bits() == b.re.to_bits() && a.im.to_bits() == b.im.to_bits()
            }
            (Boolean { value: a }, Boolean { value: b }) => a == b,
            (Str { value: a }, Str { value: b }) => a == b,
            (Bytes { value: a }, Bytes { value: b }) => a == b,
            (Code { code: a }, Code { code: b }) => std::ptr::eq(a.as_ref(), b.as_ref()),
            (Tuple { elements: a }, Tuple { elements: b }) => a == b,
            (None, None) => true,
            (Ellipsis, Ellipsis) => true,
            _ => false,
        }
    }
}

impl Eq for ConstantData {}

impl hash::Hash for ConstantData {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        use ConstantData::*;
        std::mem::discriminant(self).hash(state);
        match self {
            Integer { value } => value.hash(state),
            Float { value } => value.to_bits().hash(state),
            Complex { value } => {
                value.re.to_bits().hash(state);
                value.im.to_bits().hash(state);
            }
            Boolean { value } => value.hash(state),
            Str { value } => value.hash(state),
            Bytes { value } => value.hash(state),
            Code { code } => std::ptr::hash(code.as_ref(), state),
            Tuple { elements } => elements.hash(state),
            None => {}
            Ellipsis => {}
        }
    }
}

/// A borrowed Constant
pub enum BorrowedConstant<'a, C: Constant> {
    Integer { value: &'a BigInt },
    Float { value: f64 },
    Complex { value: Complex64 },
    Boolean { value: bool },
    Str { value: &'a str },
    Bytes { value: &'a [u8] },
    Code { code: &'a CodeObject<C> },
    Tuple { elements: BorrowedTupleIter<'a, C> },
    None,
    Ellipsis,
}

type BorrowedTupleIter<'a, C> = Box<dyn Iterator<Item = BorrowedConstant<'a, C>> + 'a>;

impl<C: Constant> BorrowedConstant<'_, C> {
    // takes `self` because we need to consume the iterator
    pub fn fmt_display(self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            BorrowedConstant::Integer { value } => write!(f, "{}", value),
            BorrowedConstant::Float { value } => write!(f, "{}", value),
            BorrowedConstant::Complex { value } => write!(f, "{}", value),
            BorrowedConstant::Boolean { value } => {
                write!(f, "{}", if value { "True" } else { "False" })
            }
            BorrowedConstant::Str { value } => write!(f, "{:?}", value),
            BorrowedConstant::Bytes { value } => write!(f, "b{:?}", value.as_bstr()),
            BorrowedConstant::Code { code } => write!(f, "{:?}", code),
            BorrowedConstant::Tuple { elements } => {
                write!(f, "(")?;
                let mut first = true;
                for c in elements {
                    if first {
                        first = false
                    } else {
                        write!(f, ", ")?;
                    }
                    c.fmt_display(f)?;
                }
                write!(f, ")")
            }
            BorrowedConstant::None => write!(f, "None"),
            BorrowedConstant::Ellipsis => write!(f, "..."),
        }
    }
    pub fn into_data(self) -> ConstantData {
        use ConstantData::*;
        match self {
            BorrowedConstant::Integer { value } => Integer {
                value: value.clone(),
            },
            BorrowedConstant::Float { value } => Float { value },
            BorrowedConstant::Complex { value } => Complex { value },
            BorrowedConstant::Boolean { value } => Boolean { value },
            BorrowedConstant::Str { value } => Str {
                value: value.to_owned(),
            },
            BorrowedConstant::Bytes { value } => Bytes {
                value: value.to_owned(),
            },
            BorrowedConstant::Code { code } => Code {
                code: Box::new(code.map_clone_bag(&BasicBag)),
            },
            BorrowedConstant::Tuple { elements } => Tuple {
                elements: elements.map(BorrowedConstant::into_data).collect(),
            },
            BorrowedConstant::None => None,
            BorrowedConstant::Ellipsis => Ellipsis,
        }
    }
}

/// The possible comparison operators
#[derive(Debug, Copy, Clone, PartialEq, Encode, Decode)]
pub enum ComparisonOperator {
    Greater,
    GreaterOrEqual,
    Less,
    LessOrEqual,
    Equal,
    NotEqual,
    In,
    NotIn,
    Is,
    IsNot,
    /// two exceptions that match?
    ExceptionMatch,
}

/// The possible Binary operators
/// # Examples
///
/// ```
/// use rustpython_bytecode::Instruction::BinaryOperation;
/// use rustpython_bytecode::BinaryOperator::Add;
/// let op = BinaryOperation {op: Add};
/// ```
#[derive(Debug, Copy, Clone, PartialEq, Encode, Decode)]
pub enum BinaryOperator {
    Power,
    Multiply,
    MatrixMultiply,
    Divide,
    FloorDivide,
    Modulo,
    Add,
    Subtract,
    Lshift,
    Rshift,
    And,
    Xor,
    Or,
}

/// The possible unary operators
#[derive(Debug, Copy, Clone, PartialEq, Encode, Decode)]
pub enum UnaryOperator {
    Not,
    Invert,
    Minus,
    Plus,
}

/*
Maintain a stack of blocks on the VM.
pub enum BlockType {
    Loop,
    Except,
}
*/

/// Argument structure
pub struct Arguments<'a, N: AsRef<str>> {
    pub posonlyargs: &'a [N],
    pub args: &'a [N],
    pub vararg: Option<&'a N>,
    pub kwonlyargs: &'a [N],
    pub varkwarg: Option<&'a N>,
}

impl<N: AsRef<str>> fmt::Debug for Arguments<'_, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        macro_rules! fmt_slice {
            ($x:expr) => {
                format_args!("[{}]", $x.iter().map(AsRef::as_ref).format(", "))
            };
        }
        f.debug_struct("Arguments")
            .field("posonlyargs", &fmt_slice!(self.posonlyargs))
            .field("args", &fmt_slice!(self.posonlyargs))
            .field("vararg", &self.vararg.map(N::as_ref))
            .field("kwonlyargs", &fmt_slice!(self.kwonlyargs))
            .field("varkwarg", &self.varkwarg.map(N::as_ref))
            .finish()
    }
}

impl<C: Constant> CodeObject<C> {
    /// Get all arguments of the code object
    /// like inspect.getargs
    pub fn arg_names(&self) -> Arguments<C::Name> {
        let nargs = self.arg_count;
        let nkwargs = self.kwonlyarg_count;
        let mut varargspos = nargs + nkwargs;
        let posonlyargs = &self.varnames[..self.posonlyarg_count];
        let args = &self.varnames[..nargs];
        let kwonlyargs = &self.varnames[nargs..varargspos];

        let vararg = if self.flags.contains(CodeFlags::HAS_VARARGS) {
            let vararg = &self.varnames[varargspos];
            varargspos += 1;
            Some(vararg)
        } else {
            None
        };
        let varkwarg = if self.flags.contains(CodeFlags::HAS_VARKEYWORDS) {
            Some(&self.varnames[varargspos])
        } else {
            None
        };

        Arguments {
            posonlyargs,
            args,
            vararg,
            kwonlyargs,
            varkwarg,
        }
    }

    /// Return the labels targeted by the instructions of this CodeObject
    pub fn label_targets(&self) -> BTreeSet<Label> {
        let mut label_targets = BTreeSet::new();
        for instruction in &*self.instructions {
            if let Some(l) = instruction.label_arg() {
                label_targets.insert(*l);
            }
        }
        label_targets
    }

    fn display_inner(
        &self,
        f: &mut fmt::Formatter,
        expand_codeobjects: bool,
        level: usize,
    ) -> fmt::Result {
        let label_targets = self.label_targets();

        for (offset, instruction) in self.instructions.iter().enumerate() {
            let arrow = if label_targets.contains(&Label(offset as u32)) {
                ">>"
            } else {
                "  "
            };
            for _ in 0..level {
                write!(f, "          ")?;
            }
            write!(f, "{} {:5} ", arrow, offset)?;
            instruction.fmt_dis(
                f,
                &self.constants,
                &self.names,
                &self.varnames,
                &self.cellvars,
                &self.freevars,
                expand_codeobjects,
                level,
            )?;
        }
        Ok(())
    }

    /// Recursively display this CodeObject
    pub fn display_expand_codeobjects(&self) -> impl fmt::Display + '_ {
        struct Display<'a, C: Constant>(&'a CodeObject<C>);
        impl<C: Constant> fmt::Display for Display<'_, C> {
            fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                self.0.display_inner(f, true, 1)
            }
        }
        Display(self)
    }

    /// Map this CodeObject to one that holds a Bag::Constant
    pub fn map_bag<Bag: ConstantBag>(self, bag: &Bag) -> CodeObject<Bag::Constant> {
        let map_names = |names: Box<[C::Name]>| {
            names
                .into_vec()
                .into_iter()
                .map(|x| C::map_name(x, bag))
                .collect::<Box<[_]>>()
        };
        CodeObject {
            constants: self
                .constants
                .into_vec()
                .into_iter()
                .map(|x| x.map_constant(bag))
                .collect(),
            names: map_names(self.names),
            varnames: map_names(self.varnames),
            cellvars: map_names(self.cellvars),
            freevars: map_names(self.freevars),
            source_path: C::map_name(self.source_path, bag),
            obj_name: C::map_name(self.obj_name, bag),

            instructions: self.instructions,
            locations: self.locations,
            flags: self.flags,
            posonlyarg_count: self.posonlyarg_count,
            arg_count: self.arg_count,
            kwonlyarg_count: self.kwonlyarg_count,
            first_line_number: self.first_line_number,
            max_stacksize: self.max_stacksize,
            cell2arg: self.cell2arg,
        }
    }

    /// Same as `map_bag` but clones `self`
    pub fn map_clone_bag<Bag: ConstantBag>(&self, bag: &Bag) -> CodeObject<Bag::Constant> {
        let map_names = |names: &[C::Name]| {
            names
                .iter()
                .map(|x| bag.make_name_ref(x.as_ref()))
                .collect()
        };
        CodeObject {
            constants: self
                .constants
                .iter()
                .map(|x| bag.make_constant_borrowed(x.borrow_constant()))
                .collect(),
            names: map_names(&self.names),
            varnames: map_names(&self.varnames),
            cellvars: map_names(&self.cellvars),
            freevars: map_names(&self.freevars),
            source_path: bag.make_name_ref(self.source_path.as_ref()),
            obj_name: bag.make_name_ref(self.obj_name.as_ref()),

            instructions: self.instructions.clone(),
            locations: self.locations.clone(),
            flags: self.flags,
            posonlyarg_count: self.posonlyarg_count,
            arg_count: self.arg_count,
            kwonlyarg_count: self.kwonlyarg_count,
            first_line_number: self.first_line_number,
            max_stacksize: self.max_stacksize,
            cell2arg: self.cell2arg.clone(),
        }
    }
}

/// Error that occurs during code deserialization
#[derive(Debug)]
#[non_exhaustive]
pub enum CodeDeserializeError {
    /// Unexpected End Of File
    Eof,
    /// Invalid Bytecode
    Other,
}

impl fmt::Display for CodeDeserializeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Eof => f.write_str("unexpected end of data"),
            Self::Other => f.write_str("invalid bytecode"),
        }
    }
}

impl std::error::Error for CodeDeserializeError {}

impl CodeObject<ConstantData> {
    /// Load a code object from bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self, CodeDeserializeError> {
        use lz4_flex::block::DecompressError;
        let raw_bincode = lz4_flex::decompress_size_prepended(data).map_err(|e| match e {
            DecompressError::OutputTooSmall { .. } | DecompressError::ExpectedAnotherByte => {
                CodeDeserializeError::Eof
            }
            _ => CodeDeserializeError::Other,
        })?;
        let (data, _) =
            bincode::decode_from_slice(&raw_bincode, bincode_config()).map_err(|e| match e {
                bincode::error::DecodeError::UnexpectedEnd => CodeDeserializeError::Eof,
                _ => CodeDeserializeError::Other,
            })?;
        Ok(data)
    }

    /// Encode this bytecode to bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let data =
            bincode::encode_to_vec(self, bincode_config()).expect("CodeObject is not serializable");
        lz4_flex::compress_prepend_size(&data)
    }
}
// to keep it consistent between enc/dec
#[inline(always)]
fn bincode_config() -> impl bincode::config::Config {
    bincode::config::legacy()
}

impl<C: Constant> fmt::Display for CodeObject<C> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.display_inner(f, false, 1)?;
        for constant in &*self.constants {
            if let BorrowedConstant::Code { code } = constant.borrow_constant() {
                writeln!(f, "\nDisassembly of {:?}", code)?;
                code.fmt(f)?;
            }
        }
        Ok(())
    }
}

impl Instruction {
    /// Gets the label stored inside this instruction, if it exists
    #[inline]
    pub fn label_arg(&self) -> Option<&Label> {
        match self {
            Jump { target: l }
            | JumpIfTrue { target: l }
            | JumpIfFalse { target: l }
            | JumpIfTrueOrPop { target: l }
            | JumpIfFalseOrPop { target: l }
            | ForIter { target: l }
            | SetupFinally { handler: l }
            | SetupExcept { handler: l }
            | SetupWith { end: l }
            | SetupAsyncWith { end: l }
            | SetupLoop { break_target: l }
            | Continue { target: l } => Some(l),
            _ => None,
        }
    }

    /// Gets a mutable reference to the label stored inside this instruction, if it exists
    #[inline]
    pub fn label_arg_mut(&mut self) -> Option<&mut Label> {
        match self {
            Jump { target: l }
            | JumpIfTrue { target: l }
            | JumpIfFalse { target: l }
            | JumpIfTrueOrPop { target: l }
            | JumpIfFalseOrPop { target: l }
            | ForIter { target: l }
            | SetupFinally { handler: l }
            | SetupExcept { handler: l }
            | SetupWith { end: l }
            | SetupAsyncWith { end: l }
            | SetupLoop { break_target: l }
            | Continue { target: l } => Some(l),
            _ => None,
        }
    }

    /// Whether this is an unconditional branching
    ///
    /// # Examples
    ///
    /// ```
    /// use rustpython_bytecode::{Instruction, Label};
    /// let label = Label(0xF);
    /// let jump_inst = Instruction::Jump {target: label};
    /// assert!(jump_inst.unconditional_branch())
    /// ```
    pub fn unconditional_branch(&self) -> bool {
        matches!(
            self,
            Jump { .. } | Continue { .. } | Break | ReturnValue | Raise { .. }
        )
    }

    /// What effect this instruction has on the stack
    ///
    /// # Examples
    ///
    /// ```
    /// use rustpython_bytecode::{Instruction, Label, UnaryOperator};
    /// let jump_instruction = Instruction::Jump {target: Label(0xF)};
    /// let invert_instruction = Instruction::UnaryOperation {op: UnaryOperator::Invert};
    /// assert_eq!(jump_instruction.stack_effect(true), 0);
    /// assert_eq!(invert_instruction.stack_effect(false), 0);
    /// ```
    ///
    pub fn stack_effect(&self, jump: bool) -> i32 {
        match self {
            ImportName { .. } | ImportNameless => -1,
            ImportStar => -1,
            ImportFrom { .. } => 1,
            LoadFast(_) | LoadNameAny(_) | LoadGlobal(_) | LoadDeref(_) | LoadClassDeref(_) => 1,
            StoreFast(_) | StoreLocal(_) | StoreGlobal(_) | StoreDeref(_) => -1,
            DeleteFast(_) | DeleteLocal(_) | DeleteGlobal(_) | DeleteDeref(_) => 0,
            LoadClosure(_) => 1,
            Subscript => -1,
            StoreSubscript => -3,
            DeleteSubscript => -2,
            LoadAttr { .. } => 0,
            StoreAttr { .. } => -2,
            DeleteAttr { .. } => -1,
            LoadConst { .. } => 1,
            UnaryOperation { .. } => 0,
            BinaryOperation { .. } | BinaryOperationInplace { .. } | CompareOperation { .. } => -1,
            Pop => -1,
            Rotate2 | Rotate3 => 0,
            Duplicate => 1,
            Duplicate2 => 2,
            GetIter => 0,
            Continue { .. } => 0,
            Break => 0,
            Jump { .. } => 0,
            JumpIfTrue { .. } | JumpIfFalse { .. } => -1,
            JumpIfTrueOrPop { .. } | JumpIfFalseOrPop { .. } => {
                if jump {
                    0
                } else {
                    -1
                }
            }
            MakeFunction(flags) => {
                -2 - flags.contains(MakeFunctionFlags::CLOSURE) as i32
                    - flags.contains(MakeFunctionFlags::ANNOTATIONS) as i32
                    - flags.contains(MakeFunctionFlags::KW_ONLY_DEFAULTS) as i32
                    - flags.contains(MakeFunctionFlags::DEFAULTS) as i32
                    + 1
            }
            CallFunctionPositional { nargs } => -(*nargs as i32) - 1 + 1,
            CallMethodPositional { nargs } => -(*nargs as i32) - 3 + 1,
            CallFunctionKeyword { nargs } => -1 - (*nargs as i32) - 1 + 1,
            CallMethodKeyword { nargs } => -1 - (*nargs as i32) - 3 + 1,
            CallFunctionEx { has_kwargs } => -1 - (*has_kwargs as i32) - 1 + 1,
            CallMethodEx { has_kwargs } => -1 - (*has_kwargs as i32) - 3 + 1,
            LoadMethod { .. } => -1 + 3,
            ForIter { .. } => {
                if jump {
                    -1
                } else {
                    1
                }
            }
            ReturnValue => -1,
            YieldValue => 0,
            YieldFrom => -1,
            SetupAnnotation
            | SetupLoop { .. }
            | SetupFinally { .. }
            | EnterFinally
            | EndFinally => 0,
            SetupExcept { .. } => {
                if jump {
                    1
                } else {
                    0
                }
            }
            SetupWith { .. } => {
                if jump {
                    0
                } else {
                    1
                }
            }
            WithCleanupStart => 0,
            WithCleanupFinish => -1,
            PopBlock => 0,
            Raise { kind } => -(*kind as u8 as i32),
            BuildString { size }
            | BuildTuple { size, .. }
            | BuildList { size, .. }
            | BuildSet { size, .. } => -(*size as i32) + 1,
            BuildMap { unpack, size, .. } => {
                let nargs = if *unpack { *size } else { *size * 2 };
                -(nargs as i32) + 1
            }
            BuildSlice { step } => -2 - (*step as i32) + 1,
            ListAppend { .. } | SetAdd { .. } => -1,
            MapAdd { .. } | MapAddRev { .. } => -2,
            PrintExpr => -1,
            LoadBuildClass => 1,
            UnpackSequence { size } => -1 + *size as i32,
            UnpackEx { before, after } => -1 + *before as i32 + 1 + *after as i32,
            FormatValue { .. } => -1,
            PopException => 0,
            Reverse { .. } => 0,
            GetAwaitable => 0,
            BeforeAsyncWith => 1,
            SetupAsyncWith { .. } => {
                if jump {
                    -1
                } else {
                    0
                }
            }
            GetAIter => 0,
            GetANext => 1,
            EndAsyncFor => -2,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn fmt_dis<C: Constant>(
        &self,
        f: &mut fmt::Formatter,
        constants: &[C],
        names: &[C::Name],
        varnames: &[C::Name],
        cellvars: &[C::Name],
        freevars: &[C::Name],
        expand_codeobjects: bool,
        level: usize,
    ) -> fmt::Result {
        macro_rules! w {
            ($variant:ident) => {
                writeln!(f, stringify!($variant))
            };
            ($variant:ident, $var:expr) => {
                writeln!(f, "{:20} ({})", stringify!($variant), $var)
            };
            ($variant:ident, $var1:expr, $var2:expr) => {
                writeln!(f, "{:20} ({}, {})", stringify!($variant), $var1, $var2)
            };
            ($variant:ident, $var1:expr, $var2:expr, $var3:expr) => {
                writeln!(
                    f,
                    "{:20} ({}, {}, {})",
                    stringify!($variant),
                    $var1,
                    $var2,
                    $var3
                )
            };
        }

        let varname = |i: u32| varnames[i as usize].as_ref();
        let name = |i: u32| names[i as usize].as_ref();
        let cellname = |i: u32| {
            cellvars
                .get(i as usize)
                .unwrap_or_else(|| &freevars[i as usize - cellvars.len()])
                .as_ref()
        };

        match self {
            ImportName { idx } => w!(ImportName, name(*idx)),
            ImportNameless => w!(ImportNameless),
            ImportStar => w!(ImportStar),
            ImportFrom { idx } => w!(ImportFrom, name(*idx)),
            LoadFast(idx) => w!(LoadFast, *idx, varname(*idx)),
            LoadNameAny(idx) => w!(LoadNameAny, *idx, name(*idx)),
            LoadGlobal(idx) => w!(LoadGlobal, *idx, name(*idx)),
            LoadDeref(idx) => w!(LoadDeref, *idx, cellname(*idx)),
            LoadClassDeref(idx) => w!(LoadClassDeref, *idx, cellname(*idx)),
            StoreFast(idx) => w!(StoreFast, *idx, varname(*idx)),
            StoreLocal(idx) => w!(StoreLocal, *idx, name(*idx)),
            StoreGlobal(idx) => w!(StoreGlobal, *idx, name(*idx)),
            StoreDeref(idx) => w!(StoreDeref, *idx, cellname(*idx)),
            DeleteFast(idx) => w!(DeleteFast, *idx, varname(*idx)),
            DeleteLocal(idx) => w!(DeleteLocal, *idx, name(*idx)),
            DeleteGlobal(idx) => w!(DeleteGlobal, *idx, name(*idx)),
            DeleteDeref(idx) => w!(DeleteDeref, *idx, cellname(*idx)),
            LoadClosure(i) => w!(LoadClosure, *i, cellname(*i)),
            Subscript => w!(Subscript),
            StoreSubscript => w!(StoreSubscript),
            DeleteSubscript => w!(DeleteSubscript),
            StoreAttr { idx } => w!(StoreAttr, name(*idx)),
            DeleteAttr { idx } => w!(DeleteAttr, name(*idx)),
            LoadConst { idx } => {
                let value = &constants[*idx as usize];
                match value.borrow_constant() {
                    BorrowedConstant::Code { code } if expand_codeobjects => {
                        writeln!(f, "{:20} ({:?}):", "LoadConst", code)?;
                        code.display_inner(f, true, level + 1)?;
                        Ok(())
                    }
                    c => {
                        write!(f, "{:20} (", "LoadConst")?;
                        c.fmt_display(f)?;
                        writeln!(f, ")")
                    }
                }
            }
            UnaryOperation { op } => w!(UnaryOperation, format_args!("{:?}", op)),
            BinaryOperation { op } => w!(BinaryOperation, format_args!("{:?}", op)),
            BinaryOperationInplace { op } => {
                w!(BinaryOperationInplace, format_args!("{:?}", op))
            }
            LoadAttr { idx } => w!(LoadAttr, name(*idx)),
            CompareOperation { op } => w!(CompareOperation, format_args!("{:?}", op)),
            Pop => w!(Pop),
            Rotate2 => w!(Rotate2),
            Rotate3 => w!(Rotate3),
            Duplicate => w!(Duplicate),
            Duplicate2 => w!(Duplicate2),
            GetIter => w!(GetIter),
            Continue { target } => w!(Continue, target),
            Break => w!(Break),
            Jump { target } => w!(Jump, target),
            JumpIfTrue { target } => w!(JumpIfTrue, target),
            JumpIfFalse { target } => w!(JumpIfFalse, target),
            JumpIfTrueOrPop { target } => w!(JumpIfTrueOrPop, target),
            JumpIfFalseOrPop { target } => w!(JumpIfFalseOrPop, target),
            MakeFunction(flags) => w!(MakeFunction, format_args!("{:?}", flags)),
            CallFunctionPositional { nargs } => w!(CallFunctionPositional, nargs),
            CallFunctionKeyword { nargs } => w!(CallFunctionKeyword, nargs),
            CallFunctionEx { has_kwargs } => w!(CallFunctionEx, has_kwargs),
            LoadMethod { idx } => w!(LoadMethod, name(*idx)),
            CallMethodPositional { nargs } => w!(CallMethodPositional, nargs),
            CallMethodKeyword { nargs } => w!(CallMethodKeyword, nargs),
            CallMethodEx { has_kwargs } => w!(CallMethodEx, has_kwargs),
            ForIter { target } => w!(ForIter, target),
            ReturnValue => w!(ReturnValue),
            YieldValue => w!(YieldValue),
            YieldFrom => w!(YieldFrom),
            SetupAnnotation => w!(SetupAnnotation),
            SetupLoop { break_target } => w!(SetupLoop, break_target),
            SetupExcept { handler } => w!(SetupExcept, handler),
            SetupFinally { handler } => w!(SetupFinally, handler),
            EnterFinally => w!(EnterFinally),
            EndFinally => w!(EndFinally),
            SetupWith { end } => w!(SetupWith, end),
            WithCleanupStart => w!(WithCleanupStart),
            WithCleanupFinish => w!(WithCleanupFinish),
            BeforeAsyncWith => w!(BeforeAsyncWith),
            SetupAsyncWith { end } => w!(SetupAsyncWith, end),
            PopBlock => w!(PopBlock),
            Raise { kind } => w!(Raise, format_args!("{:?}", kind)),
            BuildString { size } => w!(BuildString, size),
            BuildTuple { size, unpack } => w!(BuildTuple, size, unpack),
            BuildList { size, unpack } => w!(BuildList, size, unpack),
            BuildSet { size, unpack } => w!(BuildSet, size, unpack),
            BuildMap {
                size,
                unpack,
                for_call,
            } => w!(BuildMap, size, unpack, for_call),
            BuildSlice { step } => w!(BuildSlice, step),
            ListAppend { i } => w!(ListAppend, i),
            SetAdd { i } => w!(SetAdd, i),
            MapAddRev { i } => w!(MapAddRev, i),
            PrintExpr => w!(PrintExpr),
            LoadBuildClass => w!(LoadBuildClass),
            UnpackSequence { size } => w!(UnpackSequence, size),
            UnpackEx { before, after } => w!(UnpackEx, before, after),
            FormatValue { conversion } => w!(FormatValue, format_args!("{:?}", conversion)),
            PopException => w!(PopException),
            Reverse { amount } => w!(Reverse, amount),
            GetAwaitable => w!(GetAwaitable),
            GetAIter => w!(GetAIter),
            GetANext => w!(GetANext),
            EndAsyncFor => w!(EndAsyncFor),
            MapAdd { i } => w!(MapAdd, i),
        }
    }
}

impl fmt::Display for ConstantData {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.borrow_constant().fmt_display(f)
    }
}

impl<C: Constant> fmt::Debug for CodeObject<C> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "<code object {} at ??? file {:?}, line {}>",
            self.obj_name.as_ref(),
            self.source_path.as_ref(),
            self.first_line_number
        )
    }
}

/// A frozen module. Holds a code object and whether it is part of a package
#[derive(Debug)] // Encode, Decode, Debug)]
pub struct FrozenModule<C: Constant = ConstantData> {
    // #[bincode(bound(
    //     decode = "C: Decode, C::Name: Decode",
    //     encode = "C: Encode, C::Name: Encode"
    // ))]
    pub code: CodeObject<C>,
    pub package: bool,
}
impl<C: Constant> ::bincode::Decode for FrozenModule<C>
where
    C: ::bincode::Decode,
    C::Name: ::bincode::Decode,
{
    fn decode<D: ::bincode::de::Decoder>(
        decoder: &mut D,
    ) -> core::result::Result<Self, ::bincode::error::DecodeError> {
        Ok(Self {
            code: ::bincode::Decode::decode(decoder)?,
            package: ::bincode::Decode::decode(decoder)?,
        })
    }
}
impl<C: Constant> ::bincode::Encode for FrozenModule<C>
where
    C: ::bincode::Encode,
    C::Name: ::bincode::Encode,
{
    fn encode<E: ::bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> core::result::Result<(), ::bincode::error::EncodeError> {
        ::bincode::Encode::encode(&self.code, encoder)?;
        ::bincode::Encode::encode(&self.package, encoder)?;
        Ok(())
    }
}

pub mod frozen_lib {
    use super::*;
    use bincode::error::{DecodeError, EncodeError};

    /// Decode a library to a iterable of frozen modules
    pub fn decode_lib(bytes: &[u8]) -> impl ExactSizeIterator<Item = (String, FrozenModule)> {
        let data = lz4_flex::decompress_size_prepended(bytes).unwrap();
        let r = VecReader { data, pos: 0 };
        let mut de = bincode::de::DecoderImpl::new(r, bincode_config());
        let len = usize::decode(&mut de).unwrap();
        FrozenModulesIter { len, de }
    }

    pub struct FrozenModulesIter<D> {
        len: usize,
        de: D,
    }

    impl<D: bincode::de::Decoder> Iterator for FrozenModulesIter<D> {
        type Item = (String, FrozenModule);

        fn next(&mut self) -> Option<Self::Item> {
            if self.len > 0 {
                let entry = Decode::decode(&mut self.de).unwrap();
                self.len -= 1;
                Some(entry)
            } else {
                None
            }
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            (self.len, Some(self.len))
        }
    }

    impl<D: bincode::de::Decoder> ExactSizeIterator for FrozenModulesIter<D> {}

    /// Encode the given iterator of frozen modules into a compressed vector of bytes
    pub fn encode_lib<'a, I>(lib: I) -> Vec<u8>
    where
        I: IntoIterator<Item = (&'a str, &'a FrozenModule)>,
        I::IntoIter: ExactSizeIterator + Clone,
    {
        let iter = lib.into_iter();
        let data = bincode::encode_to_vec(EncodeLib { iter }, bincode_config()).unwrap();
        lz4_flex::compress_prepend_size(&data)
    }

    struct EncodeLib<I> {
        iter: I,
    }

    impl<'a, I> Encode for EncodeLib<I>
    where
        I: ExactSizeIterator<Item = (&'a str, &'a FrozenModule)> + Clone,
    {
        fn encode<E: ::bincode::enc::Encoder>(&self, encoder: &mut E) -> Result<(), EncodeError> {
            let it = self.iter.clone();
            it.len().encode(encoder)?;
            for module in it {
                module.encode(encoder)?;
            }
            Ok(())
        }
    }

    /// Owned version of bincode::de::read::SliceReader<'a>
    struct VecReader {
        data: Vec<u8>,
        pos: usize,
    }

    impl bincode::de::read::Reader for VecReader {
        fn read(&mut self, buf: &mut [u8]) -> Result<(), DecodeError> {
            let subslice = &self.data[self.pos..];
            match subslice.get(..buf.len()) {
                Some(slice) => {
                    buf.copy_from_slice(slice);
                    self.pos += buf.len();
                    Ok(())
                }
                None => Err(DecodeError::UnexpectedEnd),
            }
        }
        fn peek_read(&self, n: usize) -> Option<&[u8]> {
            self.data[self.pos..].get(..n)
        }
        fn consume(&mut self, n: usize) {
            self.pos = std::cmp::min(self.pos + n, self.data.len());
        }
    }
}
