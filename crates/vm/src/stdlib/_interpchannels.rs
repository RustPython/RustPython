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
        builtins::{PyBaseExceptionRef, PyInt, PyMemoryView, PyModule, PyType},
        convert::ToPyObject,
        function::{ArgSpec, Either, FuncArgs, PyComparisonValue},
        protocol::{PyNumber, PyNumberMethods},
        types::{AsNumber, Constructor, Hashable, PyComparisonOp, PyStructSequence, Representable},
        vm::crossinterp::{self, Fallback, SharedValue, UNBOUND_REPLACE},
    };
    use alloc::collections::BTreeMap;
    use alloc::{collections::VecDeque, sync::Arc};
    use core::time::Duration;
    use num_traits::ToPrimitive;
    use parking_lot::{Condvar, Mutex};
    use std::{sync::OnceLock, time::Instant};

    const CHANNEL_SEND: i32 = 1;
    const CHANNEL_BOTH: i32 = 0;
    const CHANNEL_RECV: i32 = -1;

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

    /// The `ERR_CHANNEL_*` codes `handle_channel_error` maps to exceptions.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum ChanErr {
        NotFound,
        Closed,
        ClosedWaiting,
        InterpClosed,
        Empty,
        NotEmpty,
    }

    impl ChanErr {
        fn into_py(self, cid: i64, vm: &VirtualMachine) -> PyBaseExceptionRef {
            let (class, msg) = match self {
                Self::NotFound => (
                    PyChannelNotFoundError::class(&vm.ctx),
                    format!("channel {cid} not found"),
                ),
                Self::Closed => (
                    PyChannelClosedError::class(&vm.ctx),
                    format!("channel {cid} is closed"),
                ),
                Self::ClosedWaiting => (
                    PyChannelClosedError::class(&vm.ctx),
                    format!("channel {cid} has closed"),
                ),
                Self::InterpClosed => (
                    PyChannelClosedError::class(&vm.ctx),
                    format!("channel {cid} is already closed"),
                ),
                Self::Empty => (
                    PyChannelEmptyError::class(&vm.ctx),
                    format!("channel {cid} is empty"),
                ),
                Self::NotEmpty => (
                    PyChannelNotEmptyError::class(&vm.ctx),
                    format!("channel {cid} may not be closed if not empty (try force=True)"),
                ),
            };
            vm.new_exception_msg(class.to_owned(), msg.into())
        }
    }

    type ChanResult<T> = Result<T, ChanErr>;

    /// A blocked `send()`; the receiver hands back whether the item was taken.
    struct Waiting {
        state: Mutex<WaitState>,
        cond: Condvar,
    }

    #[derive(Default)]
    struct WaitState {
        released: bool,
        received: bool,
    }

    impl Waiting {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                state: Mutex::new(WaitState::default()),
                cond: Condvar::new(),
            })
        }

        fn release(&self, received: bool) {
            let mut state = self.state.lock();
            if !state.released {
                state.released = true;
                state.received = received;
            }
            self.cond.notify_all();
        }
    }

    /// Release waiters with the interpreter detached: the waiting thread holds
    /// this mutex across its wait, so an attached taker could never be stopped.
    fn release_waiters(waiters: Vec<(Arc<Waiting>, bool)>, vm: &VirtualMachine) {
        if waiters.is_empty() {
            return;
        }
        vm.allow_threads(|| {
            for (w, received) in waiters {
                w.release(received);
            }
        });
    }

    fn release_waiters_detached(waiters: Vec<(Arc<Waiting>, bool)>) {
        for (w, received) in waiters {
            w.release(received);
        }
    }

    struct ChannelItem {
        /// The interpreter that added the item to the queue.
        interpid: i64,
        /// `None` once the owning interpreter is gone (the item is "unbound").
        data: Option<SharedValue>,
        unboundop: i32,
        waiting: Option<Arc<Waiting>>,
    }

    /// The interpreters bound to each end, in association order.
    #[derive(Default)]
    struct ChannelEnds {
        send: Vec<(i64, bool)>,
        recv: Vec<(i64, bool)>,
        numsendopen: isize,
        numrecvopen: isize,
    }

    impl ChannelEnds {
        fn list(&mut self, send: bool) -> &mut Vec<(i64, bool)> {
            if send { &mut self.send } else { &mut self.recv }
        }

        fn find(&self, interpid: i64, send: bool) -> Option<usize> {
            let list = if send { &self.send } else { &self.recv };
            list.iter().position(|&(id, _)| id == interpid)
        }

        fn add(&mut self, interpid: i64, send: bool) {
            self.list(send).push((interpid, true));
            if send {
                self.numsendopen += 1;
            } else {
                self.numrecvopen += 1;
            }
        }

        fn associate(&mut self, interpid: i64, send: bool) -> ChanResult<()> {
            match self.find(interpid, send) {
                Some(i) => {
                    if self.list(send)[i].1 {
                        Ok(())
                    } else {
                        Err(ChanErr::Closed)
                    }
                }
                None => {
                    self.add(interpid, send);
                    Ok(())
                }
            }
        }

        fn is_open(&self) -> bool {
            if self.numsendopen != 0 || self.numrecvopen != 0 {
                return true;
            }
            // The channel has never had any interpreters associated with it.
            self.send.is_empty() && self.recv.is_empty()
        }

        fn release_end(&mut self, index: usize, send: bool) {
            if !self.list(send)[index].1 {
                return;
            }
            self.list(send)[index].1 = false;
            if send {
                self.numsendopen -= 1;
            } else {
                self.numrecvopen -= 1;
            }
        }

        /// `which >= 0` releases the send end, `which <= 0` the recv end.
        fn release_interpreter(&mut self, interpid: i64, which: i32) {
            for (send, apply) in [(true, which >= 0), (false, which <= 0)] {
                if !apply {
                    continue;
                }
                let index = match self.find(interpid, send) {
                    Some(i) => i,
                    None => {
                        // Never associated, so add it first.
                        self.add(interpid, send);
                        self.list(send).len() - 1
                    }
                };
                self.release_end(index, send);
            }
        }

        fn release_all(&mut self) {
            for send in [true, false] {
                for i in 0..self.list(send).len() {
                    self.release_end(i, send);
                }
            }
        }

        fn clear_interpreter(&mut self, interpid: i64) {
            for send in [true, false] {
                if let Some(i) = self.find(interpid, send) {
                    self.release_end(i, send);
                }
            }
        }
    }

    struct ChannelState {
        queue: VecDeque<ChannelItem>,
        ends: ChannelEnds,
        unboundop: i32,
        fallback: i32,
        open: bool,
        /// Send is closed and the queue is draining.
        closing: bool,
    }

    struct Channel {
        state: Mutex<ChannelState>,
    }

    /// `_channelref`: the channel plus the number of live `ChannelID` objects.
    struct ChannelRef {
        chan: Option<Arc<Channel>>,
        objcount: isize,
    }

    struct Channels {
        next_id: i64,
        refs: BTreeMap<i64, ChannelRef>,
    }

    fn channels() -> &'static Mutex<Channels> {
        static CHANNELS: OnceLock<Mutex<Channels>> = OnceLock::new();
        CHANNELS.get_or_init(|| {
            Mutex::new(Channels {
                next_id: 0,
                refs: BTreeMap::new(),
            })
        })
    }

    /// `_channels_lookup`: the channel, or why it is unusable.
    fn channels_lookup(cid: i64) -> ChanResult<Arc<Channel>> {
        let table = channels().lock();
        let entry = table.refs.get(&cid).ok_or(ChanErr::NotFound)?;
        let chan = entry.chan.as_ref().ok_or(ChanErr::Closed)?;
        if !chan.state.lock().open {
            return Err(ChanErr::Closed);
        }
        Ok(chan.clone())
    }

    fn channel_create(unboundop: i32, fallback: i32) -> i64 {
        let mut table = channels().lock();
        let cid = table.next_id;
        table.next_id += 1;
        table.refs.insert(
            cid,
            ChannelRef {
                chan: Some(Arc::new(Channel {
                    state: Mutex::new(ChannelState {
                        queue: VecDeque::new(),
                        ends: ChannelEnds::default(),
                        unboundop,
                        fallback,
                        open: true,
                        closing: false,
                    }),
                })),
                objcount: 0,
            },
        );
        cid
    }

    /// `channel_destroy`: forget the channel entirely.
    fn channel_destroy(cid: i64) -> ChanResult<Vec<(Arc<Waiting>, bool)>> {
        let mut table = channels().lock();
        let entry = table.refs.remove(&cid).ok_or(ChanErr::NotFound)?;
        Ok(match entry.chan {
            Some(chan) => drain_queue(&mut chan.state.lock()),
            None => Vec::new(),
        })
    }

    fn drain_queue(state: &mut ChannelState) -> Vec<(Arc<Waiting>, bool)> {
        state
            .queue
            .drain(..)
            .filter_map(|item| item.waiting.map(|w| (w, false)))
            .collect()
    }

    /// `_channels_release_cid_object`: drop a `ChannelID` reference.
    fn release_cid_object(cid: i64) -> Vec<(Arc<Waiting>, bool)> {
        let mut table = channels().lock();
        let Some(entry) = table.refs.get_mut(&cid) else {
            // Already destroyed.
            return Vec::new();
        };
        entry.objcount -= 1;
        if entry.objcount != 0 {
            return Vec::new();
        }
        let entry = table.refs.remove(&cid).expect("just looked it up");
        match entry.chan {
            Some(chan) => drain_queue(&mut chan.state.lock()),
            None => Vec::new(),
        }
    }

    /// `_channels_add_id_object`.
    fn add_cid_object(cid: i64) -> ChanResult<()> {
        let mut table = channels().lock();
        let entry = table.refs.get_mut(&cid).ok_or(ChanErr::NotFound)?;
        entry.objcount += 1;
        Ok(())
    }

    /// `_channel_finish_closing`: a "closing" channel that just went empty is
    /// gone for good.
    fn finish_closing(cid: i64) {
        let mut table = channels().lock();
        let Some(entry) = table.refs.get_mut(&cid) else {
            return;
        };
        let Some(chan) = entry.chan.as_ref() else {
            return;
        };
        let done = {
            let mut state = chan.state.lock();
            if !state.closing || !state.queue.is_empty() {
                false
            } else {
                state.closing = false;
                state.open = false;
                true
            }
        };
        if done {
            entry.chan = None;
        }
    }

    /// `_channels_close`.
    fn channel_close(cid: i64, end: i32, force: bool) -> ChanResult<Vec<(Arc<Waiting>, bool)>> {
        let mut table = channels().lock();
        let entry = table.refs.get_mut(&cid).ok_or(ChanErr::NotFound)?;
        let chan = entry.chan.as_ref().ok_or(ChanErr::Closed)?.clone();
        let mut state = chan.state.lock();
        if !force && end == CHANNEL_SEND && state.closing {
            return Err(ChanErr::Closed);
        }
        if !state.open {
            return Err(ChanErr::Closed);
        }
        if !force && !state.queue.is_empty() {
            if end != CHANNEL_SEND {
                return Err(ChanErr::NotEmpty);
            }
            if state.closing {
                return Err(ChanErr::Closed);
            }
            // Mark the channel as closing; it is cleaned up once drained.
            state.closing = true;
            return Ok(Vec::new());
        }
        let waiters = drain_queue(&mut state);
        state.open = false;
        state.ends.release_all();
        drop(state);
        entry.chan = None;
        Ok(waiters)
    }

    /// `channel_release`: close one or both ends for the current interpreter.
    fn channel_release(cid: i64, interpid: i64, send: bool, recv: bool) -> ChanResult<()> {
        let chan = channels_lookup(cid)?;
        let mut state = chan.state.lock();
        if !state.open {
            return Err(ChanErr::Closed);
        }
        let which = i32::from(send) - i32::from(recv);
        state.ends.release_interpreter(interpid, which);
        state.open = state.ends.is_open();
        Ok(())
    }

    #[cfg_attr(not(feature = "threading"), allow(dead_code))]
    pub(crate) fn clear_interpreter(interpid: i64) {
        let chans: Vec<Arc<Channel>> = channels()
            .lock()
            .refs
            .values()
            .filter_map(|r| r.chan.clone())
            .collect();
        let mut waiters = Vec::new();
        for chan in chans {
            let mut state = chan.state.lock();
            let mut i = 0;
            while i < state.queue.len() {
                let item = &mut state.queue[i];
                if item.interpid != interpid || item.data.is_none() {
                    i += 1;
                    continue;
                }
                if item.unboundop == crossinterp::UNBOUND_REMOVE {
                    let item = state.queue.remove(i).expect("index checked");
                    if let Some(w) = item.waiting {
                        waiters.push((w, false));
                    }
                    continue;
                }
                // UNBOUND_ERROR / UNBOUND_REPLACE keep the slot but throw the
                // data away; the item is now "unbound".
                item.data = None;
                i += 1;
            }
            state.ends.clear_interpreter(interpid);
            state.open = state.ends.is_open();
        }
        release_waiters_detached(waiters);
    }

    /// `resolve_unboundop`.
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

    /// `resolve_fallback`.
    fn resolve_fallback(arg: i32, default: i32, vm: &VirtualMachine) -> PyResult<Fallback> {
        let value = if arg < 0 { default } else { arg };
        Fallback::from_i32(value)
            .ok_or_else(|| vm.new_value_error(format!("unsupported fallback {arg}")))
    }

    /// `channel_id_converter`.
    fn parse_cid(obj: &PyObject, vm: &VirtualMachine) -> PyResult<(i64, i32)> {
        if let Some(cid) = obj.downcast_ref::<ChannelID>() {
            return Ok((cid.cid, cid.end));
        }
        if !obj.number().is_index() {
            return Err(vm.new_type_error(format!(
                "channel ID must be an int, got {}",
                obj.class().name()
            )));
        }
        let n = obj.try_index(vm)?;
        let id = n
            .as_bigint()
            .to_i64()
            .ok_or_else(|| vm.new_overflow_error("int too big to convert"))?;
        if id < 0 {
            let repr = obj.repr(vm)?;
            return Err(
                vm.new_value_error(format!("channel ID must be a non-negative int, got {repr}"))
            );
        }
        Ok((id, CHANNEL_BOTH))
    }

    /// The single-`cid` signature shared by several module functions.
    fn cid_arg(args: &FuncArgs, func: &'static str, vm: &VirtualMachine) -> PyResult<i64> {
        let parsed = ArgSpec {
            fname: func,
            keywords: &["cid"],
            required: 1,
            max_positional: 1,
        }
        .parse(args, vm)?;
        Ok(parse_cid(parsed[0].as_deref().unwrap(), vm)?.0)
    }

    /// The `(cid, *, send=False, recv=False, force=False)` signature shared by
    /// `channel_close` and `channel_release`.
    fn end_args(
        args: &FuncArgs,
        func: &'static str,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<Option<PyObjectRef>>> {
        ArgSpec {
            fname: func,
            keywords: &["cid", "send", "recv", "force"],
            required: 1,
            max_positional: 1,
        }
        .parse_with(
            args,
            |i, obj, vm| match i {
                0 => parse_cid(obj, vm).map(drop),
                _ => Ok(()),
            },
            vm,
        )
    }

    /// The `p` converter: a predicate that only reads truthiness.
    fn flag(slot: Option<&PyObjectRef>, vm: &VirtualMachine) -> PyResult<bool> {
        match slot {
            Some(o) => o.clone().is_true(vm),
            None => Ok(false),
        }
    }

    /// The `i` converter.
    fn int_arg(slot: Option<&PyObject>, vm: &VirtualMachine) -> PyResult<Option<i32>> {
        slot.map(|o| {
            let ival = o.try_index(vm)?.as_bigint().to_i64().ok_or_else(|| {
                vm.new_overflow_error("Python int too large to convert to C long")
            })?;
            i32::try_from(ival).map_err(|_| {
                let msg = if ival > i32::MAX as i64 {
                    "signed integer is greater than maximum"
                } else {
                    "signed integer is less than minimum"
                };
                vm.new_overflow_error(msg)
            })
        })
        .transpose()
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
            release_waiters_detached(release_cid_object(self.cid));
        }
    }

    impl Representable for ChannelID {
        #[inline]
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            let name = zelf.class().name().to_string();
            Ok(match zelf.end {
                CHANNEL_SEND => format!("{name}({}, send=True)", zelf.cid),
                CHANNEL_RECV => format!("{name}({}, recv=True)", zelf.cid),
                _ => format!("{name}({})", zelf.cid),
            })
        }
    }

    impl Hashable for ChannelID {
        #[inline]
        fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<rustpython_common::hash::PyHash> {
            vm.ctx.new_int(zelf.cid).as_object().hash(vm)
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
        with(Representable, Hashable, AsNumber),
        flags(BASETYPE, IMMUTABLETYPE, DISALLOW_INSTANTIATION)
    )]
    impl ChannelID {
        /// `channelid_richcompare`.
        #[pyslot]
        fn slot_richcompare(
            zelf: &PyObject,
            other: &PyObject,
            op: PyComparisonOp,
            vm: &VirtualMachine,
        ) -> PyResult<Either<PyObjectRef, PyComparisonValue>> {
            if !matches!(op, PyComparisonOp::Eq | PyComparisonOp::Ne) {
                return Ok(Either::B(PyComparisonValue::NotImplemented));
            }
            let Some(zelf) = zelf.downcast_ref::<Self>() else {
                return Ok(Either::B(PyComparisonValue::NotImplemented));
            };
            let equal = if let Some(o) = other.downcast_ref::<Self>() {
                zelf.end == o.end && zelf.cid == o.cid
            } else if let Some(n) = other.downcast_ref::<PyInt>() {
                // Fast path
                n.try_to_primitive::<i64>(vm)
                    .is_ok_and(|v| v >= 0 && v == zelf.cid)
            } else if PyNumber::check(other) {
                let id_obj: PyObjectRef = vm.ctx.new_int(zelf.cid).into();
                return id_obj.rich_compare(other.to_owned(), op, vm).map(Either::A);
            } else {
                return Ok(Either::B(PyComparisonValue::NotImplemented));
            };
            Ok(Either::B(PyComparisonValue::Implemented(
                (op == PyComparisonOp::Eq) == equal,
            )))
        }

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

    /// `newchannelid`.
    pub(crate) fn channel_id_from_parts(
        cid: i64,
        end: i32,
        force: bool,
        resolve: bool,
        vm: &VirtualMachine,
    ) -> PyResult {
        match add_cid_object(cid) {
            Ok(()) => {}
            Err(ChanErr::NotFound) if force => {}
            Err(e) => return Err(e.into_py(cid, vm)),
        }
        Ok(ChannelID { cid, end, resolve }.to_pyobject(vm))
    }

    pub(crate) fn channel_id_parts(obj: &PyObject) -> Option<(i64, i32)> {
        obj.downcast_ref::<ChannelID>().map(|c| (c.cid, c.end))
    }

    /// `_channelid_new`.
    fn channel_id_new(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let parsed = ArgSpec {
            fname: "ChannelID.__new__",
            keywords: &["id", "send", "recv", "force", "_resolve"],
            required: 1,
            max_positional: 1,
        }
        .parse_with(
            &args,
            |i, obj, vm| match i {
                0 => parse_cid(obj, vm).map(drop),
                _ => Ok(()),
            },
            vm,
        )?;
        let (cid, mut end) = parse_cid(parsed[0].as_deref().unwrap(), vm)?;
        let tri = |slot: Option<&PyObjectRef>| -> PyResult<Option<bool>> {
            slot.map(|o| o.clone().is_true(vm)).transpose()
        };
        let send = tri(parsed[1].as_ref())?;
        let recv = tri(parsed[2].as_ref())?;
        let force = flag(parsed[3].as_ref(), vm)?;
        let resolve = flag(parsed[4].as_ref(), vm)?;
        match (send, recv) {
            (Some(false), Some(false)) => {
                return Err(vm.new_value_error("'send' and 'recv' cannot both be False"));
            }
            (Some(true), Some(true)) => end = CHANNEL_BOTH,
            (Some(true), _) => end = CHANNEL_SEND,
            (_, Some(true)) => end = CHANNEL_RECV,
            _ => {}
        }
        channel_id_from_parts(cid, end, force, resolve, vm)
    }

    #[pystruct_sequence_data]
    struct ChannelInfoData {
        open: bool,
        closing: bool,
        closed: bool,
        count: i64,
        num_interp_send: isize,
        num_interp_send_released: isize,
        num_interp_recv: isize,
        num_interp_recv_released: isize,
        #[pystruct_sequence(skip)]
        num_interp_both: isize,
        #[pystruct_sequence(skip)]
        num_interp_both_released: isize,
        #[pystruct_sequence(skip)]
        num_interp_both_send_released: isize,
        #[pystruct_sequence(skip)]
        num_interp_both_recv_released: isize,
        #[pystruct_sequence(skip)]
        send_associated: bool,
        #[pystruct_sequence(skip)]
        send_released: bool,
        #[pystruct_sequence(skip)]
        recv_associated: bool,
        #[pystruct_sequence(skip)]
        recv_released: bool,
    }

    #[pyattr]
    #[pystruct_sequence(
        name = "ChannelInfo",
        module = "_interpchannels",
        data = "ChannelInfoData"
    )]
    struct PyChannelInfo;

    #[pyclass(with(PyStructSequence))]
    impl PyChannelInfo {}

    #[pyfunction]
    fn create(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let parsed = ArgSpec {
            fname: "create",
            keywords: &["unboundop", "fallback"],
            required: 0,
            max_positional: 2,
        }
        .parse(&args, vm)?;
        let unboundarg = int_arg(parsed[0].as_deref(), vm)?.unwrap_or(-1);
        let fallbackarg = int_arg(parsed[1].as_deref(), vm)?.unwrap_or(-1);
        let unboundop = resolve_unboundop(unboundarg, UNBOUND_REPLACE, vm)?;
        let fallback = resolve_fallback(fallbackarg, Fallback::Full.as_i32(), vm)?;
        let cid = channel_create(unboundop, fallback.as_i32());
        channel_id_from_parts(cid, CHANNEL_BOTH, false, false, vm)
    }

    #[pyfunction]
    fn destroy(args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let cid = cid_arg(&args, "channel_destroy", vm)?;
        let waiters = channel_destroy(cid).map_err(|e| e.into_py(cid, vm))?;
        release_waiters(waiters, vm);
        Ok(())
    }

    #[pyfunction]
    fn list_all(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        if !args.args.is_empty() || !args.kwargs.is_empty() {
            return Err(vm.new_type_error(format!(
                "_interpchannels.list_all() takes no arguments ({} given)",
                args.args.len() + args.kwargs.len()
            )));
        }
        let chans: Vec<(i64, i32, i32)> = channels()
            .lock()
            .refs
            .iter()
            .filter_map(|(&cid, r)| {
                let state = r.chan.as_ref()?.state.lock();
                Some((cid, state.unboundop, state.fallback))
            })
            .collect();
        let mut items = Vec::with_capacity(chans.len());
        for (cid, unboundop, fallback) in chans {
            items.push(
                vm.ctx
                    .new_tuple(vec![
                        channel_id_from_parts(cid, CHANNEL_BOTH, false, false, vm)?,
                        vm.ctx.new_int(unboundop).into(),
                        vm.ctx.new_int(fallback).into(),
                    ])
                    .into(),
            );
        }
        Ok(vm.ctx.new_list(items).into())
    }

    #[pyfunction]
    fn list_interpreters(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let parsed = ArgSpec {
            fname: "channel_list_interpreters",
            keywords: &["cid", "send"],
            required: 2,
            max_positional: 1,
        }
        .parse_with(
            &args,
            |i, obj, vm| match i {
                0 => parse_cid(obj, vm).map(drop),
                _ => Ok(()),
            },
            vm,
        )?;
        let cid = parse_cid(parsed[0].as_deref().unwrap(), vm)?.0;
        let send = flag(parsed[1].as_ref(), vm)?;
        let chan = channels_lookup(cid).map_err(|e| e.into_py(cid, vm))?;
        let state = chan.state.lock();
        if send && state.closing {
            return Err(ChanErr::Closed.into_py(cid, vm));
        }
        let mut out = Vec::new();
        for info in crate::vm::runtime::list_interpreters() {
            if state.ends.find(info.id, send).is_some_and(|i| {
                if send {
                    state.ends.send[i].1
                } else {
                    state.ends.recv[i].1
                }
            }) {
                out.push(vm.ctx.new_int(info.id).into());
            }
        }
        Ok(vm.ctx.new_list(out).into())
    }

    /// `channel_send` up to its `closing` check, which precedes converting the
    /// object to cross-interpreter data.
    fn send_begin(cid: i64, vm: &VirtualMachine) -> PyResult<Arc<Channel>> {
        let chan = channels_lookup(cid).map_err(|e| e.into_py(cid, vm))?;
        if chan.state.lock().closing {
            return Err(ChanErr::Closed.into_py(cid, vm));
        }
        Ok(chan)
    }

    #[expect(clippy::unnecessary_wraps, reason = "Needs to comply with a signature")]
    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        crate::stdlib::_interpreters::init_xi_types(vm);
        __module_exec(vm, module);
        Ok(())
    }

    /// `_channel_add` plus, when `waiting` is set, `channel_send_wait`.
    fn do_send(
        chan: &Channel,
        cid: i64,
        value: SharedValue,
        unboundop: i32,
        blocking: bool,
        timeout: Option<Duration>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let waiting = blocking.then(Waiting::new);
        {
            let mut state = chan.state.lock();
            if !state.open {
                return Err(ChanErr::Closed.into_py(cid, vm));
            }
            state
                .ends
                .associate(vm.state.interpreter_id, true)
                .map_err(|_| ChanErr::InterpClosed.into_py(cid, vm))?;
            state.queue.push_back(ChannelItem {
                interpid: vm.state.interpreter_id,
                data: Some(value),
                unboundop,
                waiting: waiting.clone(),
            });
        }
        let Some(waiting) = waiting else {
            return Ok(());
        };

        let deadline = timeout.map(|t| Instant::now() + t);
        let timed_out = vm.allow_threads(|| {
            let mut state = waiting.state.lock();
            loop {
                if state.released {
                    return false;
                }
                let Some(deadline) = deadline else {
                    waiting.cond.wait(&mut state);
                    continue;
                };
                let now = Instant::now();
                if now >= deadline || waiting.cond.wait_until(&mut state, deadline).timed_out() {
                    return !state.released;
                }
            }
        });

        if timed_out {
            // The send is failing now, so make sure the item won't be received.
            let mut state = chan.state.lock();
            if let Some(pos) = state.queue.iter().position(|it| {
                it.waiting
                    .as_ref()
                    .is_some_and(|w| Arc::ptr_eq(w, &waiting))
            }) {
                state.queue.remove(pos);
            }
            drop(state);
            finish_closing(cid);
            if !waiting.state.lock().received {
                return Err(vm
                    .new_os_subtype_error(
                        vm.ctx.exceptions.timeout_error.to_owned(),
                        None,
                        "timed out",
                    )
                    .upcast());
            }
            return Ok(());
        }
        if !waiting.state.lock().received {
            return Err(ChanErr::ClosedWaiting.into_py(cid, vm));
        }
        Ok(())
    }

    fn send_args(
        args: &FuncArgs,
        func: &'static str,
        vm: &VirtualMachine,
    ) -> PyResult<(i64, PyObjectRef, i32, Fallback, bool, Option<Duration>)> {
        let parsed = ArgSpec {
            fname: func,
            keywords: &["cid", "obj", "unboundop", "fallback", "blocking", "timeout"],
            required: 2,
            max_positional: 4,
        }
        .parse_with(
            args,
            |i, obj, vm| match i {
                0 => parse_cid(obj, vm).map(drop),
                2 | 3 => int_arg(Some(obj), vm).map(drop),
                _ => Ok(()),
            },
            vm,
        )?;
        let cid = parse_cid(parsed[0].as_deref().unwrap(), vm)?.0;
        let obj = parsed[1].clone().unwrap();
        let unboundarg = int_arg(parsed[2].as_deref(), vm)?.unwrap_or(-1);
        let fallbackarg = int_arg(parsed[3].as_deref(), vm)?.unwrap_or(-1);
        let blocking = match &parsed[4] {
            Some(o) => o.clone().is_true(vm)?,
            None => true,
        };
        let timeout = parse_timeout(parsed[5].as_deref(), blocking, vm)?;
        // The channel is only consulted when one of the arguments needs its default.
        let (default_unboundop, default_fallback) = if unboundarg < 0 || fallbackarg < 0 {
            let chan = channels_lookup(cid).map_err(|e| e.into_py(cid, vm))?;
            let state = chan.state.lock();
            (state.unboundop, state.fallback)
        } else {
            (-1, -1)
        };
        let unboundop = resolve_unboundop(unboundarg, default_unboundop, vm)?;
        let fallback = resolve_fallback(fallbackarg, default_fallback, vm)?;
        Ok((cid, obj, unboundop, fallback, blocking, timeout))
    }

    #[pyfunction]
    fn send(args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let (cid, obj, unboundop, fallback, blocking, timeout) =
            send_args(&args, "channel_send", vm)?;
        let chan = send_begin(cid, vm)?;
        let value = SharedValue::from_object(&obj, fallback, vm)?;
        do_send(&chan, cid, value, unboundop, blocking, timeout, vm)
    }

    #[pyfunction]
    fn send_buffer(args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let (cid, obj, unboundop, fallback, blocking, timeout) =
            send_args(&args, "channel_send_buffer", vm)?;
        // The buffer is wrapped in a memoryview, and that is what gets shared.
        let view = PyMemoryView::from_object(&obj, vm)?.into_pyobject(vm);
        let chan = send_begin(cid, vm)?;
        let value = SharedValue::from_object(&view, fallback, vm)?;
        do_send(&chan, cid, value, unboundop, blocking, timeout, vm)
    }

    /// `PyThread_ParseTimeoutArg`.
    fn parse_timeout(
        timeout: Option<&PyObject>,
        blocking: bool,
        vm: &VirtualMachine,
    ) -> PyResult<Option<Duration>> {
        let Some(t) = timeout.filter(|t| !vm.is_none(t)) else {
            return Ok(None);
        };
        if !blocking {
            return Err(vm.new_value_error("can't specify a timeout for a non-blocking call"));
        }
        let secs = t.try_float(vm)?.to_f64();
        if secs < 0.0 {
            return Err(vm.new_value_error("timeout value must be a non-negative number"));
        }
        Ok(Some(Duration::from_secs_f64(secs)))
    }

    #[pyfunction]
    fn recv(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let parsed = ArgSpec {
            fname: "channel_recv",
            keywords: &["cid", "default"],
            required: 1,
            max_positional: 2,
        }
        .parse(&args, vm)?;
        let cid = parse_cid(parsed[0].as_deref().unwrap(), vm)?.0;
        let default = parsed[1].as_deref();
        let popped = channel_next(cid, vm.state.interpreter_id);
        finish_closing(cid);
        let item = match popped {
            Ok(item) => item,
            Err(ChanErr::Empty) => match default {
                Some(d) => return Ok(vm.ctx.new_tuple(vec![d.to_owned(), vm.ctx.none()]).into()),
                None => return Err(ChanErr::Empty.into_py(cid, vm)),
            },
            Err(e) => return Err(e.into_py(cid, vm)),
        };
        let (data, unboundop, waiting) = item;
        let Some(data) = data else {
            // The item was unbound.
            return Ok(vm
                .ctx
                .new_tuple(vec![vm.ctx.none(), vm.ctx.new_int(unboundop).into()])
                .into());
        };
        let obj = data.into_object(vm)?;
        if let Some(w) = waiting {
            release_waiters(vec![(w, true)], vm);
        }
        Ok(vm.ctx.new_tuple(vec![obj, vm.ctx.none()]).into())
    }

    /// `_channel_next`.
    #[allow(clippy::type_complexity)]
    fn channel_next(
        cid: i64,
        interpid: i64,
    ) -> ChanResult<(Option<SharedValue>, i32, Option<Arc<Waiting>>)> {
        let chan = channels_lookup(cid)?;
        let mut state = chan.state.lock();
        if !state.open {
            return Err(ChanErr::Closed);
        }
        state
            .ends
            .associate(interpid, false)
            .map_err(|_| ChanErr::InterpClosed)?;
        match state.queue.pop_front() {
            Some(item) => Ok((item.data, item.unboundop, item.waiting)),
            None => {
                if state.closing {
                    state.open = false;
                }
                Err(ChanErr::Empty)
            }
        }
    }

    #[pyfunction]
    fn close(args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let parsed = end_args(&args, "channel_close", vm)?;
        let cid = parse_cid(parsed[0].as_deref().unwrap(), vm)?.0;
        let send = flag(parsed[1].as_ref(), vm)?;
        let recv = flag(parsed[2].as_ref(), vm)?;
        let force = flag(parsed[3].as_ref(), vm)?;
        let end = i32::from(send) - i32::from(recv);
        let waiters = channel_close(cid, end, force).map_err(|e| e.into_py(cid, vm))?;
        release_waiters(waiters, vm);
        Ok(())
    }

    #[pyfunction]
    fn release(args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let parsed = end_args(&args, "channel_release", vm)?;
        let cid = parse_cid(parsed[0].as_deref().unwrap(), vm)?.0;
        let mut send = flag(parsed[1].as_ref(), vm)?;
        let mut recv = flag(parsed[2].as_ref(), vm)?;
        if !send && !recv {
            send = true;
            recv = true;
        }
        channel_release(cid, vm.state.interpreter_id, send, recv).map_err(|e| e.into_py(cid, vm))
    }

    #[pyfunction]
    fn get_count(args: FuncArgs, vm: &VirtualMachine) -> PyResult<usize> {
        let cid = cid_arg(&args, "get_count", vm)?;
        let chan = channels_lookup(cid).map_err(|e| e.into_py(cid, vm))?;
        let count = chan.state.lock().queue.len();
        Ok(count)
    }

    #[pyfunction]
    fn get_info(args: FuncArgs, vm: &VirtualMachine) -> PyResult<ChannelInfoData> {
        let cid = cid_arg(&args, "_get_info", vm)?;
        let interpid = vm.state.interpreter_id;
        let table = channels().lock();
        let entry = table
            .refs
            .get(&cid)
            .ok_or_else(|| ChanErr::NotFound.into_py(cid, vm))?;
        let mut info = ChannelInfoData {
            open: false,
            closing: false,
            closed: true,
            count: 0,
            num_interp_send: 0,
            num_interp_send_released: 0,
            num_interp_recv: 0,
            num_interp_recv_released: 0,
            num_interp_both: 0,
            num_interp_both_released: 0,
            num_interp_both_send_released: 0,
            num_interp_both_recv_released: 0,
            send_associated: false,
            send_released: false,
            recv_associated: false,
            recv_released: false,
        };
        let Some(chan) = entry.chan.as_ref() else {
            return Ok(info);
        };
        let state = chan.state.lock();
        if !state.open {
            return Ok(info);
        }
        info.closed = false;
        info.closing = state.closing;
        info.open = !state.closing;
        info.count = state.queue.len() as i64;

        for &(id, open) in &state.ends.send {
            if id == interpid {
                info.send_associated = open;
                info.send_released = !open;
            }
            if open {
                info.num_interp_send += 1;
            } else {
                info.num_interp_send_released += 1;
            }
        }
        for &(id, recv_open) in &state.ends.recv {
            if id == interpid {
                info.recv_associated = recv_open;
                info.recv_released = !recv_open;
            }
            match state.ends.find(id, true).map(|i| state.ends.send[i].1) {
                None => {
                    if recv_open {
                        info.num_interp_recv += 1;
                    } else {
                        info.num_interp_recv_released += 1;
                    }
                }
                Some(send_open) => match (recv_open, send_open) {
                    (true, true) => {
                        info.num_interp_both += 1;
                        info.num_interp_send -= 1;
                    }
                    (true, false) => {
                        info.num_interp_both_recv_released += 1;
                        info.num_interp_send_released -= 1;
                    }
                    (false, true) => {
                        info.num_interp_both_send_released += 1;
                        info.num_interp_send -= 1;
                    }
                    (false, false) => {
                        info.num_interp_both_released += 1;
                        info.num_interp_send_released -= 1;
                    }
                },
            }
        }
        Ok(info)
    }

    #[pyfunction]
    fn get_channel_defaults(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let cid = cid_arg(&args, "get_channel_defaults", vm)?;
        let chan = channels_lookup(cid).map_err(|e| e.into_py(cid, vm))?;
        let (unboundop, fallback) = {
            let state = chan.state.lock();
            (state.unboundop, state.fallback)
        };
        Ok(vm
            .ctx
            .new_tuple(vec![
                vm.ctx.new_int(unboundop).into(),
                vm.ctx.new_int(fallback).into(),
            ])
            .into())
    }

    #[pyfunction]
    fn _channel_id(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        channel_id_new(args, vm)
    }

    #[pyfunction]
    fn _register_end_types(args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let parsed = ArgSpec {
            fname: "_register_end_types",
            keywords: &["send", "recv"],
            required: 2,
            max_positional: 2,
        }
        .parse(&args, vm)?;
        for (slot, name) in parsed.iter().zip(["send", "recv"]) {
            if !slot.as_ref().unwrap().downcastable::<PyType>() {
                return Err(vm.new_type_error(format!("expected a type for '{name}'")));
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::ChannelEnds;

        #[test]
        fn releasing_an_end_twice_is_a_noop() {
            let mut ends = ChannelEnds::default();
            ends.add(1, true);
            ends.add(1, false);

            let send = ends.find(1, true).unwrap();
            ends.release_end(send, true);
            ends.release_end(send, true);
            assert_eq!(ends.numsendopen, 0);
            assert_eq!(ends.numrecvopen, 1);
            assert!(ends.is_open());

            let recv = ends.find(1, false).unwrap();
            ends.release_end(recv, false);
            assert_eq!(ends.numrecvopen, 0);
            assert!(!ends.is_open());
        }
    }
}
