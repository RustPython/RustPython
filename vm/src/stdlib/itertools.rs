use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::ops::{AddAssign, SubAssign};
use std::rc::Rc;

use num_bigint::BigInt;
use num_traits::ToPrimitive;

use crate::function::{OptionalArg, PyFuncArgs};
use crate::obj::objbool;
use crate::obj::objint;
use crate::obj::objint::{PyInt, PyIntRef};
use crate::obj::objiter::{call_next, get_iter, new_stop_iteration};
use crate::obj::objtuple::PyTuple;
use crate::obj::objtype;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{
    IdProtocol, PyCallable, PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::vm::VirtualMachine;

#[pyclass(name = "chain")]
#[derive(Debug)]
struct PyItertoolsChain {
    iterables: Vec<PyObjectRef>,
    cur: RefCell<(usize, Option<PyObjectRef>)>,
}

impl PyValue for PyItertoolsChain {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("itertools", "chain")
    }
}

#[pyimpl]
impl PyItertoolsChain {
    #[pymethod(name = "__new__")]
    #[allow(clippy::new_ret_no_self)]
    fn new(_cls: PyClassRef, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult {
        Ok(PyItertoolsChain {
            iterables: args.args,
            cur: RefCell::new((0, None)),
        }
        .into_ref(vm)
        .into_object())
    }

    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        let (ref mut cur_idx, ref mut cur_iter) = *self.cur.borrow_mut();
        while *cur_idx < self.iterables.len() {
            if cur_iter.is_none() {
                *cur_iter = Some(get_iter(vm, &self.iterables[*cur_idx])?);
            }

            // can't be directly inside the 'match' clause, otherwise the borrows collide.
            let obj = call_next(vm, cur_iter.as_ref().unwrap());
            match obj {
                Ok(ok) => return Ok(ok),
                Err(err) => {
                    if objtype::isinstance(&err, &vm.ctx.exceptions.stop_iteration) {
                        *cur_idx += 1;
                        *cur_iter = None;
                    } else {
                        return Err(err);
                    }
                }
            }
        }

        Err(new_stop_iteration(vm))
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }
}

#[pyclass(name = "compress")]
#[derive(Debug)]
struct PyItertoolsCompress {
    data: PyObjectRef,
    selector: PyObjectRef,
}

impl PyValue for PyItertoolsCompress {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("itertools", "compress")
    }
}

#[pyimpl]
impl PyItertoolsCompress {
    #[pymethod(name = "__new__")]
    #[allow(clippy::new_ret_no_self)]
    fn new(
        _cls: PyClassRef,
        data: PyObjectRef,
        selector: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        let data_iter = get_iter(vm, &data)?;
        let selector_iter = get_iter(vm, &selector)?;

        Ok(PyItertoolsCompress {
            data: data_iter,
            selector: selector_iter,
        }
        .into_ref(vm)
        .into_object())
    }

    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        loop {
            let sel_obj = call_next(vm, &self.selector)?;
            let verdict = objbool::boolval(vm, sel_obj.clone())?;
            let data_obj = call_next(vm, &self.data)?;

            if verdict {
                return Ok(data_obj);
            }
        }
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }
}

#[pyclass]
#[derive(Debug)]
struct PyItertoolsCount {
    cur: RefCell<BigInt>,
    step: BigInt,
}

impl PyValue for PyItertoolsCount {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("itertools", "count")
    }
}

#[pyimpl]
impl PyItertoolsCount {
    #[pymethod(name = "__new__")]
    #[allow(clippy::new_ret_no_self)]
    fn new(
        _cls: PyClassRef,
        start: OptionalArg<PyIntRef>,
        step: OptionalArg<PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let start = match start.into_option() {
            Some(int) => int.as_bigint().clone(),
            None => BigInt::from(0),
        };
        let step = match step.into_option() {
            Some(int) => int.as_bigint().clone(),
            None => BigInt::from(1),
        };

        Ok(PyItertoolsCount {
            cur: RefCell::new(start),
            step,
        }
        .into_ref(vm)
        .into_object())
    }

    #[pymethod(name = "__next__")]
    fn next(&self, _vm: &VirtualMachine) -> PyResult<PyInt> {
        let result = self.cur.borrow().clone();
        AddAssign::add_assign(&mut self.cur.borrow_mut() as &mut BigInt, &self.step);
        Ok(PyInt::new(result))
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }
}

#[pyclass]
#[derive(Debug)]
struct PyItertoolsRepeat {
    object: PyObjectRef,
    times: Option<RefCell<BigInt>>,
}

impl PyValue for PyItertoolsRepeat {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("itertools", "repeat")
    }
}

#[pyimpl]
impl PyItertoolsRepeat {
    #[pymethod(name = "__new__")]
    #[allow(clippy::new_ret_no_self)]
    fn new(
        _cls: PyClassRef,
        object: PyObjectRef,
        times: OptionalArg<PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let times = match times.into_option() {
            Some(int) => Some(RefCell::new(int.as_bigint().clone())),
            None => None,
        };

        Ok(PyItertoolsRepeat {
            object: object.clone(),
            times,
        }
        .into_ref(vm)
        .into_object())
    }

    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        if self.times.is_some() {
            match self.times.as_ref().unwrap().borrow().cmp(&BigInt::from(0)) {
                Ordering::Less | Ordering::Equal => return Err(new_stop_iteration(vm)),
                _ => (),
            };

            SubAssign::sub_assign(
                &mut self.times.as_ref().unwrap().borrow_mut() as &mut BigInt,
                &BigInt::from(1),
            );
        }

        Ok(self.object.clone())
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }
}

#[pyclass(name = "starmap")]
#[derive(Debug)]
struct PyItertoolsStarmap {
    function: PyObjectRef,
    iter: PyObjectRef,
}

impl PyValue for PyItertoolsStarmap {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("itertools", "starmap")
    }
}

#[pyimpl]
impl PyItertoolsStarmap {
    #[pymethod(name = "__new__")]
    #[allow(clippy::new_ret_no_self)]
    fn new(
        _cls: PyClassRef,
        function: PyObjectRef,
        iterable: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        let iter = get_iter(vm, &iterable)?;

        Ok(PyItertoolsStarmap { function, iter }
            .into_ref(vm)
            .into_object())
    }

    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        let obj = call_next(vm, &self.iter)?;
        let function = &self.function;

        vm.invoke(function, vm.extract_elements(&obj)?)
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }
}

#[pyclass]
#[derive(Debug)]
struct PyItertoolsTakewhile {
    predicate: PyObjectRef,
    iterable: PyObjectRef,
    stop_flag: RefCell<bool>,
}

impl PyValue for PyItertoolsTakewhile {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("itertools", "takewhile")
    }
}

#[pyimpl]
impl PyItertoolsTakewhile {
    #[pymethod(name = "__new__")]
    #[allow(clippy::new_ret_no_self)]
    fn new(
        _cls: PyClassRef,
        predicate: PyObjectRef,
        iterable: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        let iter = get_iter(vm, &iterable)?;

        Ok(PyItertoolsTakewhile {
            predicate,
            iterable: iter,
            stop_flag: RefCell::new(false),
        }
        .into_ref(vm)
        .into_object())
    }

    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        if *self.stop_flag.borrow() {
            return Err(new_stop_iteration(vm));
        }

        // might be StopIteration or anything else, which is propaged upwwards
        let obj = call_next(vm, &self.iterable)?;
        let predicate = &self.predicate;

        let verdict = vm.invoke(predicate, vec![obj.clone()])?;
        let verdict = objbool::boolval(vm, verdict)?;
        if verdict {
            Ok(obj)
        } else {
            *self.stop_flag.borrow_mut() = true;
            Err(new_stop_iteration(vm))
        }
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }
}

#[pyclass]
#[derive(Debug)]
struct PyItertoolsDropwhile {
    predicate: PyCallable,
    iterable: PyObjectRef,
    start_flag: Cell<bool>,
}

impl PyValue for PyItertoolsDropwhile {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("itertools", "dropwhile")
    }
}

type PyItertoolsDropwhileRef = PyRef<PyItertoolsDropwhile>;

#[pyimpl]
impl PyItertoolsDropwhile {
    #[pymethod(name = "__new__")]
    #[allow(clippy::new_ret_no_self)]
    fn new(
        cls: PyClassRef,
        predicate: PyCallable,
        iterable: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyItertoolsDropwhileRef> {
        let iter = get_iter(vm, &iterable)?;

        PyItertoolsDropwhile {
            predicate,
            iterable: iter,
            start_flag: Cell::new(false),
        }
        .into_ref_with_type(vm, cls)
    }

    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        let predicate = &self.predicate;
        let iterable = &self.iterable;

        if !self.start_flag.get() {
            loop {
                let obj = call_next(vm, iterable)?;
                let pred = predicate.clone();
                let pred_value = vm.invoke(&pred.into_object(), vec![obj.clone()])?;
                if !objbool::boolval(vm, pred_value)? {
                    self.start_flag.set(true);
                    return Ok(obj);
                }
            }
        }
        call_next(vm, iterable)
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }
}

#[pyclass(name = "islice")]
#[derive(Debug)]
struct PyItertoolsIslice {
    iterable: PyObjectRef,
    cur: RefCell<usize>,
    next: RefCell<usize>,
    stop: Option<usize>,
    step: usize,
}

impl PyValue for PyItertoolsIslice {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("itertools", "islice")
    }
}

fn pyobject_to_opt_usize(obj: PyObjectRef, vm: &VirtualMachine) -> Option<usize> {
    let is_int = objtype::isinstance(&obj, &vm.ctx.int_type());
    if is_int {
        objint::get_value(&obj).to_usize()
    } else {
        None
    }
}

#[pyimpl]
impl PyItertoolsIslice {
    #[pymethod(name = "__new__")]
    #[allow(clippy::new_ret_no_self)]
    fn new(_cls: PyClassRef, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult {
        let (iter, start, stop, step) = match args.args.len() {
            0 | 1 => {
                return Err(vm.new_type_error(format!(
                    "islice expected at least 2 arguments, got {}",
                    args.args.len()
                )));
            }

            2 => {
                let (iter, stop): (PyObjectRef, PyObjectRef) = args.bind(vm)?;

                (iter, 0usize, stop, 1usize)
            }
            _ => {
                let (iter, start, stop, step): (
                    PyObjectRef,
                    PyObjectRef,
                    PyObjectRef,
                    PyObjectRef,
                ) = args.bind(vm)?;

                let start = if !start.is(&vm.get_none()) {
                    pyobject_to_opt_usize(start, &vm).ok_or_else(|| {
                        vm.new_value_error(
                            "Indices for islice() must be None or an integer: 0 <= x <= sys.maxsize.".to_string(),
                        )
                    })?
                } else {
                    0usize
                };

                let step = if !step.is(&vm.get_none()) {
                    pyobject_to_opt_usize(step, &vm).ok_or_else(|| {
                        vm.new_value_error(
                            "Step for islice() must be a positive integer or None.".to_string(),
                        )
                    })?
                } else {
                    1usize
                };

                (iter, start, stop, step)
            }
        };

        let stop = if !stop.is(&vm.get_none()) {
            Some(pyobject_to_opt_usize(stop, &vm).ok_or_else(|| {
                vm.new_value_error(
                    "Stop argument for islice() must be None or an integer: 0 <= x <= sys.maxsize."
                        .to_string(),
                )
            })?)
        } else {
            None
        };

        let iter = get_iter(vm, &iter)?;

        Ok(PyItertoolsIslice {
            iterable: iter,
            cur: RefCell::new(0),
            next: RefCell::new(start),
            stop,
            step,
        }
        .into_ref(vm)
        .into_object())
    }

    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        while *self.cur.borrow() < *self.next.borrow() {
            call_next(vm, &self.iterable)?;
            *self.cur.borrow_mut() += 1;
        }

        if let Some(stop) = self.stop {
            if *self.cur.borrow() >= stop {
                return Err(new_stop_iteration(vm));
            }
        }

        let obj = call_next(vm, &self.iterable)?;
        *self.cur.borrow_mut() += 1;

        // TODO is this overflow check required? attempts to copy CPython.
        let (next, ovf) = (*self.next.borrow()).overflowing_add(self.step);
        *self.next.borrow_mut() = if ovf { self.stop.unwrap() } else { next };

        Ok(obj)
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }
}

#[pyclass]
#[derive(Debug)]
struct PyItertoolsFilterFalse {
    predicate: PyObjectRef,
    iterable: PyObjectRef,
}

impl PyValue for PyItertoolsFilterFalse {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("itertools", "filterfalse")
    }
}

#[pyimpl]
impl PyItertoolsFilterFalse {
    #[pymethod(name = "__new__")]
    #[allow(clippy::new_ret_no_self)]
    fn new(
        _cls: PyClassRef,
        predicate: PyObjectRef,
        iterable: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        let iter = get_iter(vm, &iterable)?;

        Ok(PyItertoolsFilterFalse {
            predicate,
            iterable: iter,
        }
        .into_ref(vm)
        .into_object())
    }

    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        let predicate = &self.predicate;
        let iterable = &self.iterable;

        loop {
            let obj = call_next(vm, iterable)?;
            let pred_value = if predicate.is(&vm.get_none()) {
                obj.clone()
            } else {
                vm.invoke(predicate, vec![obj.clone()])?
            };

            if !objbool::boolval(vm, pred_value)? {
                return Ok(obj);
            }
        }
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }
}

#[pyclass]
#[derive(Debug)]
struct PyItertoolsAccumulate {
    iterable: PyObjectRef,
    binop: PyObjectRef,
    acc_value: RefCell<Option<PyObjectRef>>,
}

impl PyValue for PyItertoolsAccumulate {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("itertools", "accumulate")
    }
}

#[pyimpl]
impl PyItertoolsAccumulate {
    #[pymethod(name = "__new__")]
    #[allow(clippy::new_ret_no_self)]
    fn new(
        cls: PyClassRef,
        iterable: PyObjectRef,
        binop: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<PyItertoolsAccumulate>> {
        let iter = get_iter(vm, &iterable)?;

        PyItertoolsAccumulate {
            iterable: iter,
            binop: binop.unwrap_or_else(|| vm.get_none()),
            acc_value: RefCell::from(Option::None),
        }
        .into_ref_with_type(vm, cls)
    }

    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        let iterable = &self.iterable;
        let obj = call_next(vm, iterable)?;

        let next_acc_value = match &*self.acc_value.borrow() {
            None => obj.clone(),
            Some(value) => {
                if self.binop.is(&vm.get_none()) {
                    vm._add(value.clone(), obj.clone())?
                } else {
                    vm.invoke(&self.binop, vec![value.clone(), obj.clone()])?
                }
            }
        };
        self.acc_value.replace(Option::from(next_acc_value.clone()));

        Ok(next_acc_value)
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }
}

#[derive(Debug)]
struct PyItertoolsTeeData {
    iterable: PyObjectRef,
    values: RefCell<Vec<PyObjectRef>>,
}

impl PyItertoolsTeeData {
    fn new(
        iterable: PyObjectRef,
        vm: &VirtualMachine,
    ) -> Result<Rc<PyItertoolsTeeData>, PyObjectRef> {
        Ok(Rc::new(PyItertoolsTeeData {
            iterable: get_iter(vm, &iterable)?,
            values: RefCell::new(vec![]),
        }))
    }

    fn get_item(&self, vm: &VirtualMachine, index: usize) -> PyResult {
        if self.values.borrow().len() == index {
            let result = call_next(vm, &self.iterable)?;
            self.values.borrow_mut().push(result);
        }
        Ok(self.values.borrow()[index].clone())
    }
}

#[pyclass]
#[derive(Debug)]
struct PyItertoolsTee {
    tee_data: Rc<PyItertoolsTeeData>,
    index: Cell<usize>,
}

impl PyValue for PyItertoolsTee {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("itertools", "tee")
    }
}

#[pyimpl]
impl PyItertoolsTee {
    fn from_iter(iterable: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let it = get_iter(vm, &iterable)?;
        if it.class().is(&PyItertoolsTee::class(vm)) {
            return vm.call_method(&it, "__copy__", PyFuncArgs::from(vec![]));
        }
        Ok(PyItertoolsTee {
            tee_data: PyItertoolsTeeData::new(it, vm)?,
            index: Cell::from(0),
        }
        .into_ref_with_type(vm, PyItertoolsTee::class(vm))?
        .into_object())
    }

    #[pymethod(name = "__new__")]
    #[allow(clippy::new_ret_no_self)]
    fn new(
        _cls: PyClassRef,
        iterable: PyObjectRef,
        n: OptionalArg<PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<PyTuple>> {
        let n = match n {
            OptionalArg::Present(x) => match x.as_bigint().to_usize() {
                Some(y) => y,
                None => return Err(vm.new_overflow_error(String::from("n is too big"))),
            },
            OptionalArg::Missing => 2,
        };

        let copyable = if objtype::class_has_attr(&iterable.class(), "__copy__") {
            vm.call_method(&iterable, "__copy__", PyFuncArgs::from(vec![]))?
        } else {
            PyItertoolsTee::from_iter(iterable, vm)?
        };

        let mut tee_vec: Vec<PyObjectRef> = Vec::with_capacity(n);
        for _ in 0..n {
            let no_args = PyFuncArgs::from(vec![]);
            tee_vec.push(vm.call_method(&copyable, "__copy__", no_args)?);
        }

        Ok(PyTuple::from(tee_vec).into_ref(vm))
    }

    #[pymethod(name = "__copy__")]
    fn copy(&self, vm: &VirtualMachine) -> PyResult {
        Ok(PyItertoolsTee {
            tee_data: Rc::clone(&self.tee_data),
            index: self.index.clone(),
        }
        .into_ref_with_type(vm, Self::class(vm))?
        .into_object())
    }

    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        let result = self.tee_data.get_item(vm, self.index.get());
        self.index.set(self.index.get() + 1);
        result
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let chain = PyItertoolsChain::make_class(ctx);

    let compress = PyItertoolsCompress::make_class(ctx);

    let count = ctx.new_class("count", ctx.object());
    PyItertoolsCount::extend_class(ctx, &count);

    let dropwhile = ctx.new_class("dropwhile", ctx.object());
    PyItertoolsDropwhile::extend_class(ctx, &dropwhile);

    let repeat = ctx.new_class("repeat", ctx.object());
    PyItertoolsRepeat::extend_class(ctx, &repeat);

    let starmap = PyItertoolsStarmap::make_class(ctx);

    let takewhile = ctx.new_class("takewhile", ctx.object());
    PyItertoolsTakewhile::extend_class(ctx, &takewhile);

    let islice = PyItertoolsIslice::make_class(ctx);

    let filterfalse = ctx.new_class("filterfalse", ctx.object());
    PyItertoolsFilterFalse::extend_class(ctx, &filterfalse);

    let accumulate = ctx.new_class("accumulate", ctx.object());
    PyItertoolsAccumulate::extend_class(ctx, &accumulate);

    let tee = ctx.new_class("tee", ctx.object());
    PyItertoolsTee::extend_class(ctx, &tee);

    py_module!(vm, "itertools", {
        "chain" => chain,
        "compress" => compress,
        "count" => count,
        "dropwhile" => dropwhile,
        "repeat" => repeat,
        "starmap" => starmap,
        "takewhile" => takewhile,
        "islice" => islice,
        "filterfalse" => filterfalse,
        "accumulate" => accumulate,
        "tee" => tee,
    })
}
