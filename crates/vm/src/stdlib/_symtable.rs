pub(crate) use _symtable::module_def;

#[pymodule]
mod _symtable {
    use crate::{
        AsObject, Py, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyDictRef, PyListRef, PyStrRef, PyUtf8StrRef},
        compiler,
        function::{ArgStrOrBytesLike, FsPath},
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
        source: ArgStrOrBytesLike,
        filename: FsPath,
        mode: PyUtf8StrRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<PySymbolTable>> {
        let mode = mode
            .as_str()
            .parse::<compiler::Mode>()
            .map_err(|err| vm.new_value_error(err.to_string()))?;

        let filename_obj = match &filename {
            FsPath::Str(filename) => filename.clone(),
            FsPath::Bytes(filename) => {
                let filename = FsPath::bytes_as_os_str(filename.as_bytes(), vm)?.to_owned();
                vm.fsdecode(filename)
            }
        };
        let filename = filename_obj.to_string_lossy();
        let source = match &source {
            ArgStrOrBytesLike::Str(source) => source.try_as_utf8(vm)?.as_str().to_owned(),
            ArgStrOrBytesLike::Buf(source) => vm
                .decode_source_bytes(&source.borrow_buf(), &filename, false)
                .map_err(|err| set_syntax_error_filename(err, &filename_obj, vm))?,
        };
        if source.as_bytes().contains(&0) {
            return Err(vm.new_exception_msg(
                vm.ctx.exceptions.syntax_error.to_owned(),
                "source code string cannot contain null bytes".into(),
            ));
        }
        let symtable = compiler::compile_symtable(&source, mode, &filename).map_err(|err| {
            let err = vm.new_syntax_error(&err, Some(&source));
            set_syntax_error_filename(err, &filename_obj, vm)
        })?;

        Ok(to_py_symbol_table(symtable, vm))
    }

    fn set_syntax_error_filename(
        err: PyBaseExceptionRef,
        filename: &PyStrRef,
        vm: &VirtualMachine,
    ) -> PyBaseExceptionRef {
        if err.fast_isinstance(vm.ctx.exceptions.syntax_error) {
            err.as_object()
                .set_attr("filename", filename.clone(), vm)
                .unwrap();
        }
        err
    }

    fn append_visible_child(table: SymbolTable, children: &mut Vec<SymbolTable>) {
        if table.comp_inlined {
            for child in table.sub_tables {
                append_visible_child(child, children);
            }
        } else {
            children.push(table);
        }
    }

    fn to_py_symbol_table(mut symtable: SymbolTable, vm: &VirtualMachine) -> PyRef<PySymbolTable> {
        let mut child_tables = Vec::new();
        for table in core::mem::take(&mut symtable.sub_tables) {
            append_visible_child(table, &mut child_tables);
        }
        if !symtable.future_annotations
            && let Some(annotation_block) = symtable.annotation_block.take()
        {
            child_tables.push(*annotation_block);
        }
        child_tables.sort_by_key(|table| table.block_index);

        let children = vm.ctx.new_list(
            child_tables
                .into_iter()
                .map(|table| to_py_symbol_table(table, vm).into())
                .collect(),
        );
        let symbols = vm.ctx.new_dict();
        for (name, symbol) in &symtable.symbols {
            let packed_flags =
                i32::from(symbol.flags.bits()) | (symbol.scope.as_i32() << SCOPE_OFFSET);
            symbols
                .set_item(name, vm.new_pyobj(packed_flags), vm)
                .unwrap();
        }
        let varnames = vm.ctx.new_list(
            symtable
                .varnames
                .iter()
                .map(|name| vm.ctx.new_str(name.as_str()).into())
                .collect(),
        );
        PySymbolTable {
            symtable,
            children,
            symbols,
            varnames,
        }
        .into_ref(&vm.ctx)
    }

    #[pyattr]
    #[pyclass(name = "symtable entry")]
    #[derive(PyPayload)]
    struct PySymbolTable {
        symtable: SymbolTable,
        children: PyListRef,
        symbols: PyDictRef,
        varnames: PyListRef,
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
            self.symtable.name.clone()
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
        fn children(&self) -> PyListRef {
            self.children.clone()
        }

        #[pygetset]
        fn id(&self) -> usize {
            self as *const Self as *const core::ffi::c_void as usize
        }

        #[pygetset]
        fn symbols(&self) -> PyDictRef {
            self.symbols.clone()
        }

        #[pygetset]
        fn varnames(&self) -> PyListRef {
            self.varnames.clone()
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
