//! Low-level cross-interpreter queues (`_interpqueues`).

#[cfg_attr(not(feature = "threading"), allow(unused_imports))]
pub(crate) use _interpqueues::clear_interpreter;
pub(crate) use _interpqueues::{
    is_external_queue, module_def, queue_from_xid, queue_id_from_object, queue_xid_decref,
    queue_xid_incref,
};

#[pymodule]
pub(crate) mod _interpqueues {
    use crate::{
        AsObject, Py, PyObject, PyPayload, PyResult, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyModule, PyType, PyTypeRef},
        function::{ArgSpec, FuncArgs},
        types::Constructor,
        vm::crossinterp::{self, Fallback, SharedValue, UNBOUND_REMOVE, UNBOUND_REPLACE},
    };
    use alloc::{collections::BTreeMap, collections::VecDeque, sync::Arc};
    use core::cell::Cell;
    use num_traits::{Signed, ToPrimitive};
    use parking_lot::Mutex;
    use std::sync::OnceLock;

    #[pyattr]
    #[pyexception(name = "QueueError", module = "concurrent.interpreters", base = crate::exceptions::types::PyRuntimeError)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub(crate) struct PyQueueError(crate::exceptions::types::PyRuntimeError);

    #[pyexception]
    impl PyQueueError {}

    #[pyattr]
    #[pyexception(name = "QueueNotFoundError", module = "concurrent.interpreters", base = PyQueueError)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub(crate) struct PyQueueNotFoundError(PyQueueError);

    #[pyexception]
    impl PyQueueNotFoundError {}

    /// The failures the queue store itself can report, all of which map to
    /// exceptions owned by this module.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum QueueErr {
        NoNextId,
        NotFound,
        NeverBound,
    }

    impl QueueErr {
        fn into_py(self, qid: i64, vm: &VirtualMachine) -> PyBaseExceptionRef {
            let (class, msg) = match self {
                Self::NoNextId => (
                    PyQueueError::class(&vm.ctx).to_owned(),
                    "ran out of queue IDs".to_owned(),
                ),
                Self::NotFound => (
                    PyQueueNotFoundError::class(&vm.ctx).to_owned(),
                    format!("queue {qid} not found"),
                ),
                Self::NeverBound => (
                    PyQueueError::class(&vm.ctx).to_owned(),
                    format!("queue {qid} never bound"),
                ),
            };
            vm.new_exception_msg(class, msg.into())
        }
    }

    /// `queue.Empty`, which belongs to the `_queues` module and is therefore
    /// only reachable once that module has been imported.
    fn queue_empty_error(qid: i64, vm: &VirtualMachine) -> PyResult<PyBaseExceptionRef> {
        let class = ensure_external_types(vm)?.empty;
        Ok(vm.new_exception_msg(class, format!("queue {qid} is empty").into()))
    }

    /// `queue.Full`, which belongs to the `_queues` module and is therefore
    /// only reachable once that module has been imported.
    fn queue_full_error(qid: i64, vm: &VirtualMachine) -> PyResult<PyBaseExceptionRef> {
        let class = ensure_external_types(vm)?.full;
        Ok(vm.new_exception_msg(class, format!("queue {qid} is full").into()))
    }

    type QueueResult<T> = Result<T, QueueErr>;

    #[derive(Clone)]
    struct ExternalTypes {
        queue: PyTypeRef,
        empty: PyTypeRef,
        full: PyTypeRef,
    }

    #[cfg(feature = "threading")]
    fn external_types() -> &'static Mutex<BTreeMap<i64, ExternalTypes>> {
        static TYPES: OnceLock<Mutex<BTreeMap<i64, ExternalTypes>>> = OnceLock::new();
        TYPES.get_or_init(|| Mutex::new(BTreeMap::new()))
    }

    #[cfg(not(feature = "threading"))]
    std::thread_local! {
        static EXTERNAL_TYPES: core::cell::RefCell<BTreeMap<i64, ExternalTypes>> =
            const { core::cell::RefCell::new(BTreeMap::new()) };
    }

    fn get_external_types(interpid: i64) -> Option<ExternalTypes> {
        #[cfg(feature = "threading")]
        {
            external_types().lock().get(&interpid).cloned()
        }
        #[cfg(not(feature = "threading"))]
        {
            EXTERNAL_TYPES.with(|types| types.borrow().get(&interpid).cloned())
        }
    }

    fn set_external_types(interpid: i64, types: ExternalTypes) {
        #[cfg(feature = "threading")]
        {
            external_types().lock().insert(interpid, types);
        }
        #[cfg(not(feature = "threading"))]
        {
            EXTERNAL_TYPES.with(|registered| {
                registered.borrow_mut().insert(interpid, types);
            });
        }
    }

    #[cfg_attr(not(feature = "threading"), allow(dead_code))]
    fn remove_external_types(interpid: i64) {
        #[cfg(feature = "threading")]
        {
            external_types().lock().remove(&interpid);
        }
        #[cfg(not(feature = "threading"))]
        {
            EXTERNAL_TYPES.with(|types| {
                types.borrow_mut().remove(&interpid);
            });
        }
    }

    fn ensure_highlevel_module_loaded(vm: &VirtualMachine) -> PyResult<()> {
        vm.import("concurrent.interpreters._queues", 0).map(drop)
    }

    fn ensure_external_types(vm: &VirtualMachine) -> PyResult<ExternalTypes> {
        let interpid = vm.state.interpreter_id;
        if let Some(types) = get_external_types(interpid) {
            return Ok(types);
        }
        ensure_highlevel_module_loaded(vm)?;
        get_external_types(interpid)
            .ok_or_else(|| vm.new_runtime_error("queue types were not registered"))
    }

    #[cfg_attr(not(feature = "threading"), allow(dead_code))]
    struct QueueItem {
        interpid: i64,
        data: Option<SharedValue>,
        unboundop: i32,
    }

    struct Queue {
        alive: bool,
        maxsize: isize,
        items: VecDeque<QueueItem>,
        unboundop: i32,
        fallback: Fallback,
    }

    impl Queue {
        fn is_full(&self) -> bool {
            self.maxsize > 0 && self.items.len() >= self.maxsize as usize
        }
    }

    struct QueueRef {
        queue: Arc<Mutex<Queue>>,
        refcount: isize,
    }

    struct Queues {
        refs: BTreeMap<i64, QueueRef>,
        next_id: i64,
    }

    fn queues() -> &'static Mutex<Queues> {
        static QUEUES: OnceLock<Mutex<Queues>> = OnceLock::new();
        QUEUES.get_or_init(|| {
            Mutex::new(Queues {
                refs: BTreeMap::new(),
                next_id: 1,
            })
        })
    }

    fn queue_lookup(qid: i64) -> QueueResult<Arc<Mutex<Queue>>> {
        queues()
            .lock()
            .refs
            .get(&qid)
            .map(|r| r.queue.clone())
            .ok_or(QueueErr::NotFound)
    }

    fn queue_create(maxsize: isize, unboundop: i32, fallback: Fallback) -> QueueResult<i64> {
        let mut queues = queues().lock();
        let qid = queues.next_id;
        if qid < 0 {
            return Err(QueueErr::NoNextId);
        }
        queues.next_id = qid.checked_add(1).unwrap_or(-1);
        queues.refs.insert(
            qid,
            QueueRef {
                queue: Arc::new(Mutex::new(Queue {
                    alive: true,
                    maxsize,
                    items: VecDeque::new(),
                    unboundop,
                    fallback,
                })),
                refcount: 0,
            },
        );
        Ok(qid)
    }

    fn kill_queue(queue: &Arc<Mutex<Queue>>) {
        queue.lock().alive = false;
    }

    fn queue_destroy(qid: i64) -> QueueResult<()> {
        let queue = queues()
            .lock()
            .refs
            .remove(&qid)
            .ok_or(QueueErr::NotFound)?;
        kill_queue(&queue.queue);
        drop(queue);
        Ok(())
    }

    fn queue_defaults(qid: i64) -> QueueResult<(i32, Fallback)> {
        let queue = queue_lookup(qid)?;
        let state = queue.lock();
        if !state.alive {
            return Err(QueueErr::NotFound);
        }
        Ok((state.unboundop, state.fallback))
    }

    fn queue_bind(qid: i64) -> QueueResult<()> {
        let mut queues = queues().lock();
        let queue = queues.refs.get_mut(&qid).ok_or(QueueErr::NotFound)?;
        queue.refcount += 1;
        Ok(())
    }

    fn queue_release(qid: i64) -> QueueResult<()> {
        let removed = {
            let mut queues = queues().lock();
            let queue = queues.refs.get_mut(&qid).ok_or(QueueErr::NotFound)?;
            if queue.refcount == 0 {
                return Err(QueueErr::NeverBound);
            }
            queue.refcount -= 1;
            if queue.refcount == 0 {
                queues.refs.remove(&qid)
            } else {
                None
            }
        };
        if let Some(queue) = removed {
            kill_queue(&queue.queue);
            drop(queue);
        }
        Ok(())
    }

    fn resolve_unboundop(arg: i32, default: i32, vm: &VirtualMachine) -> PyResult<i32> {
        if arg < 0 {
            return Ok(default);
        }
        match arg {
            crossinterp::UNBOUND_REMOVE
            | crossinterp::UNBOUND_ERROR
            | crossinterp::UNBOUND_REPLACE => Ok(arg),
            _ => Err(vm.new_value_error(format!("unsupported unboundop {arg}"))),
        }
    }

    fn resolve_fallback(arg: i32, default: i32, vm: &VirtualMachine) -> PyResult<Fallback> {
        let value = if arg < 0 { default } else { arg };
        Fallback::from_i32(value)
            .ok_or_else(|| vm.new_value_error(format!("unsupported fallback {arg}")))
    }

    fn int_arg(obj: &PyObject, vm: &VirtualMachine) -> PyResult<i32> {
        let value =
            obj.try_index(vm)?.as_bigint().to_i64().ok_or_else(|| {
                vm.new_overflow_error("Python int too large to convert to C long")
            })?;
        i32::try_from(value).map_err(|_| {
            let msg = if value > i32::MAX as i64 {
                "signed integer is greater than maximum"
            } else {
                "signed integer is less than minimum"
            };
            vm.new_overflow_error(msg)
        })
    }

    fn ssize_arg(obj: &PyObject, vm: &VirtualMachine) -> PyResult<isize> {
        obj.try_index(vm)?
            .as_bigint()
            .to_isize()
            .ok_or_else(|| vm.new_overflow_error("Python int too large to convert to C ssize_t"))
    }

    fn parse_qid(obj: &PyObject, vm: &VirtualMachine) -> PyResult<i64> {
        if !obj.number().is_index() {
            return Err(vm.new_type_error(format!(
                "queue ID must be an int, got {}",
                obj.class().name()
            )));
        }
        let value = obj.try_index(vm)?;
        let bigint = value.as_bigint();
        if bigint.is_negative() {
            let repr = obj.repr(vm)?;
            return Err(
                vm.new_value_error(format!("queue ID must be a non-negative int, got {repr}"))
            );
        }
        if let Some(qid) = bigint.to_i64() {
            return Ok(qid);
        }
        let repr = obj.repr(vm)?;
        Err(vm.new_overflow_error(format!("max queue ID is {}, got {repr}", i64::MAX)))
    }

    fn qid_arg(args: &FuncArgs, func: &'static str, vm: &VirtualMachine) -> PyResult<i64> {
        let qid = Cell::new(None);
        ArgSpec {
            fname: func,
            keywords: &["qid"],
            required: 1,
            max_positional: 1,
        }
        .parse_with(
            args,
            |_, obj, vm| {
                qid.set(Some(parse_qid(obj, vm)?));
                Ok(())
            },
            vm,
        )?;
        Ok(qid.get().unwrap())
    }

    #[expect(clippy::unnecessary_wraps, reason = "Needs to comply with a signature")]
    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        crate::stdlib::_interpreters::init_xi_types(vm);
        __module_exec(vm, module);
        Ok(())
    }

    #[pyfunction]
    fn create(args: FuncArgs, vm: &VirtualMachine) -> PyResult<i64> {
        let maxsize = Cell::new(None);
        let unboundarg = Cell::new(None);
        let fallbackarg = Cell::new(None);
        ArgSpec {
            fname: "create",
            keywords: &["maxsize", "unboundop", "fallback"],
            required: 1,
            max_positional: 3,
        }
        .parse_with(
            &args,
            |i, obj, vm| {
                match i {
                    0 => maxsize.set(Some(ssize_arg(obj, vm)?)),
                    1 => unboundarg.set(Some(int_arg(obj, vm)?)),
                    2 => fallbackarg.set(Some(int_arg(obj, vm)?)),
                    _ => unreachable!(),
                }
                Ok(())
            },
            vm,
        )?;
        let unboundop = resolve_unboundop(unboundarg.get().unwrap_or(-1), UNBOUND_REPLACE, vm)?;
        let fallback =
            resolve_fallback(fallbackarg.get().unwrap_or(-1), Fallback::Full.as_i32(), vm)?;
        queue_create(maxsize.get().unwrap(), unboundop, fallback).map_err(|err| err.into_py(-1, vm))
    }

    #[pyfunction]
    fn destroy(args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let qid = qid_arg(&args, "destroy", vm)?;
        queue_destroy(qid).map_err(|err| err.into_py(qid, vm))
    }

    #[pyfunction]
    fn list_all(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        if !args.kwargs.is_empty() {
            return Err(vm.new_type_error("_interpqueues.list_all() takes no keyword arguments"));
        }
        if !args.args.is_empty() {
            return Err(vm.new_type_error(format!(
                "_interpqueues.list_all() takes no arguments ({} given)",
                args.args.len()
            )));
        }
        let entries: Vec<_> = queues()
            .lock()
            .refs
            .iter()
            .rev()
            .map(|(&qid, queue)| (qid, queue.queue.clone()))
            .collect();
        let mut result = Vec::with_capacity(entries.len());
        for (qid, queue) in entries {
            let state = queue.lock();
            if state.alive {
                result.push(
                    vm.ctx
                        .new_tuple(vec![
                            vm.ctx.new_int(qid).into(),
                            vm.ctx.new_int(state.unboundop).into(),
                            vm.ctx.new_int(state.fallback.as_i32()).into(),
                        ])
                        .into(),
                );
            }
        }
        Ok(vm.ctx.new_list(result).into())
    }

    #[pyfunction]
    fn put(args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let qid = Cell::new(None);
        let unboundarg = Cell::new(None);
        let fallbackarg = Cell::new(None);
        let parsed = ArgSpec {
            fname: "put",
            keywords: &["qid", "obj", "unboundop", "fallback"],
            required: 2,
            max_positional: 4,
        }
        .parse_with(
            &args,
            |i, obj, vm| {
                match i {
                    0 => qid.set(Some(parse_qid(obj, vm)?)),
                    2 => unboundarg.set(Some(int_arg(obj, vm)?)),
                    3 => fallbackarg.set(Some(int_arg(obj, vm)?)),
                    _ => {}
                }
                Ok(())
            },
            vm,
        )?;
        let qid = qid.get().unwrap();
        let obj = parsed[1].as_deref().unwrap();
        let unboundarg = unboundarg.get().unwrap_or(-1);
        let fallbackarg = fallbackarg.get().unwrap_or(-1);
        // The queue is only consulted when one of the arguments needs its default.
        let (default_unboundop, default_fallback) = if unboundarg < 0 || fallbackarg < 0 {
            let (unboundop, fallback) = queue_defaults(qid).map_err(|err| err.into_py(qid, vm))?;
            (unboundop, fallback.as_i32())
        } else {
            (-1, -1)
        };
        let unboundop = resolve_unboundop(unboundarg, default_unboundop, vm)?;
        let fallback = resolve_fallback(fallbackarg, default_fallback, vm)?;
        let value = SharedValue::from_object(obj, fallback, vm)?;
        let queue = queue_lookup(qid).map_err(|err| err.into_py(qid, vm))?;
        let mut state = queue.lock();
        if !state.alive {
            return Err(QueueErr::NotFound.into_py(qid, vm));
        }
        if state.is_full() {
            drop(state);
            return Err(queue_full_error(qid, vm)?);
        }
        state.items.push_back(QueueItem {
            interpid: vm.state.interpreter_id,
            data: Some(value),
            unboundop,
        });
        Ok(())
    }

    #[pyfunction]
    fn get(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let qid = qid_arg(&args, "get", vm)?;
        let item = {
            let queue = queue_lookup(qid).map_err(|err| err.into_py(qid, vm))?;
            let mut state = queue.lock();
            if !state.alive {
                return Err(QueueErr::NotFound.into_py(qid, vm));
            }
            match state.items.pop_front() {
                Some(item) => item,
                None => {
                    drop(state);
                    return Err(queue_empty_error(qid, vm)?);
                }
            }
        };
        let (obj, unboundop) = match item.data {
            Some(data) => (data.into_object(vm)?, vm.ctx.none()),
            None => (vm.ctx.none(), vm.ctx.new_int(item.unboundop).into()),
        };
        Ok(vm.ctx.new_tuple(vec![obj, unboundop]).into())
    }

    #[pyfunction]
    fn bind(args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let qid = qid_arg(&args, "bind", vm)?;
        queue_bind(qid).map_err(|err| err.into_py(qid, vm))
    }

    #[pyfunction]
    fn release(args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let qid = qid_arg(&args, "release", vm)?;
        queue_release(qid).map_err(|err| err.into_py(qid, vm))
    }

    #[pyfunction]
    fn get_maxsize(args: FuncArgs, vm: &VirtualMachine) -> PyResult<isize> {
        let qid = qid_arg(&args, "get_maxsize", vm)?;
        let queue = queue_lookup(qid).map_err(|err| err.into_py(qid, vm))?;
        let state = queue.lock();
        if !state.alive {
            return Err(QueueErr::NotFound.into_py(qid, vm));
        }
        Ok(state.maxsize)
    }

    #[pyfunction]
    fn get_queue_defaults(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let qid = qid_arg(&args, "get_queue_defaults", vm)?;
        let (unboundop, fallback) = queue_defaults(qid).map_err(|err| err.into_py(qid, vm))?;
        Ok(vm
            .ctx
            .new_tuple(vec![
                vm.ctx.new_int(unboundop).into(),
                vm.ctx.new_int(fallback.as_i32()).into(),
            ])
            .into())
    }

    #[pyfunction]
    fn is_full(args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
        let qid = qid_arg(&args, "is_full", vm)?;
        let queue = queue_lookup(qid).map_err(|err| err.into_py(qid, vm))?;
        let state = queue.lock();
        if !state.alive {
            return Err(QueueErr::NotFound.into_py(qid, vm));
        }
        Ok(state.is_full())
    }

    #[pyfunction]
    fn get_count(args: FuncArgs, vm: &VirtualMachine) -> PyResult<usize> {
        let qid = qid_arg(&args, "get_count", vm)?;
        let queue = queue_lookup(qid).map_err(|err| err.into_py(qid, vm))?;
        let state = queue.lock();
        if !state.alive {
            return Err(QueueErr::NotFound.into_py(qid, vm));
        }
        Ok(state.items.len())
    }

    #[pyfunction]
    fn _register_heap_types(args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let parsed = ArgSpec {
            fname: "_register_heap_types",
            keywords: &["queuetype", "emptyerror", "fullerror"],
            required: 3,
            max_positional: 3,
        }
        .parse(&args, vm)?;
        let queue = parsed[0]
            .clone()
            .unwrap()
            .downcast::<PyType>()
            .map_err(|_| vm.new_type_error("expected a type for 'queuetype'"))?;
        let empty = parsed[1]
            .clone()
            .unwrap()
            .downcast::<PyType>()
            .ok()
            .filter(|ty| ty.fast_issubclass(vm.ctx.exceptions.base_exception_type))
            .ok_or_else(|| vm.new_type_error("expected an exception type for 'emptyerror'"))?;
        let full = parsed[2]
            .clone()
            .unwrap()
            .downcast::<PyType>()
            .ok()
            .filter(|ty| ty.fast_issubclass(vm.ctx.exceptions.base_exception_type))
            .ok_or_else(|| vm.new_type_error("expected an exception type for 'fullerror'"))?;
        set_external_types(
            vm.state.interpreter_id,
            ExternalTypes { queue, empty, full },
        );
        Ok(())
    }

    pub(crate) fn is_external_queue(obj: &PyObject, vm: &VirtualMachine) -> bool {
        get_external_types(vm.state.interpreter_id)
            .is_some_and(|types| obj.class().is(&types.queue))
    }

    pub(crate) fn queue_id_from_object(
        obj: &PyObject,
        vm: &VirtualMachine,
    ) -> Option<PyResult<i64>> {
        if !is_external_queue(obj, vm) {
            return None;
        }
        Some(obj.get_attr("_id", vm).and_then(|id| parse_qid(&id, vm)))
    }

    pub(crate) fn queue_from_xid(qid: i64, vm: &VirtualMachine) -> PyResult {
        let queue_type = ensure_external_types(vm)?.queue;
        queue_type.as_object().call((vm.ctx.new_int(qid),), vm)
    }

    pub(crate) fn queue_xid_incref(qid: i64) -> bool {
        queue_bind(qid).is_ok()
    }

    pub(crate) fn queue_xid_decref(qid: i64) {
        match queue_release(qid) {
            // Already destroyed.
            Err(QueueErr::NotFound) => {}
            // The reference being released was taken by `queue_xid_incref`,
            // so the queue cannot be unbound here.
            res => debug_assert!(res.is_ok()),
        }
    }

    #[cfg_attr(not(feature = "threading"), allow(dead_code))]
    pub(crate) fn clear_interpreter(interpid: i64) {
        remove_external_types(interpid);
        let queues: Vec<_> = queues()
            .lock()
            .refs
            .values()
            .map(|queue| queue.queue.clone())
            .collect();
        let mut dropped = Vec::new();
        for queue in queues {
            let mut state = queue.lock();
            let mut index = 0;
            while index < state.items.len() {
                if state.items[index].interpid != interpid {
                    index += 1;
                    continue;
                }
                if state.items[index].unboundop == UNBOUND_REMOVE {
                    dropped.push(state.items.remove(index).and_then(|item| item.data));
                } else {
                    dropped.push(state.items[index].data.take());
                    index += 1;
                }
            }
        }
        drop(dropped);
    }
}
