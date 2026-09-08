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
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        atomic_func,
        builtins::{PyDict, PyDictRef, PyList, PyModule, PyStr, PyType, PyTypeRef},
        function::{FuncArgs, OptionalArg, PySetterValue},
        protocol::{PyMappingMethods, PyNumberMethods, PySequenceMethods},
        sliceable::{SequenceIndex, SliceableSequenceOp},
        types::{
            AsMapping, AsNumber, AsSequence, Constructor, Initializer, IterNext, Iterable,
            Representable, SelfIter,
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
