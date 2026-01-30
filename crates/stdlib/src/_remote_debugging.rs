pub(crate) use _remote_debugging::module_def;

#[pymodule]
mod _remote_debugging {
    use crate::vm::{
        Py, PyObjectRef, PyResult, VirtualMachine,
        builtins::PyType,
        function::FuncArgs,
        types::{Constructor, PyStructSequence},
    };

    #[pystruct_sequence_data]
    struct FrameInfoData {
        filename: String,
        lineno: i64,
        funcname: String,
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
            Err(vm.new_not_implemented_error("_remote_debugging is not available".to_owned()))
        }
    }

    #[pyclass(with(Constructor))]
    impl RemoteUnwinder {}
}
