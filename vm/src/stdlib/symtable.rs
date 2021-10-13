pub(crate) use symtable::make_module;

#[pymodule]
mod symtable {
    use crate::{
        builtins::PyStrRef,
        compile::{self, Symbol, SymbolScope, SymbolTable, SymbolTableType},
        PyObjectRef, PyRef, PyResult, PyValue, VirtualMachine,
    };
    use std::fmt;

    /// symtable. Return top level SymbolTable.
    /// See docs: https://docs.python.org/3/library/symtable.html?highlight=symtable#symtable.symtable
    #[pyfunction]
    fn symtable(
        source: PyStrRef,
        filename: PyStrRef,
        mode: PyStrRef,
        vm: &VirtualMachine,
    ) -> PyResult<PySymbolTableRef> {
        let mode = mode
            .as_str()
            .parse::<compile::Mode>()
            .map_err(|err| vm.new_value_error(err.to_string()))?;

        let symtable = compile::compile_symtable(source.as_str(), mode, filename.as_str())
            .map_err(|err| vm.new_syntax_error(&err))?;

        let py_symbol_table = to_py_symbol_table(symtable);
        Ok(py_symbol_table.into_ref(vm))
    }

    fn to_py_symbol_table(symtable: SymbolTable) -> PySymbolTable {
        PySymbolTable { symtable }
    }

    type PySymbolTableRef = PyRef<PySymbolTable>;
    type PySymbolRef = PyRef<PySymbol>;

    #[pyattr]
    #[pyclass(name = "SymbolTable")]
    #[derive(PyValue)]
    struct PySymbolTable {
        symtable: SymbolTable,
    }

    impl fmt::Debug for PySymbolTable {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "SymbolTable()")
        }
    }

    #[pyimpl]
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
        fn get_lineno(&self) -> usize {
            self.symtable.line_number
        }

        #[pymethod]
        fn is_nested(&self) -> bool {
            self.symtable.is_nested
        }

        #[pymethod]
        fn is_optimized(&self) -> bool {
            self.symtable.typ == SymbolTableType::Function
        }

        #[pymethod]
        fn lookup(&self, name: PyStrRef, vm: &VirtualMachine) -> PyResult<PySymbolRef> {
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
                }
                .into_ref(vm))
            } else {
                Err(vm.new_key_error(vm.ctx.new_str(format!("lookup {} failed", name)).into()))
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
                    })
                    .into_ref(vm)
                    .into()
                })
                .collect();
            Ok(symbols)
        }

        #[pymethod]
        fn has_children(&self) -> bool {
            !self.symtable.sub_tables.is_empty()
        }

        #[pymethod]
        fn get_children(&self, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
            let children = self
                .symtable
                .sub_tables
                .iter()
                .map(|t| to_py_symbol_table(t.clone()).into_object(vm))
                .collect();
            Ok(children)
        }
    }

    #[pyattr]
    #[pyclass(name = "Symbol")]
    #[derive(PyValue)]
    struct PySymbol {
        symbol: Symbol,
        namespaces: Vec<SymbolTable>,
    }

    impl fmt::Debug for PySymbol {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "Symbol()")
        }
    }

    #[pyimpl]
    impl PySymbol {
        #[pymethod]
        fn get_name(&self) -> String {
            self.symbol.name.clone()
        }

        #[pymethod]
        fn is_global(&self) -> bool {
            self.symbol.is_global()
        }

        #[pymethod]
        fn is_local(&self) -> bool {
            self.symbol.is_local()
        }

        #[pymethod]
        fn is_imported(&self) -> bool {
            self.symbol.is_imported
        }

        #[pymethod]
        fn is_nested(&self) -> bool {
            // TODO
            false
        }

        #[pymethod]
        fn is_nonlocal(&self) -> bool {
            self.symbol.is_nonlocal
        }

        #[pymethod]
        fn is_referenced(&self) -> bool {
            self.symbol.is_referenced
        }

        #[pymethod]
        fn is_assigned(&self) -> bool {
            self.symbol.is_assigned
        }

        #[pymethod]
        fn is_parameter(&self) -> bool {
            self.symbol.is_parameter
        }

        #[pymethod]
        fn is_free(&self) -> bool {
            matches!(self.symbol.scope, SymbolScope::Free)
        }

        #[pymethod]
        fn is_namespace(&self) -> bool {
            !self.namespaces.is_empty()
        }

        #[pymethod]
        fn is_annotated(&self) -> bool {
            self.symbol.is_annotated
        }

        #[pymethod]
        fn get_namespaces(&self, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
            let namespaces = self
                .namespaces
                .iter()
                .map(|table| to_py_symbol_table(table.clone()).into_object(vm))
                .collect();
            Ok(namespaces)
        }

        #[pymethod]
        fn get_namespace(&self, vm: &VirtualMachine) -> PyResult {
            if self.namespaces.len() != 1 {
                return Err(
                    vm.new_value_error("namespace is bound to multiple namespaces".to_owned())
                );
            }
            Ok(to_py_symbol_table(self.namespaces.first().unwrap().clone())
                .into_ref(vm)
                .into())
        }
    }
}
