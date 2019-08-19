use std::cell::RefCell;
use std::cmp::Ordering;
use std::ops::{AddAssign, SubAssign};

use num_bigint::BigInt;
use num_traits::ToPrimitive;

use crate::function::{OptionalArg, PyFuncArgs};
use crate::obj::objbool;
use crate::obj::objint;
use crate::obj::objint::{PyInt, PyIntRef};
use crate::obj::objiter::{call_next, get_iter, new_stop_iteration};
use crate::obj::objtype;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{IdProtocol, PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue};
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

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let chain = PyItertoolsChain::make_class(ctx);

    let count = ctx.new_class("count", ctx.object());
    PyItertoolsCount::extend_class(ctx, &count);

    let repeat = ctx.new_class("repeat", ctx.object());
    PyItertoolsRepeat::extend_class(ctx, &repeat);

    let starmap = PyItertoolsStarmap::make_class(ctx);

    let takewhile = ctx.new_class("takewhile", ctx.object());
    PyItertoolsTakewhile::extend_class(ctx, &takewhile);

    let islice = PyItertoolsIslice::make_class(ctx);

    let filterfalse = ctx.new_class("filterfalse", ctx.object());
    PyItertoolsFilterFalse::extend_class(ctx, &filterfalse);

    py_module!(vm, "itertools", {
        "chain" => chain,
        "count" => count,
        "repeat" => repeat,
        "starmap" => starmap,
        "takewhile" => takewhile,
        "islice" => islice,
        "filterfalse" => filterfalse,
    })
}
