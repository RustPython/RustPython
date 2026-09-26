#![recursion_limit = "128"]
#![doc(html_logo_url = "https://raw.githubusercontent.com/RustPython/RustPython/main/logo.png")]
#![doc(html_root_url = "https://docs.rs/rustpython-derive/")]

use proc_macro::TokenStream;
use rustpython_derive_impl as derive_impl;
use syn::parse_macro_input;
use syn::punctuated::Punctuated;

/// Derive `FromArgs` for a struct whose fields are Python arguments.
///
/// Each field takes one `#[pyarg(...)]`. The first item is the parameter kind.
/// # Arguments
/// - `positional`: positional-only.
/// - `any`: positional or keyword.
/// - `named`: keyword-only.
/// - `flatten`: take this field from the same argument list. No other keys.
/// - `name = "..."`: Python parameter name. The field name is used when omitted.
/// - `default`: missing argument stores `Default::default()`. Affects parsing.
///   The signature text is `0`, `False` or `0.0` for a primitive integer, `bool`
///   or float field, and `<unrepresentable>` otherwise, unless `py_default` is set.
/// - `default = <expr>`: missing argument stores that Rust value. Affects parsing.
///   A string, byte-string, integer (optionally negated), float, or bool literal
///   on any field that is not a Rust primitive (`i8`..`i128`, `u8`..`u128`,
///   `isize`, `usize`, `f32`, `f64`, `bool`), `&'static str`, `Option<T>`, or
///   `OptionalArg<T>` is converted only when the argument is missing:
///   `<FieldTy as TryFromObject>::try_from_object(vm, ToPyObject::to_pyobject(LIT, vm))?`.
///   A byte-string literal becomes a Python `bytes` object.
/// - `default = ::NAME`: `NAME` is one identifier. The signature copies that
///   name, and the missing argument stores `Into::into(NAME)` (the leading
///   `::` is not Rust syntax for a local constant). A longer `::` path is a
///   compile error. An explicit `py_default` still wins. A path without a
///   leading `::` stays a typed value.
/// - `optional`: same parsing as a bare `default`. The field type must implement
///   `OptionalArgDefault`. `Option<T>` renders `None`; `OptionalArg<T>` renders
///   `<unrepresentable>`. Any other type is rejected.
/// - `py_default = "<python source>"`: text copied verbatim into `__text_signature__`.
///   Never affects parsing. Overrides every other signature default.
///   `py_default = "<unrepresentable>"` is a compile error. Use `OptionalArg`
///   when a missing argument is a distinct state, or give the default's type a
///   real `py_default()`.
/// - `error_msg = "..."`: type-error text when conversion fails.
/// # Signature default
/// An explicit `py_default` is used as written. Otherwise:
/// - a literal, a negative literal, or the path `None` becomes a typed default
///   (`None`, `True`/`False`, an int, a quoted str, a bytes literal, a char;
///   a float literal keeps its source text);
/// - a non-literal on a primitive integer field becomes that value as a decimal int;
/// - a path on a `bool` field becomes `True` or `False`;
/// - `::NAME` is the name, verbatim;
/// - any other expression uses `const V: FieldTy = <expr>; V.py_default()`.
///
/// A function argument whose pattern is a one-field tuple struct takes the
/// parameter name from that field. `Fildes(fd): Fildes` is `fd`. A reference
/// or parentheses around the inner pattern are skipped. When the argument
/// type supplies parameters, this name is ignored.
///
/// `py_default` is an inherent `pub const fn py_default(&self) -> DefaultRepr`.
/// A type defines it once, and every `default = <expr>` of that type reuses it:
///
/// ```rust, ignore
/// impl ArgByteOrder {
///     pub const fn py_default(&self) -> DefaultRepr {
///         match self {
///             Self::Big => DefaultRepr::Str("big"),
///             Self::Little => DefaultRepr::Str("little"),
///         }
///     }
/// }
///
/// #[pyarg(any, default = ArgByteOrder::Big)]
/// byteorder: ArgByteOrder, // signature shows 'big'
/// ```
///
/// A bare `optional` renders the field type's default:
///
/// | Rust type | Meaning | Clinic equivalent | Signature default |
/// | --- | --- | --- | --- |
/// | `OptionalArg<T>` | the argument may be omitted (`Missing`). That is distinct from every Python value, including `None` | `= NULL` | `<unrepresentable>` |
/// | `Option<T>` | `None` or a value. A missing argument and an explicit `None` are the same | `= None` | `None` |
/// | `OptionalOption<T>` (`OptionalArg<Option<T>>`) | missing, `None`, and a value are all distinct | `= NULL`, and `None` is accepted | `<unrepresentable>` |
///
/// Keep `py_default` when the expression needs `vm` (so it is not const), or
/// when an `OptionalArg` is missing in the body and the shown default is a
/// concrete value the Rust type cannot store.
/// ```rust, ignore
/// #[derive(FromArgs)]
/// struct OpenArgs {
///     #[pyarg(any, default = 0o777)]
///     mode: i32, // signature shows 511
///     #[pyarg(named, default = "main")]
///     name: PyStrRef, // signature shows 'main'
/// }
///
/// #[derive(FromArgs)]
/// struct PrintOptions {
///     // None means a space; the string is filled in when printing.
///     #[pyarg(named, default, py_default = "' '")]
///     sep: Option<PyStrRef>,
///     #[pyarg(named, default = None)]
///     file: Option<PyObjectRef>,
/// }
/// ```
#[proc_macro_derive(FromArgs, attributes(pyarg))]
pub fn derive_from_args(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    derive_impl::derive_from_args(input).into()
}

/// The attribute can be applied either to a struct, trait, or impl.
/// # Struct
/// This implements `MaybeTraverse`, `PyClassDef`, and `StaticType` for the struct.
/// Consider deriving `Traverse` to implement it.
/// ## Arguments
/// - `module`: the module which contains the class --  can be omitted if in a `#[pymodule]`.
/// - `name`: the name of the Python class, by default it is the name of the struct.
/// - `base`: the base class of the Python class.
///   This does not cause inheritance of functions or attributes that must be done by a separate trait.
/// # Impl
/// This part implements `PyClassImpl` for the struct.
/// This includes methods, getters/setters, etc.; only annotated methods will be included.
/// Common functions and abilities like instantiation and `__call__` are often implemented by
/// traits rather than in the `impl` itself; see `Constructor` and `Callable` respectively for those.
/// ## Arguments
/// - `name`: the name of the Python class, when no name is provided the struct name is used.
/// - `flags`: the flags of the class, see `PyTypeFlags`.
///     - `BASETYPE`: allows the class to be inheritable.
///     - `IMMUTABLETYPE`: class attributes are immutable.
/// - `with`: which trait implementations are to be included in the python class.
/// ```rust, ignore
/// #[pyclass(module = "my_module", name = "MyClass", base = BaseClass)]
/// struct MyStruct {
///    x: i32,
/// }
///
/// impl Constructor for MyStruct {
///     ...
/// }
///
/// #[pyclass(with(Constructor))]
/// impl MyStruct {
///    ...
/// }
/// ```
/// ## Inner markers
/// ### pymethod/pyclassmethod/pystaticmethod
/// `pymethod` is used to mark a method of the Python class.
/// `pyclassmethod` is used to mark a class method.
/// `pystaticmethod` is used to mark a static method.
/// #### Method signature
/// The first parameter can be either `&self` or `<var>: PyRef<Self>` for `pymethod`.
/// The first parameter can be `cls: PyTypeRef` for `pyclassmethod`.
/// There is no mandatory parameter for `pystaticmethod`.
/// Both are valid and essentially the same, but the latter can yield more control.
/// The last parameter can optionally be of the type `&VirtualMachine` to access the VM.
/// All other values must implement `IntoPyResult`.
/// Numeric types, `String`, `bool`, and `PyObjectRef` implement this trait,
/// but so does any object that implements `PyValue`.
/// Consider using `OptionalArg` for optional arguments.
/// #### Arguments
/// - `name`: the name of the method in Python,
///   by default it is the same as the Rust method, or surrounded by double underscores if magic is present.
///   This overrides `magic` and the default name and cannot be used with `magic` to prevent ambiguity.
/// ### pygetset
/// This is used to mark a getter/setter pair.
/// #### Arguments
/// - `setter`: marks the method as a setter, it acts as a getter by default.
///   Setter method names should be prefixed with `set_`.
/// - `name`: the name of the attribute in Python, by default it is the same as the Rust method.
/// - `magic`: marks the method as a magic method: the method name is surrounded with double underscores.
///   This cannot be used with `name` to prevent ambiguity.
///
/// Ensure both the getter and setter are marked with `name` and `magic` in the same manner.
/// #### Examples
/// ```rust, ignore
/// #[pyclass]
/// impl MyStruct {
///    #[pygetset]
///    fn x(&self) -> PyResult<i32> {
///       Ok(self.x.lock())
///     }
///    #[pygetset(setter)]
///   fn set_x(&mut self, value: i32) -> PyResult<()> {
///      self.x.set(value);
///     Ok(())
///     }
/// }
/// ```
/// ### pyslot
/// This is used to mark a slot method it should be marked by prefixing the method in rust with `slot_`.
/// #### Arguments
/// - name: the name of the slot method.
/// ### pyattr
/// ### extend_class
/// This helps inherit attributes from a parent class.
/// The method this is applied on should be called `extend_class_with_fields`.
/// #### Examples
/// ```rust, ignore
/// #[extend_class]
/// fn extend_class_with_fields(ctx: &Context, class: &'static Py<PyType>) {
///     class.set_attr(
///         identifier!(ctx, _fields),
///         ctx.new_tuple(vec![
///             ctx.new_str(ascii!("body")).into(),
///             ctx.new_str(ascii!("type_ignores")).into(),
///         ])
///         .into(),
///     );
///     class.set_attr(identifier!(ctx, _attributes), ctx.new_list(vec![]).into());
/// }
/// ```
/// ### pymember
/// Declares an offset member on a payload field. The struct `#[pyclass]` builds
/// a `PyClassDef::MEMBERS` table and `extend_class` registers one
/// `member_descriptor` per entry. The field type is checked against the member kind.
///
/// - `type`: `"object"` (default), `"object_ex"`, `"bool"`, `"double"`,
///   `"int"` (`i32`), or `"uint"` (`u32`).
/// - `readonly`: reject stores. A writable object member must be
///   `PyAtomicRef<PyObject>` (never null) or `PyAtomicRef<Option<PyObject>>`
///   (nullable), a writable bool must be `AtomicBool`, a writable int must be
///   `AtomicI32`, a writable uint must be `AtomicU32`, and a writable double
///   must be an atomic 64-bit cell (`AtomicU64`) holding the `f64` bits.
/// - `no_doc`: store no docstring, even when attribute documentation exists.
/// - `audit_read`: audit `object.__getattr__` before the load.
/// - `name`: Python attribute name. Defaults to the field name.
/// - `path`: subfield of the annotated field (`value` with `path = "re"`).
/// - `doc`: docstring. Omitted docs fall back to the stored attribute documentation.
///
/// A struct-level `#[pymember]` (after `#[pyclass]`) has no field. `offset` is
/// required there and rejected on a field. One field may carry several
/// `#[pymember]` attributes. A `#[cfg]` on the field gates that entry.
/// ```rust, ignore
/// #[pymember(name = "fget", readonly)]
/// getter: PyAtomicRef<Option<PyObject>>,
/// ```
/// # Trait
/// `#[pyclass]` on traits functions a lot like `#[pyclass]` on `impl` blocks.
/// Note that associated functions that are annotated with `#[pymethod]` or similar **must**
/// have a body, abstract functions should be wrapped before applying an annotation.
#[proc_macro_attribute]
pub fn pyclass(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = parse_macro_input!(attr with Punctuated::parse_terminated);
    let item = parse_macro_input!(item);
    derive_impl::pyclass(attr, item).into()
}

/// Helper macro to define `Exception` types.
/// More-or-less is an alias to `pyclass` macro.
///
/// This macro serves a goal of generating multiple
/// `BaseException` / `Exception`
/// subtypes in a uniform and convenient manner.
/// It looks like `SimpleExtendsException` in `CPython`.
/// <https://github.com/python/cpython/blob/main/Objects/exceptions.c>
#[proc_macro_attribute]
pub fn pyexception(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = parse_macro_input!(attr with Punctuated::parse_terminated);
    let item = parse_macro_input!(item);
    derive_impl::pyexception(attr, item).into()
}

/// This attribute must be applied to an inline module.
/// It defines a Python module in the form of a `module_def` function in the module;
/// this has to be used in a `add_native_module` to properly register the module.
/// Additionally, this macro defines 'MODULE_NAME' and 'DOC' in the module.
/// # Arguments
/// - `name`: the name of the python module,
///   by default, it is the name of the module, but this can be configured.
/// ```rust, ignore
/// // This will create a module named `my_module`
/// #[pymodule(name = "my_module")]
/// mod module {
/// }
/// ```
/// - `sub`: declare the module as a submodule of another module.
/// ```rust, ignore
/// #[pymodule(sub)]
/// mod submodule {
/// }
///
/// #[pymodule(with(submodule))]
/// mod my_module {
/// }
/// ```
/// - `with`: declare the list of submodules that this module contains (see `sub` for example).
/// ## Inner markers
/// ### pyattr
/// `pyattr` is a multipurpose marker that can be used in a pymodule.
/// The most common use is to mark a function or class as a part of the module.
/// This can be done by applying it to a function or struct prior to the `#[pyfunction]` or `#[pyclass]` macro.
/// If applied to a constant, it will be added to the module as an attribute.
/// If applied to a function not marked with `pyfunction`,
/// it will also be added to the module as an attribute but the value is the result of the function.
/// If `#[pyattr(once)]` is used in this case, the function will be called once
/// and the result will be stored using a `static_cell`.
/// #### Examples
/// ```rust, ignore
/// #[pymodule]
/// mod my_module {
///     #[pyattr]
///     const MY_CONSTANT: i32 = 42;
///     #[pyattr]
///    fn another_constant() -> PyResult<i32> {
///       Ok(42)
///    }
///   #[pyattr(once)]
///   fn once() -> PyResult<i32> {
///     // This will only be called once and the result will be stored.
///     Ok(2 ** 24)
///  }
///
///     #[pyattr]
///     #[pyfunction]
///     fn my_function(vm: &VirtualMachine) -> PyResult<()> {
///         ...
///     }
/// }
/// ```
/// ### pyfunction
/// This is used to create a python function.
/// #### Function signature
/// The last argument can optionally be of the type `&VirtualMachine` to access the VM.
/// Refer to the `pymethod` documentation (located in the `pyclass` macro documentation)
/// for more information on what regular argument types are permitted.
/// #### Arguments
/// - `name`: the name of the function in Python, by default it is the same as the associated Rust function.
#[proc_macro_attribute]
pub fn pymodule(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = parse_macro_input!(attr as derive_impl::PyModuleArgs);
    let item = parse_macro_input!(item);
    derive_impl::pymodule(attr, item).into()
}

/// Attribute macro for defining Python struct sequence types.
///
/// This macro is applied to an empty struct to create a Python type
/// that wraps a Data struct.
///
/// # Example
/// ```ignore
/// #[pystruct_sequence_data]
/// struct StructTimeData {
///     pub tm_year: PyObjectRef,
///     #[pystruct_sequence(skip)]
///     pub tm_gmtoff: PyObjectRef,
/// }
///
/// #[pystruct_sequence(name = "struct_time", module = "time", data = "StructTimeData")]
/// struct PyStructTime;
/// ```
#[proc_macro_attribute]
pub fn pystruct_sequence(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = parse_macro_input!(attr with Punctuated::parse_terminated);
    let item = parse_macro_input!(item);
    derive_impl::pystruct_sequence(attr, item).into()
}

/// Attribute macro for struct sequence Data structs.
///
/// Generates field name constants, index constants, and `into_tuple()` method.
///
/// # Example
/// ```ignore
/// #[pystruct_sequence_data]
/// struct StructTimeData {
///     pub tm_year: PyObjectRef,
///     pub tm_mon: PyObjectRef,
///     #[pystruct_sequence(skip)]  // optional field, not included in tuple
///     pub tm_gmtoff: PyObjectRef,
/// }
/// ```
///
/// # Options
/// - `try_from_object`: Generate `try_from_elements()` method and `TryFromObject` impl
///
/// ```ignore
/// #[pystruct_sequence_data(try_from_object)]
/// struct StructTimeData { ... }
/// ```
#[proc_macro_attribute]
pub fn pystruct_sequence_data(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = parse_macro_input!(attr with Punctuated::parse_terminated);
    let item = parse_macro_input!(item);
    derive_impl::pystruct_sequence_data(attr, item).into()
}

struct Compiler;
impl derive_impl::Compiler for Compiler {
    fn compile(
        &self,
        source: &str,
        mode: rustpython_compiler::Mode,
        module_name: &str,
    ) -> Result<rustpython_compiler::CodeObject, Box<dyn core::error::Error>> {
        use rustpython_compiler::{CompileOpts, compile};
        Ok(compile(source, mode, module_name, CompileOpts::default())?)
    }
}

#[proc_macro]
pub fn py_compile(input: TokenStream) -> TokenStream {
    derive_impl::py_compile(input.into(), &Compiler).into()
}

#[proc_macro]
pub fn py_freeze(input: TokenStream) -> TokenStream {
    derive_impl::py_freeze(input.into(), &Compiler).into()
}

#[proc_macro_derive(PyPayload)]
pub fn pypayload(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    derive_impl::pypayload(input).into()
}

/// use on struct with named fields like `struct A{x:PyRef<B>, y:PyRef<C>}` to impl `Traverse` for datatype.
///
/// use `#[pytraverse(skip)]` on fields you wish not to trace
///
/// add `trace` attr to `#[pyclass]` to make it impl `MaybeTraverse` that will call `Traverse`'s `traverse` method so make it
/// traceable(Even from type-erased PyObject)(i.e. write `#[pyclass(trace)]`).
/// # Example
/// ```rust, ignore
/// #[pyclass(module = false, traverse)]
/// #[derive(Default, Traverse)]
/// pub struct PyList {
///     elements: PyRwLock<Vec<PyObjectRef>>,
///     #[pytraverse(skip)]
///     len: AtomicCell<usize>,
/// }
/// ```
/// This create both `MaybeTraverse` that call `Traverse`'s `traverse` method and `Traverse` that impl `Traverse`
/// for `PyList` which call elements' `traverse` method and ignore `len` field.
#[proc_macro_derive(Traverse, attributes(pytraverse))]
pub fn pytraverse(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let item = parse_macro_input!(item);
    derive_impl::pytraverse(item).into()
}
