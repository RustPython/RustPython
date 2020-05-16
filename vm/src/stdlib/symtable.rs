use std::fmt;

use rustpython_compiler::{compile, error::CompileError, symboltable};
use rustpython_parser::parser;

use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let symbol_table_type = PySymbolTable::make_class(ctx);
    let symbol_type = PySymbol::make_class(ctx);

    py_module!(vm, "symtable", {
        "symtable" => ctx.new_function(symtable_symtable),
        "SymbolTable" => symbol_table_type,
        "Symbol" => symbol_type,
    })
}

/// symtable. Return top level SymbolTable.
/// See docs: https://docs.python.org/3/library/symtable.html?highlight=symtable#symtable.symtable
fn symtable_symtable(
    source: PyStringRef,
    filename: PyStringRef,
    mode: PyStringRef,
    vm: &VirtualMachine,
) -> PyResult<PySymbolTableRef> {
    let mode = mode
        .as_str()
        .parse::<compile::Mode>()
        .map_err(|err| vm.new_value_error(err.to_string()))?;
    let symtable = source_to_symtable(source.as_str(), mode).map_err(|mut err| {
        err.update_source_path(filename.as_str());
        vm.new_syntax_error(&err)
    })?;

    let py_symbol_table = to_py_symbol_table(symtable);
    Ok(py_symbol_table.into_ref(vm))
}

fn source_to_symtable(
    source: &str,
    mode: compile::Mode,
) -> Result<symboltable::SymbolTable, CompileError> {
    let symtable = match mode {
        compile::Mode::Exec | compile::Mode::Single => {
            let ast = parser::parse_program(source)?;
            symboltable::make_symbol_table(&ast)?
        }
        compile::Mode::Eval => {
            let statement = parser::parse_statement(source)?;
            symboltable::statements_to_symbol_table(&statement)?
        }
    };

    Ok(symtable)
}

fn to_py_symbol_table(symtable: symboltable::SymbolTable) -> PySymbolTable {
    PySymbolTable { symtable }
}

type PySymbolTableRef = PyRef<PySymbolTable>;
type PySymbolRef = PyRef<PySymbol>;

#[pyclass(name = "SymbolTable")]
struct PySymbolTable {
    symtable: symboltable::SymbolTable,
}

impl fmt::Debug for PySymbolTable {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "SymbolTable()")
    }
}

impl PyValue for PySymbolTable {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("symtable", "SymbolTable")
    }
}

#[pyimpl]
impl PySymbolTable {
    #[pymethod(name = "get_name")]
    fn get_name(&self) -> String {
        self.symtable.name.clone()
    }

    #[pymethod(name = "get_type")]
    fn get_type(&self) -> String {
        self.symtable.typ.to_string()
    }

    #[pymethod(name = "get_lineno")]
    fn get_lineno(&self) -> usize {
        self.symtable.line_number
    }

    #[pymethod(name = "lookup")]
    fn lookup(&self, name: PyStringRef, vm: &VirtualMachine) -> PyResult<PySymbolRef> {
        let name = name.as_str();
        if let Some(symbol) = self.symtable.symbols.get(name) {
            Ok(PySymbol {
                symbol: symbol.clone(),
            }
            .into_ref(vm))
        } else {
            Err(vm.new_lookup_error(name.to_owned()))
        }
    }

    #[pymethod(name = "get_identifiers")]
    fn get_identifiers(&self, vm: &VirtualMachine) -> PyResult {
        let symbols = self
            .symtable
            .symbols
            .keys()
            .map(|s| vm.ctx.new_str(s.to_owned()))
            .collect();
        Ok(vm.ctx.new_list(symbols))
    }

    #[pymethod(name = "get_symbols")]
    fn get_symbols(&self, vm: &VirtualMachine) -> PyResult {
        let symbols = self
            .symtable
            .symbols
            .values()
            .map(|s| (PySymbol { symbol: s.clone() }).into_ref(vm).into_object())
            .collect();
        Ok(vm.ctx.new_list(symbols))
    }

    #[pymethod(name = "has_children")]
    fn has_children(&self) -> bool {
        !self.symtable.sub_tables.is_empty()
    }

    #[pymethod(name = "get_children")]
    fn get_children(&self, vm: &VirtualMachine) -> PyResult {
        let children = self
            .symtable
            .sub_tables
            .iter()
            .map(|t| to_py_symbol_table(t.clone()).into_ref(vm).into_object())
            .collect();
        Ok(vm.ctx.new_list(children))
    }
}

#[pyclass(name = "Symbol")]
struct PySymbol {
    symbol: symboltable::Symbol,
}

impl fmt::Debug for PySymbol {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Symbol()")
    }
}

impl PyValue for PySymbol {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("symtable", "Symbol")
    }
}

#[pyimpl]
impl PySymbol {
    #[pymethod(name = "get_name")]
    fn get_name(&self) -> String {
        self.symbol.name.clone()
    }

    #[pymethod(name = "is_global")]
    fn is_global(&self) -> bool {
        self.symbol.is_global()
    }

    #[pymethod(name = "is_local")]
    fn is_local(&self) -> bool {
        self.symbol.is_local()
    }

    #[pymethod(name = "is_referenced")]
    fn is_referenced(&self) -> bool {
        self.symbol.is_referenced
    }

    #[pymethod(name = "is_assigned")]
    fn is_assigned(&self) -> bool {
        self.symbol.is_assigned
    }

    #[pymethod(name = "is_parameter")]
    fn is_parameter(&self) -> bool {
        self.symbol.is_parameter
    }

    #[pymethod(name = "is_free")]
    fn is_free(&self) -> bool {
        self.symbol.is_free
    }

    #[pymethod(name = "is_namespace")]
    fn is_namespace(&self) -> bool {
        // TODO
        false
    }
}
