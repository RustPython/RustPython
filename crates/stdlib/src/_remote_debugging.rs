mod binary;

pub(crate) use _remote_debugging::module_def;

#[pymodule]
mod _remote_debugging {
    use super::binary;
    use crate::vm::{
        AsObject, Py, PyObject, PyObjectRef, PyRef, PyResult, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyDictRef, PyFloat, PyList, PyTuple, PyType},
        common::{lock::PyMutex, rc::PyRc},
        convert::{IntoPyException, ToPyObject},
        function::{FsPath, FuncArgs},
        types::{Constructor, DefaultConstructor, Initializer, PyStructSequence},
    };
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicBool, Ordering};

    #[pystruct_sequence_data]
    struct LocationInfoData {
        lineno: PyObjectRef,
        end_lineno: PyObjectRef,
        col_offset: PyObjectRef,
        end_col_offset: PyObjectRef,
    }

    #[pyattr]
    #[pystruct_sequence(
        name = "LocationInfo",
        module = "_remote_debugging",
        data = "LocationInfoData"
    )]
    struct LocationInfo;

    #[pyclass(with(PyStructSequence))]
    impl LocationInfo {}

    #[pystruct_sequence_data]
    struct FrameInfoData {
        filename: PyObjectRef,
        location: PyObjectRef,
        funcname: PyObjectRef,
        opcode: PyObjectRef,
    }

    #[pyattr]
    #[pystruct_sequence(
        name = "FrameInfo",
        module = "_remote_debugging",
        data = "FrameInfoData"
    )]
    struct FrameInfo;

    #[pyclass(with(PyStructSequence))]
    impl FrameInfo {}

    #[pystruct_sequence_data]
    struct TaskInfoData {
        task_id: PyObjectRef,
        task_name: PyObjectRef,
        coroutine_stack: PyObjectRef,
        awaited_by: PyObjectRef,
    }

    #[pyattr]
    #[pystruct_sequence(name = "TaskInfo", module = "_remote_debugging", data = "TaskInfoData")]
    struct TaskInfo;

    #[pyclass(with(PyStructSequence))]
    impl TaskInfo {}

    #[pystruct_sequence_data]
    struct CoroInfoData {
        call_stack: PyObjectRef,
        task_name: PyObjectRef,
    }

    #[pyattr]
    #[pystruct_sequence(name = "CoroInfo", module = "_remote_debugging", data = "CoroInfoData")]
    struct CoroInfo;

    #[pyclass(with(PyStructSequence))]
    impl CoroInfo {}

    #[pystruct_sequence_data]
    struct ThreadInfoData {
        thread_id: PyObjectRef,
        status: PyObjectRef,
        frame_info: PyObjectRef,
    }

    #[pyattr]
    #[pystruct_sequence(
        name = "ThreadInfo",
        module = "_remote_debugging",
        data = "ThreadInfoData"
    )]
    struct ThreadInfo;

    #[pyclass(with(PyStructSequence))]
    impl ThreadInfo {}

    #[pystruct_sequence_data]
    struct InterpreterInfoData {
        interpreter_id: PyObjectRef,
        threads: PyObjectRef,
    }

    #[pyattr]
    #[pystruct_sequence(
        name = "InterpreterInfo",
        module = "_remote_debugging",
        data = "InterpreterInfoData"
    )]
    struct InterpreterInfo;

    #[pyclass(with(PyStructSequence))]
    impl InterpreterInfo {}

    #[pyattr]
    const THREAD_STATUS_HAS_GIL: u8 = 1;
    #[pyattr]
    const THREAD_STATUS_ON_CPU: u8 = 2;
    #[pyattr]
    const THREAD_STATUS_UNKNOWN: u8 = 4;
    #[pyattr]
    const THREAD_STATUS_GIL_REQUESTED: u8 = 8;
    #[pyattr]
    const THREAD_STATUS_HAS_EXCEPTION: u8 = 16;
    #[pyattr]
    const THREAD_STATUS_MAIN_THREAD: u8 = 32;

    #[pyfunction]
    fn zstd_available() -> bool {
        false
    }

    #[pystruct_sequence_data]
    struct AwaitedInfoData {
        thread_id: PyObjectRef,
        awaited_by: PyObjectRef,
    }

    #[pyattr]
    #[pystruct_sequence(
        name = "AwaitedInfo",
        module = "_remote_debugging",
        data = "AwaitedInfoData"
    )]
    struct AwaitedInfo;

    #[pyclass(with(PyStructSequence))]
    impl AwaitedInfo {}

    fn codec_error(error: binary::Error, vm: &VirtualMachine) -> PyBaseExceptionRef {
        match error {
            binary::Error::Io(error) => error.into_pyexception(vm),
            binary::Error::Value(message) => vm.new_value_error(message),
            binary::Error::Overflow(message) => vm.new_overflow_error(message),
            binary::Error::Memory => vm.new_memory_error("unable to allocate profiling data"),
            binary::Error::Runtime(message) => vm.new_runtime_error(message),
        }
    }

    fn codec_path_error(
        error: binary::Error,
        path: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyBaseExceptionRef {
        let is_io = matches!(error, binary::Error::Io(_));
        let exception = codec_error(error, vm);
        if is_io {
            let _ = exception.as_object().set_attr("filename", path, vm);
        }
        exception
    }

    fn list_items(value: &PyObject, name: &str, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        value
            .downcast_ref::<PyList>()
            .map(|list| list.borrow_vec().to_vec())
            .ok_or_else(|| vm.new_type_error(format!("{name} must be a list")))
    }

    fn record_fields<'a>(
        value: &'a PyObject,
        count: usize,
        name: &str,
        vm: &VirtualMachine,
    ) -> PyResult<&'a [PyObjectRef]> {
        value
            .downcast_ref::<PyTuple>()
            .map(|tuple| tuple.as_slice())
            .filter(|fields| fields.len() >= count)
            .ok_or_else(|| {
                vm.new_type_error(format!(
                    "{name} must be a tuple with at least {count} fields"
                ))
            })
    }

    fn normalize_frame(value: &PyObject, vm: &VirtualMachine) -> PyResult<binary::Frame> {
        let fields = record_fields(value, 4, "frame", vm)?;
        let mut location = [-1; 4];
        if !vm.is_none(&fields[1]) {
            let values = record_fields(&fields[1], 4, "location", vm)?;
            for (target, value) in location.iter_mut().zip(values) {
                // Match CPython: C long is 32-bit on Windows, 64-bit on LP64.
                #[allow(clippy::unnecessary_cast)]
                let normalized = value.try_to_value::<core::ffi::c_long>(vm).unwrap_or(-1) as i32;
                *target = normalized;
            }
        }
        let opcode = fields[3]
            .try_to_value::<core::ffi::c_long>(vm)
            .ok()
            .filter(|value| (0..=254).contains(value))
            .map_or(255, |value| value as u8);
        Ok(binary::Frame {
            filename: fields[0].try_to_value::<String>(vm)?,
            funcname: fields[2].try_to_value::<String>(vm)?,
            location,
            opcode,
        })
    }

    #[derive(FromArgs)]
    struct BinaryWriterArgs {
        #[pyarg(any)]
        filename: PyObjectRef,
        #[pyarg(any)]
        sample_interval_us: u64,
        #[pyarg(any)]
        start_time_us: u64,
        #[pyarg(named, default = 0)]
        compression: i32,
        #[pyarg(named, default = -1)]
        mode: i32,
        #[pyarg(named, default = -1)]
        capture_features: i32,
    }

    #[derive(Debug, Default)]
    struct WriterState {
        writer: Option<binary::Writer>,
        cached_total_samples: u64,
    }

    #[pyattr]
    #[pyclass(name = "BinaryWriter", module = "_remote_debugging")]
    #[derive(Debug, PyPayload, Default)]
    struct BinaryWriter {
        state: PyMutex<WriterState>,
    }

    impl DefaultConstructor for BinaryWriter {}

    impl Initializer for BinaryWriter {
        type Args = BinaryWriterArgs;
        fn init(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
            if !(-1..=4).contains(&args.mode) {
                return Err(vm.new_value_error("invalid profiling mode"));
            }
            if !(-1..=31).contains(&args.capture_features) {
                return Err(vm.new_value_error("invalid capture features"));
            }
            let path =
                FsPath::try_from_path_like(args.filename.clone(), true, vm)?.to_path_buf(vm)?;
            let old = {
                let mut state = zelf.state.lock();
                state.cached_total_samples = 0;
                state.writer.take()
            };
            drop(old);
            let writer = binary::Writer::create(
                &path,
                args.sample_interval_us,
                args.start_time_us,
                args.compression,
                args.mode,
                args.capture_features,
                [
                    crate::vm::version::MAJOR as u8,
                    crate::vm::version::MINOR as u8,
                    crate::vm::version::MICRO as u8,
                ],
            )
            .map_err(|error| codec_path_error(error, args.filename, vm))?;
            zelf.state.lock().writer = Some(writer);
            Ok(())
        }
    }

    #[derive(FromArgs)]
    struct BinaryExitArgs {
        #[pyarg(any, optional)]
        exc_type: Option<PyObjectRef>,
        #[pyarg(any, optional, name = "exc_val")]
        _exc_value: Option<PyObjectRef>,
        #[pyarg(any, optional, name = "exc_tb")]
        _exc_tb: Option<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct SampleArgs {
        #[pyarg(any)]
        stack_frames: PyObjectRef,
        #[pyarg(any)]
        timestamp_us: PyObjectRef,
    }

    fn profile_float(obj: &PyObject, vm: &VirtualMachine) -> PyResult<f64> {
        // PyFloat_AsDouble reads float subclasses without calling __float__.
        Ok(match obj.downcast_ref::<PyFloat>() {
            Some(value) => value.to_f64(),
            None => obj.try_float(vm)?.to_f64(),
        })
    }

    #[derive(FromArgs)]
    struct SetStatsArgs {
        #[pyarg(any)]
        duration_sec: PyObjectRef,
        #[pyarg(any)]
        sample_rate: PyObjectRef,
        #[pyarg(any, optional)]
        error_rate: Option<PyObjectRef>,
        #[pyarg(any, optional)]
        missed_samples: Option<PyObjectRef>,
    }

    fn optional_float(value: Option<PyObjectRef>, vm: &VirtualMachine) -> PyResult<Option<f64>> {
        value
            .filter(|value| !vm.is_none(value))
            .map(|value| profile_float(&value, vm))
            .transpose()
    }

    #[pyclass(with(Constructor, Initializer))]
    impl BinaryWriter {
        #[pymethod]
        fn write_sample(zelf: &Py<Self>, args: SampleArgs, vm: &VirtualMachine) -> PyResult<()> {
            let SampleArgs {
                stack_frames,
                timestamp_us,
            } = args;
            let timestamp_us = timestamp_us.try_into_value::<u64>(vm)?;
            if zelf.state.lock().writer.is_none() {
                return Err(vm.new_value_error("Writer is closed"));
            }
            for interpreter in list_items(&stack_frames, "stack_frames", vm)? {
                let fields = record_fields(&interpreter, 2, "interpreter", vm)?;
                let interpreter_id = fields[0].try_to_value::<u32>(vm)?;
                for thread in list_items(&fields[1], "threads", vm)? {
                    let fields = record_fields(&thread, 3, "thread", vm)?;
                    let thread_id = fields[0].try_to_value::<u64>(vm)?;
                    let status = fields[1].try_to_value::<core::ffi::c_long>(vm)? as u8;
                    let frame_objects = {
                        let list = fields[2]
                            .downcast_ref::<PyList>()
                            .ok_or_else(|| vm.new_type_error("frame_info must be a list"))?;
                        list.borrow_vec()
                            .iter()
                            .take(256)
                            .cloned()
                            .collect::<Vec<_>>()
                    };
                    let frames = frame_objects
                        .into_iter()
                        .map(|frame| normalize_frame(&frame, vm))
                        .collect::<PyResult<Vec<_>>>()?;
                    let sample = binary::Sample {
                        interpreter_id,
                        thread_id,
                        status,
                        frames,
                        timestamp_us,
                    };
                    // All Python conversions above have finished before borrowing file state.
                    let result = {
                        let mut state = zelf.state.lock();
                        state
                            .writer
                            .as_mut()
                            .map(|writer| writer.write_sample(&sample))
                    };
                    result
                        .ok_or_else(|| vm.new_value_error("Writer is closed"))?
                        .map_err(|error| codec_error(error, vm))?;
                }
            }
            Ok(())
        }

        #[pymethod]
        fn set_stats(zelf: &Py<Self>, args: SetStatsArgs, vm: &VirtualMachine) -> PyResult<()> {
            let duration_sec = profile_float(&args.duration_sec, vm)?;
            let sample_rate = profile_float(&args.sample_rate, vm)?;
            let is_open = zelf.state.lock().writer.is_some();
            if !is_open {
                return Err(vm.new_value_error("Writer is closed"));
            }
            let profile = binary::ProfileStats {
                duration_sec,
                sample_rate,
                error_rate: optional_float(args.error_rate, vm)?,
                missed_samples: optional_float(args.missed_samples, vm)?,
            };
            let result = {
                let mut state = zelf.state.lock();
                state
                    .writer
                    .as_mut()
                    .map(|writer| writer.set_stats(profile))
            };
            result
                .ok_or_else(|| vm.new_value_error("Writer is closed"))?
                .map_err(|error| codec_error(error, vm))
        }

        #[pymethod]
        fn finalize(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            let result = {
                let mut state = zelf.state.lock();
                if let Some(writer) = state.writer.as_mut() {
                    match writer.finalize() {
                        Ok(()) => {
                            state.cached_total_samples = writer.total_samples();
                            Ok(state.writer.take())
                        }
                        Err(error) => Err(Some(error)),
                    }
                } else {
                    Err(None)
                }
            };
            match result {
                Ok(writer) => {
                    drop(writer);
                    Ok(())
                }
                Err(Some(error)) => Err(codec_error(error, vm)),
                Err(None) => Err(vm.new_value_error("Writer is already closed")),
            }
        }

        #[pymethod]
        fn close(zelf: &Py<Self>) {
            let writer = zelf.state.lock().writer.take();
            drop(writer);
        }

        #[pygetset]
        fn total_samples(zelf: &Py<Self>) -> u64 {
            let state = zelf.state.lock();
            state
                .writer
                .as_ref()
                .map_or(state.cached_total_samples, binary::Writer::total_samples)
        }

        #[pymethod]
        fn get_stats(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyDictRef> {
            let stats = zelf
                .state
                .lock()
                .writer
                .as_ref()
                .map(|writer| *writer.stats());
            let stats = stats.ok_or_else(|| vm.new_value_error("Writer is closed"))?;
            let dict = vm.ctx.new_dict();
            for (key, value) in [
                ("repeat_records", stats.repeat_records),
                ("repeat_samples", stats.repeat_samples),
                ("full_records", stats.full_records),
                ("suffix_records", stats.suffix_records),
                ("pop_push_records", stats.pop_push_records),
                ("total_records", stats.total_records()),
                ("total_samples", stats.total_samples),
                ("total_frames_written", stats.total_frames_written),
                ("frames_saved", stats.frames_saved),
                ("bytes_written", stats.bytes_written),
            ] {
                dict.set_item(key, value.to_pyobject(vm), vm)?;
            }
            dict.set_item(
                "frame_compression_pct",
                stats.frame_compression_pct().to_pyobject(vm),
                vm,
            )?;
            Ok(dict)
        }

        #[pymethod]
        fn __enter__(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }

        #[pymethod]
        fn __exit__(zelf: &Py<Self>, args: BinaryExitArgs, vm: &VirtualMachine) -> PyResult<bool> {
            if args.exc_type.as_ref().is_none_or(|value| vm.is_none(value)) {
                let is_open = zelf.state.lock().writer.is_some();
                if is_open && let Err(error) = Self::finalize(zelf, vm) {
                    Self::close(zelf);
                    return Err(error);
                }
            } else {
                Self::close(zelf);
            }
            Ok(false)
        }
    }

    #[derive(Debug)]
    struct ReaderOpen {
        reader: Arc<binary::Reader>,
        stats: PyMutex<binary::ReaderStats>,
        replaying: AtomicBool,
    }

    struct ReplayLease(PyRc<ReaderOpen>);
    impl Drop for ReplayLease {
        fn drop(&mut self) {
            self.0.replaying.store(false, Ordering::Release);
        }
    }

    #[pyattr]
    #[pyclass(name = "BinaryReader", module = "_remote_debugging")]
    #[derive(Debug, PyPayload, Default)]
    struct BinaryReader {
        state: PyMutex<Option<PyRc<ReaderOpen>>>,
    }

    impl DefaultConstructor for BinaryReader {}

    #[derive(FromArgs)]
    struct BinaryReaderArgs {
        #[pyarg(any)]
        filename: PyObjectRef,
    }

    impl Initializer for BinaryReader {
        type Args = BinaryReaderArgs;
        fn init(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
            let path =
                FsPath::try_from_path_like(args.filename.clone(), true, vm)?.to_path_buf(vm)?;
            let old = zelf.state.lock().take();
            drop(old);
            let reader = binary::Reader::open(&path)
                .map_err(|error| codec_path_error(error, args.filename, vm))?;
            *zelf.state.lock() = Some(PyRc::new(ReaderOpen {
                reader: Arc::new(reader),
                stats: PyMutex::new(binary::ReaderStats::default()),
                replaying: AtomicBool::new(false),
            }));
            Ok(())
        }
    }

    fn replay_records(sample: &binary::Sample, vm: &VirtualMachine) -> PyObjectRef {
        let frames = sample
            .frames
            .iter()
            .map(|frame| {
                let location = if frame.location[0] == -1 {
                    vm.ctx.none()
                } else {
                    vm.ctx
                        .new_tuple(
                            frame
                                .location
                                .iter()
                                .map(|value| value.to_pyobject(vm))
                                .collect(),
                        )
                        .into()
                };
                FrameInfo::from_data(
                    FrameInfoData {
                        filename: frame.filename.clone().to_pyobject(vm),
                        location,
                        funcname: frame.funcname.clone().to_pyobject(vm),
                        opcode: if frame.opcode == 255 {
                            vm.ctx.none()
                        } else {
                            frame.opcode.to_pyobject(vm)
                        },
                    },
                    vm,
                )
                .into()
            })
            .collect();
        let thread = ThreadInfo::from_data(
            ThreadInfoData {
                thread_id: sample.thread_id.to_pyobject(vm),
                status: sample.status.to_pyobject(vm),
                frame_info: vm.ctx.new_list(frames).into(),
            },
            vm,
        );
        let interpreter = InterpreterInfo::from_data(
            InterpreterInfoData {
                interpreter_id: sample.interpreter_id.to_pyobject(vm),
                threads: vm.ctx.new_list(vec![thread.into()]).into(),
            },
            vm,
        );
        vm.ctx.new_list(vec![interpreter.into()]).into()
    }

    #[derive(FromArgs)]
    struct ReplayArgs {
        #[pyarg(any)]
        collector: PyObjectRef,
        #[pyarg(any, optional)]
        progress_callback: Option<PyObjectRef>,
    }

    #[pyclass(with(Constructor, Initializer))]
    impl BinaryReader {
        fn opened(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyRc<ReaderOpen>> {
            let open = zelf.state.lock().clone();
            open.ok_or_else(|| vm.new_value_error("Reader is closed"))
        }

        #[pymethod]
        fn close(zelf: &Py<Self>) {
            let old = zelf.state.lock().take();
            drop(old);
        }

        #[pygetset]
        fn sample_count(zelf: &Py<Self>) -> u64 {
            zelf.state
                .lock()
                .as_ref()
                .map_or(0, |open| open.reader.metadata().sample_count)
        }

        #[pygetset]
        fn sample_interval_us(zelf: &Py<Self>) -> u64 {
            zelf.state
                .lock()
                .as_ref()
                .map_or(0, |open| open.reader.metadata().sample_interval_us)
        }

        #[pymethod]
        fn get_info(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyDictRef> {
            let open = Self::opened(zelf, vm)?;
            let info = open.reader.metadata();
            let dict = vm.ctx.new_dict();
            dict.set_item("version", info.version.to_pyobject(vm), vm)?;
            dict.set_item(
                "python_version",
                vm.ctx
                    .new_tuple(
                        info.python_version
                            .iter()
                            .map(|value| value.to_pyobject(vm))
                            .collect(),
                    )
                    .into(),
                vm,
            )?;
            for (key, value) in [
                ("start_time_us", info.start_time_us),
                ("sample_interval_us", info.sample_interval_us),
                ("sample_count", info.sample_count),
                ("thread_count", u64::from(info.thread_count)),
                ("string_count", u64::from(info.string_count)),
                ("frame_count", u64::from(info.frame_count)),
            ] {
                dict.set_item(key, value.to_pyobject(vm), vm)?;
            }
            dict.set_item(
                "compression_type",
                (info.compression_type as i32).to_pyobject(vm),
                vm,
            )?;
            dict.set_item("mode", info.mode.to_pyobject(vm), vm)?;
            dict.set_item(
                "capture_features",
                info.capture_features.to_pyobject(vm),
                vm,
            )?;
            let profile = info.profile_stats.as_ref();
            for (key, value) in [
                ("duration_sec", profile.map(|value| value.duration_sec)),
                ("sample_rate", profile.map(|value| value.sample_rate)),
                ("error_rate", profile.and_then(|value| value.error_rate)),
                (
                    "missed_samples",
                    profile.and_then(|value| value.missed_samples),
                ),
            ] {
                dict.set_item(key, value.to_pyobject(vm), vm)?;
            }
            Ok(dict)
        }

        #[pymethod]
        fn get_stats(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyDictRef> {
            let open = Self::opened(zelf, vm)?;
            let stats = *open.stats.lock();
            let dict = vm.ctx.new_dict();
            for (key, value) in [
                ("repeat_records", stats.repeat_records),
                ("repeat_samples", stats.repeat_samples),
                ("full_records", stats.full_records),
                ("suffix_records", stats.suffix_records),
                ("pop_push_records", stats.pop_push_records),
                ("total_records", stats.total_records()),
                ("total_samples", stats.total_samples),
                ("stack_reconstructions", stats.stack_reconstructions),
            ] {
                dict.set_item(key, value.to_pyobject(vm), vm)?;
            }
            Ok(dict)
        }

        #[pymethod]
        fn replay(zelf: &Py<Self>, args: ReplayArgs, vm: &VirtualMachine) -> PyResult<u64> {
            let open = Self::opened(zelf, vm)?;
            if open
                .replaying
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                return Err(vm.new_runtime_error("A replay is already active on this reader"));
            }
            let _lease = ReplayLease(open.clone());
            // The per-open snapshot survives close/reinitialization, but cannot
            // publish counters into a replacement reader. No lock crosses Python.
            let initial_collect = args.collector.get_attr("collect", vm).map_err(|error| {
                if error.fast_isinstance(vm.ctx.exceptions.attribute_error) {
                    vm.new_type_error("Collector must have a collect() method")
                } else {
                    error
                }
            })?;
            drop(initial_collect);
            let progress = args.progress_callback.filter(|value| !vm.is_none(value));
            let total = open.reader.metadata().sample_count;
            if let Some(callback) = &progress {
                callback.call((0_u64, total), vm)?;
            }
            let mut replay = open.reader.replay();
            let mut committed_samples = 0;
            loop {
                let batch = replay.next_batch();
                let mut stats = *replay.stats();
                stats.total_samples = committed_samples;
                *open.stats.lock() = stats;
                let Some(batch) = batch.map_err(|error| codec_error(error, vm))? else {
                    break;
                };
                let records = replay_records(&batch.sample, vm);
                let timestamps: PyObjectRef = vm
                    .ctx
                    .new_list(
                        batch
                            .timestamps
                            .iter()
                            .map(|value| value.to_pyobject(vm))
                            .collect(),
                    )
                    .into();
                args.collector
                    .get_attr("collect", vm)?
                    .call((records, timestamps), vm)?;
                if batch.record_complete {
                    committed_samples = replay.replayed();
                    *open.stats.lock() = *replay.stats();
                    if let Some(callback) = &progress {
                        if batch.is_repeat
                            && committed_samples % 1000 < u64::from(batch.record_samples)
                        {
                            callback.call((committed_samples, total), vm)?;
                        }
                        if committed_samples % 1000 == 0 {
                            callback.call((committed_samples, total), vm)?;
                        }
                    }
                }
            }
            if let Some(callback) = &progress {
                callback.call((replay.replayed(), total), vm)?;
            }
            Ok(replay.replayed())
        }

        #[pymethod]
        fn __enter__(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }

        #[pymethod]
        fn __exit__(zelf: &Py<Self>, _args: BinaryExitArgs) -> bool {
            Self::close(zelf);
            false
        }
    }

    #[pyattr]
    #[pyclass(name = "RemoteUnwinder", module = "_remote_debugging")]
    #[derive(Debug, PyPayload)]
    struct RemoteUnwinder {}

    impl Constructor for RemoteUnwinder {
        type Args = FuncArgs;

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            Err(vm.new_not_implemented_error("_remote_debugging is not available"))
        }
    }

    #[pyclass(with(Constructor))]
    impl RemoteUnwinder {}
}
