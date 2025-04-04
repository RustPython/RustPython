#![recursion_limit = "128"]
#![doc(html_logo_url = "https://raw.githubusercontent.com/RustPython/RustPython/main/logo.png")]
#![doc(html_root_url = "https://docs.rs/rustpython-derive/")]

use proc_macro::TokenStream;
use rustpython_derive_impl as derive_impl;
use syn::parse_macro_input;
use syn::punctuated::Punctuated;

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
/// #[pyclass(module = "my_module", name = "MyClass", base = "BaseClass")]
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
/// - `magic`: marks the method as a magic method: the method name is surrounded with double underscores.
/// ```rust, ignore
/// #[pyclass]
/// impl MyStruct {
///     // This will be called as the `__add__` method in Python.
///     #[pymethod(magic)]
///     fn add(&self, other: &Self) -> PyResult<i32> {
///        ...
///     }
/// }
/// ```
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
/// It defines a Python module in the form a `make_module` function in the module;
/// this has to be used in a `get_module_inits` to properly register the module.
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
    let attr = parse_macro_input!(attr with Punctuated::parse_terminated);
    let item = parse_macro_input!(item);
    derive_impl::pymodule(attr, item).into()
}

#[proc_macro_derive(PyStructSequence, attributes(pystruct))]
pub fn pystruct_sequence(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    derive_impl::pystruct_sequence(input).into()
}

#[proc_macro_derive(TryIntoPyStructSequence, attributes(pystruct))]
pub fn pystruct_sequence_try_from_object(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);
    derive_impl::pystruct_sequence_try_from_object(input).into()
}

struct Compiler;
impl derive_impl::Compiler for Compiler {
    fn compile(
        &self,
        source: &str,
        mode: rustpython_compiler::Mode,
        module_name: String,
    ) -> Result<rustpython_compiler::CodeObject, Box<dyn std::error::Error>> {
        use rustpython_compiler::{CompileOpts, compile};
        Ok(compile(source, mode, &module_name, CompileOpts::default())?)
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
