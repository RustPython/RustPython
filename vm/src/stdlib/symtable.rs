pub(crate) use symtable::make_module;

#[pymodule]
mod symtable {
    use crate::{
        PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine, builtins::PyStrRef, compiler,
    };
    use rustpython_codegen::symboltable::{
        CompilerScope, Symbol, SymbolFlags, SymbolScope, SymbolTable,
    };
    use std::fmt;

    // Consts as defined at
    // https://github.com/python/cpython/blob/6cb20a219a860eaf687b2d968b41c480c7461909/Include/internal/pycore_symtable.h#L156

    #[pyattr]
    pub const DEF_GLOBAL: i32 = 1;

    #[pyattr]
    pub const DEF_LOCAL: i32 = 2;

    #[pyattr]
    pub const DEF_PARAM: i32 = 2 << 1;

    #[pyattr]
    pub const DEF_NONLOCAL: i32 = 2 << 2;

    #[pyattr]
    pub const USE: i32 = 2 << 3;

    #[pyattr]
    pub const DEF_FREE: i32 = 2 << 4;

    #[pyattr]
    pub const DEF_FREE_CLASS: i32 = 2 << 5;

    #[pyattr]
    pub const DEF_IMPORT: i32 = 2 << 6;

    #[pyattr]
    pub const DEF_ANNOT: i32 = 2 << 7;

    #[pyattr]
    pub const DEF_COMP_ITER: i32 = 2 << 8;

    #[pyattr]
    pub const DEF_TYPE_PARAM: i32 = 2 << 9;

    #[pyattr]
    pub const DEF_COMP_CELL: i32 = 2 << 10;

    #[pyattr]
    pub const DEF_BOUND: i32 = DEF_LOCAL | DEF_PARAM | DEF_IMPORT;

    #[pyattr]
    pub const SCOPE_OFFSET: i32 = 12;

    #[pyattr]
    pub const SCOPE_MASK: i32 = DEF_GLOBAL | DEF_LOCAL | DEF_PARAM | DEF_NONLOCAL;

    #[pyattr]
    pub const LOCAL: i32 = 1;

    #[pyattr]
    pub const GLOBAL_EXPLICIT: i32 = 2;

    #[pyattr]
    pub const GLOBAL_IMPLICIT: i32 = 3;

    #[pyattr]
    pub const FREE: i32 = 4;

    #[pyattr]
    pub const CELL: i32 = 5;

    #[pyattr]
    pub const GENERATOR: i32 = 1;

    #[pyattr]
    pub const GENERATOR_EXPRESSION: i32 = 2;

    #[pyfunction]
    fn symtable(
        source: PyStrRef,
        filename: PyStrRef,
        mode: PyStrRef,
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
    #[pyclass(name = "SymbolTable")]
    #[derive(PyPayload)]
    struct PySymbolTable {
        symtable: SymbolTable,
    }

    impl fmt::Debug for PySymbolTable {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "SymbolTable()")
        }
    }

    #[pyclass]
    impl PySymbolTable {
        #[pymethod]
        fn get_name(&self) -> String {
            self.symtable.name.clone()
        }

        #[pymethod]
        fn get_type(&self) -> String {
            self.symtable.typ.to_string()
        }

        #[pymethod]
        const fn get_lineno(&self) -> u32 {
            self.symtable.line_number
        }

        #[pymethod]
        const fn is_nested(&self) -> bool {
            self.symtable.is_nested
        }

        #[pymethod]
        fn is_optimized(&self) -> bool {
            matches!(
                self.symtable.typ,
                CompilerScope::Function | CompilerScope::AsyncFunction
            )
        }

        #[pymethod]
        fn lookup(&self, name: PyStrRef, vm: &VirtualMachine) -> PyResult<PyRef<PySymbol>> {
            let name = name.as_str();
            if let Some(symbol) = self.symtable.symbols.get(name) {
                Ok(PySymbol {
                    symbol: symbol.clone(),
                    namespaces: self
                        .symtable
                        .sub_tables
                        .iter()
                        .filter(|table| table.name == name)
                        .cloned()
                        .collect(),
                    is_top_scope: self.symtable.name == "top",
                }
                .into_ref(&vm.ctx))
            } else {
                Err(vm.new_key_error(vm.ctx.new_str(format!("lookup {name} failed")).into()))
            }
        }

        #[pymethod]
        fn get_identifiers(&self, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
            let symbols = self
                .symtable
                .symbols
                .keys()
                .map(|s| vm.ctx.new_str(s.as_str()).into())
                .collect();
            Ok(symbols)
        }

        #[pymethod]
        fn get_symbols(&self, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
            let symbols = self
                .symtable
                .symbols
                .values()
                .map(|s| {
                    (PySymbol {
                        symbol: s.clone(),
                        namespaces: self
                            .symtable
                            .sub_tables
                            .iter()
                            .filter(|&table| table.name == s.name)
                            .cloned()
                            .collect(),
                        is_top_scope: self.symtable.name == "top",
                    })
                    .into_ref(&vm.ctx)
                    .into()
                })
                .collect();
            Ok(symbols)
        }

        #[pymethod]
        const fn has_children(&self) -> bool {
            !self.symtable.sub_tables.is_empty()
        }

        #[pymethod]
        fn get_children(&self, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
            let children = self
                .symtable
                .sub_tables
                .iter()
                .map(|t| to_py_symbol_table(t.clone()).into_pyobject(vm))
                .collect();
            Ok(children)
        }
    }

    #[pyattr]
    #[pyclass(name = "Symbol")]
    #[derive(PyPayload)]
    struct PySymbol {
        symbol: Symbol,
        namespaces: Vec<SymbolTable>,
        is_top_scope: bool,
    }

    impl fmt::Debug for PySymbol {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "Symbol()")
        }
    }

    #[pyclass]
    impl PySymbol {
        #[pymethod]
        fn get_name(&self) -> String {
            self.symbol.name.clone()
        }

        #[pymethod]
        const fn is_global(&self) -> bool {
            self.symbol.is_global() || (self.is_top_scope && self.symbol.is_bound())
        }

        #[pymethod]
        const fn is_declared_global(&self) -> bool {
            matches!(self.symbol.scope, SymbolScope::GlobalExplicit)
        }

        #[pymethod]
        const fn is_local(&self) -> bool {
            self.symbol.is_local() || (self.is_top_scope && self.symbol.is_bound())
        }

        #[pymethod]
        const fn is_imported(&self) -> bool {
            self.symbol.flags.contains(SymbolFlags::IMPORTED)
        }

        #[pymethod]
        const fn is_nested(&self) -> bool {
            // TODO
            false
        }

        #[pymethod]
        const fn is_nonlocal(&self) -> bool {
            self.symbol.flags.contains(SymbolFlags::NONLOCAL)
        }

        #[pymethod]
        const fn is_referenced(&self) -> bool {
            self.symbol.flags.contains(SymbolFlags::REFERENCED)
        }

        #[pymethod]
        const fn is_assigned(&self) -> bool {
            self.symbol.flags.contains(SymbolFlags::ASSIGNED)
        }

        #[pymethod]
        const fn is_parameter(&self) -> bool {
            self.symbol.flags.contains(SymbolFlags::PARAMETER)
        }

        #[pymethod]
        const fn is_free(&self) -> bool {
            matches!(self.symbol.scope, SymbolScope::Free)
        }

        #[pymethod]
        const fn is_namespace(&self) -> bool {
            !self.namespaces.is_empty()
        }

        #[pymethod]
        const fn is_annotated(&self) -> bool {
            self.symbol.flags.contains(SymbolFlags::ANNOTATED)
        }

        #[pymethod]
        fn get_namespaces(&self, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
            let namespaces = self
                .namespaces
                .iter()
                .map(|table| to_py_symbol_table(table.clone()).into_pyobject(vm))
                .collect();
            Ok(namespaces)
        }

        #[pymethod]
        fn get_namespace(&self, vm: &VirtualMachine) -> PyResult {
            if self.namespaces.len() != 1 {
                return Err(vm.new_value_error("namespace is bound to multiple namespaces"));
            }
            Ok(to_py_symbol_table(self.namespaces.first().unwrap().clone())
                .into_ref(&vm.ctx)
                .into())
        }
    }
}
