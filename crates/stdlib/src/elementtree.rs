//! `_elementtree`, the accelerator behind `xml.etree.ElementTree`.
//!
//! `Lib/xml/etree/ElementTree.py` ends with `from _elementtree import *`, so
//! the names defined here (`Element`, `SubElement`, `TreeBuilder`,
//! `XMLParser`, `ParseError`) replace the pure-Python ones, and
//! `_set_factories` hands this module the `Comment`/`ProcessingInstruction`
//! factories that only exist on the Python side. The pure-Python classes stay
//! importable (as `_Element_Py` and friends) and remain the fallback whenever
//! this module is missing, so everything here has to match their observable
//! behaviour rather than merely be fast.
//!
//! Module-level state (the two factories, plus the lazily imported
//! `xml.etree.ElementPath` and `copy.deepcopy` helpers) lives in an
//! `ElementTreeState` instance stored on the module object rather than in a
//! Rust `static`, so two `VirtualMachine`s never share it; `module_state()`
//! finds it back through `sys.modules` without going through the full import
//! machinery.

// spell-checker: ignore elementtree elementpath iterfind findtext itertext
// spell-checker: ignore treebuilder xmlparser subelement makeelement pis

pub(crate) use _elementtree::module_def;

#[pymodule(name = "_elementtree")]
pub(crate) mod _elementtree {
    use crate::vm::{
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
        VirtualMachine, atomic_func,
        builtins::{PyDict, PyDictRef, PyList, PyModule, PyStr, PyType, PyTypeRef},
        function::{FuncArgs, OptionalArg, PySetterValue},
        protocol::{PyMappingMethods, PyNumberMethods, PySequenceMethods},
        sliceable::{SequenceIndex, SliceableSequenceOp},
        types::{
            AsMapping, AsNumber, AsSequence, Constructor, DefaultConstructor, Initializer,
            IterNext, Iterable, Representable, SelfIter,
        },
    };
    use rustpython_common::lock::PyRwLock;

    // -----------------------------------------------------------------
    // module state
    // -----------------------------------------------------------------

    /// The per-module bookkeeping `_elementtree.c` keeps in its
    /// `elementtreestate`: the `Comment`/`ProcessingInstruction` factories
    /// installed by `_set_factories`, and two helpers imported from Python
    /// the first time they are needed.
    #[pyclass(no_attr, module = "_elementtree", name = "_elementtree_state")]
    #[derive(Debug, Default, PyPayload)]
    pub(crate) struct ElementTreeState {
        comment_factory: PyRwLock<Option<PyObjectRef>>,
        pi_factory: PyRwLock<Option<PyObjectRef>>,
        element_path: PyRwLock<Option<PyObjectRef>>,
        deepcopy: PyRwLock<Option<PyObjectRef>>,
        parse_error: PyRwLock<Option<PyTypeRef>>,
    }

    #[pyclass(flags(DISALLOW_INSTANTIATION))]
    impl ElementTreeState {
        /// `xml.etree.ElementPath`, imported on first use. `Element.find` and
        /// friends delegate every non-trivial path to it, exactly as the C
        /// accelerator does.
        fn element_path(&self, vm: &VirtualMachine) -> PyResult {
            let cached = self.element_path.read().clone();
            if let Some(m) = cached {
                return Ok(m);
            }
            // `vm.import()` hands back the *top* package, so the submodule
            // itself is picked up from `sys.modules` afterwards.
            vm.import("xml.etree.ElementPath", 0)?;
            let module = vm
                .sys_module
                .get_attr("modules", vm)?
                .get_item("xml.etree.ElementPath", vm)?;
            *self.element_path.write() = Some(module.clone());
            Ok(module)
        }

        fn deepcopy(&self, vm: &VirtualMachine) -> PyResult {
            let cached = self.deepcopy.read().clone();
            if let Some(f) = cached {
                return Ok(f);
            }
            let f = vm.import("copy", 0)?.get_attr("deepcopy", vm)?;
            *self.deepcopy.write() = Some(f.clone());
            Ok(f)
        }
    }

    /// The state object stashed on the module by `module_exec`.
    ///
    /// Looked up through `sys.modules` rather than `vm.import()`: the module
    /// is always already imported by the time any of its own code runs, and
    /// this keeps the lookup down to two dictionary hits.
    fn module_state(vm: &VirtualMachine) -> PyResult<PyRef<ElementTreeState>> {
        let modules = vm.sys_module.get_attr("modules", vm)?;
        let module = match modules.get_item("_elementtree", vm) {
            Ok(module) => module,
            Err(_) => vm.import("_elementtree", 0)?,
        };
        module
            .get_attr("_state", vm)?
            .downcast::<ElementTreeState>()
            .map_err(|_| vm.new_runtime_error("_elementtree state is corrupted"))
    }

    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        __module_exec(vm, module);
        let state = ElementTreeState::default().into_ref(&vm.ctx);
        *state.parse_error.write() = module.get_attr("ParseError", vm)?.downcast::<PyType>().ok();
        module.set_attr("_state", state, vm)?;
        Ok(())
    }

    #[pyattr(name = "ParseError", once)]
    fn parse_error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "xml.etree.ElementTree",
            "ParseError",
            Some(vec![vm.ctx.exceptions.syntax_error.to_owned()]),
        )
    }

    #[pyfunction]
    fn _set_factories(
        comment_factory: PyObjectRef,
        pi_factory: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<(PyObjectRef, PyObjectRef)> {
        if !vm.is_none(&comment_factory) && !comment_factory.is_callable() {
            return Err(vm.new_type_error(format!(
                "Comment factory must be callable, not {}",
                comment_factory.class().name()
            )));
        }
        if !vm.is_none(&pi_factory) && !pi_factory.is_callable() {
            return Err(vm.new_type_error(format!(
                "PI factory must be callable, not {}",
                pi_factory.class().name()
            )));
        }
        let state = module_state(vm)?;
        let old_comment = state.comment_factory.read().clone();
        let old_pi = state.pi_factory.read().clone();
        *state.comment_factory.write() = (!vm.is_none(&comment_factory)).then_some(comment_factory);
        *state.pi_factory.write() = (!vm.is_none(&pi_factory)).then_some(pi_factory);
        Ok((
            old_comment.unwrap_or_else(|| vm.ctx.none()),
            old_pi.unwrap_or_else(|| vm.ctx.none()),
        ))
    }

    // -----------------------------------------------------------------
    // Element
    // -----------------------------------------------------------------

    /// `text`/`tail` as the C accelerator stores them: either the value the
    /// user sees, or a list of fragments the `TreeBuilder` accumulated that
    /// is joined lazily the first time the attribute is read. Keeping the
    /// deferred form is what makes character data arriving in many expat
    /// callbacks cost one join per element rather than one per callback.
    #[derive(Debug)]
    struct Joined {
        obj: PyObjectRef,
        /// `obj` is a list of fragments still to be joined.
        pending: bool,
    }

    impl Joined {
        fn plain(obj: PyObjectRef) -> Self {
            Self {
                obj,
                pending: false,
            }
        }
    }

    #[derive(Debug)]
    struct ElementInner {
        tag: PyObjectRef,
        text: Joined,
        tail: Joined,
        /// `None` until an attribute is actually needed, mirroring the C
        /// implementation's lazily allocated `extra->attrib`.
        attrib: Option<PyDictRef>,
        children: Vec<PyObjectRef>,
    }

    #[pyattr]
    #[pyclass(
        module = "xml.etree.ElementTree",
        name = "Element",
        traverse = "manual"
    )]
    #[derive(Debug, PyPayload)]
    pub(crate) struct PyElement {
        inner: PyRwLock<ElementInner>,
    }

    // SAFETY: every owned PyObjectRef reachable from an element is visited.
    unsafe impl crate::vm::object::Traverse for PyElement {
        fn traverse(&self, traverse_fn: &mut crate::vm::object::TraverseFn<'_>) {
            let Some(inner) = self.inner.try_read() else {
                return;
            };
            inner.tag.traverse(traverse_fn);
            inner.text.obj.traverse(traverse_fn);
            inner.tail.obj.traverse(traverse_fn);
            inner.attrib.traverse(traverse_fn);
            inner.children.traverse(traverse_fn);
        }

        fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
            // Only reached for an object the collector has already proven
            // unreachable, so grabbing the lock cannot race with user code.
            let Some(mut inner) = self.inner.try_write() else {
                return;
            };
            let none = || -> PyObjectRef { crate::vm::Context::genesis().none.to_owned().into() };
            out.append(&mut inner.children);
            if let Some(attrib) = inner.attrib.take() {
                out.push(attrib.into());
            }
            out.push(core::mem::replace(&mut inner.tag, none()));
            out.push(core::mem::replace(&mut inner.text.obj, none()));
            out.push(core::mem::replace(&mut inner.tail.obj, none()));
            inner.text.pending = false;
            inner.tail.pending = false;
        }
    }

    impl PyElement {
        fn new(tag: PyObjectRef, attrib: Option<PyDictRef>, vm: &VirtualMachine) -> Self {
            let attrib = attrib.filter(|d| !d.is_empty());
            Self {
                inner: PyRwLock::new(ElementInner {
                    tag,
                    text: Joined::plain(vm.ctx.none()),
                    tail: Joined::plain(vm.ctx.none()),
                    attrib,
                    children: Vec::new(),
                }),
            }
        }

        fn len(&self) -> usize {
            self.inner.read().children.len()
        }

        fn child(&self, index: usize) -> Option<PyObjectRef> {
            self.inner.read().children.get(index).cloned()
        }

        pub(crate) fn tag_obj(&self) -> PyObjectRef {
            self.inner.read().tag.clone()
        }

        pub(crate) fn push(&self, child: PyObjectRef) {
            self.inner.write().children.push(child);
        }

        /// Read `text`/`tail`, joining a pending fragment list in place.
        fn joined_get(&self, tail: bool, vm: &VirtualMachine) -> PyResult {
            {
                let inner = self.inner.read();
                let slot = if tail { &inner.tail } else { &inner.text };
                if !slot.pending {
                    return Ok(slot.obj.clone());
                }
                if !slot.obj.downcastable::<PyList>() {
                    return Ok(slot.obj.clone());
                }
            }
            // The join calls back into Python (str.join), so the lock must be
            // released around it and the slot re-checked afterwards.
            let list = {
                let inner = self.inner.read();
                let slot = if tail { &inner.tail } else { &inner.text };
                slot.obj.clone()
            };
            let joined = list_join(&list, vm)?;
            let mut inner = self.inner.write();
            let slot = if tail {
                &mut inner.tail
            } else {
                &mut inner.text
            };
            if slot.pending && slot.obj.is(&list) {
                *slot = Joined::plain(joined.clone());
            }
            Ok(joined)
        }

        fn attrib_or_new(&self, vm: &VirtualMachine) -> PyDictRef {
            let mut inner = self.inner.write();
            inner
                .attrib
                .get_or_insert_with(|| vm.ctx.new_dict())
                .clone()
        }

        fn attrib_opt(&self) -> Option<PyDictRef> {
            self.inner.read().attrib.clone()
        }
    }

    /// `"".join(list)`, the C helper `list_join`.
    fn list_join(list: &PyObject, vm: &VirtualMachine) -> PyResult {
        let empty = vm.ctx.new_str("");
        vm.call_method(empty.as_object(), "join", (list.to_owned(),))
    }

    fn check_element(obj: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
        if obj.downcastable::<PyElement>() {
            Ok(())
        } else {
            Err(vm.new_type_error(format!(
                "expected an Element, not \"{}\"",
                obj.class().name()
            )))
        }
    }

    fn as_element<'a>(obj: &'a PyObject, vm: &VirtualMachine) -> PyResult<&'a Py<PyElement>> {
        obj.downcast_ref::<PyElement>().ok_or_else(|| {
            vm.new_type_error(format!(
                "expected an Element, not \"{}\"",
                obj.class().name()
            ))
        })
    }

    /// Split `Element(tag, attrib, **extra)` style arguments into the tag and
    /// a freshly owned attribute dictionary, matching
    /// `get_attrib_from_keywords`: a positional `attrib` is copied, an
    /// `attrib` keyword is popped and copied, and the rest of the keywords
    /// are merged over it.
    fn parse_attrib_args(
        args: FuncArgs,
        func_name: &str,
        extra_positional: usize,
        vm: &VirtualMachine,
    ) -> PyResult<(PyObjectRef, Option<PyDictRef>)> {
        let mut args = args;
        let nargs = args.args.len();
        if nargs < extra_positional + 1 {
            return Err(vm.new_type_error(format!(
                "{func_name}() takes at least {} argument{} ({nargs} given)",
                extra_positional + 1,
                if extra_positional == 0 { "" } else { "s" }
            )));
        }
        if nargs > extra_positional + 2 {
            return Err(vm.new_type_error(format!(
                "{func_name}() takes at most {} arguments ({nargs} given)",
                extra_positional + 2
            )));
        }
        let tag = args.args[extra_positional].clone();
        let positional_attrib = args.args.get(extra_positional + 1).cloned();
        let attrib = match positional_attrib {
            Some(obj) => {
                let dict = obj.downcast::<PyDict>().map_err(|obj| {
                    vm.new_type_error(format!(
                        "{func_name}() argument {} must be dict, not {}",
                        extra_positional + 2,
                        obj.class().name()
                    ))
                })?;
                let copied = dict.copy().into_ref(&vm.ctx);
                for (key, value) in args.kwargs.drain(..) {
                    copied.set_item(&key, value, vm)?;
                }
                Some(copied)
            }
            None if !args.kwargs.is_empty() => {
                let popped = args.kwargs.shift_remove("attrib");
                let copied = match popped {
                    Some(obj) => {
                        let dict = obj.downcast::<PyDict>().map_err(|obj| {
                            vm.new_type_error(format!(
                                "attrib must be dict, not {}",
                                obj.class().name()
                            ))
                        })?;
                        dict.copy().into_ref(&vm.ctx)
                    }
                    None => vm.ctx.new_dict(),
                };
                for (key, value) in args.kwargs.drain(..) {
                    copied.set_item(&key, value, vm)?;
                }
                Some(copied)
            }
            None => None,
        };
        Ok((tag, attrib))
    }

    #[derive(FromArgs)]
    struct FindArgs {
        #[pyarg(any)]
        path: PyObjectRef,
        #[pyarg(any, optional)]
        namespaces: OptionalArg<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct FindTextArgs {
        #[pyarg(any)]
        path: PyObjectRef,
        #[pyarg(any, optional)]
        default: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        namespaces: OptionalArg<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct GetArgs {
        #[pyarg(any)]
        key: PyObjectRef,
        #[pyarg(any, optional)]
        default: OptionalArg<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct IterArgs {
        #[pyarg(any, optional)]
        tag: OptionalArg<PyObjectRef>,
    }

    impl Constructor for PyElement {
        type Args = FuncArgs;

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self::new(vm.ctx.none(), None, vm))
        }
    }

    impl Initializer for PyElement {
        type Args = FuncArgs;

        fn init(zelf: PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
            let (tag, attrib) = parse_attrib_args(args, "Element", 0, vm)?;
            let attrib = attrib.filter(|d| !d.is_empty());
            let _recycle = {
                let mut inner = zelf.inner.write();
                let old_attrib = if attrib.is_some() {
                    core::mem::replace(&mut inner.attrib, attrib)
                } else {
                    None
                };
                (
                    core::mem::replace(&mut inner.tag, tag),
                    core::mem::replace(&mut inner.text, Joined::plain(vm.ctx.none())),
                    core::mem::replace(&mut inner.tail, Joined::plain(vm.ctx.none())),
                    old_attrib,
                )
            };
            Ok(())
        }
    }

    #[pyclass(
        with(
            Constructor,
            Initializer,
            AsMapping,
            AsSequence,
            AsNumber,
            Representable
        ),
        flags(BASETYPE, HAS_WEAKREF)
    )]
    impl PyElement {
        #[pygetset]
        fn tag(&self) -> PyObjectRef {
            self.inner.read().tag.clone()
        }

        #[pygetset(setter)]
        fn set_tag(&self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
            let value = setter_value(value, vm)?;
            // Whatever is replaced may run __del__ on drop, and that code is
            // free to touch this very element, so it must not be dropped
            // while the lock is held (test_bpo_31728).
            let _recycle = core::mem::replace(&mut self.inner.write().tag, value);
            Ok(())
        }

        #[pygetset]
        fn text(&self, vm: &VirtualMachine) -> PyResult {
            self.joined_get(false, vm)
        }

        #[pygetset(setter)]
        fn set_text(&self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
            let value = setter_value(value, vm)?;
            let _recycle = core::mem::replace(&mut self.inner.write().text, Joined::plain(value));
            Ok(())
        }

        #[pygetset]
        fn tail(&self, vm: &VirtualMachine) -> PyResult {
            self.joined_get(true, vm)
        }

        #[pygetset(setter)]
        fn set_tail(&self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
            let value = setter_value(value, vm)?;
            let _recycle = core::mem::replace(&mut self.inner.write().tail, Joined::plain(value));
            Ok(())
        }

        #[pygetset]
        fn attrib(&self, vm: &VirtualMachine) -> PyDictRef {
            self.attrib_or_new(vm)
        }

        #[pygetset(setter)]
        fn set_attrib(&self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
            let value = setter_value(value, vm)?;
            let dict = value.downcast::<PyDict>().map_err(|obj| {
                vm.new_type_error(format!("attrib must be dict, not {}", obj.class().name()))
            })?;
            let _recycle = self.inner.write().attrib.replace(dict);
            Ok(())
        }

        #[pymethod]
        fn append(&self, subelement: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            check_element(&subelement, vm)?;
            self.inner.write().children.push(subelement);
            Ok(())
        }

        #[pymethod]
        fn extend(&self, elements: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let items: Vec<PyObjectRef> = elements.try_to_value(vm)?;
            for element in &items {
                check_element(element, vm)?;
            }
            self.inner.write().children.extend(items);
            Ok(())
        }

        #[pymethod]
        fn insert(
            &self,
            index: isize,
            subelement: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            check_element(&subelement, vm)?;
            let mut inner = self.inner.write();
            let len = inner.children.len();
            let pos = if index < 0 {
                let i = index + len as isize;
                if i < 0 { 0 } else { i as usize }
            } else {
                (index as usize).min(len)
            };
            inner.children.insert(pos, subelement);
            Ok(())
        }

        #[pymethod]
        fn remove(&self, subelement: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            check_element(&subelement, vm)?;
            // Identity first, then equality, the way `element_remove` does;
            // the equality test can run arbitrary code, so the lock is
            // dropped for it and the list re-read afterwards.
            let mut found = None;
            let mut i = 0;
            loop {
                let child = {
                    let inner = self.inner.read();
                    match inner.children.get(i) {
                        Some(child) => child.clone(),
                        None => break,
                    }
                };
                if child.is(&subelement) {
                    found = Some(i);
                    break;
                }
                if vm.bool_eq(&child, &subelement)? {
                    found = Some(i);
                    break;
                }
                i += 1;
            }
            let Some(i) = found else {
                return Err(vm.new_value_error("Element.remove(x): element not found"));
            };
            let _recycle = {
                let mut inner = self.inner.write();
                (i < inner.children.len()).then(|| inner.children.remove(i))
            };
            Ok(())
        }

        #[pymethod]
        fn clear(&self, vm: &VirtualMachine) {
            let _recycle = {
                let mut inner = self.inner.write();
                (
                    inner.attrib.take(),
                    core::mem::take(&mut inner.children),
                    core::mem::replace(&mut inner.text, Joined::plain(vm.ctx.none())),
                    core::mem::replace(&mut inner.tail, Joined::plain(vm.ctx.none())),
                )
            };
        }

        #[pymethod]
        fn get(&self, args: GetArgs, vm: &VirtualMachine) -> PyResult {
            let default = args.default.unwrap_or_none(vm);
            let Some(attrib) = self.attrib_opt() else {
                return Ok(default);
            };
            Ok(attrib.get_item_opt(&*args.key, vm)?.unwrap_or(default))
        }

        #[pymethod]
        fn set(&self, key: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let attrib = self.attrib_or_new(vm);
            attrib.set_item(&*key, value, vm)
        }

        #[pymethod]
        fn keys(&self) -> Vec<PyObjectRef> {
            let Some(attrib) = self.attrib_opt() else {
                return vec![];
            };
            attrib.into_iter().map(|(k, _)| k).collect()
        }

        #[pymethod]
        fn items(&self, vm: &VirtualMachine) -> Vec<PyObjectRef> {
            let Some(attrib) = self.attrib_opt() else {
                return vec![];
            };
            attrib
                .into_iter()
                .map(|(k, v)| vm.ctx.new_tuple(vec![k, v]).into())
                .collect()
        }

        #[pymethod]
        fn makeelement(
            zelf: &Py<Self>,
            tag: PyObjectRef,
            attrib: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult {
            let attrib = attrib.downcast::<PyDict>().map_err(|obj| {
                vm.new_type_error(format!(
                    "makeelement() argument 2 must be dict, not {}",
                    obj.class().name()
                ))
            })?;
            let attrib = attrib.copy().into_ref(&vm.ctx);
            create_element(zelf.class().to_owned(), tag, Some(attrib), vm)
        }

        #[pymethod]
        fn find(zelf: &Py<Self>, args: FindArgs, vm: &VirtualMachine) -> PyResult {
            let FindArgs { path, namespaces } = args;
            let namespaces = namespaces.unwrap_or_none(vm);
            if check_path(&path) || !vm.is_none(&namespaces) {
                let state = module_state(vm)?;
                let element_path = state.element_path(vm)?;
                return vm.call_method(&element_path, "find", (zelf.to_owned(), path, namespaces));
            }
            let mut i = 0;
            while let Some(child) = zelf.child(i) {
                let tag = as_element(&child, vm)?.tag_obj();
                if vm.bool_eq(&tag, &path)? {
                    return Ok(child);
                }
                i += 1;
            }
            Ok(vm.ctx.none())
        }

        #[pymethod]
        fn findtext(zelf: &Py<Self>, args: FindTextArgs, vm: &VirtualMachine) -> PyResult {
            let FindTextArgs {
                path,
                default,
                namespaces,
            } = args;
            let default = default.unwrap_or_none(vm);
            let namespaces = namespaces.unwrap_or_none(vm);
            if check_path(&path) || !vm.is_none(&namespaces) {
                let state = module_state(vm)?;
                let element_path = state.element_path(vm)?;
                return vm.call_method(
                    &element_path,
                    "findtext",
                    (zelf.to_owned(), path, default, namespaces),
                );
            }
            let mut i = 0;
            while let Some(child) = zelf.child(i) {
                let elem = as_element(&child, vm)?;
                if vm.bool_eq(&elem.tag_obj(), &path)? {
                    let text = elem.joined_get(false, vm)?;
                    return Ok(if vm.is_none(&text) {
                        vm.ctx.new_str("").into()
                    } else {
                        text
                    });
                }
                i += 1;
            }
            Ok(default)
        }

        #[pymethod]
        fn findall(zelf: &Py<Self>, args: FindArgs, vm: &VirtualMachine) -> PyResult {
            let FindArgs { path, namespaces } = args;
            let namespaces = namespaces.unwrap_or_none(vm);
            if check_path(&path) || !vm.is_none(&namespaces) {
                let state = module_state(vm)?;
                let element_path = state.element_path(vm)?;
                return vm.call_method(
                    &element_path,
                    "findall",
                    (zelf.to_owned(), path, namespaces),
                );
            }
            let mut out = Vec::new();
            let mut i = 0;
            while let Some(child) = zelf.child(i) {
                let tag = as_element(&child, vm)?.tag_obj();
                if vm.bool_eq(&tag, &path)? {
                    out.push(child);
                }
                i += 1;
            }
            Ok(vm.ctx.new_list(out).into())
        }

        #[pymethod]
        fn iterfind(zelf: &Py<Self>, args: FindArgs, vm: &VirtualMachine) -> PyResult {
            let FindArgs { path, namespaces } = args;
            let namespaces = namespaces.unwrap_or_none(vm);
            let state = module_state(vm)?;
            let element_path = state.element_path(vm)?;
            vm.call_method(
                &element_path,
                "iterfind",
                (zelf.to_owned(), path, namespaces),
            )
        }

        #[pymethod]
        fn iter(zelf: PyRef<Self>, args: IterArgs, vm: &VirtualMachine) -> PyElementIter {
            let tag = args.tag.unwrap_or_none(vm);
            let tag = match tag.downcast_ref::<PyStr>() {
                Some(s) if s.as_wtf8() == "*" => vm.ctx.none(),
                _ => tag,
            };
            PyElementIter::new(zelf, tag, false)
        }

        #[pymethod]
        fn itertext(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyElementIter {
            PyElementIter::new(zelf, vm.ctx.none(), true)
        }

        #[pymethod]
        fn __copy__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            let (tag, attrib, text, tail, children) = {
                let inner = zelf.inner.read();
                (
                    inner.tag.clone(),
                    inner.attrib.clone(),
                    Joined {
                        obj: inner.text.obj.clone(),
                        pending: inner.text.pending,
                    },
                    Joined {
                        obj: inner.tail.obj.clone(),
                        pending: inner.tail.pending,
                    },
                    inner.children.clone(),
                )
            };
            let new = create_element(zelf.class().to_owned(), tag, attrib, vm)?;
            {
                let elem = as_element(&new, vm)?;
                let mut inner = elem.inner.write();
                inner.text = text;
                inner.tail = tail;
                inner.children = children;
                drop(inner);
            }
            Ok(new)
        }

        #[pymethod]
        fn __deepcopy__(zelf: &Py<Self>, memo: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let memo = memo.downcast::<PyDict>().map_err(|obj| {
                vm.new_type_error(format!(
                    "__deepcopy__() argument 1 must be dict, not {}",
                    obj.class().name()
                ))
            })?;
            deepcopy_element(zelf, &memo, vm)
        }

        #[pymethod]
        fn __getstate__(&self, vm: &VirtualMachine) -> PyResult<PyDictRef> {
            let inner = self.inner.read();
            let children = vm.ctx.new_list(inner.children.clone());
            let attrib = inner.attrib.clone().unwrap_or_else(|| vm.ctx.new_dict());
            let text = inner.text.obj.clone();
            let tail = inner.tail.obj.clone();
            let tag = inner.tag.clone();
            drop(inner);
            let state = vm.ctx.new_dict();
            state.set_item("tag", tag, vm)?;
            state.set_item("_children", children.into(), vm)?;
            state.set_item("attrib", attrib.into(), vm)?;
            state.set_item("text", text, vm)?;
            state.set_item("tail", tail, vm)?;
            Ok(state)
        }

        #[pymethod]
        fn __setstate__(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let dict = state.downcast_ref::<PyDict>().ok_or_else(|| {
                vm.new_type_error(format!(
                    "Don't know how to unpickle \"{}\" as an Element",
                    state.repr(vm).map(|s| s.to_string()).unwrap_or_default()
                ))
            })?;
            let tag = dict.get_item_opt("tag", vm)?;
            let attrib = dict.get_item_opt("attrib", vm)?;
            let text = dict.get_item_opt("text", vm)?;
            let tail = dict.get_item_opt("tail", vm)?;
            let children = dict.get_item_opt("_children", vm)?;
            let Some(tag) = tag else {
                return Err(vm.new_type_error("tag may not be NULL"));
            };
            let children = match children {
                Some(children) => {
                    let list = children
                        .downcast_ref::<PyList>()
                        .ok_or_else(|| vm.new_type_error("'_children' is not a list"))?;
                    let children = list.borrow_vec().to_vec();
                    for child in &children {
                        check_element(child, vm)?;
                    }
                    Some(children)
                }
                None => None,
            };
            let attrib = match attrib {
                Some(attrib) if !vm.is_none(&attrib) => {
                    Some(attrib.downcast::<PyDict>().map_err(|obj| {
                        vm.new_type_error(format!(
                            "attrib must be dict, not {}",
                            obj.class().name()
                        ))
                    })?)
                }
                _ => None,
            };
            let new_text = match text {
                Some(text) => {
                    let pending = text.downcastable::<PyList>();
                    Joined { obj: text, pending }
                }
                None => Joined::plain(vm.ctx.none()),
            };
            let new_tail = match tail {
                Some(tail) => {
                    let pending = tail.downcastable::<PyList>();
                    Joined { obj: tail, pending }
                }
                None => Joined::plain(vm.ctx.none()),
            };
            let _recycle = {
                let mut inner = self.inner.write();
                let old_tag = core::mem::replace(&mut inner.tag, tag);
                let old_text = core::mem::replace(&mut inner.text, new_text);
                let old_tail = core::mem::replace(&mut inner.tail, new_tail);
                let old_children = match children {
                    Some(children) => Some(core::mem::replace(&mut inner.children, children)),
                    None => None,
                };
                let old_attrib = if attrib.is_some() {
                    core::mem::replace(&mut inner.attrib, attrib)
                } else {
                    None
                };
                (old_tag, old_text, old_tail, old_children, old_attrib)
            };
            Ok(())
        }

        fn _getitem(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult {
            match sequence_index(vm, needle)? {
                SequenceIndex::Int(i) => self
                    .inner
                    .read()
                    .children
                    .getitem_by_index(vm, i)
                    .map_err(|_| vm.new_index_error("child index out of range")),
                SequenceIndex::Slice(slice) => {
                    let items = self.inner.read().children.getitem_by_slice(vm, slice)?;
                    Ok(vm.ctx.new_list(items).into())
                }
            }
        }

        fn _setitem(
            &self,
            needle: &PyObject,
            value: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            // `recycle` outlives the lock guard on purpose: dropping a
            // displaced child can run __del__, which may come back into this
            // element.
            #[expect(clippy::collection_is_never_read, reason = "dropped after the lock")]
            let mut recycle: Vec<PyObjectRef> = Vec::new();
            match sequence_index(vm, needle)? {
                SequenceIndex::Int(i) => {
                    check_element(&value, vm)?;
                    let mut inner = self.inner.write();
                    let pos = inner
                        .children
                        .wrap_index(i)
                        .ok_or_else(|| vm.new_index_error("child assignment index out of range"))?;
                    recycle.push(core::mem::replace(&mut inner.children[pos], value));
                    Ok(())
                }
                SequenceIndex::Slice(slice) => {
                    let items: Vec<PyObjectRef> = value.try_to_value(vm)?;
                    for item in &items {
                        check_element(item, vm)?;
                    }
                    let mut inner = self.inner.write();
                    let (range, step, slice_len) = slice.adjust_indices(inner.children.len());
                    if step == 1 {
                        recycle.extend(inner.children.splice(range, items));
                        Ok(())
                    } else if slice_len == items.len() {
                        let iter = crate::vm::sliceable::SaturatedSliceIter::from_adjust_indices(
                            range, step, slice_len,
                        );
                        for (pos, item) in iter.zip(items) {
                            recycle.push(core::mem::replace(&mut inner.children[pos], item));
                        }
                        Ok(())
                    } else {
                        Err(vm.new_value_error(format!(
                            "attempt to assign sequence of size {} to extended slice of size {}",
                            items.len(),
                            slice_len
                        )))
                    }
                }
            }
        }

        fn _delitem(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
            #[expect(clippy::collection_is_never_read, reason = "dropped after the lock")]
            let mut recycle: Vec<PyObjectRef> = Vec::new();
            match sequence_index(vm, needle)? {
                SequenceIndex::Int(i) => {
                    let mut inner = self.inner.write();
                    let pos = inner
                        .children
                        .wrap_index(i)
                        .ok_or_else(|| vm.new_index_error("child assignment index out of range"))?;
                    recycle.push(inner.children.remove(pos));
                    Ok(())
                }
                SequenceIndex::Slice(slice) => {
                    let mut inner = self.inner.write();
                    let (range, step, slice_len) = slice.adjust_indices(inner.children.len());
                    if slice_len == 0 {
                        return Ok(());
                    }
                    let mut doomed: Vec<usize> =
                        crate::vm::sliceable::SaturatedSliceIter::from_adjust_indices(
                            range, step, slice_len,
                        )
                        .collect();
                    doomed.sort_unstable();
                    for pos in doomed.into_iter().rev() {
                        recycle.push(inner.children.remove(pos));
                    }
                    Ok(())
                }
            }
        }
    }

    fn setter_value(value: PySetterValue, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        match value {
            PySetterValue::Assign(v) => Ok(v),
            PySetterValue::Delete => Err(vm.new_attribute_error("can't delete element attribute")),
        }
    }

    fn sequence_index(vm: &VirtualMachine, needle: &PyObject) -> PyResult<SequenceIndex> {
        SequenceIndex::try_from_borrowed_object(vm, needle, "element")
            .map_err(|_| vm.new_type_error("element indices must be integers"))
    }

    /// Instantiate `cls` (which may be an `Element` subclass) directly,
    /// bypassing `__init__`, the way `create_new_element` does.
    fn create_element(
        cls: PyTypeRef,
        tag: PyObjectRef,
        attrib: Option<PyDictRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let element = PyElement::new(tag, attrib, vm);
        Ok(element.into_ref_with_type(vm, cls)?.into())
    }

    fn deepcopy_obj(obj: &PyObject, memo: &Py<PyDict>, vm: &VirtualMachine) -> PyResult {
        if vm.is_none(obj) || obj.class().is(vm.ctx.types.str_type) {
            return Ok(obj.to_owned());
        }
        if let Some(elem) = obj.downcast_ref::<PyElement>()
            && elem.class().is(PyElement::class(&vm.ctx))
        {
            return deepcopy_element(elem, memo, vm);
        }
        let state = module_state(vm)?;
        state
            .deepcopy(vm)?
            .call((obj.to_owned(), memo.to_owned()), vm)
    }

    fn deepcopy_element(zelf: &Py<PyElement>, memo: &Py<PyDict>, vm: &VirtualMachine) -> PyResult {
        // A chain of elements is copied by native recursion, so the depth
        // has to be charged to the interpreter's recursion budget or a deep
        // enough tree overflows the real stack instead of raising.
        vm.with_recursion(" in Element.__deepcopy__", || {
            deepcopy_element_inner(zelf, memo, vm)
        })
    }

    fn deepcopy_element_inner(
        zelf: &Py<PyElement>,
        memo: &Py<PyDict>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let (tag, attrib, text, text_pending, tail, tail_pending, children) = {
            let inner = zelf.inner.read();
            (
                inner.tag.clone(),
                inner.attrib.clone(),
                inner.text.obj.clone(),
                inner.text.pending,
                inner.tail.obj.clone(),
                inner.tail.pending,
                inner.children.clone(),
            )
        };
        let tag = deepcopy_obj(&tag, memo, vm)?;
        let attrib = match attrib {
            Some(attrib) => Some(
                deepcopy_obj(attrib.as_object(), memo, vm)?
                    .downcast::<PyDict>()
                    .map_err(|_| vm.new_type_error("attrib must be dict"))?,
            ),
            None => None,
        };
        let new = create_element(zelf.class().to_owned(), tag, attrib, vm)?;
        let elem = as_element(&new, vm)?;
        let text = deepcopy_obj(&text, memo, vm)?;
        let tail = deepcopy_obj(&tail, memo, vm)?;
        // Re-read the child list on every step instead of iterating a
        // snapshot: a user __deepcopy__ is free to clear or grow the
        // original while the copy is in progress, and the C accelerator
        // follows the live list (test_deepcopy_clear/test_deepcopy_grow).
        let _ = children;
        let mut new_children: Vec<PyObjectRef> = Vec::new();
        let mut i = 0;
        loop {
            let Some(child) = zelf.child(i) else { break };
            let copied = deepcopy_obj(&child, memo, vm)?;
            check_element(&copied, vm)?;
            new_children.push(copied);
            i += 1;
        }
        {
            let mut inner = elem.inner.write();
            inner.text = Joined {
                obj: text,
                pending: text_pending,
            };
            inner.tail = Joined {
                obj: tail,
                pending: tail_pending,
            };
            inner.children = new_children;
        }
        let key: PyObjectRef = vm.ctx.new_int(zelf.get_id()).into();
        memo.set_item(&*key, new.clone(), vm)?;
        Ok(new)
    }

    /// `checkpath`: does this look like an XPath expression rather than a
    /// plain tag, in which case `find`/`findall` must go through
    /// `ElementPath`?
    fn check_path(tag: &PyObject) -> bool {
        let bytes = if let Some(s) = tag.downcast_ref::<PyStr>() {
            s.as_bytes()
        } else if let Some(b) = tag.downcast_ref::<crate::vm::builtins::PyBytes>() {
            b.as_bytes()
        } else {
            // Unknown type; might be a path expression.
            return true;
        };
        if bytes.len() >= 3
            && bytes[0] == b'{'
            && (bytes[1] == b'}' || (bytes[1] == b'*' && bytes[2] == b'}'))
        {
            return true;
        }
        let mut check = true;
        for &ch in bytes {
            match ch {
                b'{' => check = false,
                b'}' => check = true,
                b'/' | b'*' | b'[' | b'@' | b'.' if check => return true,
                _ => {}
            }
        }
        false
    }

    impl AsMapping for PyElement {
        fn as_mapping() -> &'static PyMappingMethods {
            static AS_MAPPING: PyMappingMethods = PyMappingMethods {
                length: atomic_func!(|mapping, _vm| Ok(PyElement::mapping_downcast(mapping).len())),
                subscript: atomic_func!(
                    |mapping, needle, vm| PyElement::mapping_downcast(mapping)._getitem(needle, vm)
                ),
                ass_subscript: atomic_func!(|mapping, needle, value, vm| {
                    let zelf = PyElement::mapping_downcast(mapping);
                    if let Some(value) = value {
                        zelf._setitem(needle, value, vm)
                    } else {
                        zelf._delitem(needle, vm)
                    }
                }),
            };
            &AS_MAPPING
        }
    }

    impl AsSequence for PyElement {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
                length: atomic_func!(|seq, _vm| Ok(PyElement::sequence_downcast(seq).len())),
                item: atomic_func!(|seq, i, vm| {
                    let zelf = PyElement::sequence_downcast(seq);
                    zelf.child(
                        i.try_into()
                            .map_err(|_| vm.new_index_error("child index out of range"))?,
                    )
                    .ok_or_else(|| vm.new_index_error("child index out of range"))
                }),
                ass_item: atomic_func!(|seq, i, value, vm| {
                    let zelf = PyElement::sequence_downcast(seq);
                    let recycle = if let Some(value) = value {
                        check_element(&value, vm)?;
                        let mut inner = zelf.inner.write();
                        let pos = inner.children.wrap_index(i).ok_or_else(|| {
                            vm.new_index_error("child assignment index out of range")
                        })?;
                        core::mem::replace(&mut inner.children[pos], value)
                    } else {
                        let mut inner = zelf.inner.write();
                        let pos = inner.children.wrap_index(i).ok_or_else(|| {
                            vm.new_index_error("child assignment index out of range")
                        })?;
                        inner.children.remove(pos)
                    };
                    drop(recycle);
                    Ok(())
                }),
                ..PySequenceMethods::NOT_IMPLEMENTED
            };
            &AS_SEQUENCE
        }
    }

    impl AsNumber for PyElement {
        fn as_number() -> &'static PyNumberMethods {
            static AS_NUMBER: PyNumberMethods = PyNumberMethods {
                boolean: Some(|number, vm| {
                    let zelf = number.obj.downcast_ref::<PyElement>().unwrap();
                    crate::vm::warn::warn(
                        vm.ctx
                            .new_str(
                                "Testing an element's truth value will always return True in \
                                 future versions.  Use specific 'len(elem)' or 'elem is not None' \
                                 test instead.",
                            )
                            .into(),
                        Some(vm.ctx.exceptions.deprecation_warning.to_owned()),
                        1,
                        None,
                        vm,
                    )?;
                    Ok(zelf.len() != 0)
                }),
                ..PyNumberMethods::NOT_IMPLEMENTED
            };
            &AS_NUMBER
        }
    }

    impl Representable for PyElement {
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let tag = zelf.tag_obj();
            Ok(format!(
                "<Element {} at {:#x}>",
                tag.repr(vm)?,
                zelf.get_id()
            ))
        }
    }

    #[pyfunction(name = "SubElement")]
    fn sub_element(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let parent = args.args.first().cloned().ok_or_else(|| {
            vm.new_type_error("SubElement() takes at least 2 arguments (0 given)")
        })?;
        let parent = parent.downcast::<PyElement>().map_err(|obj| {
            vm.new_type_error(format!(
                "SubElement() argument 1 must be xml.etree.ElementTree.Element, not {}",
                obj.class().name()
            ))
        })?;
        let (tag, attrib) = parse_attrib_args(args, "SubElement", 1, vm)?;
        let element = create_element(parent.class().to_owned(), tag, attrib, vm)?;
        parent.push(element.clone());
        Ok(element)
    }

    // -----------------------------------------------------------------
    // TreeBuilder
    // -----------------------------------------------------------------

    #[derive(Debug, Default)]
    struct TreeBuilderState {
        /// First element created; what `close()` hands back.
        root: Option<PyObjectRef>,
        /// Element currently open (`None` == CPython's `Py_None`).
        this: Option<PyObjectRef>,
        /// Most recently created element.
        last: Option<PyObjectRef>,
        /// Most recently *closed* node, i.e. the one character data now
        /// belongs to as a tail rather than as text.
        last_for_tail: Option<PyObjectRef>,
        /// Character data seen since the last flush: a single string while
        /// there is only one fragment, a list once there are more.
        data: Option<PyObjectRef>,
        stack: Vec<Option<PyObjectRef>>,
        element_factory: Option<PyObjectRef>,
        comment_factory: Option<PyObjectRef>,
        pi_factory: Option<PyObjectRef>,
        /// `.append` of the XMLPullParser event queue, once `_setevents`
        /// has wired this builder up for event reporting.
        events_append: Option<PyObjectRef>,
        start_event: Option<PyObjectRef>,
        end_event: Option<PyObjectRef>,
        start_ns_event: Option<PyObjectRef>,
        end_ns_event: Option<PyObjectRef>,
        comment_event: Option<PyObjectRef>,
        pi_event: Option<PyObjectRef>,
        insert_comments: bool,
        insert_pis: bool,
    }

    #[pyattr]
    #[pyclass(
        module = "xml.etree.ElementTree",
        name = "TreeBuilder",
        traverse = "manual"
    )]
    #[derive(Debug, PyPayload)]
    pub(crate) struct PyTreeBuilder {
        state: PyRwLock<TreeBuilderState>,
    }

    // SAFETY: every owned PyObjectRef held by the builder is visited.
    unsafe impl crate::vm::object::Traverse for PyTreeBuilder {
        fn traverse(&self, traverse_fn: &mut crate::vm::object::TraverseFn<'_>) {
            let Some(st) = self.state.try_read() else {
                return;
            };
            st.root.traverse(traverse_fn);
            st.this.traverse(traverse_fn);
            st.last.traverse(traverse_fn);
            st.last_for_tail.traverse(traverse_fn);
            st.data.traverse(traverse_fn);
            st.stack.traverse(traverse_fn);
            st.element_factory.traverse(traverse_fn);
            st.comment_factory.traverse(traverse_fn);
            st.pi_factory.traverse(traverse_fn);
            st.events_append.traverse(traverse_fn);
            st.start_event.traverse(traverse_fn);
            st.end_event.traverse(traverse_fn);
            st.start_ns_event.traverse(traverse_fn);
            st.end_ns_event.traverse(traverse_fn);
            st.comment_event.traverse(traverse_fn);
            st.pi_event.traverse(traverse_fn);
        }

        fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
            let Some(mut st) = self.state.try_write() else {
                return;
            };
            out.extend(st.stack.drain(..).flatten());
            out.extend(
                [
                    st.root.take(),
                    st.this.take(),
                    st.last.take(),
                    st.last_for_tail.take(),
                    st.data.take(),
                    st.element_factory.take(),
                    st.comment_factory.take(),
                    st.pi_factory.take(),
                    st.events_append.take(),
                    st.start_event.take(),
                    st.end_event.take(),
                    st.start_ns_event.take(),
                    st.end_ns_event.take(),
                    st.comment_event.take(),
                    st.pi_event.take(),
                ]
                .into_iter()
                .flatten(),
            );
        }
    }

    #[derive(FromArgs)]
    pub(crate) struct TreeBuilderArgs {
        #[pyarg(any, default)]
        element_factory: Option<PyObjectRef>,
        #[pyarg(named, default)]
        comment_factory: Option<PyObjectRef>,
        #[pyarg(named, default)]
        pi_factory: Option<PyObjectRef>,
        #[pyarg(named, default = false)]
        insert_comments: bool,
        #[pyarg(named, default = false)]
        insert_pis: bool,
    }

    impl DefaultConstructor for PyTreeBuilder {}

    impl Default for PyTreeBuilder {
        fn default() -> Self {
            Self {
                state: PyRwLock::new(TreeBuilderState::default()),
            }
        }
    }

    impl Initializer for PyTreeBuilder {
        type Args = TreeBuilderArgs;

        fn init(zelf: PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
            let module = module_state(vm)?;
            let element_factory = args.element_factory.filter(|f| !vm.is_none(f));
            let comment_factory = match args.comment_factory {
                Some(f) if !vm.is_none(&f) => Some(f),
                // A `None` comment_factory means "use whatever
                // `_set_factories` installed", not "no factory".
                _ => module.comment_factory.read().clone(),
            };
            let pi_factory = match args.pi_factory {
                Some(f) if !vm.is_none(&f) => Some(f),
                _ => module.pi_factory.read().clone(),
            };
            let _recycle = {
                let mut st = zelf.state.write();
                st.insert_comments = comment_factory.is_some() && args.insert_comments;
                st.insert_pis = pi_factory.is_some() && args.insert_pis;
                (
                    core::mem::replace(&mut st.element_factory, element_factory),
                    core::mem::replace(&mut st.comment_factory, comment_factory),
                    core::mem::replace(&mut st.pi_factory, pi_factory),
                )
            };
            Ok(())
        }
    }

    /// Append `child` to `element`, taking the direct route when the parent
    /// really is one of our elements and falling back to `.append()` for
    /// subclasses and foreign targets.
    fn tb_add_subelement(
        element: &PyObject,
        child: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if element.class().is(PyElement::class(&vm.ctx)) {
            check_element(&child, vm)?;
            element.downcast_ref::<PyElement>().unwrap().push(child);
            Ok(())
        } else {
            vm.call_method(element, "append", (child,))?;
            Ok(())
        }
    }

    /// Attach the buffered character data to `element` as its text or tail.
    fn extend_text_or_tail(
        element: &PyObject,
        data: PyObjectRef,
        is_tail: bool,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if element.class().is(PyElement::class(&vm.ctx)) {
            let elem = element.downcast_ref::<PyElement>().unwrap();
            enum Fast {
                Done,
                Pending(PyObjectRef),
                Slow,
            }
            let outcome = {
                let mut inner = elem.inner.write();
                let slot = if is_tail {
                    &mut inner.tail
                } else {
                    &mut inner.text
                };
                if vm.is_none(&slot.obj) {
                    // Nothing there yet: adopt the fragment (or fragment
                    // list) as-is and let the join happen on first read.
                    let pending = data.downcastable::<PyList>();
                    slot.obj = data.clone();
                    slot.pending = pending;
                    Fast::Done
                } else if slot.pending {
                    Fast::Pending(slot.obj.clone())
                } else {
                    Fast::Slow
                }
            };
            match outcome {
                Fast::Done => return Ok(()),
                Fast::Pending(list) => {
                    let list = list
                        .downcast::<PyList>()
                        .map_err(|_| vm.new_type_error("internal error (text)"))?;
                    if let Some(more) = data.downcast_ref::<PyList>() {
                        let items = more.borrow_vec().to_vec();
                        list.borrow_vec_mut().extend(items);
                    } else {
                        list.borrow_vec_mut().push(data);
                    }
                    return Ok(());
                }
                Fast::Slow => {}
            }
        }
        let name = if is_tail { "tail" } else { "text" };
        let previous = element.get_attr(name, vm)?;
        let joined = list_join(&data, vm)?;
        let joined = if vm.is_none(&previous) {
            joined
        } else {
            vm._add(&previous, &joined)?
        };
        element.set_attr(name, joined, vm)?;
        Ok(())
    }

    impl PyTreeBuilder {
        fn append_event(
            &self,
            action: Option<PyObjectRef>,
            node: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let Some(action) = action else { return Ok(()) };
            let append = self.state.read().events_append.clone();
            let Some(append) = append else { return Ok(()) };
            let event = vm.ctx.new_tuple(vec![action, node]);
            append.call((event,), vm)?;
            Ok(())
        }

        fn flush_data(&self, vm: &VirtualMachine) -> PyResult<()> {
            let Some((data, target, is_tail)) = ({
                let mut st = self.state.write();
                match st.data.take() {
                    None => None,
                    Some(data) => match st.last_for_tail.clone() {
                        Some(target) => Some((data, target, true)),
                        None => st.last.clone().map(|target| (data, target, false)),
                    },
                }
            }) else {
                return Ok(());
            };
            extend_text_or_tail(&target, data, is_tail, vm)
        }

        pub(crate) fn handle_start(
            &self,
            tag: PyObjectRef,
            attrib: Option<PyDictRef>,
            vm: &VirtualMachine,
        ) -> PyResult {
            self.flush_data(vm)?;
            let factory = self.state.read().element_factory.clone();
            let node = match factory {
                // CPython adopts the caller's dict rather than copying it.
                None => PyElement::new(tag, attrib, vm).into_ref(&vm.ctx).into(),
                Some(factory) => {
                    let attrib = attrib.unwrap_or_else(|| vm.ctx.new_dict());
                    factory.call((tag, attrib), vm)?
                }
            };
            let this = {
                let mut st = self.state.write();
                st.last_for_tail = None;
                st.this.clone()
            };
            match &this {
                Some(this) => tb_add_subelement(this, node.clone(), vm)?,
                None => {
                    let mut st = self.state.write();
                    if st.root.is_some() {
                        drop(st);
                        return Err(new_parse_error(
                            "multiple elements on top level",
                            None,
                            None,
                            vm,
                        )?);
                    }
                    st.root = Some(node.clone());
                }
            }
            let start_event = {
                let mut st = self.state.write();
                st.stack.push(this);
                st.this = Some(node.clone());
                st.last = Some(node.clone());
                st.start_event.clone()
            };
            self.append_event(start_event, node.clone(), vm)?;
            Ok(node)
        }

        pub(crate) fn handle_data(&self, data: PyObjectRef, vm: &VirtualMachine) {
            let existing = {
                let mut st = self.state.write();
                match st.data.take() {
                    None => {
                        if st.last.is_none() {
                            // Data before the first start tag is dropped.
                            return;
                        }
                        st.data = Some(data);
                        return;
                    }
                    Some(existing) => existing,
                }
            };
            // Growing the fragment list touches only the list's own lock.
            if let Some(list) = existing.downcast_ref::<PyList>() {
                list.borrow_vec_mut().push(data);
                self.state.write().data = Some(existing);
            } else {
                let list = vm.ctx.new_list(vec![existing, data]);
                self.state.write().data = Some(list.into());
            }
        }

        pub(crate) fn handle_end(&self, vm: &VirtualMachine) -> PyResult {
            self.flush_data(vm)?;
            let (last, end_event) = {
                let mut st = self.state.write();
                let Some(parent) = st.stack.pop() else {
                    return Err(vm.new_index_error("pop from empty stack"));
                };
                let closed = st.this.take();
                st.last.clone_from(&closed);
                st.last_for_tail.clone_from(&closed);
                st.this = parent;
                (closed, st.end_event.clone())
            };
            let last = last.unwrap_or_else(|| vm.ctx.none());
            self.append_event(end_event, last.clone(), vm)?;
            Ok(last)
        }

        pub(crate) fn handle_comment(&self, text: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            self.flush_data(vm)?;
            let factory = self.state.read().comment_factory.clone();
            let comment = match factory {
                Some(factory) => {
                    let comment = factory.call((text,), vm)?;
                    let (insert, this) = {
                        let st = self.state.read();
                        (st.insert_comments, st.this.clone())
                    };
                    if insert && let Some(this) = this {
                        tb_add_subelement(&this, comment.clone(), vm)?;
                        self.state.write().last_for_tail = Some(comment.clone());
                    }
                    comment
                }
                None => text,
            };
            let event = self.state.read().comment_event.clone();
            self.append_event(event, comment.clone(), vm)?;
            Ok(comment)
        }

        pub(crate) fn handle_pi(
            &self,
            target: PyObjectRef,
            text: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult {
            self.flush_data(vm)?;
            let factory = self.state.read().pi_factory.clone();
            let pi = match factory {
                Some(factory) => {
                    let pi = factory.call((target, text), vm)?;
                    let (insert, this) = {
                        let st = self.state.read();
                        (st.insert_pis, st.this.clone())
                    };
                    if insert && let Some(this) = this {
                        tb_add_subelement(&this, pi.clone(), vm)?;
                        self.state.write().last_for_tail = Some(pi.clone());
                    }
                    pi
                }
                None => vm.ctx.new_tuple(vec![target, text]).into(),
            };
            let event = self.state.read().pi_event.clone();
            self.append_event(event, pi.clone(), vm)?;
            Ok(pi)
        }

        pub(crate) fn handle_start_ns(
            &self,
            prefix: PyObjectRef,
            uri: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let event = self.state.read().start_ns_event.clone();
            if event.is_none() {
                return Ok(());
            }
            let parcel = vm.ctx.new_tuple(vec![prefix, uri]);
            self.append_event(event, parcel.into(), vm)
        }

        pub(crate) fn handle_end_ns(
            &self,
            prefix: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let event = self.state.read().end_ns_event.clone();
            if event.is_none() {
                return Ok(());
            }
            self.append_event(event, prefix, vm)
        }

        fn done(&self, vm: &VirtualMachine) -> PyObjectRef {
            self.state
                .read()
                .root
                .clone()
                .unwrap_or_else(|| vm.ctx.none())
        }

        fn set_events(
            &self,
            events_append: PyObjectRef,
            events_to_report: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            {
                let mut st = self.state.write();
                st.events_append = Some(events_append);
                st.start_event = None;
                st.end_event = None;
                st.start_ns_event = None;
                st.end_ns_event = None;
                st.comment_event = None;
                st.pi_event = None;
                if vm.is_none(&events_to_report) {
                    st.end_event = Some(vm.ctx.new_str("end").into());
                    return Ok(());
                }
            }
            let events: Vec<PyObjectRef> = events_to_report
                .try_to_value(vm)
                .map_err(|_| vm.new_type_error("events must be a sequence"))?;
            for event in events {
                let name = if let Some(s) = event.downcast_ref::<PyStr>() {
                    s.as_bytes().to_vec()
                } else if let Some(b) = event.downcast_ref::<crate::vm::builtins::PyBytes>() {
                    b.as_bytes().to_vec()
                } else {
                    return Err(vm.new_value_error("invalid events sequence"));
                };
                let mut st = self.state.write();
                match name.as_slice() {
                    b"start" => st.start_event = Some(event),
                    b"end" => st.end_event = Some(event),
                    b"start-ns" => st.start_ns_event = Some(event),
                    b"end-ns" => st.end_ns_event = Some(event),
                    b"comment" => st.comment_event = Some(event),
                    b"pi" => st.pi_event = Some(event),
                    _ => {
                        drop(st);
                        return Err(
                            vm.new_value_error(format!("unknown event {}", event.repr(vm)?))
                        );
                    }
                }
            }
            Ok(())
        }

        fn wants_pi(&self) -> bool {
            let st = self.state.read();
            st.insert_pis || (st.events_append.is_some() && st.pi_event.is_some())
        }
    }

    #[pyclass(with(Constructor, Initializer), flags(BASETYPE, HAS_WEAKREF))]
    impl PyTreeBuilder {
        #[pymethod]
        fn start(&self, tag: PyObjectRef, attrs: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let attrs = attrs.downcast::<PyDict>().map_err(|obj| {
                vm.new_type_error(format!(
                    "start() argument 2 must be dict, not {}",
                    obj.class().name()
                ))
            })?;
            self.handle_start(tag, Some(attrs), vm)
        }

        #[pymethod]
        fn data(&self, data: PyObjectRef, vm: &VirtualMachine) {
            self.handle_data(data, vm);
        }

        #[pymethod]
        fn end(&self, _tag: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            self.handle_end(vm)
        }

        #[pymethod]
        fn comment(&self, text: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            self.handle_comment(text, vm)
        }

        #[pymethod]
        fn pi(
            &self,
            target: PyObjectRef,
            text: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult {
            self.handle_pi(target, text.unwrap_or_none(vm), vm)
        }

        #[pymethod]
        fn close(&self, vm: &VirtualMachine) -> PyObjectRef {
            self.done(vm)
        }
    }

    // -----------------------------------------------------------------
    // XMLParser
    // -----------------------------------------------------------------

    fn new_parse_error(
        message: &str,
        code: Option<i32>,
        position: Option<(i64, i64)>,
        vm: &VirtualMachine,
    ) -> PyResult<crate::vm::builtins::PyBaseExceptionRef> {
        let state = module_state(vm)?;
        let cls = state
            .parse_error
            .read()
            .clone()
            .ok_or_else(|| vm.new_runtime_error("ParseError is missing"))?;
        let exc = vm.invoke_exception(&cls, vec![vm.ctx.new_str(message).into()])?;
        exc.as_object()
            .set_attr("code", vm.ctx.new_int(code.unwrap_or(0)), vm)?;
        let (line, column) = position.unwrap_or((0, 0));
        exc.as_object().set_attr(
            "position",
            vm.ctx.new_tuple(vec![
                vm.ctx.new_int(line).into(),
                vm.ctx.new_int(column).into(),
            ]),
            vm,
        )?;
        Ok(exc)
    }

    #[derive(Debug, Default)]
    struct XMLParserState {
        /// The `pyexpat.xmlparser` doing the actual scanning, or `None`
        /// before `__init__` (and after `close()` dropped it).
        parser: Option<PyObjectRef>,
        target: Option<PyObjectRef>,
        /// The target again, when it is exactly our own `TreeBuilder`, so a
        /// per-event dispatch is one clone rather than a type check.
        native_target: Option<PyRef<PyTreeBuilder>>,
        entity: Option<PyDictRef>,
        /// Cache of raw expat names to their `{uri}local` form.
        names: Option<PyDictRef>,
        handle_start: Option<PyObjectRef>,
        handle_end: Option<PyObjectRef>,
        handle_data: Option<PyObjectRef>,
        handle_comment: Option<PyObjectRef>,
        handle_pi: Option<PyObjectRef>,
        handle_start_ns: Option<PyObjectRef>,
        handle_end_ns: Option<PyObjectRef>,
        handle_close: Option<PyObjectRef>,
        handle_doctype: Option<PyObjectRef>,
    }

    #[pyattr]
    #[pyclass(
        module = "xml.etree.ElementTree",
        name = "XMLParser",
        traverse = "manual"
    )]
    #[derive(Debug, PyPayload)]
    pub(crate) struct PyXMLParser {
        state: PyRwLock<XMLParserState>,
    }

    // SAFETY: every owned PyObjectRef held by the parser is visited.
    unsafe impl crate::vm::object::Traverse for PyXMLParser {
        fn traverse(&self, traverse_fn: &mut crate::vm::object::TraverseFn<'_>) {
            let Some(st) = self.state.try_read() else {
                return;
            };
            st.parser.traverse(traverse_fn);
            st.target.traverse(traverse_fn);
            st.native_target.traverse(traverse_fn);
            st.entity.traverse(traverse_fn);
            st.names.traverse(traverse_fn);
            st.handle_start.traverse(traverse_fn);
            st.handle_end.traverse(traverse_fn);
            st.handle_data.traverse(traverse_fn);
            st.handle_comment.traverse(traverse_fn);
            st.handle_pi.traverse(traverse_fn);
            st.handle_start_ns.traverse(traverse_fn);
            st.handle_end_ns.traverse(traverse_fn);
            st.handle_close.traverse(traverse_fn);
            st.handle_doctype.traverse(traverse_fn);
        }

        fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
            let Some(mut st) = self.state.try_write() else {
                return;
            };
            out.extend(
                [
                    st.parser.take(),
                    st.target.take(),
                    st.native_target.take().map(Into::into),
                    st.entity.take().map(Into::into),
                    st.names.take().map(Into::into),
                    st.handle_start.take(),
                    st.handle_end.take(),
                    st.handle_data.take(),
                    st.handle_comment.take(),
                    st.handle_pi.take(),
                    st.handle_start_ns.take(),
                    st.handle_end_ns.take(),
                    st.handle_close.take(),
                    st.handle_doctype.take(),
                ]
                .into_iter()
                .flatten(),
            );
        }
    }

    #[derive(FromArgs)]
    pub(crate) struct XMLParserArgs {
        #[pyarg(named, default)]
        target: Option<PyObjectRef>,
        #[pyarg(named, default)]
        encoding: Option<PyObjectRef>,
    }

    impl DefaultConstructor for PyXMLParser {}

    impl Default for PyXMLParser {
        fn default() -> Self {
            Self {
                state: PyRwLock::new(XMLParserState::default()),
            }
        }
    }

    /// `getattr(target, name)`, treating a missing attribute as "the target
    /// does not implement this callback" rather than an error.
    fn optional_handler(
        target: &PyObject,
        name: &'static str,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        vm.get_attribute_opt(target.to_owned(), name)
    }

    impl Initializer for PyXMLParser {
        type Args = XMLParserArgs;

        fn init(zelf: PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
            let encoding = match args.encoding {
                Some(e) if !vm.is_none(&e) => {
                    if !e.downcastable::<PyStr>() {
                        return Err(vm.new_type_error(format!(
                            "XMLParser() argument 'encoding' must be str or None, not {}",
                            e.class().name()
                        )));
                    }
                    e
                }
                _ => vm.ctx.none(),
            };
            let expat = vm.import("pyexpat", 0)?;
            // The "}" namespace separator makes expat report names as
            // "uri}local", which `makeuniversal` turns into "{uri}local".
            let parser = vm.call_method(&expat, "ParserCreate", (encoding, "}"))?;
            parser.set_attr("buffer_text", vm.ctx.new_int(1), vm)?;
            parser.set_attr("ordered_attributes", vm.ctx.new_int(1), vm)?;

            let target = match args.target {
                Some(t) if !vm.is_none(&t) => t,
                _ => PyTreeBuilder::default().into_ref(&vm.ctx).into(),
            };

            let handlers = XMLParserState {
                parser: Some(parser.clone()),
                entity: Some(vm.ctx.new_dict()),
                names: Some(vm.ctx.new_dict()),
                handle_start_ns: optional_handler(&target, "start_ns", vm)?,
                handle_end_ns: optional_handler(&target, "end_ns", vm)?,
                handle_start: optional_handler(&target, "start", vm)?,
                handle_data: optional_handler(&target, "data", vm)?,
                handle_end: optional_handler(&target, "end", vm)?,
                handle_comment: optional_handler(&target, "comment", vm)?,
                handle_pi: optional_handler(&target, "pi", vm)?,
                handle_close: optional_handler(&target, "close", vm)?,
                handle_doctype: optional_handler(&target, "doctype", vm)?,
                native_target: target
                    .class()
                    .is(PyTreeBuilder::class(&vm.ctx))
                    .then(|| target.clone().downcast::<PyTreeBuilder>().ok())
                    .flatten(),
                target: Some(target),
            };
            let has_comment = handlers.handle_comment.is_some();
            let has_pi = handlers.handle_pi.is_some();
            let has_ns = handlers.handle_start_ns.is_some() || handlers.handle_end_ns.is_some();
            *zelf.state.write() = handlers;

            // Expat calls back into these; they dispatch straight to the
            // target (and, when it is our own TreeBuilder, straight into
            // Rust) without a Python frame in between.
            let this = zelf.as_object();
            parser.set_attr(
                "StartElementHandler",
                this.get_attr("_expat_start", vm)?,
                vm,
            )?;
            parser.set_attr("EndElementHandler", this.get_attr("_expat_end", vm)?, vm)?;
            parser.set_attr(
                "CharacterDataHandler",
                this.get_attr("_expat_data", vm)?,
                vm,
            )?;
            if has_comment {
                parser.set_attr("CommentHandler", this.get_attr("_expat_comment", vm)?, vm)?;
            }
            if has_pi {
                parser.set_attr(
                    "ProcessingInstructionHandler",
                    this.get_attr("_expat_pi", vm)?,
                    vm,
                )?;
            }
            if has_ns {
                install_ns_handlers(zelf.as_object(), &parser, vm)?;
            }
            Ok(())
        }
    }

    /// Namespace declarations are only reported when someone asked for
    /// them, either by giving the target `start_ns`/`end_ns` methods or by
    /// requesting the `start-ns`/`end-ns` pull-parser events.
    fn install_ns_handlers(
        this: &PyObject,
        parser: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        parser.set_attr(
            "StartNamespaceDeclHandler",
            this.get_attr("_expat_start_ns", vm)?,
            vm,
        )?;
        parser.set_attr(
            "EndNamespaceDeclHandler",
            this.get_attr("_expat_end_ns", vm)?,
            vm,
        )?;
        Ok(())
    }

    impl PyXMLParser {
        fn check(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let st = self.state.read();
            if st.target.is_none() {
                return Err(vm.new_value_error("XMLParser.__init__() wasn't called"));
            }
            st.parser
                .clone()
                .ok_or_else(|| vm.new_value_error("XMLParser.__init__() wasn't called"))
        }

        /// The target, when it is exactly our own `TreeBuilder` and can
        /// therefore be driven without going back through Python.
        fn native_target(&self, _vm: &VirtualMachine) -> Option<PyRef<PyTreeBuilder>> {
            self.state.read().native_target.clone()
        }

        /// Turn expat's "uri}local" into ElementTree's "{uri}local",
        /// memoized so a repeated tag costs one dictionary hit.
        fn make_universal(&self, name: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let names = self.state.read().names.clone();
            let Some(names) = names else { return Ok(name) };
            if let Some(cached) = names.get_item_opt(&*name, vm)? {
                return Ok(cached);
            }
            let value = match name.downcast_ref::<PyStr>() {
                Some(s) if s.as_bytes().contains(&b'}') => {
                    vm.ctx.new_str(format!("{{{}", s.as_wtf8())).into()
                }
                _ => name.clone(),
            };
            names.set_item(&*name, value.clone(), vm)?;
            Ok(value)
        }

        fn raise_expat_error(&self, err: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let message = err.str(vm)?;
            let code = err
                .get_attr("code", vm)
                .ok()
                .and_then(|c| i32::try_from_object(vm, c).ok());
            let lineno = err
                .get_attr("lineno", vm)
                .ok()
                .and_then(|c| i64::try_from_object(vm, c).ok());
            let offset = err
                .get_attr("offset", vm)
                .ok()
                .and_then(|c| i64::try_from_object(vm, c).ok());
            let position = match (lineno, offset) {
                (Some(l), Some(o)) => Some((l, o)),
                _ => None,
            };
            Err(new_parse_error(message.as_ref(), code, position, vm)?)
        }

        /// Feed one chunk through expat, translating its `ExpatError` into
        /// the `ParseError` callers of `xml.etree` expect.
        fn expat_parse(
            &self,
            data: PyObjectRef,
            final_: bool,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let parser = self.check(vm)?;
            match vm.call_method(&parser, "Parse", (data, final_)) {
                Ok(_) => Ok(()),
                Err(e) => {
                    let expat = vm.import("pyexpat", 0)?;
                    let error_type = expat.get_attr("error", vm)?;
                    if let Ok(error_type) = error_type.downcast::<PyType>()
                        && e.fast_isinstance(&error_type)
                    {
                        self.raise_expat_error(e.into(), vm)?;
                        unreachable!()
                    }
                    Err(e)
                }
            }
        }
    }

    #[pyclass(with(Constructor, Initializer), flags(BASETYPE, HAS_WEAKREF))]
    impl PyXMLParser {
        #[pygetset]
        fn entity(&self, vm: &VirtualMachine) -> PyObjectRef {
            self.state
                .read()
                .entity
                .clone()
                .map_or_else(|| vm.ctx.none(), Into::into)
        }

        #[pygetset]
        fn target(&self, vm: &VirtualMachine) -> PyObjectRef {
            self.state
                .read()
                .target
                .clone()
                .unwrap_or_else(|| vm.ctx.none())
        }

        #[pygetset]
        fn version(&self, vm: &VirtualMachine) -> PyResult<String> {
            let expat = vm.import("pyexpat", 0)?;
            let info: Vec<PyObjectRef> = expat.get_attr("version_info", vm)?.try_to_value(vm)?;
            let part = |i: usize| -> String {
                info.get(i)
                    .map(|o| o.str(vm).map(|s| s.to_string()).unwrap_or_default())
                    .unwrap_or_default()
            };
            Ok(format!("Expat {}.{}.{}", part(0), part(1), part(2)))
        }

        #[pymethod]
        fn feed(&self, data: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            self.expat_parse(data, false, vm)
        }

        #[pymethod]
        fn close(&self, vm: &VirtualMachine) -> PyResult {
            self.expat_parse(vm.ctx.new_bytes(vec![]).into(), true, vm)?;
            if let Some(builder) = self.native_target(vm) {
                return Ok(builder.done(vm));
            }
            let close = self.state.read().handle_close.clone();
            match close {
                Some(close) => close.call((), vm),
                None => Ok(vm.ctx.none()),
            }
        }

        #[pymethod]
        fn flush(&self, vm: &VirtualMachine) -> PyResult<()> {
            // This backend has no reparse deferral to turn off, so, like the
            // C accelerator built against an expat without
            // XML_SetReparseDeferralEnabled, there is nothing to flush.
            self.check(vm)?;
            Ok(())
        }

        #[pymethod]
        fn _parse_whole(&self, file: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            self.check(vm)?;
            let read = file.get_attr("read", vm)?;
            loop {
                let buffer = read.call((64 * 1024,), vm)?;
                if let Some(s) = buffer.downcast_ref::<PyStr>() {
                    if s.is_empty() {
                        break;
                    }
                } else if let Some(b) = buffer.downcast_ref::<crate::vm::builtins::PyBytes>() {
                    if b.as_bytes().is_empty() {
                        break;
                    }
                } else {
                    break;
                }
                self.expat_parse(buffer, false, vm)?;
            }
            self.expat_parse(vm.ctx.new_bytes(vec![]).into(), true, vm)?;
            if let Some(builder) = self.native_target(vm) {
                return Ok(builder.done(vm));
            }
            Ok(vm.ctx.none())
        }

        #[pymethod]
        fn _setevents(
            zelf: &Py<Self>,
            events_queue: PyObjectRef,
            events_to_report: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let parser = zelf.check(vm)?;
            let Some(builder) = zelf.native_target(vm) else {
                return Err(vm.new_type_error(
                    "event handling only supported for ElementTree.TreeBuilder targets",
                ));
            };
            let append = events_queue.get_attr("append", vm)?;
            builder.set_events(append, events_to_report.unwrap_or_none(vm), vm)?;
            // Comments and processing instructions are only reported once
            // asked for, so their handlers are installed lazily here.
            let this = zelf.as_object();
            parser.set_attr("CommentHandler", this.get_attr("_expat_comment", vm)?, vm)?;
            parser.set_attr(
                "ProcessingInstructionHandler",
                this.get_attr("_expat_pi", vm)?,
                vm,
            )?;
            install_ns_handlers(this, &parser, vm)?;
            Ok(())
        }

        #[pymethod]
        fn _expat_start(
            zelf: &Py<Self>,
            tag: PyObjectRef,
            attr_list: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let tag = zelf.make_universal(tag, vm)?;
            let items = attr_list
                .downcast_ref::<PyList>()
                .map(|l| l.borrow_vec().to_vec())
                .unwrap_or_default();
            let attrib = if items.is_empty() {
                None
            } else {
                let dict = vm.ctx.new_dict();
                let mut i = 0;
                while i + 1 < items.len() {
                    let key = zelf.make_universal(items[i].clone(), vm)?;
                    dict.set_item(&*key, items[i + 1].clone(), vm)?;
                    i += 2;
                }
                Some(dict)
            };
            if let Some(builder) = zelf.native_target(vm) {
                builder.handle_start(tag, attrib, vm)?;
                return Ok(());
            }
            let handler = zelf.state.read().handle_start.clone();
            if let Some(handler) = handler {
                let attrib = attrib.unwrap_or_else(|| vm.ctx.new_dict());
                handler.call((tag, attrib), vm)?;
            }
            Ok(())
        }

        #[pymethod]
        fn _expat_end(zelf: &Py<Self>, tag: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            if let Some(builder) = zelf.native_target(vm) {
                // The standard tree builder does not look at the end tag.
                builder.handle_end(vm)?;
                return Ok(());
            }
            let handler = zelf.state.read().handle_end.clone();
            if let Some(handler) = handler {
                let tag = zelf.make_universal(tag, vm)?;
                handler.call((tag,), vm)?;
            }
            Ok(())
        }

        #[pymethod]
        fn _expat_data(zelf: &Py<Self>, data: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            if let Some(builder) = zelf.native_target(vm) {
                builder.handle_data(data, vm);
                return Ok(());
            }
            let handler = zelf.state.read().handle_data.clone();
            if let Some(handler) = handler {
                handler.call((data,), vm)?;
            }
            Ok(())
        }

        #[pymethod]
        fn _expat_comment(zelf: &Py<Self>, text: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            if let Some(builder) = zelf.native_target(vm) {
                builder.handle_comment(text, vm)?;
                return Ok(());
            }
            let handler = zelf.state.read().handle_comment.clone();
            if let Some(handler) = handler {
                handler.call((text,), vm)?;
            }
            Ok(())
        }

        #[pymethod]
        fn _expat_start_ns(
            zelf: &Py<Self>,
            prefix: PyObjectRef,
            uri: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if let Some(builder) = zelf.native_target(vm) {
                // The standard tree builder has no start_ns() of its own; it
                // only forwards the event when one was asked for.
                return builder.handle_start_ns(prefix, uri, vm);
            }
            let handler = zelf.state.read().handle_start_ns.clone();
            if let Some(handler) = handler {
                handler.call((prefix, uri), vm)?;
            }
            Ok(())
        }

        #[pymethod]
        fn _expat_end_ns(
            zelf: &Py<Self>,
            prefix: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if let Some(builder) = zelf.native_target(vm) {
                return builder.handle_end_ns(vm.ctx.none(), vm);
            }
            let handler = zelf.state.read().handle_end_ns.clone();
            if let Some(handler) = handler {
                handler.call((prefix,), vm)?;
            }
            Ok(())
        }

        #[pymethod]
        fn _expat_pi(
            zelf: &Py<Self>,
            target: PyObjectRef,
            data: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if let Some(builder) = zelf.native_target(vm) {
                if builder.wants_pi() {
                    builder.handle_pi(target, data, vm)?;
                }
                return Ok(());
            }
            let handler = zelf.state.read().handle_pi.clone();
            if let Some(handler) = handler {
                handler.call((target, data), vm)?;
            }
            Ok(())
        }
    }

    // -----------------------------------------------------------------
    // Element iterator
    // -----------------------------------------------------------------

    #[derive(Debug)]
    struct ParentLocator {
        parent: PyRef<PyElement>,
        child_index: usize,
    }

    /// Pre-order traversal shared by `Element.iter()` and
    /// `Element.itertext()`, kept as an explicit parent stack so a deep tree
    /// costs no Rust recursion.
    #[pyclass(no_attr, module = "_elementtree", name = "_element_iterator")]
    #[derive(Debug, PyPayload)]
    pub(crate) struct PyElementIter {
        state: PyRwLock<ElementIterState>,
    }

    #[derive(Debug)]
    struct ElementIterState {
        parent_stack: Vec<ParentLocator>,
        root: Option<PyRef<PyElement>>,
        sought_tag: PyObjectRef,
        gettext: bool,
    }

    impl PyElementIter {
        fn new(root: PyRef<PyElement>, sought_tag: PyObjectRef, gettext: bool) -> Self {
            Self {
                state: PyRwLock::new(ElementIterState {
                    parent_stack: Vec::new(),
                    root: Some(root),
                    sought_tag,
                    gettext,
                }),
            }
        }
    }

    #[pyclass(with(IterNext, Iterable), flags(DISALLOW_INSTANTIATION))]
    impl PyElementIter {}

    impl SelfIter for PyElementIter {}

    impl IterNext for PyElementIter {
        fn next(
            zelf: &Py<Self>,
            vm: &VirtualMachine,
        ) -> PyResult<crate::vm::protocol::PyIterReturn> {
            use crate::vm::protocol::PyIterReturn;
            loop {
                // Pull the next candidate element out of the traversal, or a
                // pending tail text when running for itertext().
                enum Step {
                    Elem(PyRef<PyElement>),
                    Text(PyObjectRef),
                    Done,
                }
                let step = {
                    let mut state = zelf.state.write();
                    if state.parent_stack.is_empty() {
                        match state.root.take() {
                            Some(root) => Step::Elem(root),
                            None => Step::Done,
                        }
                    } else {
                        let gettext = state.gettext;
                        let last = state.parent_stack.len() - 1;
                        let item = &mut state.parent_stack[last];
                        let parent = item.parent.clone();
                        let child_index = item.child_index;
                        match parent.child(child_index) {
                            Some(child) => {
                                item.child_index += 1;
                                drop(state);
                                let child = child.downcast::<PyElement>().map_err(|obj| {
                                    vm.new_type_error(format!(
                                        "expected an Element, not \"{}\"",
                                        obj.class().name()
                                    ))
                                })?;
                                Step::Elem(child)
                            }
                            None => {
                                state.parent_stack.pop();
                                // itertext() reports only *inner* text, so
                                // the tail of the element iteration started
                                // from is skipped.
                                let want_tail = gettext && !state.parent_stack.is_empty();
                                drop(state);
                                if want_tail {
                                    Step::Text(parent.joined_get(true, vm)?)
                                } else {
                                    continue;
                                }
                            }
                        }
                    }
                };
                let elem = match step {
                    Step::Done => return Ok(PyIterReturn::StopIteration(None)),
                    Step::Text(text) => {
                        if !vm.is_none(&text) && text.clone().is_true(vm)? {
                            return Ok(PyIterReturn::Return(text));
                        }
                        continue;
                    }
                    Step::Elem(elem) => elem,
                };
                let (gettext, sought_tag) = {
                    let state = zelf.state.read();
                    (state.gettext, state.sought_tag.clone())
                };
                zelf.state.write().parent_stack.push(ParentLocator {
                    parent: elem.clone(),
                    child_index: 0,
                });
                if gettext {
                    let tag = elem.tag_obj();
                    if !vm.is_none(&tag) && !tag.downcastable::<PyStr>() {
                        continue;
                    }
                    let text = elem.joined_get(false, vm)?;
                    if !vm.is_none(&text) && text.clone().is_true(vm)? {
                        return Ok(PyIterReturn::Return(text));
                    }
                    continue;
                }
                if vm.is_none(&sought_tag) {
                    return Ok(PyIterReturn::Return(elem.into()));
                }
                if vm.bool_eq(&elem.tag_obj(), &sought_tag)? {
                    return Ok(PyIterReturn::Return(elem.into()));
                }
            }
        }
    }
}
