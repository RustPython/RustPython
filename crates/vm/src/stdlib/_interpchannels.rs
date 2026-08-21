//! Low-level cross-interpreter channels (`_interpchannels`).
//!
//! Mirrors CPython `Modules/_interpchannelsmodule.c`.

#[cfg_attr(not(feature = "threading"), allow(unused_imports))]
pub(crate) use _interpchannels::clear_interpreter;
pub(crate) use _interpchannels::module_def;
pub(crate) use _interpchannels::{channel_id_from_parts, channel_id_parts};

#[pymodule]
pub(crate) mod _interpchannels {
    use crate::{
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyResult, VirtualMachine,
        builtins::{PyFloat, PyInt},
        convert::ToPyObject,
        function::FuncArgs,
        protocol::PyNumberMethods,
        types::{AsNumber, Comparable, Constructor, Hashable, PyComparisonOp, Representable},
        vm::crossinterp::{self, SharedValue, UNBOUND_REPLACE},
    };
    use alloc::{
        collections::{BTreeSet, VecDeque},
        sync::Arc,
    };
    use core::time::Duration;
    use parking_lot::{Condvar, Mutex};
    use std::{collections::HashMap, sync::OnceLock, time::Instant};

    const CHANNEL_BOTH: i32 = 0;
    const CHANNEL_SEND: i32 = 1;
    const CHANNEL_RECV: i32 = 2;

    #[pyattr]
    #[pyexception(name = "ChannelError", base = crate::exceptions::types::PyRuntimeError)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub(crate) struct PyChannelError(crate::exceptions::types::PyRuntimeError);

    #[pyexception]
    impl PyChannelError {}

    #[pyattr]
    #[pyexception(name = "ChannelNotFoundError", base = PyChannelError)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub(crate) struct PyChannelNotFoundError(PyChannelError);

    #[pyexception]
    impl PyChannelNotFoundError {}

    #[pyattr]
    #[pyexception(name = "ChannelClosedError", base = PyChannelError)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub(crate) struct PyChannelClosedError(PyChannelError);

    #[pyexception]
    impl PyChannelClosedError {}

    #[pyattr]
    #[pyexception(name = "ChannelEmptyError", base = PyChannelError)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub(crate) struct PyChannelEmptyError(PyChannelError);

    #[pyexception]
    impl PyChannelEmptyError {}

    #[pyattr]
    #[pyexception(name = "ChannelNotEmptyError", base = PyChannelError)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub(crate) struct PyChannelNotEmptyError(PyChannelError);

    #[pyexception]
    impl PyChannelNotEmptyError {}

    fn not_found(vm: &VirtualMachine, cid: i64) -> crate::builtins::PyBaseExceptionRef {
        vm.new_exception_msg(
            PyChannelNotFoundError::class(&vm.ctx).to_owned(),
            format!("channel {cid} not found").into(),
        )
    }

    fn closed_err(vm: &VirtualMachine, cid: i64) -> crate::builtins::PyBaseExceptionRef {
        vm.new_exception_msg(
            PyChannelClosedError::class(&vm.ctx).to_owned(),
            format!("channel {cid} is closed").into(),
        )
    }

    fn empty_err(vm: &VirtualMachine, cid: i64) -> crate::builtins::PyBaseExceptionRef {
        vm.new_exception_msg(
            PyChannelEmptyError::class(&vm.ctx).to_owned(),
            format!("channel {cid} is empty").into(),
        )
    }

    fn not_empty_err(vm: &VirtualMachine) -> crate::builtins::PyBaseExceptionRef {
        vm.new_exception_msg(
            PyChannelNotEmptyError::class(&vm.ctx).to_owned(),
            "channel is not empty".to_owned().into(),
        )
    }

    struct ItemWaiter {
        received: Mutex<bool>,
        cond: Condvar,
        closed: Mutex<bool>,
    }

    struct ChannelItem {
        #[allow(dead_code)]
        interpid: i64,
        payload: Option<SharedValue>,
        unboundop: i32,
        waiter: Option<Arc<ItemWaiter>>,
    }

    struct ChannelInner {
        items: VecDeque<ChannelItem>,
        send_assoc: BTreeSet<i64>,
        recv_assoc: BTreeSet<i64>,
        send_closed: bool,
        recv_closed: bool,
        destroyed: bool,
        /// `close()` hides the channel from `list_all`; `release()` does not.
        hidden_from_list: bool,
        unboundop: i32,
        fallback: i32,
        id_refs: isize,
    }

    impl ChannelInner {
        fn fully_closed(&self) -> bool {
            self.destroyed || (self.send_closed && self.recv_closed)
        }
    }

    struct Channel {
        #[allow(dead_code)]
        id: i64,
        inner: Mutex<ChannelInner>,
        cond: Condvar,
    }

    struct ChannelTable {
        next_id: i64,
        channels: HashMap<i64, Arc<Channel>>,
    }

    fn table() -> &'static Mutex<ChannelTable> {
        static TABLE: OnceLock<Mutex<ChannelTable>> = OnceLock::new();
        TABLE.get_or_init(|| {
            Mutex::new(ChannelTable {
                next_id: 0,
                channels: HashMap::new(),
            })
        })
    }

    fn lookup(cid: i64) -> Option<Arc<Channel>> {
        table().lock().channels.get(&cid).cloned()
    }

    fn create_channel(unboundop: i32, fallback: i32) -> i64 {
        let mut t = table().lock();
        let id = t.next_id;
        t.next_id += 1;
        t.channels.insert(
            id,
            Arc::new(Channel {
                id,
                inner: Mutex::new(ChannelInner {
                    items: VecDeque::new(),
                    send_assoc: BTreeSet::new(),
                    recv_assoc: BTreeSet::new(),
                    send_closed: false,
                    recv_closed: false,
                    destroyed: false,
                    hidden_from_list: false,
                    unboundop,
                    fallback,
                    id_refs: 0,
                }),
                cond: Condvar::new(),
            }),
        );
        id
    }

    fn destroy_channel(cid: i64) -> Result<(), ChannelOpErr> {
        let mut t = table().lock();
        match t.channels.remove(&cid) {
            Some(ch) => {
                let mut inner = ch.inner.lock();
                inner.destroyed = true;
                inner.send_closed = true;
                inner.recv_closed = true;
                for item in inner.items.drain(..) {
                    if let Some(w) = item.waiter {
                        *w.closed.lock() = true;
                        w.cond.notify_all();
                    }
                }
                ch.cond.notify_all();
                Ok(())
            }
            None => Err(ChannelOpErr::NotFound),
        }
    }

    enum ChannelOpErr {
        NotFound,
    }

    impl ChannelOpErr {
        fn into_py(self, vm: &VirtualMachine, cid: i64) -> crate::builtins::PyBaseExceptionRef {
            match self {
                Self::NotFound => not_found(vm, cid),
            }
        }
    }

    fn parse_cid(obj: &PyObject, vm: &VirtualMachine) -> PyResult<(i64, i32)> {
        if let Some(cid) = obj.downcast_ref::<ChannelID>() {
            return Ok((cid.cid, cid.end));
        }
        // Accept any indexable object (`__index__`), matching CPython.
        let n = obj.try_index(vm)?;
        let id = n.try_to_primitive::<i64>(vm)?;
        if id < 0 {
            return Err(
                vm.new_value_error(format!("channel ID must be a non-negative int, got {id}"))
            );
        }
        Ok((id, CHANNEL_BOTH))
    }

    fn associate(inner: &mut ChannelInner, interpid: i64, send: bool) {
        if send {
            inner.send_assoc.insert(interpid);
        } else {
            inner.recv_assoc.insert(interpid);
        }
    }

    #[allow(dead_code)]
    fn apply_unbound(item: &mut ChannelItem) {
        match item.unboundop {
            crossinterp::UNBOUND_REMOVE => {
                item.payload = None;
            }
            _ => {
                item.payload = None;
            }
        }
    }

    #[cfg_attr(not(feature = "threading"), allow(dead_code))]
    pub(crate) fn clear_interpreter(interpid: i64) {
        let channels: Vec<Arc<Channel>> = table().lock().channels.values().cloned().collect();
        for ch in channels {
            let mut inner = ch.inner.lock();
            inner.send_assoc.remove(&interpid);
            inner.recv_assoc.remove(&interpid);
            let mut i = 0;
            while i < inner.items.len() {
                if inner.items[i].interpid == interpid {
                    if inner.items[i].unboundop == crossinterp::UNBOUND_REMOVE {
                        if let Some(w) = inner.items[i].waiter.take() {
                            *w.closed.lock() = true;
                            w.cond.notify_all();
                        }
                        inner.items.remove(i);
                        continue;
                    }
                    apply_unbound(&mut inner.items[i]);
                    if let Some(w) = inner.items[i].waiter.take() {
                        *w.received.lock() = true;
                        w.cond.notify_all();
                    }
                }
                i += 1;
            }
            if inner.send_assoc.is_empty() && inner.recv_assoc.is_empty() {
                for item in inner.items.drain(..) {
                    if let Some(w) = item.waiter {
                        *w.closed.lock() = true;
                        w.cond.notify_all();
                    }
                }
                inner.send_closed = true;
                inner.recv_closed = true;
                ch.cond.notify_all();
            }
        }
    }

    fn resolve_unboundop(arg: i32, default: i32) -> i32 {
        if arg < 0 { default } else { arg }
    }

    #[pyattr]
    #[pyclass(name = "ChannelID", module = "_interpchannels")]
    #[derive(Debug, PyPayload)]
    pub(crate) struct ChannelID {
        cid: i64,
        end: i32,
        resolve: bool,
    }

    impl Drop for ChannelID {
        fn drop(&mut self) {
            if let Some(ch) = lookup(self.cid) {
                let remove = {
                    let mut inner = ch.inner.lock();
                    inner.id_refs = inner.id_refs.saturating_sub(1);
                    inner.id_refs <= 0 && (inner.hidden_from_list || inner.destroyed)
                };
                if remove {
                    table().lock().channels.remove(&self.cid);
                }
            }
        }
    }

    impl Constructor for ChannelID {
        type Args = FuncArgs;

        fn py_new(
            _cls: &crate::Py<crate::builtins::PyType>,
            args: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            channel_id_new(args, vm)
        }
    }

    impl Representable for ChannelID {
        #[inline]
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok(match zelf.end {
                CHANNEL_SEND => format!("ChannelID({}, send=True)", zelf.cid),
                CHANNEL_RECV => format!("ChannelID({}, recv=True)", zelf.cid),
                _ => format!("ChannelID({})", zelf.cid),
            })
        }
    }

    impl Hashable for ChannelID {
        #[inline]
        fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<rustpython_common::hash::PyHash> {
            vm.ctx.new_int(zelf.cid).as_object().hash(vm)
        }
    }

    impl Comparable for ChannelID {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            vm: &VirtualMachine,
        ) -> PyResult<crate::function::PyComparisonValue> {
            use crate::function::PyComparisonValue;
            if !matches!(op, PyComparisonOp::Eq | PyComparisonOp::Ne) {
                return Ok(PyComparisonValue::NotImplemented);
            }
            let equal = if let Some(o) = other.downcast_ref::<Self>() {
                zelf.cid == o.cid && zelf.end == o.end
            } else if let Some(n) = other.downcast_ref::<PyInt>() {
                n.try_to_primitive::<i64>(vm).is_ok_and(|v| v == zelf.cid)
            } else if let Some(f) = other.downcast_ref::<PyFloat>() {
                let fv = f.to_f64();
                fv.is_finite() && fv == zelf.cid as f64 && (fv as i64) == zelf.cid
            } else {
                return Ok(PyComparisonValue::NotImplemented);
            };
            Ok(PyComparisonValue::Implemented(
                if op == PyComparisonOp::Eq {
                    equal
                } else {
                    !equal
                },
            ))
        }
    }

    impl AsNumber for ChannelID {
        fn as_number() -> &'static PyNumberMethods {
            static METHODS: PyNumberMethods = PyNumberMethods {
                int: Some(|obj, vm| {
                    let zelf = obj.downcast_ref::<ChannelID>().unwrap();
                    Ok(vm.ctx.new_int(zelf.cid).into())
                }),
                index: Some(|obj, vm| {
                    let zelf = obj.downcast_ref::<ChannelID>().unwrap();
                    Ok(vm.ctx.new_int(zelf.cid).into())
                }),
                ..PyNumberMethods::NOT_IMPLEMENTED
            };
            &METHODS
        }
    }

    #[pyclass(
        with(Constructor, Representable, Hashable, Comparable, AsNumber),
        flags(BASETYPE)
    )]
    impl ChannelID {
        #[pygetset]
        fn end(&self) -> String {
            match self.end {
                CHANNEL_SEND => "send".to_owned(),
                CHANNEL_RECV => "recv".to_owned(),
                _ => "both".to_owned(),
            }
        }

        #[pygetset(name = "send")]
        fn send_end(&self, vm: &VirtualMachine) -> PyResult {
            channel_id_from_parts(self.cid, CHANNEL_SEND, true, self.resolve, vm)
        }

        #[pygetset(name = "recv")]
        fn recv_end(&self, vm: &VirtualMachine) -> PyResult {
            channel_id_from_parts(self.cid, CHANNEL_RECV, true, self.resolve, vm)
        }

        #[pymethod]
        fn __str__(&self) -> String {
            self.cid.to_string()
        }
    }

    fn channel_id_new(args: FuncArgs, vm: &VirtualMachine) -> PyResult<ChannelID> {
        let id_obj = args
            .args
            .first()
            .ok_or_else(|| vm.new_type_error("ChannelID() missing required argument: 'id'"))?;
        let (cid, mut end) = parse_cid(id_obj, vm)?;
        let send = args
            .kwargs
            .get("send")
            .map(|o| o.clone().is_true(vm))
            .transpose()?;
        let recv = args
            .kwargs
            .get("recv")
            .map(|o| o.clone().is_true(vm))
            .transpose()?;
        let force = args
            .kwargs
            .get("force")
            .map(|o| o.clone().is_true(vm))
            .transpose()?
            .unwrap_or(false);
        let resolve = args
            .kwargs
            .get("_resolve")
            .map(|o| o.clone().is_true(vm))
            .transpose()?
            .unwrap_or(false);
        match (send, recv) {
            (Some(false), Some(false)) => {
                return Err(vm.new_value_error("'send' and 'recv' cannot both be False"));
            }
            (Some(true), Some(false) | None) => end = CHANNEL_SEND,
            (Some(false) | None, Some(true)) => end = CHANNEL_RECV,
            (Some(true), Some(true)) => end = CHANNEL_BOTH,
            _ => {}
        }
        if !force && lookup(cid).is_none() {
            return Err(not_found(vm, cid));
        }
        if let Some(ch) = lookup(cid) {
            ch.inner.lock().id_refs += 1;
        }
        Ok(ChannelID { cid, end, resolve })
    }

    pub(crate) fn channel_id_from_parts(
        cid: i64,
        end: i32,
        force: bool,
        resolve: bool,
        vm: &VirtualMachine,
    ) -> PyResult {
        if !force && lookup(cid).is_none() {
            return Err(not_found(vm, cid));
        }
        if let Some(ch) = lookup(cid) {
            ch.inner.lock().id_refs += 1;
        }
        Ok(ChannelID { cid, end, resolve }.to_pyobject(vm))
    }

    pub(crate) fn channel_id_parts(obj: &PyObject) -> Option<(i64, i32)> {
        obj.downcast_ref::<ChannelID>().map(|c| (c.cid, c.end))
    }

    #[pyfunction]
    fn create(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let unbound = match args.args.first() {
            Some(o) => o.try_index(vm)?.try_to_primitive::<i32>(vm)?,
            None => UNBOUND_REPLACE,
        };
        let fallback = match args.kwargs.get("fallback") {
            Some(o) => o.try_index(vm)?.try_to_primitive::<i32>(vm)?,
            None => -1,
        };
        let cid = create_channel(unbound, fallback);
        channel_id_from_parts(cid, CHANNEL_BOTH, false, false, vm)
    }

    #[pyfunction]
    fn destroy(cid: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let (cid, _) = parse_cid(&cid, vm)?;
        destroy_channel(cid).map_err(|e| e.into_py(vm, cid))
    }

    #[pyfunction]
    fn list_all(vm: &VirtualMachine) -> PyResult {
        let chans: Vec<(i64, i32, i32)> = {
            let t = table().lock();
            t.channels
                .iter()
                .filter_map(|(&id, ch)| {
                    let inner = ch.inner.lock();
                    if inner.hidden_from_list || inner.destroyed {
                        None
                    } else {
                        Some((id, inner.unboundop, inner.fallback))
                    }
                })
                .collect()
        };
        let mut items = Vec::new();
        for (id, unbound, fallback) in chans {
            let cid = channel_id_from_parts(id, CHANNEL_BOTH, false, false, vm)?;
            items.push(
                vm.ctx
                    .new_tuple(vec![
                        cid,
                        vm.ctx.new_int(unbound).into(),
                        vm.ctx.new_int(fallback).into(),
                    ])
                    .into(),
            );
        }
        Ok(vm.ctx.new_list(items).into())
    }

    #[pyfunction]
    fn list_interpreters(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let cid_obj = args
            .args
            .first()
            .ok_or_else(|| vm.new_type_error("list_interpreters() missing argument 1"))?;
        let (cid, _) = parse_cid(cid_obj, vm)?;
        let send = args
            .kwargs
            .get("send")
            .ok_or_else(|| vm.new_type_error("list_interpreters() missing keyword 'send'"))?
            .clone()
            .is_true(vm)?;
        let ch = lookup(cid).ok_or_else(|| not_found(vm, cid))?;
        let inner = ch.inner.lock();
        if inner.destroyed || inner.fully_closed() || (send && inner.send_closed) {
            return Err(closed_err(vm, cid));
        }
        if !send && inner.recv_closed && inner.items.is_empty() {
            return Err(closed_err(vm, cid));
        }
        let ids = if send {
            &inner.send_assoc
        } else {
            &inner.recv_assoc
        };
        let mut out = Vec::new();
        for &id in ids {
            if crate::vm::runtime::lookup_interpreter(id).is_some() {
                out.push(vm.ctx.new_int(id).into());
            }
        }
        Ok(vm.ctx.new_list(out).into())
    }

    fn do_send(
        cid: i64,
        value: SharedValue,
        unboundop: i32,
        blocking: bool,
        timeout: Option<Duration>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let ch = lookup(cid).ok_or_else(|| not_found(vm, cid))?;
        let waiter = if blocking {
            Some(Arc::new(ItemWaiter {
                received: Mutex::new(false),
                cond: Condvar::new(),
                closed: Mutex::new(false),
            }))
        } else {
            None
        };
        {
            let mut inner = ch.inner.lock();
            if inner.destroyed {
                return Err(not_found(vm, cid));
            }
            if inner.send_closed || inner.fully_closed() {
                return Err(closed_err(vm, cid));
            }
            associate(&mut inner, vm.state.interpreter_id, true);
            inner.items.push_back(ChannelItem {
                interpid: vm.state.interpreter_id,
                payload: Some(value),
                unboundop,
                waiter: waiter.clone(),
            });
            ch.cond.notify_all();
        }
        if let Some(w) = waiter {
            let deadline = timeout.map(|t| Instant::now() + t);
            vm.allow_threads(|| {
                let mut recvd = w.received.lock();
                loop {
                    if *recvd {
                        return Ok(());
                    }
                    if *w.closed.lock() {
                        return Err(());
                    }
                    if let Some(dl) = deadline {
                        let now = Instant::now();
                        if now >= dl {
                            return Err(());
                        }
                        let leftover = dl - now;
                        if w.cond.wait_for(&mut recvd, leftover).timed_out() {
                            return Err(());
                        }
                    } else {
                        w.cond.wait(&mut recvd);
                    }
                }
            })
            .map_err(|()| {
                // timed out or closed: remove item if still queued
                let mut inner = ch.inner.lock();
                if let Some(pos) = inner
                    .items
                    .iter()
                    .position(|it| it.waiter.as_ref().is_some_and(|iw| Arc::ptr_eq(iw, &w)))
                {
                    inner.items.remove(pos);
                    if timeout.is_some() && !*w.closed.lock() {
                        return vm
                            .new_os_subtype_error(
                                vm.ctx.exceptions.timeout_error.to_owned(),
                                None,
                                "timed out",
                            )
                            .upcast();
                    }
                    return closed_err(vm, cid);
                }
                if *w.closed.lock() {
                    closed_err(vm, cid)
                } else {
                    vm.new_os_subtype_error(
                        vm.ctx.exceptions.timeout_error.to_owned(),
                        None,
                        "timed out",
                    )
                    .upcast()
                }
            })?;
        }
        Ok(())
    }

    #[pyfunction]
    fn send(args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let cid_obj = args
            .args
            .first()
            .ok_or_else(|| vm.new_type_error("send() missing argument 1"))?;
        let obj = args
            .args
            .get(1)
            .ok_or_else(|| vm.new_type_error("send() missing argument 2"))?;
        let (cid, _) = parse_cid(cid_obj, vm)?;
        let unboundarg = args
            .args
            .get(2)
            .or_else(|| args.kwargs.get("unboundop"))
            .map(|o| o.try_index(vm).and_then(|i| i.try_to_primitive::<i32>(vm)))
            .transpose()?
            .unwrap_or(-1);
        let blocking = args
            .kwargs
            .get("blocking")
            .map(|o| o.clone().is_true(vm))
            .transpose()?
            .unwrap_or(true);
        let timeout = parse_timeout(&args, blocking, vm)?;
        let ch = lookup(cid).ok_or_else(|| not_found(vm, cid))?;
        let default_unbound = ch.inner.lock().unboundop;
        let unboundop = resolve_unboundop(unboundarg, default_unbound);
        let value = SharedValue::from_object(obj, vm)?;
        do_send(cid, value, unboundop, blocking, timeout, vm)
    }

    #[pyfunction]
    fn send_buffer(args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let cid_obj = args
            .args
            .first()
            .ok_or_else(|| vm.new_type_error("send_buffer() missing argument 1"))?;
        let obj = args
            .args
            .get(1)
            .ok_or_else(|| vm.new_type_error("send_buffer() missing argument 2"))?;
        let (cid, _) = parse_cid(cid_obj, vm)?;
        let unboundarg = args
            .args
            .get(2)
            .or_else(|| args.kwargs.get("unboundop"))
            .map(|o| o.try_index(vm).and_then(|i| i.try_to_primitive::<i32>(vm)))
            .transpose()?
            .unwrap_or(-1);
        let blocking = args
            .kwargs
            .get("blocking")
            .map(|o| o.clone().is_true(vm))
            .transpose()?
            .unwrap_or(true);
        let timeout = parse_timeout(&args, blocking, vm)?;
        let ch = lookup(cid).ok_or_else(|| not_found(vm, cid))?;
        let default_unbound = ch.inner.lock().unboundop;
        let unboundop = resolve_unboundop(unboundarg, default_unbound);
        let value = SharedValue::from_buffer_object(obj, vm)?;
        do_send(cid, value, unboundop, blocking, timeout, vm)
    }

    fn parse_timeout(
        args: &FuncArgs,
        blocking: bool,
        vm: &VirtualMachine,
    ) -> PyResult<Option<Duration>> {
        let Some(t) = args.kwargs.get("timeout") else {
            return Ok(None);
        };
        if vm.is_none(t) {
            return Ok(None);
        }
        if !blocking {
            return Err(vm.new_value_error("can't specify a timeout for a non-blocking call"));
        }
        let secs = t.try_float(vm)?.to_f64();
        if secs < 0.0 {
            return Err(vm.new_value_error("timeout value must be non-negative"));
        }
        Ok(Some(Duration::from_secs_f64(secs)))
    }

    #[pyfunction]
    fn recv(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let cid_obj = args
            .args
            .first()
            .ok_or_else(|| vm.new_type_error("recv() missing argument 1"))?;
        let (cid, _) = parse_cid(cid_obj, vm)?;
        let default = args.args.get(1).cloned();
        let ch = lookup(cid).ok_or_else(|| not_found(vm, cid))?;
        let mut inner = ch.inner.lock();
        if inner.destroyed {
            return Err(not_found(vm, cid));
        }
        associate(&mut inner, vm.state.interpreter_id, false);
        if inner.items.is_empty() {
            if inner.recv_closed
                || inner.fully_closed()
                || inner.send_closed && inner.send_assoc.is_empty()
            {
                return Err(closed_err(vm, cid));
            }
            if let Some(d) = default {
                return Ok(vm.ctx.new_tuple(vec![d, vm.ctx.none()]).into());
            }
            return Err(empty_err(vm, cid));
        }
        let item = inner.items.pop_front().unwrap();
        if inner.items.is_empty() && inner.send_closed {
            inner.recv_closed = true;
        }
        drop(inner);
        if let Some(w) = &item.waiter {
            *w.received.lock() = true;
            w.cond.notify_all();
        }
        match item.payload {
            Some(val) => {
                let obj = val.into_object(vm)?;
                Ok(vm.ctx.new_tuple(vec![obj, vm.ctx.none()]).into())
            }
            None => Ok(vm
                .ctx
                .new_tuple(vec![vm.ctx.none(), vm.ctx.new_int(item.unboundop).into()])
                .into()),
        }
    }

    #[pyfunction]
    fn close(args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let cid_obj = args
            .args
            .first()
            .ok_or_else(|| vm.new_type_error("close() missing argument 1"))?;
        let (cid, _) = parse_cid(cid_obj, vm)?;
        let send = args
            .kwargs
            .get("send")
            .map(|o| o.clone().is_true(vm))
            .transpose()?
            .unwrap_or(false);
        let recv = args
            .kwargs
            .get("recv")
            .map(|o| o.clone().is_true(vm))
            .transpose()?
            .unwrap_or(false);
        let force = args
            .kwargs
            .get("force")
            .map(|o| o.clone().is_true(vm))
            .transpose()?
            .unwrap_or(false);
        let ch = lookup(cid).ok_or_else(|| not_found(vm, cid))?;
        let mut inner = ch.inner.lock();
        if inner.destroyed {
            return Err(not_found(vm, cid));
        }
        if inner.fully_closed() {
            return Err(closed_err(vm, cid));
        }
        let empty = inner.items.is_empty();
        if empty {
            inner.send_closed = true;
            inner.recv_closed = true;
            inner.hidden_from_list = true;
            ch.cond.notify_all();
            return Ok(());
        }
        if force {
            for item in inner.items.drain(..) {
                if let Some(w) = item.waiter {
                    *w.closed.lock() = true;
                    w.cond.notify_all();
                }
            }
            inner.send_closed = true;
            inner.recv_closed = true;
            inner.hidden_from_list = true;
            ch.cond.notify_all();
            return Ok(());
        }
        // not empty, not force
        if recv || (!send && !recv) {
            return Err(not_empty_err(vm));
        }
        // send only
        inner.send_closed = true;
        ch.cond.notify_all();
        Ok(())
    }

    #[pyfunction]
    fn release(args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let cid_obj = args
            .args
            .first()
            .ok_or_else(|| vm.new_type_error("release() missing argument 1"))?;
        let (cid, _) = parse_cid(cid_obj, vm)?;
        let mut send = args
            .kwargs
            .get("send")
            .map(|o| o.clone().is_true(vm))
            .transpose()?
            .unwrap_or(false);
        let mut recv = args
            .kwargs
            .get("recv")
            .map(|o| o.clone().is_true(vm))
            .transpose()?
            .unwrap_or(false);
        if !send && !recv {
            send = true;
            recv = true;
        }
        let ch = lookup(cid).ok_or_else(|| not_found(vm, cid))?;
        let mut inner = ch.inner.lock();
        if inner.destroyed {
            return Err(not_found(vm, cid));
        }
        if inner.fully_closed() {
            return Err(closed_err(vm, cid));
        }
        let interpid = vm.state.interpreter_id;
        if send {
            inner.send_assoc.remove(&interpid);
        }
        if recv {
            inner.recv_assoc.remove(&interpid);
        }
        if inner.send_assoc.is_empty() && inner.recv_assoc.is_empty() {
            inner.send_closed = true;
            inner.recv_closed = true;
            for item in inner.items.drain(..) {
                if let Some(w) = item.waiter {
                    *w.closed.lock() = true;
                    w.cond.notify_all();
                }
            }
            ch.cond.notify_all();
        }
        Ok(())
    }

    #[pyfunction]
    fn get_count(cid: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        let (cid, _) = parse_cid(&cid, vm)?;
        let ch = lookup(cid).ok_or_else(|| not_found(vm, cid))?;
        Ok(ch.inner.lock().items.len())
    }

    #[pyfunction]
    fn get_channel_defaults(cid: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let (cid, _) = parse_cid(&cid, vm)?;
        let ch = lookup(cid).ok_or_else(|| not_found(vm, cid))?;
        let inner = ch.inner.lock();
        Ok(vm
            .ctx
            .new_tuple(vec![
                vm.ctx.new_int(inner.unboundop).into(),
                vm.ctx.new_int(inner.fallback).into(),
            ])
            .into())
    }

    #[pyfunction]
    fn _channel_id(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Ok(channel_id_new(args, vm)?.to_pyobject(vm))
    }

    #[pyfunction]
    fn _register_end_types(_send: PyObjectRef, _recv: PyObjectRef) {}
}
