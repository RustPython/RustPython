//! The compiler's constant cache (`c_const_cache`).
//!
//! Equal constants and code fields produced by one compilation share a single
//! object, merged under their `_PyCode_ConstantKey`.

use super::*;
use crate::builtins::{PyDict, PyDictRef, PyTuple};

/// `const_cache_insert`. Items of a tuple or frozenset constant are created
/// through the cache before the container, so a single non-recursive insert
/// also merges them.
fn const_cache_insert(
    const_cache: &Py<PyDict>,
    o: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<PyObjectRef> {
    // None and Ellipsis are singletons, and key is the singleton.
    if vm.is_none(&o) || o.is(&vm.ctx.ellipsis) {
        return Ok(o);
    }
    let key = constant_key(&o, vm)?;
    let key = match const_cache.get_item_opt(&*key, vm)? {
        Some(existing) => existing,
        None => {
            const_cache.set_item(&*key, key.clone(), vm)?;
            key
        }
    };
    // See constant_key()
    Ok(match key.downcast_ref_if_exact::<PyTuple>(vm) {
        Some(key) => key.as_slice()[1].clone(),
        None => key,
    })
}

/// A [`ConstantBag`] creating the constants of one compilation through its
/// constant cache.
#[derive(Clone, Copy)]
struct CompileBag<'a> {
    vm: &'a VirtualMachine,
    const_cache: &'a Py<PyDict>,
}

impl CompileBag<'_> {
    /// `_PyCompile_ConstCacheMergeOne`
    fn merge_one<T: PyPayload>(&self, obj: PyRef<T>) -> PyRef<T> {
        self.merge(obj.into())
            .downcast()
            .expect("an equal constant key implies the same type")
    }

    fn merge(&self, obj: PyObjectRef) -> PyObjectRef {
        const_cache_insert(self.const_cache, obj, self.vm)
            .expect("constant keys are hashable and comparable")
    }

    /// Create a code object whose fields are merged the way `assemble.c`
    /// merges them, in the same order.
    fn new_code(&self, code: CodeObject) -> PyCode {
        let mut code = PyCode::new(code, &self.vm.ctx);
        code.apply_debug_ranges(self.vm);
        code.co_exceptiontable = self.merge_one(code.co_exceptiontable);
        code.co_linetable = self.merge_one(code.co_linetable);
        code.co_names = self.merge_one(code.co_names);
        let consts = self.merge_one(code.code.constants.as_tuple().as_pytuple().clone());
        code.code.constants = bytecode::Constants::new(LiteralTuple(consts));
        code.co_localsplusnames = self.merge_one(code.co_localsplusnames);
        code
    }
}

impl ConstantBag for CompileBag<'_> {
    type Constant = Literal;

    fn make_constant<C: Constant>(&self, constant: BorrowedConstant<'_, C>) -> Literal {
        let vm = self.vm;
        let obj = match constant {
            BorrowedConstant::Code { code } => return self.make_code(code.map_clone_bag(self)),
            BorrowedConstant::Tuple { elements } => vm
                .ctx
                .new_tuple(
                    elements
                        .iter()
                        .map(|item| self.make_constant(item.borrow_constant()).0)
                        .collect(),
                )
                .into(),
            BorrowedConstant::Frozenset { elements } => PyFrozenSet::from_iter(
                vm,
                elements
                    .iter()
                    .map(|item| self.make_constant(item.borrow_constant()).0),
            )
            .unwrap()
            .into_ref(&vm.ctx)
            .into(),
            BorrowedConstant::Slice { elements } => {
                let [start, stop, step] = elements
                    .each_ref()
                    .map(|item| self.make_constant(item.borrow_constant()).0);
                crate::builtins::PySlice {
                    start: Some(start),
                    stop,
                    step: Some(step),
                }
                .into_ref(&vm.ctx)
                .into()
            }
            _ => PyVmBag(vm).make_constant(constant).0,
        };
        Literal(self.merge(obj))
    }

    fn make_name(&self, name: &str) -> &'static PyStrInterned {
        self.vm.ctx.intern_str(name)
    }

    fn make_int(&self, value: BigInt) -> Literal {
        Literal(self.merge(self.vm.ctx.new_int(value).into()))
    }

    fn make_tuple(&self, elements: impl Iterator<Item = Literal>) -> Literal {
        let tuple = self.vm.ctx.new_tuple(elements.map(|item| item.0).collect());
        Literal(self.merge(tuple.into()))
    }

    fn make_code(&self, code: CodeObject) -> Literal {
        let code = self.new_code(code).into_ref(&self.vm.ctx);
        Literal(self.merge(code.into()))
    }
}

pub(super) fn into_code_object(code: bytecode::CodeObject, vm: &VirtualMachine) -> CodeObject {
    let const_cache: PyDictRef = vm.ctx.new_dict();
    code.map_bag(CompileBag {
        vm,
        const_cache: &const_cache,
    })
}

pub(super) fn into_pycode(code: bytecode::CodeObject, vm: &VirtualMachine) -> PyCode {
    let const_cache: PyDictRef = vm.ctx.new_dict();
    let bag = CompileBag {
        vm,
        const_cache: &const_cache,
    };
    bag.new_code(code.map_bag(bag))
}
