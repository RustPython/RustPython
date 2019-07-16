use crate::eval::get_compile_mode;
use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;
use rustpython_compiler::{compile, error::CompileError, symboltable};
use rustpython_parser::parser;
use std::fmt;

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let symbol_table_type = PySymbolTable::make_class(ctx);
    let symbol_type = PySymbol::make_class(ctx);

    py_module!(vm, "symtable", {
        "symtable" => ctx.new_rustfunc(symtable_symtable),
        "SymbolTable" => symbol_table_type,
        "Symbol" => symbol_type,
    })
}

/// symtable. Return top level SymbolTable.
/// See docs: https://docs.python.org/3/library/symtable.html?highlight=symtable#symtable.symtable
fn symtable_symtable(
    source: PyStringRef,
    _filename: PyStringRef,
    mode: PyStringRef,
    vm: &VirtualMachine,
) -> PyResult<PySymbolTableRef> {
    let mode = get_compile_mode(vm, &mode.value)?;
    let symtable =
        source_to_symtable(&source.value, mode).map_err(|err| vm.new_syntax_error(&err))?;

    let py_symbol_table = to_py_symbol_table("top".to_string(), symtable);
    Ok(py_symbol_table.into_ref(vm))
}

fn source_to_symtable(
    source: &str,
    mode: compile::Mode,
) -> Result<symboltable::SymbolScope, CompileError> {
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

fn to_py_symbol_table(name: String, symtable: symboltable::SymbolScope) -> PySymbolTable {
    PySymbolTable { name, symtable }
}

type PySymbolTableRef = PyRef<PySymbolTable>;
type PySymbolRef = PyRef<PySymbol>;

#[pyclass(name = "SymbolTable")]
struct PySymbolTable {
    name: String,
    symtable: symboltable::SymbolScope,
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
    fn get_name(&self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_str(self.name.clone()))
    }

    #[pymethod(name = "lookup")]
    fn lookup(&self, name: PyStringRef, vm: &VirtualMachine) -> PyResult<PySymbolRef> {
        let name = &name.value;
        if self.symtable.symbols.contains_key(name) {
            let role = self.symtable.symbols.get(name).unwrap().clone();
            Ok(PySymbol {
                name: name.to_string(),
                role,
            }
            .into_ref(vm))
        } else {
            Err(vm.ctx.new_str(name.to_string()))
        }
    }

    #[pymethod(name = "get_symbols")]
    fn get_symbols(&self, vm: &VirtualMachine) -> PyResult {
        let symbols = self
            .symtable
            .symbols
            .iter()
            .map(|s| vm.ctx.new_str(s.0.clone()))
            .collect();
        Ok(vm.ctx.new_list(symbols))
    }

    #[pymethod(name = "get_children")]
    fn get_children(&self, vm: &VirtualMachine) -> PyResult {
        let children = self
            .symtable
            .sub_scopes
            .iter()
            .map(|s| {
                to_py_symbol_table("bla".to_string(), s.clone())
                    .into_ref(vm)
                    .into_object()
            })
            .collect();
        Ok(vm.ctx.new_list(children))
    }
}

#[pyclass(name = "Symbol")]
struct PySymbol {
    name: String,
    role: symboltable::SymbolRole,
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
    fn get_name(&self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_str(self.name.clone()))
    }

    #[pymethod(name = "is_global")]
    fn is_global(&self, vm: &VirtualMachine) -> PyResult {
        // TODO: figure out how this is determined.
        let is_local = if let symboltable::SymbolRole::Used = self.role {
            true
        } else {
            false
        };
        Ok(vm.ctx.new_bool(is_local))
    }

    #[pymethod(name = "is_local")]
    fn is_local(&self, vm: &VirtualMachine) -> PyResult {
        // TODO: figure out how this is determined.
        let is_local = if let symboltable::SymbolRole::Used = self.role {
            true
        } else {
            false
        };
        Ok(vm.ctx.new_bool(is_local))
    }

    #[pymethod(name = "is_referenced")]
    fn is_referenced(&self, vm: &VirtualMachine) -> PyResult {
        let is_local = if let symboltable::SymbolRole::Used = self.role {
            true
        } else {
            false
        };
        Ok(vm.ctx.new_bool(is_local))
    }

    #[pymethod(name = "is_assigned")]
    fn is_assigned(&self, vm: &VirtualMachine) -> PyResult {
        let is_local = if let symboltable::SymbolRole::Assigned = self.role {
            true
        } else {
            false
        };
        Ok(vm.ctx.new_bool(is_local))
    }

    #[pymethod(name = "is_parameter")]
    fn is_parameter(&self, vm: &VirtualMachine) -> PyResult {
        // TODO
        Ok(vm.ctx.new_bool(false))
    }

    #[pymethod(name = "is_namespace")]
    fn is_namespace(&self, vm: &VirtualMachine) -> PyResult {
        // TODO
        Ok(vm.ctx.new_bool(false))
    }
}
