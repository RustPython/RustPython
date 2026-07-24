pub(crate) use _symtable::module_def;

#[pymodule]
mod _symtable {
    use crate::{
        Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyDictRef, PyUtf8StrRef},
        compiler,
        types::Representable,
    };
    use alloc::fmt;
    use rustpython_codegen::symboltable::{CompilerScope, SymbolFlags, SymbolScope, SymbolTable};

    /// [CPython's `SCOPE_OFFSET`](https://github.com/python/cpython/blob/v3.14.6/Include/internal/pycore_symtable.h#L176)
    const SCOPE_OFFSET: i32 = 12;

    // Consts as defined at
    // https://github.com/python/cpython/blob/6cb20a219a860eaf687b2d968b41c480c7461909/Include/internal/pycore_symtable.h#L156

    #[pyattr]
    pub(super) const DEF_GLOBAL: i32 = SymbolFlags::DEF_GLOBAL.bits() as i32;

    #[pyattr]
    pub(super) const DEF_LOCAL: i32 = SymbolFlags::DEF_LOCAL.bits() as i32;

    #[pyattr]
    pub(super) const DEF_PARAM: i32 = SymbolFlags::DEF_PARAM.bits() as i32;

    #[pyattr]
    pub(super) const DEF_NONLOCAL: i32 = SymbolFlags::DEF_NONLOCAL.bits() as i32;

    #[pyattr]
    pub(super) const USE: i32 = SymbolFlags::USE.bits() as i32;

    #[pyattr]
    pub(super) const DEF_FREE_CLASS: i32 = SymbolFlags::DEF_FREE_CLASS.bits() as i32;

    #[pyattr]
    pub(super) const DEF_IMPORT: i32 = SymbolFlags::DEF_IMPORT.bits() as i32;

    #[pyattr]
    pub(super) const DEF_ANNOT: i32 = SymbolFlags::DEF_ANNOT.bits() as i32;

    #[pyattr]
    pub(super) const DEF_COMP_ITER: i32 = SymbolFlags::DEF_COMP_ITER.bits() as i32;

    #[pyattr]
    pub(super) const DEF_TYPE_PARAM: i32 = SymbolFlags::DEF_TYPE_PARAM.bits() as i32;

    #[pyattr]
    pub(super) const DEF_COMP_CELL: i32 = SymbolFlags::DEF_COMP_CELL.bits() as i32;

    #[pyattr]
    pub(super) const DEF_BOUND: i32 = SymbolFlags::DEF_BOUND.bits() as i32;

    #[pyattr]
    pub(super) const SCOPE_MASK: i32 = DEF_GLOBAL | DEF_LOCAL | DEF_PARAM | DEF_NONLOCAL;

    #[pyattr]
    pub(super) const LOCAL: i32 = SymbolScope::Local.as_i32();

    #[pyattr]
    pub(super) const GLOBAL_EXPLICIT: i32 = SymbolScope::GlobalExplicit.as_i32();

    #[pyattr]
    pub(super) const GLOBAL_IMPLICIT: i32 = SymbolScope::GlobalImplicit.as_i32();

    #[pyattr]
    pub(super) const FREE: i32 = SymbolScope::Free.as_i32();

    #[pyattr]
    pub(super) const CELL: i32 = SymbolScope::Cell.as_i32();

    #[pyattr]
    pub(super) const SCOPE_OFF: i32 = SCOPE_OFFSET;

    #[pyattr]
    pub(super) const TYPE_FUNCTION: i32 = 0;

    #[pyattr]
    pub(super) const TYPE_CLASS: i32 = 1;

    #[pyattr]
    pub(super) const TYPE_MODULE: i32 = 2;

    #[pyattr]
    pub(super) const TYPE_ANNOTATION: i32 = 3;

    #[pyattr]
    pub(super) const TYPE_TYPE_ALIAS: i32 = 4;

    #[pyattr]
    pub(super) const TYPE_TYPE_PARAMETERS: i32 = 5;

    #[pyattr]
    pub(super) const TYPE_TYPE_VARIABLE: i32 = 6;

    #[pyfunction]
    fn symtable(
        source: PyUtf8StrRef,
        filename: PyUtf8StrRef,
        mode: PyUtf8StrRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<PySymbolTable>> {
        let mode = mode
            .as_str()
            .parse::<compiler::Mode>()
            .map_err(|err| vm.new_value_error(err.to_string()))?;

        let symtable = compiler::compile_symtable(source.as_str(), mode, filename.as_str())
            .map_err(|err| vm.new_syntax_error(&err, Some(source.as_str())))?;

        let py_symbol_table = to_py_symbol_table(symtable);
        Ok(py_symbol_table.into_ref(&vm.ctx))
    }

    const fn to_py_symbol_table(symtable: SymbolTable) -> PySymbolTable {
        PySymbolTable { symtable }
    }

    #[pyattr]
    #[pyclass(name = "symtable entry")]
    #[derive(PyPayload)]
    struct PySymbolTable {
        symtable: SymbolTable,
    }

    impl fmt::Debug for PySymbolTable {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "SymbolTable()")
        }
    }

    #[pyclass(with(Representable))]
    impl PySymbolTable {
        #[pygetset]
        fn name(&self) -> String {
            self.symtable.name.to_string()
        }

        #[pygetset(name = "type")]
        fn typ(&self) -> i32 {
            match self.symtable.typ {
                CompilerScope::Function
                | CompilerScope::AsyncFunction
                | CompilerScope::Lambda
                | CompilerScope::Comprehension => TYPE_FUNCTION,
                CompilerScope::Class => TYPE_CLASS,
                CompilerScope::Module => TYPE_MODULE,
                CompilerScope::Annotation => TYPE_ANNOTATION,
                CompilerScope::TypeAlias => TYPE_TYPE_ALIAS,
                CompilerScope::TypeParams => TYPE_TYPE_PARAMETERS,
                CompilerScope::TypeVariable => TYPE_TYPE_VARIABLE,
            }
        }

        #[pygetset]
        const fn lineno(&self) -> u32 {
            self.symtable.line_number
        }

        #[pygetset]
        fn children(&self, vm: &VirtualMachine) -> Vec<PyObjectRef> {
            self.symtable
                .sub_tables
                .iter()
                .flat_map(|t| {
                    if t.comp_inlined {
                        // Flatten: replace inlined comprehension tables with their children
                        t.sub_tables.iter().collect::<Vec<_>>()
                    } else {
                        vec![t]
                    }
                })
                .map(|t| to_py_symbol_table(t.clone()).into_pyobject(vm))
                .collect()
        }

        #[pygetset]
        fn id(&self) -> usize {
            self as *const Self as *const core::ffi::c_void as usize
        }

        #[pygetset]
        fn symbols(&self, vm: &VirtualMachine) -> PyDictRef {
            let dict = vm.ctx.new_dict();
            for (name, symbol) in &self.symtable.symbols {
                dict.set_item(name.as_str(), vm.new_pyobj(symbol.flags.bits()), vm)
                    .unwrap();
            }
            dict
        }

        #[pygetset]
        const fn nested(&self) -> bool {
            self.symtable.is_nested
        }
    }

    impl Representable for PySymbolTable {
        #[inline]
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            Ok(format!(
                "<{} {}({}), line {}>",
                Self::class(&vm.ctx).name(),
                zelf.symtable.name,
                zelf.id(),
                zelf.symtable.line_number
            ))
        }
    }
}
