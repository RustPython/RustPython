pub(crate) use _remote_debugging::module_def;

#[path = "_remote_debugging/binary.rs"]
mod binary;

#[pymodule]
mod _remote_debugging {
    use super::binary;
    use crate::vm::{
        AsObject, Py, PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyDictRef, PyList, PyTuple, PyType},
        common::lock::PyMutex,
        convert::{ToPyException, ToPyObject},
        function::{ArgIntoFloat, FsPath, FuncArgs, OptionalArg},
        host_env::fs as host_fs,
        types::{Constructor, PyStructSequence},
    };
    use std::{fs::File, sync::Arc};

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

    #[pyattr]
    const PROCESS_VM_READV_SUPPORTED: u8 = 0;

    #[pyfunction]
    fn zstd_available() -> bool {
        false
    }

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
        filename: String,
        location: PyObjectRef,
        funcname: String,
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

    fn file_error(error: binary::Error, vm: &VirtualMachine) -> PyBaseExceptionRef {
        match error {
            binary::Error::Io(error) => error.to_pyexception(vm),
            binary::Error::Value(message) => vm.new_value_error(message),
            binary::Error::Runtime(message) => vm.new_runtime_error(message),
            binary::Error::Overflow(message) => vm.new_overflow_error(message),
            binary::Error::Memory => vm.no_memory_error(),
        }
    }

    fn open_file(filename: PyObjectRef, write: bool, vm: &VirtualMachine) -> PyResult<File> {
        let path = FsPath::try_from_path_like(filename.clone(), true, vm)?.to_path_buf(vm)?;
        let result = if write {
            host_fs::create(path)
        } else {
            host_fs::open(path)
        };
        result.map_err(|error| {
            let exception = error.to_pyexception(vm);
            // Py_fopen preserves the original path-like object on the error.
            let _ = exception.as_object().set_attr("filename", filename, vm);
            exception
        })
    }

    fn sequence<'a>(
        object: &'a PyObjectRef,
        length: usize,
        vm: &VirtualMachine,
    ) -> PyResult<&'a [PyObjectRef]> {
        object
            .downcast_ref::<PyTuple>()
            .map(|value| value.as_slice())
            .filter(|value| value.len() == length)
            .ok_or_else(|| vm.new_type_error(format!("expected a {length}-item tuple")))
    }

    fn list(
        object: &PyObjectRef,
        message: &'static str,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<PyObjectRef>> {
        object
            .downcast_ref::<PyList>()
            .map(|value| value.borrow_vec().to_vec())
            .ok_or_else(|| vm.new_type_error(message))
    }

    fn parse_frame(object: &PyObjectRef, vm: &VirtualMachine) -> PyResult<binary::Frame> {
        let items = sequence(object, 4, vm)?;
        let mut location = [-1; 4];
        if !vm.is_none(&items[1]) {
            for (output, item) in location.iter_mut().zip(sequence(&items[1], 4, vm)?) {
                *output = item.try_to_value::<i32>(vm).unwrap_or(-1);
            }
        }
        Ok(binary::Frame {
            filename: items[0].try_to_value(vm)?,
            funcname: items[2].try_to_value(vm)?,
            location,
            opcode: items[3].try_to_value::<u8>(vm).unwrap_or(255),
        })
    }

    #[derive(FromArgs)]
    struct WriterArgs {
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

    #[derive(Debug)]
    struct WriterState {
        writer: Option<binary::Writer>,
        finalized_samples: u64,
    }

    #[pyattr]
    #[pyclass(name = "BinaryWriter", module = "_remote_debugging")]
    #[derive(Debug, PyPayload)]
    struct BinaryWriter {
        state: PyMutex<WriterState>,
    }

    impl Constructor for BinaryWriter {
        type Args = WriterArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            if !(-1..=4).contains(&args.mode) {
                return Err(vm.new_value_error("invalid profiling mode"));
            }
            if !(-1..=31).contains(&args.capture_features) {
                return Err(vm.new_value_error("invalid capture features"));
            }
            match args.compression {
                0 => (),
                1 => return Err(vm.new_runtime_error("zstd support not compiled in")),
                _ => return Err(vm.new_value_error("invalid compression type")),
            }
            let file = open_file(args.filename, true, vm)?;
            let writer = binary::Writer::new(
                file,
                args.sample_interval_us,
                args.start_time_us,
                args.mode,
                args.capture_features,
            )
            .map_err(|error| file_error(error, vm))?;
            Ok(Self {
                state: PyMutex::new(WriterState {
                    writer: Some(writer),
                    finalized_samples: 0,
                }),
            })
        }
    }

    #[derive(FromArgs)]
    struct ExitArgs {
        #[pyarg(any, optional)]
        exc_type: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional, name = "exc_val")]
        _exc_val: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional, name = "exc_tb")]
        _exc_tb: OptionalArg<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct SampleArgs {
        #[pyarg(any)]
        stack_frames: PyObjectRef,
        #[pyarg(any)]
        timestamp_us: u64,
    }

    #[derive(FromArgs)]
    struct ProfileStatsArgs {
        #[pyarg(any)]
        duration_sec: ArgIntoFloat,
        #[pyarg(any)]
        sample_rate: ArgIntoFloat,
        #[pyarg(any, optional)]
        error_rate: OptionalArg<Option<ArgIntoFloat>>,
        #[pyarg(any, optional)]
        missed_samples: OptionalArg<Option<ArgIntoFloat>>,
    }

    #[pyclass(with(Constructor))]
    impl BinaryWriter {
        #[pymethod]
        fn write_sample(zelf: &Py<Self>, args: SampleArgs, vm: &VirtualMachine) -> PyResult<()> {
            if zelf.state.lock().writer.is_none() {
                return Err(vm.new_value_error("Writer is closed"));
            }
            // Convert Python values before acquiring the writer lock. Integer
            // conversion may call user code, including close() on this writer.
            let mut samples = Vec::new();
            for interpreter in list(&args.stack_frames, "stack_frames must be a list", vm)? {
                let items = sequence(&interpreter, 2, vm)?;
                let interpreter_id = items[0].try_to_value::<u32>(vm)?;
                for thread in list(&items[1], "threads must be a list", vm)? {
                    let items = sequence(&thread, 3, vm)?;
                    let id = items[0].try_to_value::<u64>(vm)?;
                    let status = items[1].try_to_value::<i64>(vm)? as u8;
                    let frames = list(&items[2], "frames must be a list", vm)?
                        .iter()
                        .take(256)
                        .map(|frame| parse_frame(frame, vm))
                        .collect::<PyResult<Vec<_>>>()?;
                    samples.push((id, interpreter_id, status, frames));
                }
            }
            let mut state = zelf.state.lock();
            let writer = state
                .writer
                .as_mut()
                .ok_or_else(|| vm.new_value_error("Writer is closed"))?;
            for (id, interpreter, status, frames) in samples {
                writer
                    .sample(id, interpreter, status, frames, args.timestamp_us)
                    .map_err(|error| file_error(error, vm))?;
            }
            Ok(())
        }

        #[pymethod]
        fn set_stats(zelf: &Py<Self>, args: ProfileStatsArgs, vm: &VirtualMachine) -> PyResult<()> {
            let mut state = zelf.state.lock();
            let writer = state
                .writer
                .as_mut()
                .ok_or_else(|| vm.new_value_error("Writer is closed"))?;
            let stats = binary::ProfileStats {
                duration: args.duration_sec.into_float(),
                sample_rate: args.sample_rate.into_float(),
                error_rate: args.error_rate.flatten().map(ArgIntoFloat::into_float),
                missed_samples: args.missed_samples.flatten().map(ArgIntoFloat::into_float),
            };
            stats.validate().map_err(|error| file_error(error, vm))?;
            writer.profile_stats = Some(stats);
            Ok(())
        }

        #[pymethod]
        fn finalize(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            let mut state = zelf.state.lock();
            let writer = state
                .writer
                .as_mut()
                .ok_or_else(|| vm.new_value_error("Writer is already closed"))?;
            let version = crate::vm::version::MAJOR as u8;
            writer
                .finalize([
                    version,
                    crate::vm::version::MINOR as u8,
                    crate::vm::version::MICRO as u8,
                ])
                .map_err(|error| file_error(error, vm))?;
            state.finalized_samples = writer.stats.total_samples;
            state.writer = None;
            Ok(())
        }

        #[pymethod]
        fn close(zelf: &Py<Self>) {
            zelf.state.lock().writer = None;
        }

        #[pygetset]
        fn total_samples(zelf: &Py<Self>) -> u64 {
            let state = zelf.state.lock();
            state
                .writer
                .as_ref()
                .map_or(state.finalized_samples, |writer| writer.stats.total_samples)
        }

        #[pymethod]
        fn get_stats(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyDictRef> {
            let state = zelf.state.lock();
            let writer = state
                .writer
                .as_ref()
                .ok_or_else(|| vm.new_value_error("Writer is closed"))?;
            let stats = &writer.stats;
            let dict = stats_dict(stats, vm)?;
            for (name, value) in [
                ("total_frames_written", stats.total_frames_written),
                ("frames_saved", stats.frames_saved),
                ("bytes_written", stats.bytes_written),
            ] {
                dict.set_item(name, value.to_pyobject(vm), vm)?;
            }
            let frames = stats.total_frames_written + stats.frames_saved;
            let percentage = if frames == 0 {
                0.0
            } else {
                stats.frames_saved as f64 / frames as f64 * 100.0
            };
            dict.set_item("frame_compression_pct", percentage.to_pyobject(vm), vm)?;
            Ok(dict)
        }

        #[pymethod]
        fn __enter__(zelf: PyObjectRef) -> PyObjectRef {
            zelf
        }

        #[pymethod]
        fn __exit__(zelf: &Py<Self>, args: ExitArgs, vm: &VirtualMachine) -> PyResult<bool> {
            if zelf.state.lock().writer.is_some() {
                if args
                    .exc_type
                    .into_option()
                    .is_none_or(|value| vm.is_none(&value))
                {
                    let result = Self::finalize(zelf, vm);
                    if result.is_err() {
                        Self::close(zelf);
                    }
                    result?;
                } else {
                    Self::close(zelf);
                }
            }
            Ok(false)
        }
    }

    fn stats_dict(stats: &binary::Stats, vm: &VirtualMachine) -> PyResult<PyDictRef> {
        let dict = vm.ctx.new_dict();
        for (name, value) in [
            ("repeat_records", stats.records[0]),
            ("full_records", stats.records[1]),
            ("suffix_records", stats.records[2]),
            ("pop_push_records", stats.records[3]),
            ("repeat_samples", stats.repeat_samples),
            ("total_records", stats.records.iter().sum()),
            ("total_samples", stats.total_samples),
        ] {
            dict.set_item(name, value.to_pyobject(vm), vm)?;
        }
        Ok(dict)
    }

    #[pyattr]
    #[pyclass(name = "BinaryReader", module = "_remote_debugging")]
    #[derive(Debug, PyPayload)]
    struct BinaryReader {
        reader: PyMutex<Option<Arc<binary::Reader>>>,
        stats: PyMutex<binary::Stats>,
    }

    #[derive(FromArgs)]
    struct ReaderArgs {
        #[pyarg(any)]
        filename: PyObjectRef,
    }

    impl Constructor for BinaryReader {
        type Args = ReaderArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let file = open_file(args.filename, false, vm)?;
            let reader = binary::Reader::open(file).map_err(|error| file_error(error, vm))?;
            Ok(Self {
                reader: PyMutex::new(Some(Arc::new(reader))),
                stats: PyMutex::new(binary::Stats::default()),
            })
        }
    }

    #[derive(FromArgs)]
    struct ReplayArgs {
        #[pyarg(any)]
        collector: PyObjectRef,
        #[pyarg(any, optional)]
        progress_callback: OptionalArg<PyObjectRef>,
    }

    impl BinaryReader {
        fn reader(&self, vm: &VirtualMachine) -> PyResult<Arc<binary::Reader>> {
            self.reader
                .lock()
                .clone()
                .ok_or_else(|| vm.new_value_error("Reader is closed"))
        }
    }

    #[pyclass(with(Constructor))]
    impl BinaryReader {
        #[pymethod]
        fn get_info(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyDictRef> {
            let reader = zelf.reader(vm)?;
            let dict = vm.ctx.new_dict();
            for (name, value) in [
                ("version", 1),
                ("start_time_us", reader.start_time),
                ("sample_interval_us", reader.interval),
                ("sample_count", reader.sample_count),
                ("thread_count", reader.thread_count as u64),
                ("string_count", reader.strings.len() as u64),
                ("frame_count", reader.frame_count() as u64),
                ("compression_type", 0),
            ] {
                dict.set_item(name, value.to_pyobject(vm), vm)?;
            }
            let [major, minor, micro] = reader.python_version;
            dict.set_item("python_version", (major, minor, micro).to_pyobject(vm), vm)?;
            dict.set_item("mode", reader.mode.to_pyobject(vm), vm)?;
            dict.set_item("capture_features", reader.features.to_pyobject(vm), vm)?;
            for (name, value) in [
                (
                    "duration_sec",
                    reader.profile_stats.map(|stats| stats.duration),
                ),
                (
                    "sample_rate",
                    reader.profile_stats.map(|stats| stats.sample_rate),
                ),
                (
                    "error_rate",
                    reader.profile_stats.and_then(|stats| stats.error_rate),
                ),
                (
                    "missed_samples",
                    reader.profile_stats.and_then(|stats| stats.missed_samples),
                ),
            ] {
                dict.set_item(name, value.to_pyobject(vm), vm)?;
            }
            Ok(dict)
        }

        #[pymethod]
        fn get_stats(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyDictRef> {
            zelf.reader(vm)?;
            let stats = zelf.stats.lock();
            let dict = stats_dict(&stats, vm)?;
            dict.set_item(
                "stack_reconstructions",
                stats.stack_reconstructions.to_pyobject(vm),
                vm,
            )?;
            Ok(dict)
        }

        #[pymethod]
        fn replay(zelf: &Py<Self>, args: ReplayArgs, vm: &VirtualMachine) -> PyResult<u64> {
            let reader = zelf.reader(vm)?;
            args.collector.get_attr("collect", vm).map_err(|error| {
                if error.fast_isinstance(vm.ctx.exceptions.attribute_error) {
                    vm.new_type_error("Collector must have a collect() method")
                } else {
                    error
                }
            })?;
            let progress = args
                .progress_callback
                .into_option()
                .filter(|callback| !vm.is_none(callback));
            if let Some(callback) = &progress {
                callback.call((0, reader.sample_count), vm)?;
            }
            let mut replay = reader.replay();
            let result = (|| {
                while let Some(batch) = replay.next().map_err(|error| file_error(error, vm))? {
                    if batch.timestamps.is_empty() {
                        continue;
                    }
                    let mut frames = Vec::with_capacity(batch.stack.len());
                    for index in batch.stack {
                        let frame = reader.frame(index).map_err(|error| file_error(error, vm))?;
                        let [line, end_line, column, end_column] = frame.location;
                        let location = if line == -1 {
                            vm.ctx.none()
                        } else {
                            (line, end_line, column, end_column).to_pyobject(vm)
                        };
                        frames.push(
                            FrameInfoData {
                                filename: frame.filename,
                                funcname: frame.funcname,
                                location,
                                opcode: (frame.opcode != 255)
                                    .then_some(frame.opcode)
                                    .to_pyobject(vm),
                            }
                            .to_pyobject(vm),
                        );
                    }
                    let thread = ThreadInfoData {
                        thread_id: batch.id.to_pyobject(vm),
                        status: batch.status.to_pyobject(vm),
                        frame_info: vm.ctx.new_list(frames).into(),
                    }
                    .to_pyobject(vm);
                    let interpreter = InterpreterInfoData {
                        interpreter_id: batch.interpreter.to_pyobject(vm),
                        threads: vm.ctx.new_list(vec![thread]).into(),
                    }
                    .to_pyobject(vm);
                    let count = batch.timestamps.len() as u64;
                    let timestamps = vm.ctx.new_list(
                        batch
                            .timestamps
                            .into_iter()
                            .map(|value| value.to_pyobject(vm))
                            .collect(),
                    );
                    args.collector
                        .get_attr("collect", vm)?
                        .call((vm.ctx.new_list(vec![interpreter]), timestamps), vm)?;
                    if replay.stats.total_samples / 1000
                        != (replay.stats.total_samples - count) / 1000
                        && let Some(callback) = &progress
                    {
                        callback.call((replay.stats.total_samples, reader.sample_count), vm)?;
                    }
                }
                if let Some(callback) = &progress {
                    callback.call((replay.stats.total_samples, reader.sample_count), vm)?;
                }
                Ok(replay.stats.total_samples)
            })();
            *zelf.stats.lock() = replay.stats;
            result
        }

        #[pymethod]
        fn close(zelf: &Py<Self>) {
            *zelf.reader.lock() = None;
        }

        #[pygetset]
        fn sample_count(zelf: &Py<Self>) -> u64 {
            zelf.reader
                .lock()
                .as_ref()
                .map_or(0, |reader| reader.sample_count)
        }

        #[pygetset]
        fn sample_interval_us(zelf: &Py<Self>) -> u64 {
            zelf.reader
                .lock()
                .as_ref()
                .map_or(0, |reader| reader.interval)
        }

        #[pymethod]
        fn __enter__(zelf: PyObjectRef) -> PyObjectRef {
            zelf
        }

        #[pymethod]
        fn __exit__(zelf: &Py<Self>, _args: ExitArgs) -> bool {
            Self::close(zelf);
            false
        }
    }
}
