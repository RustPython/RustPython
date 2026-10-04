use crate::{
    Context, Py, PyPayload, PyResult, VirtualMachine,
    builtins::{PyModule, PyStaticMethod, PyType},
    class::PyClassImpl,
    function::{Callee, FuncArgs, ItemDoc, PyMethodDef, PyMethodFlags},
    types::Constructor,
};

const MODULE: u8 = 0;
const INSTANCE: u8 = 1;
const CLASS: u8 = 2;
const STATIC: u8 = 3;
const VARARGS: u8 = 0;
const VARARGS_KEYWORDS: u8 = 1;
const O: u8 = 2;
const NOARGS: u8 = 3;
const FASTCALL: u8 = 4;
const FASTCALL_KEYWORDS: u8 = 5;

const NAMES: [&str; 6] = [
    "meth_varargs",
    "meth_varargs_keywords",
    "meth_o",
    "meth_noargs",
    "meth_fastcall",
    "meth_fastcall_keywords",
];

// These fixtures expose the arguments delivered by the native call machinery.
// Both its regular-call and vectorcall paths use FuncArgs in RustPython.
fn call<const BINDING: u8, const CONVENTION: u8>(
    vm: &VirtualMachine,
    mut args: FuncArgs,
    _callee: Callee,
) -> PyResult {
    let name = NAMES[CONVENTION as usize];
    let owner = match BINDING {
        MODULE => "_testcapi",
        INSTANCE => "MethInstance",
        CLASS => "MethClass",
        STATIC => "MethStatic",
        _ => unreachable!(),
    };
    let zelf = if BINDING == STATIC {
        vm.ctx.none()
    } else {
        if args.args.is_empty() {
            return Err(
                vm.new_type_error(format!("unbound method {owner}.{name}() needs an argument"))
            );
        }
        args.args.remove(0)
    };
    let accepts_keywords = matches!(CONVENTION, VARARGS_KEYWORDS | FASTCALL_KEYWORDS);
    if !accepts_keywords && !args.kwargs.is_empty() {
        // METH_VARARGS uses the older, unqualified error spelling.
        let qualified = if CONVENTION == VARARGS {
            name.to_owned()
        } else {
            format!("{owner}.{name}")
        };
        return Err(vm.new_type_error(format!("{qualified}() takes no keyword arguments")));
    }
    let given = args.args.len();
    if CONVENTION == NOARGS {
        if given != 0 {
            return Err(vm.new_type_error(format!(
                "{owner}.{name}() takes no arguments ({given} given)"
            )));
        }
        return Ok(zelf);
    }
    if CONVENTION == O {
        if given != 1 {
            return Err(vm.new_type_error(format!(
                "{owner}.{name}() takes exactly one argument ({given} given)"
            )));
        }
        return Ok(vm.ctx.new_tuple(vec![zelf, args.args.remove(0)]).into());
    }
    let positional = vm.ctx.new_tuple(args.args);
    if accepts_keywords {
        let keywords = vm.ctx.new_dict();
        for (name, value) in args.kwargs {
            keywords.set_item(&*vm.ctx.new_str(name), value, vm)?;
        }
        Ok(vm
            .ctx
            .new_tuple(vec![zelf, positional.into(), keywords.into()])
            .into())
    } else {
        Ok(vm.ctx.new_tuple(vec![zelf, positional.into()]).into())
    }
}

const fn method<const BINDING: u8, const CONVENTION: u8>() -> PyMethodDef {
    let convention = match CONVENTION {
        VARARGS => PyMethodFlags::VARARGS,
        VARARGS_KEYWORDS => PyMethodFlags::VARARGS.union(PyMethodFlags::KEYWORDS),
        O => PyMethodFlags::O,
        NOARGS => PyMethodFlags::NOARGS,
        FASTCALL => PyMethodFlags::FASTCALL,
        FASTCALL_KEYWORDS => PyMethodFlags::FASTCALL.union(PyMethodFlags::KEYWORDS),
        _ => unreachable!(),
    };
    let binding = match BINDING {
        MODULE => PyMethodFlags::EMPTY,
        INSTANCE => PyMethodFlags::METHOD,
        CLASS => PyMethodFlags::CLASS,
        STATIC => PyMethodFlags::STATIC,
        _ => unreachable!(),
    };
    let doc = match (BINDING, CONVENTION) {
        (CLASS, O) => ItemDoc::static_text("meth_o($type, object, /)\n--\n\n"),
        (STATIC, O) => ItemDoc::static_text("meth_o(object, /)\n--\n\n"),
        (_, O) => ItemDoc::static_text("meth_o($self, object, /)\n--\n\n"),
        (CLASS, NOARGS) => ItemDoc::static_text("meth_noargs($type, /)\n--\n\n"),
        (STATIC, NOARGS) => ItemDoc::static_text("meth_noargs()\n--\n\n"),
        (_, NOARGS) => ItemDoc::static_text("meth_noargs($self, /)\n--\n\n"),
        _ => ItemDoc::NONE,
    };
    PyMethodDef::new_raw_const(
        NAMES[CONVENTION as usize],
        call::<BINDING, CONVENTION>,
        convention.union(binding),
        doc,
    )
}

const fn methods<const BINDING: u8>() -> [PyMethodDef; 6] {
    [
        method::<BINDING, VARARGS>(),
        method::<BINDING, VARARGS_KEYWORDS>(),
        method::<BINDING, O>(),
        method::<BINDING, NOARGS>(),
        method::<BINDING, FASTCALL>(),
        method::<BINDING, FASTCALL_KEYWORDS>(),
    ]
}

fn extend_class(ctx: &Context, class: &'static Py<PyType>, methods: &'static [PyMethodDef]) {
    for method in methods {
        let value = if method.flags.contains(PyMethodFlags::STATIC) {
            let function = method.build_staticmethod(ctx, class);
            drop(
                function
                    .func
                    .module
                    .store(Some(ctx.new_str("builtins").into())),
            );
            PyStaticMethod::new(function.into()).into_ref(ctx).into()
        } else {
            method.to_proper_method(class, ctx)
        };
        class.set_str_attr(method.name, value, ctx);
    }
}

/// Class with normal (instance) methods to test calling conventions
#[pyclass(module = false, name = "MethInstance")]
#[derive(Debug, PyPayload)]
struct MethInstance;

/// Class with class methods to test calling conventions
#[pyclass(module = false, name = "MethClass")]
#[derive(Debug, PyPayload)]
struct MethClass;

/// Class with static methods to test calling conventions
#[pyclass(module = false, name = "MethStatic")]
#[derive(Debug, PyPayload)]
struct MethStatic;

macro_rules! implement_fixture {
    ($class:ident, $binding:ident) => {
        #[pyclass(with(Constructor), flags(IMMUTABLETYPE))]
        impl $class {
            #[extend_class]
            fn extend_class(ctx: &Context, class: &'static Py<PyType>) {
                const METHODS: [PyMethodDef; 6] = methods::<$binding>();
                extend_class(ctx, class, &METHODS);
            }
        }

        impl Constructor for $class {
            type Args = FuncArgs;

            fn py_new(_cls: &Py<PyType>, _args: FuncArgs, _vm: &VirtualMachine) -> PyResult<Self> {
                // PyType_GenericNew ignores both positional and keyword args.
                Ok(Self)
            }
        }
    };
}

implement_fixture!(MethInstance, INSTANCE);
implement_fixture!(MethClass, CLASS);
implement_fixture!(MethStatic, STATIC);

pub(super) fn extend_module(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
    module.set_attr("MethInstance", MethInstance::make_static_type(), vm)?;
    module.set_attr("MethClass", MethClass::make_static_type(), vm)?;
    module.set_attr("MethStatic", MethStatic::make_static_type(), vm)?;
    const METHODS: [PyMethodDef; 6] = methods::<MODULE>();
    for method in &METHODS {
        let function = method.build_bound_function(&vm.ctx, module.to_owned().into());
        drop(
            function
                .module
                .store(Some(vm.ctx.new_str("_testcapi").into())),
        );
        module.set_attr(method.name, function, vm)?;
    }
    Ok(())
}
