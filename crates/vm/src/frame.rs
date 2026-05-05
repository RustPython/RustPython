// spell-checker: ignore compactlong compactlongs

use crate::anystr::AnyStr;

use crate::{
    AsObject, Py, PyExact, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, PyStackRef,
    TryFromObject, VirtualMachine,
    builtins::{
        PyBaseException, PyBaseExceptionRef, PyBaseObject, PyCode, PyCoroutine, PyDict, PyDictRef,
        PyFloat, PyFrozenSet, PyGenerator, PyInt, PyInterpolation, PyList, PyModule, PyProperty,
        PySet, PySlice, PyStr, PyStrInterned, PyTemplate, PyTraceback, PyType, PyUtf8Str,
        builtin_func::PyNativeFunction,
        descriptor::{MemberGetter, PyMemberDescriptor, PyMethodDescriptor},
        frame::stack_analysis,
        function::{
            PyBoundMethod, PyCell, PyCellRef, PyFunction, datastack_frame_size_bytes_for_code,
            vectorcall_function,
        },
        list::PyListIterator,
        range::PyRangeIterator,
        tuple::{PyTuple, PyTupleIterator, PyTupleRef},
    },
    bytecode::{
        self, ADAPTIVE_COOLDOWN_VALUE, Instruction, LoadAttr, LoadSuperAttr, Opcode, SpecialMethod,
    },
    convert::{ToPyObject, ToPyResult},
    coroutine::Coro,
    exceptions::ExceptionCtor,
    function::{ArgMapping, Either, FuncArgs, PyMethodFlags},
    object::PyAtomicBorrow,
    object::{Traverse, TraverseFn},
    protocol::{PyIter, PyIterReturn, PyMapping},
    scope::Scope,
    sliceable::SliceableSequenceOp,
    stdlib::{_typing, builtins, sys::monitoring},
    types::{PyComparisonOp, PyTypeFlags},
    vm::{Context, PyMethod},
};
use alloc::fmt;
use bstr::ByteSlice;
use core::cell::UnsafeCell;
use core::sync::atomic;
use core::sync::atomic::AtomicPtr;
use core::sync::atomic::Ordering::{Acquire, Relaxed};
use indexmap::IndexMap;
use itertools::Itertools;
use malachite_bigint::BigInt;
use num_traits::Zero;
use rustpython_common::atomic::{PyAtomic, Radium};
use rustpython_common::{
    lock::{OnceCell, PyMutex},
    wtf8::{Wtf8, Wtf8Buf, wtf8_concat},
};
use rustpython_compiler_core::SourceLocation;

pub type FrameRef = PyRef<Frame>;

/// The reason why we might be unwinding a block.
/// This could be return of function, exception being
/// raised, a break or continue being hit, etc..
#[derive(Clone, Debug)]
enum UnwindReason {
    /// We are returning a value from a return statement.
    Returning { value: PyObjectRef },

    /// We hit an exception, so unwind any try-except and finally blocks. The exception should be
    /// on top of the vm exception stack.
    Raising { exception: PyBaseExceptionRef },
}

/// Tracks who owns a frame.
// = `_PyFrameOwner`
#[repr(i8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameOwner {
    /// Being executed by a thread (FRAME_OWNED_BY_THREAD).
    Thread = 0,
    /// Owned by a generator/coroutine (FRAME_OWNED_BY_GENERATOR).
    Generator = 1,
    /// Not executing; held only by a frame object or traceback
    /// (FRAME_OWNED_BY_FRAME_OBJECT).
    FrameObject = 2,
}

impl FrameOwner {
    pub(crate) fn from_i8(v: i8) -> Self {
        match v {
            0 => Self::Thread,
            1 => Self::Generator,
            _ => Self::FrameObject,
        }
    }
}

/// Lock-free mutable storage for frame-internal data.
///
/// # Safety
/// Frame execution is single-threaded: only one thread at a time executes
/// a given frame (enforced by the owner field and generator running flag).
/// External readers (e.g. `f_locals`) are on the same thread as execution
/// (trace callback) or the frame is not executing.
pub(crate) struct FrameUnsafeCell<T>(UnsafeCell<T>);

impl<T> FrameUnsafeCell<T> {
    fn new(value: T) -> Self {
        Self(UnsafeCell::new(value))
    }

    /// # Safety
    /// Caller must ensure no concurrent mutable access.
    #[inline(always)]
    unsafe fn get(&self) -> *mut T {
        self.0.get()
    }
}

// SAFETY: Frame execution is single-threaded. See FrameUnsafeCell doc.
#[cfg(feature = "threading")]
unsafe impl<T: Send> Send for FrameUnsafeCell<T> {}
#[cfg(feature = "threading")]
unsafe impl<T: Send> Sync for FrameUnsafeCell<T> {}

/// Unified storage for local variables and evaluation stack.
///
/// Memory layout (each slot is `usize`-sized):
///   `[0..nlocalsplus)` — fastlocals (`Option<PyObjectRef>`)
///   `[nlocalsplus..nlocalsplus+stack_top)` — active evaluation stack (`Option<PyStackRef>`)
///   `[nlocalsplus+stack_top..capacity)` — unused stack capacity
///
/// Both `Option<PyObjectRef>` and `Option<PyStackRef>` are `usize`-sized
/// (niche optimization on NonNull / NonZeroUsize). The raw storage is
/// `usize` to unify them; typed access is provided through methods.
pub struct LocalsPlus {
    /// Backing storage.
    data: LocalsPlusData,
    /// Number of fastlocals slots (nlocals + ncells + nfrees).
    nlocalsplus: u32,
    /// Current evaluation stack depth.
    stack_top: u32,
}

enum LocalsPlusData {
    /// Heap-allocated storage (generators, coroutines, exec/eval frames).
    Heap(Box<[usize]>),
    /// Data stack allocated storage (normal function calls).
    /// The pointer is valid while the enclosing data stack frame is alive.
    DataStack { ptr: *mut usize, capacity: usize },
}

// SAFETY: DataStack variant points to thread-local DataStack memory.
// Frame execution is single-threaded (enforced by owner field).
#[cfg(feature = "threading")]
unsafe impl Send for LocalsPlusData {}
#[cfg(feature = "threading")]
unsafe impl Sync for LocalsPlusData {}

const _: () = {
    assert!(core::mem::size_of::<Option<PyObjectRef>>() == core::mem::size_of::<usize>());
    // PyStackRef size is checked in object/core.rs
};

impl LocalsPlus {
    /// Create a new heap-backed LocalsPlus.  All slots start as None (0).
    fn new(nlocalsplus: usize, stacksize: usize) -> Self {
        let capacity = nlocalsplus
            .checked_add(stacksize)
            .expect("LocalsPlus capacity overflow");
        let nlocalsplus_u32 = u32::try_from(nlocalsplus).expect("nlocalsplus exceeds u32");
        Self {
            data: LocalsPlusData::Heap(vec![0usize; capacity].into_boxed_slice()),
            nlocalsplus: nlocalsplus_u32,
            stack_top: 0,
        }
    }

    /// Create a new LocalsPlus backed by the thread data stack.
    /// All slots are zero-initialized.
    ///
    /// The caller must call `materialize_localsplus()` when the frame finishes
    /// to migrate data to the heap, then `datastack_pop()` to free the memory.
    fn new_on_datastack(nlocalsplus: usize, stacksize: usize, vm: &VirtualMachine) -> Self {
        let capacity = nlocalsplus
            .checked_add(stacksize)
            .expect("LocalsPlus capacity overflow");
        let byte_size = capacity
            .checked_mul(core::mem::size_of::<usize>())
            .expect("LocalsPlus byte size overflow");
        let nlocalsplus_u32 = u32::try_from(nlocalsplus).expect("nlocalsplus exceeds u32");
        let ptr = vm.datastack_push(byte_size) as *mut usize;
        // Zero-initialize all slots (0 = None for both PyObjectRef and PyStackRef).
        unsafe { core::ptr::write_bytes(ptr, 0, capacity) };
        Self {
            data: LocalsPlusData::DataStack { ptr, capacity },
            nlocalsplus: nlocalsplus_u32,
            stack_top: 0,
        }
    }

    /// Migrate data-stack-backed storage to the heap, preserving all values.
    /// Returns the data stack base pointer for `DataStack::pop()`.
    /// Returns `None` if already heap-backed.
    fn materialize_to_heap(&mut self) -> Option<*mut u8> {
        if let LocalsPlusData::DataStack { ptr, capacity } = &self.data {
            let base = *ptr as *mut u8;
            let heap_data = unsafe { core::slice::from_raw_parts(*ptr, *capacity) }
                .to_vec()
                .into_boxed_slice();
            self.data = LocalsPlusData::Heap(heap_data);
            Some(base)
        } else {
            None
        }
    }

    /// Drop all contained values without freeing the backing storage.
    fn drop_values(&mut self) {
        self.stack_clear();
        let fastlocals = self.fastlocals_mut();
        for slot in fastlocals.iter_mut() {
            let _ = slot.take();
        }
    }

    // -- Data access helpers --

    #[inline(always)]
    fn data_as_slice(&self) -> &[usize] {
        match &self.data {
            LocalsPlusData::Heap(b) => b,
            LocalsPlusData::DataStack { ptr, capacity } => unsafe {
                core::slice::from_raw_parts(*ptr, *capacity)
            },
        }
    }

    #[inline(always)]
    fn data_as_mut_slice(&mut self) -> &mut [usize] {
        match &mut self.data {
            LocalsPlusData::Heap(b) => b,
            LocalsPlusData::DataStack { ptr, capacity } => unsafe {
                core::slice::from_raw_parts_mut(*ptr, *capacity)
            },
        }
    }

    /// Total capacity (fastlocals + stack).
    #[inline(always)]
    fn capacity(&self) -> usize {
        match &self.data {
            LocalsPlusData::Heap(b) => b.len(),
            LocalsPlusData::DataStack { capacity, .. } => *capacity,
        }
    }

    /// Stack capacity (max stack depth).
    #[inline(always)]
    fn stack_capacity(&self) -> usize {
        self.capacity() - self.nlocalsplus as usize
    }

    // -- Fastlocals access --

    /// Immutable access to fastlocals as `Option<PyObjectRef>` slice.
    #[inline(always)]
    fn fastlocals(&self) -> &[Option<PyObjectRef>] {
        let data = self.data_as_slice();
        let ptr = data.as_ptr() as *const Option<PyObjectRef>;
        unsafe { core::slice::from_raw_parts(ptr, self.nlocalsplus as usize) }
    }

    /// Mutable access to fastlocals as `Option<PyObjectRef>` slice.
    #[inline(always)]
    fn fastlocals_mut(&mut self) -> &mut [Option<PyObjectRef>] {
        let nlocalsplus = self.nlocalsplus as usize;
        let data = self.data_as_mut_slice();
        let ptr = data.as_mut_ptr() as *mut Option<PyObjectRef>;
        unsafe { core::slice::from_raw_parts_mut(ptr, nlocalsplus) }
    }

    // -- Stack access --

    /// Current stack depth.
    #[inline(always)]
    fn stack_len(&self) -> usize {
        self.stack_top as usize
    }

    /// Whether the stack is empty.
    #[inline(always)]
    fn stack_is_empty(&self) -> bool {
        self.stack_top == 0
    }

    /// Push a value onto the evaluation stack.
    #[inline(always)]
    fn stack_push(&mut self, val: Option<PyStackRef>) {
        let idx = self.nlocalsplus as usize + self.stack_top as usize;
        debug_assert!(
            idx < self.capacity(),
            "stack overflow: stack_top={}, capacity={}",
            self.stack_top,
            self.stack_capacity()
        );
        let data = self.data_as_mut_slice();
        data[idx] = unsafe { core::mem::transmute::<Option<PyStackRef>, usize>(val) };
        self.stack_top += 1;
    }

    /// Try to push; returns Err if stack is full.
    #[inline(always)]
    fn stack_try_push(&mut self, val: Option<PyStackRef>) -> Result<(), Option<PyStackRef>> {
        let idx = self.nlocalsplus as usize + self.stack_top as usize;
        if idx >= self.capacity() {
            return Err(val);
        }
        let data = self.data_as_mut_slice();
        data[idx] = unsafe { core::mem::transmute::<Option<PyStackRef>, usize>(val) };
        self.stack_top += 1;
        Ok(())
    }

    /// Pop a value from the evaluation stack.
    #[inline(always)]
    fn stack_pop(&mut self) -> Option<PyStackRef> {
        debug_assert!(self.stack_top > 0, "stack underflow");
        self.stack_top -= 1;
        let idx = self.nlocalsplus as usize + self.stack_top as usize;
        let data = self.data_as_mut_slice();
        let raw = core::mem::replace(&mut data[idx], 0);
        unsafe { core::mem::transmute::<usize, Option<PyStackRef>>(raw) }
    }

    /// Immutable view of the active stack as `Option<PyStackRef>` slice.
    #[inline(always)]
    fn stack_as_slice(&self) -> &[Option<PyStackRef>] {
        let data = self.data_as_slice();
        let base = self.nlocalsplus as usize;
        let ptr = unsafe { (data.as_ptr().add(base)) as *const Option<PyStackRef> };
        unsafe { core::slice::from_raw_parts(ptr, self.stack_top as usize) }
    }

    /// Get a reference to a stack slot by index from the bottom.
    #[inline(always)]
    fn stack_index(&self, idx: usize) -> &Option<PyStackRef> {
        debug_assert!(idx < self.stack_top as usize);
        let data = self.data_as_slice();
        let raw_idx = self.nlocalsplus as usize + idx;
        unsafe { &*(data.as_ptr().add(raw_idx) as *const Option<PyStackRef>) }
    }

    /// Get a mutable reference to a stack slot by index from the bottom.
    #[inline(always)]
    fn stack_index_mut(&mut self, idx: usize) -> &mut Option<PyStackRef> {
        debug_assert!(idx < self.stack_top as usize);
        let raw_idx = self.nlocalsplus as usize + idx;
        let data = self.data_as_mut_slice();
        unsafe { &mut *(data.as_mut_ptr().add(raw_idx) as *mut Option<PyStackRef>) }
    }

    /// Get the last stack element (top of stack).
    #[inline(always)]
    fn stack_last(&self) -> Option<&Option<PyStackRef>> {
        if self.stack_top == 0 {
            None
        } else {
            Some(self.stack_index(self.stack_top as usize - 1))
        }
    }

    /// Get mutable reference to the last stack element.
    #[inline(always)]
    fn stack_last_mut(&mut self) -> Option<&mut Option<PyStackRef>> {
        if self.stack_top == 0 {
            None
        } else {
            let idx = self.stack_top as usize - 1;
            Some(self.stack_index_mut(idx))
        }
    }

    /// Swap two stack elements.
    #[inline(always)]
    fn stack_swap(&mut self, a: usize, b: usize) {
        let base = self.nlocalsplus as usize;
        let data = self.data_as_mut_slice();
        data.swap(base + a, base + b);
    }

    /// Truncate the stack to `new_len` elements, dropping excess values.
    fn stack_truncate(&mut self, new_len: usize) {
        debug_assert!(new_len <= self.stack_top as usize);
        while self.stack_top as usize > new_len {
            let _ = self.stack_pop();
        }
    }

    /// Clear the stack, dropping all values.
    fn stack_clear(&mut self) {
        while self.stack_top > 0 {
            let _ = self.stack_pop();
        }
    }

    /// Drain stack elements from `from` to the end, returning an iterator
    /// that yields `Option<PyStackRef>` in forward order and shrinks the stack.
    fn stack_drain(
        &mut self,
        from: usize,
    ) -> impl ExactSizeIterator<Item = Option<PyStackRef>> + '_ {
        let end = self.stack_top as usize;
        debug_assert!(from <= end);
        // Reduce stack_top now; the drain iterator owns the elements.
        self.stack_top = from as u32;
        LocalsPlusStackDrain {
            localsplus: self,
            current: from,
            end,
        }
    }

    /// Extend the stack with values from an iterator.
    fn stack_extend(&mut self, iter: impl Iterator<Item = Option<PyStackRef>>) {
        for val in iter {
            self.stack_push(val);
        }
    }
}

/// Iterator for draining stack elements in forward order.
struct LocalsPlusStackDrain<'a> {
    localsplus: &'a mut LocalsPlus,
    /// Current read position (stack-relative index).
    current: usize,
    /// End position (exclusive, stack-relative index).
    end: usize,
}

impl Iterator for LocalsPlusStackDrain<'_> {
    type Item = Option<PyStackRef>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current >= self.end {
            return None;
        }
        let idx = self.localsplus.nlocalsplus as usize + self.current;
        let data = self.localsplus.data_as_mut_slice();
        let raw = core::mem::replace(&mut data[idx], 0);
        self.current += 1;
        Some(unsafe { core::mem::transmute::<usize, Option<PyStackRef>>(raw) })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.end - self.current;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for LocalsPlusStackDrain<'_> {}

impl Drop for LocalsPlusStackDrain<'_> {
    fn drop(&mut self) {
        while self.current < self.end {
            let idx = self.localsplus.nlocalsplus as usize + self.current;
            let data = self.localsplus.data_as_mut_slice();
            let raw = core::mem::replace(&mut data[idx], 0);
            let _ = unsafe { core::mem::transmute::<usize, Option<PyStackRef>>(raw) };
            self.current += 1;
        }
    }
}

impl Drop for LocalsPlus {
    fn drop(&mut self) {
        // drop_values handles both stack and fastlocals.
        // For DataStack-backed storage, the caller should have called
        // materialize_localsplus() + datastack_pop() before drop.
        // If not (e.g. panic), the DataStack memory is leaked but
        // values are still dropped safely.
        self.drop_values();
    }
}

unsafe impl Traverse for LocalsPlus {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.fastlocals().traverse(tracer_fn);
        self.stack_as_slice().traverse(tracer_fn);
    }
}

/// Lazy locals dict for frames. For NEWLOCALS frames, the dict is
/// only allocated on first access (most function frames never need it).
pub struct FrameLocals {
    inner: OnceCell<ArgMapping>,
}

impl FrameLocals {
    /// Create with an already-initialized locals mapping (non-NEWLOCALS frames).
    fn with_locals(locals: ArgMapping) -> Self {
        let cell = OnceCell::new();
        let _ = cell.set(locals);
        Self { inner: cell }
    }

    /// Create an empty lazy locals (for NEWLOCALS frames).
    /// The dict will be created on first access.
    fn lazy() -> Self {
        Self {
            inner: OnceCell::new(),
        }
    }

    /// Get the locals mapping, creating it lazily if needed.
    #[inline]
    pub fn get_or_create(&self, vm: &VirtualMachine) -> &ArgMapping {
        self.inner
            .get_or_init(|| ArgMapping::from_dict_exact(vm.ctx.new_dict()))
    }

    /// Get the locals mapping if already created.
    #[inline]
    pub fn get(&self) -> Option<&ArgMapping> {
        self.inner.get()
    }

    #[inline]
    pub fn mapping(&self, vm: &VirtualMachine) -> crate::protocol::PyMapping<'_> {
        self.get_or_create(vm).mapping()
    }

    #[inline]
    pub fn clone_mapping(&self, vm: &VirtualMachine) -> ArgMapping {
        self.get_or_create(vm).clone()
    }

    pub fn into_object(&self, vm: &VirtualMachine) -> PyObjectRef {
        self.clone_mapping(vm).into()
    }

    pub fn as_object(&self, vm: &VirtualMachine) -> &PyObject {
        self.get_or_create(vm).obj()
    }
}

impl fmt::Debug for FrameLocals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameLocals")
            .field("initialized", &self.inner.get().is_some())
            .finish()
    }
}

impl Clone for FrameLocals {
    fn clone(&self) -> Self {
        let cell = OnceCell::new();
        if let Some(locals) = self.inner.get() {
            let _ = cell.set(locals.clone());
        }
        Self { inner: cell }
    }
}

unsafe impl Traverse for FrameLocals {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        if let Some(locals) = self.inner.get() {
            locals.traverse(tracer_fn);
        }
    }
}

/// Lightweight execution frame. Not a PyObject.
/// Analogous to CPython's `_PyInterpreterFrame`.
///
/// Currently always embedded inside a `Frame` PyObject via `FrameUnsafeCell`.
/// In future PRs this will be usable independently for normal function calls
/// (allocated on the Rust stack + DataStack), eliminating PyObject overhead.
pub struct InterpreterFrame {
    pub code: PyRef<PyCode>,
    pub func_obj: Option<PyObjectRef>,

    /// Unified storage for local variables and evaluation stack.
    pub(crate) localsplus: LocalsPlus,
    pub locals: FrameLocals,
    pub globals: PyDictRef,
    pub builtins: PyObjectRef,

    /// index of last instruction ran
    pub lasti: PyAtomic<u32>,
    /// tracer function for this frame (usually is None)
    pub trace: PyMutex<PyObjectRef>,

    /// Previous line number for LINE event suppression.
    pub(crate) prev_line: u32,

    // member
    pub trace_lines: PyMutex<bool>,
    pub trace_opcodes: PyMutex<bool>,
    pub temporary_refs: PyMutex<Vec<PyObjectRef>>,
    /// Back-reference to owning generator/coroutine/async generator.
    /// Borrowed reference (not ref-counted) to avoid Generator↔Frame cycle.
    /// Cleared by the generator's Drop impl.
    pub generator: PyAtomicBorrow,
    /// Previous frame in the call chain for signal-safe traceback walking.
    /// Mirrors `_PyInterpreterFrame.previous`.
    pub(crate) previous: AtomicPtr<Frame>,
    /// Who owns this frame. Mirrors `_PyInterpreterFrame.owner`.
    /// Used by `frame.clear()` to reject clearing an executing frame,
    /// even when called from a different thread.
    pub(crate) owner: atomic::AtomicI8,
    /// Set when f_locals is accessed. Cleared after locals_to_fast() sync.
    pub(crate) locals_dirty: atomic::AtomicBool,
    /// Persistent overlay for `frame.f_locals` when hidden locals need a
    /// snapshot separate from the backing locals mapping.
    pub(crate) f_locals_hidden_overlay: PyMutex<Option<PyDictRef>>,
    /// Number of stack entries to pop after set_f_lineno returns to the
    /// execution loop.  set_f_lineno cannot pop directly because the
    /// execution loop holds the state mutex.
    pub(crate) pending_stack_pops: PyAtomic<u32>,
    /// The encoded stack state that set_f_lineno wants to unwind *from*.
    /// Used together with `pending_stack_pops` to identify Except entries
    /// that need special exception-state handling.
    pub(crate) pending_unwind_from_stack: PyAtomic<i64>,
}

/// Python-visible frame object. Currently always wraps an `InterpreterFrame`.
/// Analogous to CPython's `PyFrameObject`.
#[pyclass(module = false, name = "frame", traverse = "manual")]
pub struct Frame {
    pub(crate) iframe: FrameUnsafeCell<InterpreterFrame>,
}

impl core::ops::Deref for Frame {
    type Target = InterpreterFrame;
    /// Transparent access to InterpreterFrame fields.
    ///
    /// # Safety argument
    /// Immutable fields (code, globals, builtins, func_obj, locals) are safe
    /// to access at any time. Atomic/mutex fields (lasti, trace, owner, etc.)
    /// provide their own synchronization. Mutable fields (localsplus, prev_line)
    /// are only mutated during single-threaded execution via `with_exec`.
    #[inline(always)]
    fn deref(&self) -> &InterpreterFrame {
        unsafe { &*self.iframe.get() }
    }
}

impl PyPayload for Frame {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.frame_type
    }
}

unsafe impl Traverse for Frame {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        // SAFETY: GC traversal does not run concurrently with frame execution.
        let iframe = unsafe { &*self.iframe.get() };
        iframe.code.traverse(tracer_fn);
        iframe.func_obj.traverse(tracer_fn);
        iframe.localsplus.traverse(tracer_fn);
        iframe.locals.traverse(tracer_fn);
        iframe.globals.traverse(tracer_fn);
        iframe.builtins.traverse(tracer_fn);
        iframe.trace.traverse(tracer_fn);
        iframe.temporary_refs.traverse(tracer_fn);
        iframe.f_locals_hidden_overlay.traverse(tracer_fn);
    }
}

// Running a frame can result in one of the below:
pub enum ExecutionResult {
    Return(PyObjectRef),
    Yield(PyObjectRef),
}

/// A valid execution result, or an exception
type FrameResult = PyResult<Option<ExecutionResult>>;

impl Frame {
    pub(crate) fn new(
        code: PyRef<PyCode>,
        scope: Scope,
        builtins: PyObjectRef,
        closure: &[PyCellRef],
        func_obj: Option<PyObjectRef>,
        use_datastack: bool,
        vm: &VirtualMachine,
    ) -> Self {
        let nlocalsplus = code.localspluskinds.len();
        let max_stackdepth = code.max_stackdepth as usize;
        let mut localsplus = if use_datastack {
            LocalsPlus::new_on_datastack(nlocalsplus, max_stackdepth, vm)
        } else {
            LocalsPlus::new(nlocalsplus, max_stackdepth)
        };

        // Pre-copy closure cells into free var slots so that locals() works
        // even before COPY_FREE_VARS runs (e.g. coroutine before first send).
        // COPY_FREE_VARS will overwrite these on first execution.
        {
            let nfrees = code.freevars.len();
            if nfrees > 0 {
                let freevar_start = nlocalsplus - nfrees;
                let fastlocals = localsplus.fastlocals_mut();
                for (i, cell) in closure.iter().enumerate() {
                    fastlocals[freevar_start + i] = Some(cell.clone().into());
                }
            }
        }

        // For generators/coroutines, initialize prev_line to the def line
        // so that preamble instructions (RETURN_GENERATOR, POP_TOP) don't
        // fire spurious LINE events.
        let prev_line = if code
            .flags
            .intersects(bytecode::CodeFlags::GENERATOR | bytecode::CodeFlags::COROUTINE)
        {
            code.first_line_number.map_or(0, |line| line.get() as u32)
        } else {
            0
        };

        let iframe = InterpreterFrame {
            localsplus,
            locals: match scope.locals {
                Some(locals) => FrameLocals::with_locals(locals),
                None if code.flags.contains(bytecode::CodeFlags::NEWLOCALS) => FrameLocals::lazy(),
                None => {
                    FrameLocals::with_locals(ArgMapping::from_dict_exact(scope.globals.clone()))
                }
            },
            globals: scope.globals,
            builtins,
            code,
            func_obj,
            lasti: Radium::new(0),
            prev_line,
            trace: PyMutex::new(vm.ctx.none()),
            trace_lines: PyMutex::new(true),
            trace_opcodes: PyMutex::new(false),
            temporary_refs: PyMutex::new(vec![]),
            generator: PyAtomicBorrow::new(),
            previous: AtomicPtr::new(core::ptr::null_mut()),
            owner: atomic::AtomicI8::new(FrameOwner::FrameObject as i8),
            locals_dirty: atomic::AtomicBool::new(false),
            f_locals_hidden_overlay: PyMutex::new(None),
            pending_stack_pops: Default::default(),
            pending_unwind_from_stack: Default::default(),
        };
        Self {
            iframe: FrameUnsafeCell::new(iframe),
        }
    }

    /// Access fastlocals immutably.
    ///
    /// # Safety
    /// Caller must ensure no concurrent mutable access (frame not executing,
    /// or called from the same thread during trace callback).
    #[inline(always)]
    pub unsafe fn fastlocals(&self) -> &[Option<PyObjectRef>] {
        unsafe { (*self.iframe.get()).localsplus.fastlocals() }
    }

    /// Access fastlocals mutably.
    ///
    /// # Safety
    /// Caller must ensure exclusive access (frame not executing).
    #[inline(always)]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn fastlocals_mut(&self) -> &mut [Option<PyObjectRef>] {
        unsafe { (*self.iframe.get()).localsplus.fastlocals_mut() }
    }

    /// Migrate data-stack-backed storage to the heap, preserving all values,
    /// and return the data stack base pointer for `DataStack::pop()`.
    /// Returns `None` if already heap-backed.
    ///
    /// # Safety
    /// Caller must ensure the frame is not executing and the returned
    /// pointer is passed to `VirtualMachine::datastack_pop()`.
    pub(crate) unsafe fn materialize_localsplus(&self) -> Option<*mut u8> {
        unsafe { (*self.iframe.get()).localsplus.materialize_to_heap() }
    }

    /// Clear evaluation stack and state-owned cell/free references.
    /// For full local/cell cleanup, call `clear_locals_and_stack()`.
    pub(crate) fn clear_stack_and_cells(&self) {
        // SAFETY: Called when frame is not executing (generator closed).
        // Cell refs in fastlocals[nlocals..] are cleared by clear_locals_and_stack().
        unsafe {
            (*self.iframe.get()).localsplus.stack_clear();
        }
    }

    /// Clear locals and stack after generator/coroutine close.
    /// Releases references held by the frame, matching _PyFrame_ClearLocals.
    pub(crate) fn clear_locals_and_stack(&self) {
        self.clear_stack_and_cells();
        // SAFETY: Frame is not executing (generator closed).
        let fastlocals = unsafe { (*self.iframe.get()).localsplus.fastlocals_mut() };
        for slot in fastlocals.iter_mut() {
            *slot = None;
        }
        self.f_locals_hidden_overlay.lock().take();
    }

    /// Get cell contents by localsplus index.
    pub(crate) fn get_cell_contents(&self, localsplus_idx: usize) -> Option<PyObjectRef> {
        // SAFETY: Frame not executing; no concurrent mutation.
        let fastlocals = unsafe { (*self.iframe.get()).localsplus.fastlocals() };
        fastlocals
            .get(localsplus_idx)
            .and_then(|slot| slot.as_ref())
            .and_then(|obj| obj.downcast_ref::<PyCell>())
            .and_then(|cell| cell.get())
    }

    /// Store a borrowed back-reference to the owning generator/coroutine.
    /// The caller must ensure the generator outlives the frame.
    pub fn set_generator(&self, generator: &PyObject) {
        self.generator.store(generator);
        self.owner
            .store(FrameOwner::Generator as i8, atomic::Ordering::Release);
    }

    /// Clear the generator back-reference. Called when the generator is finalized.
    pub fn clear_generator(&self) {
        self.generator.clear();
        self.owner
            .store(FrameOwner::FrameObject as i8, atomic::Ordering::Release);
    }

    pub fn current_location(&self) -> SourceLocation {
        self.code.locations[self.lasti() as usize - 1].0
    }

    /// Get the previous frame pointer for signal-safe traceback walking.
    pub fn previous_frame(&self) -> *const Frame {
        self.previous.load(atomic::Ordering::Relaxed)
    }

    pub fn lasti(&self) -> u32 {
        self.lasti.load(Relaxed)
    }

    pub fn set_lasti(&self, val: u32) {
        self.lasti.store(val, Relaxed);
    }

    pub(crate) fn pending_stack_pops(&self) -> u32 {
        self.pending_stack_pops.load(Relaxed)
    }

    pub(crate) fn set_pending_stack_pops(&self, val: u32) {
        self.pending_stack_pops.store(val, Relaxed);
    }

    pub(crate) fn pending_unwind_from_stack(&self) -> i64 {
        self.pending_unwind_from_stack.load(Relaxed)
    }

    pub(crate) fn set_pending_unwind_from_stack(&self, val: i64) {
        self.pending_unwind_from_stack.store(val, Relaxed);
    }

    /// Sync locals dict back to fastlocals. Called before generator/coroutine resume
    /// to apply any modifications made via f_locals.
    pub fn locals_to_fast(&self, vm: &VirtualMachine) -> PyResult<()> {
        if !self.locals_dirty.load(atomic::Ordering::Acquire) {
            return Ok(());
        }
        let code = &**self.code;
        let overlay_locals = self
            .has_active_hidden_locals()
            .then(|| self.f_locals_hidden_overlay.lock().clone())
            .flatten()
            .map(ArgMapping::from_dict_exact);
        let locals_map = overlay_locals
            .as_ref()
            .map_or_else(|| self.locals.mapping(vm), ArgMapping::mapping);
        // SAFETY: Called before generator resume; no concurrent access.
        let fastlocals = unsafe { (*self.iframe.get()).localsplus.fastlocals_mut() };
        for (i, &varname) in code.varnames.iter().enumerate() {
            if i >= fastlocals.len() {
                break;
            }
            match locals_map.subscript(varname, vm) {
                Ok(value) => fastlocals[i] = Some(value),
                Err(e) if e.fast_isinstance(vm.ctx.exceptions.key_error) => {}
                Err(e) => return Err(e),
            }
        }
        self.locals_dirty.store(false, atomic::Ordering::Release);
        Ok(())
    }

    fn has_active_hidden_locals(&self) -> bool {
        use rustpython_compiler_core::bytecode::{CO_FAST_CELL, CO_FAST_FREE, CO_FAST_HIDDEN};
        let code = &**self.code;
        let fastlocals = unsafe { (*self.iframe.get()).localsplus.fastlocals() };
        let is_optimized = code.flags.contains(bytecode::CodeFlags::OPTIMIZED);
        !is_optimized
            && code.localspluskinds.iter().enumerate().any(|(i, &kind)| {
                if kind & CO_FAST_HIDDEN == 0 {
                    return false;
                }
                match fastlocals[i].as_ref() {
                    None => false,
                    Some(obj) => {
                        if kind & (CO_FAST_CELL | CO_FAST_FREE) != 0 {
                            obj.downcast_ref::<PyCell>()
                                .is_none_or(|cell| cell.get().is_some())
                        } else {
                            true
                        }
                    }
                }
            })
    }

    fn sync_visible_locals_to_mapping(
        &self,
        locals_map: PyMapping<'_>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        use rustpython_compiler_core::bytecode::{
            CO_FAST_CELL, CO_FAST_FREE, CO_FAST_HIDDEN, CO_FAST_LOCAL,
        };
        // SAFETY: Either the frame is not executing (caller checked owner),
        // or we're in a trace callback on the same thread that's executing.
        let code = &**self.code;
        let fastlocals = unsafe { (*self.iframe.get()).localsplus.fastlocals() };

        // Iterate through all localsplus slots using localspluskinds
        let nlocalsplus = code.localspluskinds.len();
        let nfrees = code.freevars.len();
        let free_start = nlocalsplus - nfrees;
        let is_optimized = code.flags.contains(bytecode::CodeFlags::OPTIMIZED);

        // Track which non-merged cellvar index we're at
        let mut nonmerged_cell_idx = 0;

        for (i, &kind) in code.localspluskinds.iter().enumerate() {
            if kind & CO_FAST_HIDDEN != 0 {
                // Hidden variables are only skipped when their slot is empty.
                // After a comprehension restores values, they should appear in locals().
                let slot_empty = match fastlocals[i].as_ref() {
                    None => true,
                    Some(obj) => {
                        if kind & (CO_FAST_CELL | CO_FAST_FREE) != 0 {
                            // If it's a PyCell, check if the cell is empty.
                            // If it's a raw value (merged cell during inlined comp), not empty.
                            obj.downcast_ref::<PyCell>()
                                .is_some_and(|cell| cell.get().is_none())
                        } else {
                            false
                        }
                    }
                };
                if slot_empty {
                    continue;
                }
            }

            // Free variables only included for optimized (function-like) scopes.
            // Class/module scopes should not expose free vars in locals().
            if kind == CO_FAST_FREE && !is_optimized {
                continue;
            }

            // Get the name for this slot
            let name = if kind & CO_FAST_LOCAL != 0 {
                code.varnames[i]
            } else if kind & CO_FAST_FREE != 0 {
                code.freevars[i - free_start]
            } else if kind & CO_FAST_CELL != 0 {
                // Non-merged cell: find the name by skipping merged cellvars
                let mut found_name = None;
                let mut skip = nonmerged_cell_idx;
                for cv in code.cellvars.iter() {
                    let is_merged = code.varnames.contains(cv);
                    if !is_merged {
                        if skip == 0 {
                            found_name = Some(*cv);
                            break;
                        }
                        skip -= 1;
                    }
                }
                nonmerged_cell_idx += 1;
                match found_name {
                    Some(n) => n,
                    None => continue,
                }
            } else {
                continue;
            };

            // Get the value
            let value = if kind & (CO_FAST_CELL | CO_FAST_FREE) != 0 {
                // Cell or free var: extract value from PyCell.
                // During inlined comprehensions, a merged cell slot may hold a raw
                // value (not a PyCell) after LOAD_FAST_AND_CLEAR + STORE_FAST.
                fastlocals[i].as_ref().and_then(|obj| {
                    if let Some(cell) = obj.downcast_ref::<PyCell>() {
                        cell.get()
                    } else {
                        Some(obj.clone())
                    }
                })
            } else {
                // Regular local
                fastlocals[i].clone()
            };

            let result = locals_map.ass_subscript(name, value, vm);
            match result {
                Ok(()) => {}
                Err(e) if e.fast_isinstance(vm.ctx.exceptions.key_error) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    pub fn f_locals_mapping(&self, vm: &VirtualMachine) -> PyResult<ArgMapping> {
        if !self.has_active_hidden_locals() {
            self.f_locals_hidden_overlay.lock().take();
            return self.locals(vm);
        }

        let needs_refresh = !self.locals_dirty.load(atomic::Ordering::Acquire);
        let overlay_dict = {
            let mut overlay = self.f_locals_hidden_overlay.lock();
            match overlay.as_ref() {
                Some(dict) => dict.clone(),
                None => {
                    let dict = vm.ctx.new_dict();
                    *overlay = Some(dict.clone());
                    dict
                }
            }
        };
        if needs_refresh {
            PyDict::clear(&overlay_dict);
            let overlay = ArgMapping::from_dict_exact(overlay_dict.clone());
            self.sync_visible_locals_to_mapping(overlay.mapping(), vm)?;
        }
        Ok(ArgMapping::from_dict_exact(overlay_dict))
    }

    pub fn locals(&self, vm: &VirtualMachine) -> PyResult<ArgMapping> {
        if self.has_active_hidden_locals() {
            // Match CPython's locals() behavior for frames with PEP 709 hidden
            // locals: return a fresh snapshot instead of the backing mapping.
            let overlay = ArgMapping::from_dict_exact(vm.ctx.new_dict());
            self.sync_visible_locals_to_mapping(overlay.mapping(), vm)?;
            Ok(overlay)
        } else {
            self.sync_visible_locals_to_mapping(self.locals.mapping(vm), vm)?;
            Ok(self.locals.clone_mapping(vm))
        }
    }
}

impl Py<Frame> {
    #[inline(always)]
    fn with_exec<R>(&self, vm: &VirtualMachine, f: impl FnOnce(ExecutingFrame<'_>) -> R) -> R {
        // SAFETY: Frame execution is single-threaded. Only one thread at a time
        // executes a given frame (enforced by the owner field and generator
        // running flag). Same safety argument as FastLocals (UnsafeCell).
        let iframe = unsafe { &mut *self.iframe.get() };
        let exec = ExecutingFrame {
            code: &iframe.code,
            localsplus: &mut iframe.localsplus,
            locals: &iframe.locals,
            globals: &iframe.globals,
            builtins: &iframe.builtins,
            builtins_dict: if iframe.globals.class().is(vm.ctx.types.dict_type) {
                iframe
                    .builtins
                    .downcast_ref_if_exact::<PyDict>(vm)
                    // SAFETY: downcast_ref_if_exact already verified exact type
                    .map(|d| unsafe { PyExact::ref_unchecked(d) })
            } else {
                None
            },
            lasti: &iframe.lasti,
            object: self,
            prev_line: &mut iframe.prev_line,
            monitoring_mask: 0,
        };
        f(exec)
    }

    // #[cfg_attr(feature = "flame-it", flame("Frame"))]
    pub fn run(&self, vm: &VirtualMachine) -> PyResult<ExecutionResult> {
        self.with_exec(vm, |mut exec| exec.run(vm))
    }

    pub(crate) fn resume(
        &self,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<ExecutionResult> {
        self.with_exec(vm, |mut exec| {
            if let Some(value) = value {
                exec.push_value(value)
            }
            exec.run(vm)
        })
    }

    pub(crate) fn gen_throw(
        &self,
        vm: &VirtualMachine,
        exc_type: PyObjectRef,
        exc_val: PyObjectRef,
        exc_tb: PyObjectRef,
    ) -> PyResult<ExecutionResult> {
        self.with_exec(vm, |mut exec| exec.gen_throw(vm, exc_type, exc_val, exc_tb))
    }

    pub fn yield_from_target(&self) -> Option<PyObjectRef> {
        // If the frame is currently executing (owned by thread), it has no
        // yield-from target to report.
        let owner = FrameOwner::from_i8(self.owner.load(atomic::Ordering::Acquire));
        if owner == FrameOwner::Thread {
            return None;
        }
        // SAFETY: Frame is not executing, so UnsafeCell access is safe.
        let iframe = unsafe { &mut *self.iframe.get() };
        let exec = ExecutingFrame {
            code: &iframe.code,
            localsplus: &mut iframe.localsplus,
            locals: &iframe.locals,
            globals: &iframe.globals,
            builtins: &iframe.builtins,
            builtins_dict: None,
            lasti: &iframe.lasti,
            object: self,
            prev_line: &mut iframe.prev_line,
            monitoring_mask: 0,
        };
        exec.yield_from_target().map(PyObject::to_owned)
    }

    pub fn is_internal_frame(&self) -> bool {
        let code = self.f_code();
        let filename = code.co_filename();
        let filename = filename.as_bytes();
        filename.find(b"importlib").is_some() && filename.find(b"_bootstrap").is_some()
    }

    pub fn next_external_frame(&self, vm: &VirtualMachine) -> Option<FrameRef> {
        let mut frame = self.f_back(vm);
        while let Some(ref f) = frame {
            if !f.is_internal_frame() {
                break;
            }
            frame = f.f_back(vm);
        }
        frame
    }
}

/// An executing frame; borrows mutable frame-internal data for the duration
/// of bytecode execution.
struct ExecutingFrame<'a> {
    code: &'a PyRef<PyCode>,
    localsplus: &'a mut LocalsPlus,
    locals: &'a FrameLocals,
    globals: &'a PyDictRef,
    builtins: &'a PyObjectRef,
    /// Cached downcast of builtins to PyDict for fast LOAD_GLOBAL.
    /// Only set when both globals and builtins are exact dict types (not
    /// subclasses), so that `__missing__` / `__getitem__` overrides are
    /// not bypassed.
    builtins_dict: Option<&'a PyExact<PyDict>>,
    object: &'a Py<Frame>,
    lasti: &'a PyAtomic<u32>,
    prev_line: &'a mut u32,
    /// Cached monitoring events mask. Reloaded at Resume instruction only,
    monitoring_mask: u32,
}

#[inline]
fn specialization_compact_int_value(i: &PyInt, vm: &VirtualMachine) -> Option<isize> {
    // _PyLong_IsCompact(): a one-digit PyLong (base 2^30),
    // i.e. abs(value) <= 2^30 - 1.
    const CPYTHON_COMPACT_LONG_ABS_MAX: i64 = (1i64 << 30) - 1;
    let v = i.try_to_primitive::<i64>(vm).ok()?;
    if (-CPYTHON_COMPACT_LONG_ABS_MAX..=CPYTHON_COMPACT_LONG_ABS_MAX).contains(&v) {
        Some(v as isize)
    } else {
        None
    }
}

#[inline]
fn compact_int_from_obj(obj: &PyObject, vm: &VirtualMachine) -> Option<isize> {
    obj.downcast_ref_if_exact::<PyInt>(vm)
        .and_then(|i| specialization_compact_int_value(i, vm))
}

#[inline]
fn exact_float_from_obj(obj: &PyObject, vm: &VirtualMachine) -> Option<f64> {
    obj.downcast_ref_if_exact::<PyFloat>(vm).map(|f| f.to_f64())
}

#[inline]
fn specialization_nonnegative_compact_index(i: &PyInt, vm: &VirtualMachine) -> Option<usize> {
    // _PyLong_IsNonNegativeCompact(): a single base-2^30 digit.
    const CPYTHON_COMPACT_LONG_MAX: u64 = (1u64 << 30) - 1;
    let v = i.try_to_primitive::<u64>(vm).ok()?;
    if v <= CPYTHON_COMPACT_LONG_MAX {
        Some(v as usize)
    } else {
        None
    }
}

fn release_datastack_frame(frame: &Py<Frame>, vm: &VirtualMachine) {
    unsafe {
        if let Some(base) = frame.materialize_localsplus() {
            vm.datastack_pop(base);
        }
    }
}

type BinaryOpExtendGuard = fn(&PyObject, &PyObject, &VirtualMachine) -> bool;
type BinaryOpExtendAction = fn(&PyObject, &PyObject, &VirtualMachine) -> Option<PyObjectRef>;

struct BinaryOpExtendSpecializationDescr {
    oparg: bytecode::BinaryOperator,
    guard: BinaryOpExtendGuard,
    action: BinaryOpExtendAction,
}

const BINARY_OP_EXTEND_EXTERNAL_CACHE_OFFSET: usize = 1;

#[inline]
fn compactlongs_guard(lhs: &PyObject, rhs: &PyObject, vm: &VirtualMachine) -> bool {
    compact_int_from_obj(lhs, vm).is_some() && compact_int_from_obj(rhs, vm).is_some()
}

macro_rules! bitwise_longs_action {
    ($name:ident, $op:tt) => {
        #[inline]
        fn $name(lhs: &PyObject, rhs: &PyObject, vm: &VirtualMachine) -> Option<PyObjectRef> {
            let lhs_val = compact_int_from_obj(lhs, vm)?;
            let rhs_val = compact_int_from_obj(rhs, vm)?;
            Some(vm.ctx.new_int(lhs_val $op rhs_val).into())
        }
    };
}
bitwise_longs_action!(compactlongs_or, |);
bitwise_longs_action!(compactlongs_and, &);
bitwise_longs_action!(compactlongs_xor, ^);

#[inline]
fn float_compactlong_guard(lhs: &PyObject, rhs: &PyObject, vm: &VirtualMachine) -> bool {
    exact_float_from_obj(lhs, vm).is_some_and(|f| !f.is_nan())
        && compact_int_from_obj(rhs, vm).is_some()
}

#[inline]
fn nonzero_float_compactlong_guard(lhs: &PyObject, rhs: &PyObject, vm: &VirtualMachine) -> bool {
    float_compactlong_guard(lhs, rhs, vm) && compact_int_from_obj(rhs, vm).is_some_and(|v| v != 0)
}

macro_rules! float_long_action {
    ($name:ident, $op:tt) => {
        #[inline]
        fn $name(lhs: &PyObject, rhs: &PyObject, vm: &VirtualMachine) -> Option<PyObjectRef> {
            let lhs_val = exact_float_from_obj(lhs, vm)?;
            let rhs_val = compact_int_from_obj(rhs, vm)?;
            Some(vm.ctx.new_float(lhs_val $op rhs_val as f64).into())
        }
    };
}
float_long_action!(float_compactlong_add, +);
float_long_action!(float_compactlong_subtract, -);
float_long_action!(float_compactlong_multiply, *);
float_long_action!(float_compactlong_true_div, /);

#[inline]
fn compactlong_float_guard(lhs: &PyObject, rhs: &PyObject, vm: &VirtualMachine) -> bool {
    compact_int_from_obj(lhs, vm).is_some()
        && exact_float_from_obj(rhs, vm).is_some_and(|f| !f.is_nan())
}

#[inline]
fn nonzero_compactlong_float_guard(lhs: &PyObject, rhs: &PyObject, vm: &VirtualMachine) -> bool {
    compactlong_float_guard(lhs, rhs, vm) && exact_float_from_obj(rhs, vm).is_some_and(|f| f != 0.0)
}

macro_rules! long_float_action {
    ($name:ident, $op:tt) => {
        #[inline]
        fn $name(lhs: &PyObject, rhs: &PyObject, vm: &VirtualMachine) -> Option<PyObjectRef> {
            let lhs_val = compact_int_from_obj(lhs, vm)?;
            let rhs_val = exact_float_from_obj(rhs, vm)?;
            Some(vm.ctx.new_float(lhs_val as f64 $op rhs_val).into())
        }
    };
}
long_float_action!(compactlong_float_add, +);
long_float_action!(compactlong_float_subtract, -);
long_float_action!(compactlong_float_multiply, *);
long_float_action!(compactlong_float_true_div, /);

static BINARY_OP_EXTEND_DESCRIPTORS: &[BinaryOpExtendSpecializationDescr] = &[
    // long-long arithmetic
    BinaryOpExtendSpecializationDescr {
        oparg: bytecode::BinaryOperator::Or,
        guard: compactlongs_guard,
        action: compactlongs_or,
    },
    BinaryOpExtendSpecializationDescr {
        oparg: bytecode::BinaryOperator::And,
        guard: compactlongs_guard,
        action: compactlongs_and,
    },
    BinaryOpExtendSpecializationDescr {
        oparg: bytecode::BinaryOperator::Xor,
        guard: compactlongs_guard,
        action: compactlongs_xor,
    },
    BinaryOpExtendSpecializationDescr {
        oparg: bytecode::BinaryOperator::InplaceOr,
        guard: compactlongs_guard,
        action: compactlongs_or,
    },
    BinaryOpExtendSpecializationDescr {
        oparg: bytecode::BinaryOperator::InplaceAnd,
        guard: compactlongs_guard,
        action: compactlongs_and,
    },
    BinaryOpExtendSpecializationDescr {
        oparg: bytecode::BinaryOperator::InplaceXor,
        guard: compactlongs_guard,
        action: compactlongs_xor,
    },
    // float-long arithmetic
    BinaryOpExtendSpecializationDescr {
        oparg: bytecode::BinaryOperator::Add,
        guard: float_compactlong_guard,
        action: float_compactlong_add,
    },
    BinaryOpExtendSpecializationDescr {
        oparg: bytecode::BinaryOperator::Subtract,
        guard: float_compactlong_guard,
        action: float_compactlong_subtract,
    },
    BinaryOpExtendSpecializationDescr {
        oparg: bytecode::BinaryOperator::TrueDivide,
        guard: nonzero_float_compactlong_guard,
        action: float_compactlong_true_div,
    },
    BinaryOpExtendSpecializationDescr {
        oparg: bytecode::BinaryOperator::Multiply,
        guard: float_compactlong_guard,
        action: float_compactlong_multiply,
    },
    // long-float arithmetic
    BinaryOpExtendSpecializationDescr {
        oparg: bytecode::BinaryOperator::Add,
        guard: compactlong_float_guard,
        action: compactlong_float_add,
    },
    BinaryOpExtendSpecializationDescr {
        oparg: bytecode::BinaryOperator::Subtract,
        guard: compactlong_float_guard,
        action: compactlong_float_subtract,
    },
    BinaryOpExtendSpecializationDescr {
        oparg: bytecode::BinaryOperator::TrueDivide,
        guard: nonzero_compactlong_float_guard,
        action: compactlong_float_true_div,
    },
    BinaryOpExtendSpecializationDescr {
        oparg: bytecode::BinaryOperator::Multiply,
        guard: compactlong_float_guard,
        action: compactlong_float_multiply,
    },
];

impl fmt::Debug for ExecutingFrame<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecutingFrame")
            .field("code", self.code)
            .field("stack_len", &self.localsplus.stack_len())
            .finish()
    }
}

impl ExecutingFrame<'_> {
    #[inline]
    fn monitoring_disabled_for_code(&self, vm: &VirtualMachine) -> bool {
        self.code.is(&vm.ctx.init_cleanup_code)
    }

    fn specialization_new_init_cleanup_frame(&self, vm: &VirtualMachine) -> FrameRef {
        Frame::new(
            vm.ctx.init_cleanup_code.clone(),
            Scope::new(
                Some(ArgMapping::from_dict_exact(vm.ctx.new_dict())),
                self.globals.clone(),
            ),
            self.builtins.clone(),
            &[],
            None,
            true,
            vm,
        )
        .into_ref(&vm.ctx)
    }

    fn specialization_run_init_cleanup_shim(
        &self,
        new_obj: PyObjectRef,
        init_func: &Py<PyFunction>,
        pos_args: Vec<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let shim = self.specialization_new_init_cleanup_frame(vm);
        let shim_result = vm.with_frame_untraced(shim.clone(), |shim| {
            shim.with_exec(vm, |mut exec| exec.push_value(new_obj.clone()));

            let mut all_args = Vec::with_capacity(pos_args.len() + 1);
            all_args.push(new_obj.clone());
            all_args.extend(pos_args);

            let init_frame = init_func.prepare_exact_args_frame(all_args, vm);
            let init_result = vm.run_frame(init_frame.clone());
            release_datastack_frame(&init_frame, vm);
            let init_result = init_result?;

            shim.with_exec(vm, |mut exec| exec.push_value(init_result));
            match shim.run(vm)? {
                ExecutionResult::Return(value) => Ok(value),
                ExecutionResult::Yield(_) => unreachable!("_Py_InitCleanup shim cannot yield"),
            }
        });
        release_datastack_frame(&shim, vm);
        shim_result
    }

    #[inline(always)]
    fn update_lasti(&mut self, f: impl FnOnce(&mut u32)) {
        let mut val = self.lasti.load(Relaxed);
        f(&mut val);
        self.lasti.store(val, Relaxed);
    }

    #[inline(always)]
    fn lasti(&self) -> u32 {
        self.lasti.load(Relaxed)
    }

    /// Access the PyCellRef at the given localsplus index.
    #[inline(always)]
    fn cell_ref(&self, localsplus_idx: usize) -> &PyCell {
        let fastlocals = self.localsplus.fastlocals();
        let slot = &fastlocals[localsplus_idx];
        slot.as_ref()
            .expect("cell slot empty")
            .downcast_ref::<PyCell>()
            .expect("cell slot is not a PyCell")
    }

    /// Perform deferred stack unwinding after set_f_lineno.
    ///
    /// set_f_lineno cannot pop the value stack directly because the execution
    /// loop holds the state mutex.  Instead it records the work in
    /// `pending_stack_pops` / `pending_unwind_from_stack` and we execute it
    /// here, inside the execution loop where we already own the state.
    fn unwind_stack_for_lineno(&mut self, pop_count: usize, from_stack: i64, vm: &VirtualMachine) {
        let mut cur_stack = from_stack;
        for _ in 0..pop_count {
            let val = self.pop_value_opt();
            if stack_analysis::top_of_stack(cur_stack) == stack_analysis::Kind::Except as i64
                && let Some(exc_obj) = val
            {
                if vm.is_none(&exc_obj) {
                    vm.set_exception(None);
                } else {
                    let exc = exc_obj.downcast::<PyBaseException>().ok();
                    vm.set_exception(exc);
                }
            }
            cur_stack = stack_analysis::pop_value(cur_stack);
        }
    }

    /// Fire 'exception' trace event (sys.settrace) with (type, value, traceback) tuple.
    /// Matches `_PyEval_MonitorRaise` → `PY_MONITORING_EVENT_RAISE` →
    /// `sys_trace_exception_func` in legacy_tracing.c.
    fn fire_exception_trace(&self, exc: &PyBaseExceptionRef, vm: &VirtualMachine) -> PyResult<()> {
        if vm.use_tracing.get() && !vm.is_none(&self.object.trace.lock()) {
            let exc_type: PyObjectRef = exc.class().to_owned().into();
            let exc_value: PyObjectRef = exc.clone().into();
            let exc_tb: PyObjectRef = exc
                .__traceback__()
                .map(|tb| -> PyObjectRef { tb.into() })
                .unwrap_or_else(|| vm.ctx.none());
            let tuple = vm.ctx.new_tuple(vec![exc_type, exc_value, exc_tb]).into();
            vm.trace_event(crate::protocol::TraceEvent::Exception, Some(tuple))?;
        }
        Ok(())
    }

    fn run(&mut self, vm: &VirtualMachine) -> PyResult<ExecutionResult> {
        flame_guard!(format!(
            "Frame::run({obj_name})",
            obj_name = self.code.obj_name
        ));
        // Execute until return or exception:
        let mut arg_state = bytecode::OpArgState::default();
        loop {
            let idx = self.lasti() as usize;
            // Advance lasti past the current instruction BEFORE firing the
            // line event.  This ensures that f_lineno (which reads
            // locations[lasti - 1]) returns the line of the instruction
            // being traced, not the previous one.
            self.update_lasti(|i| *i += 1);

            // Fire 'line' trace event when line number changes.
            // Only fire if this frame has a per-frame trace function set
            // (frames entered before sys.settrace() have trace=None).
            // Skip RESUME – it should not generate user-visible line events.
            if vm.use_tracing.get()
                && !vm.is_none(&self.object.trace.lock())
                && !matches!(
                    self.code.instructions.read_op(idx),
                    Instruction::Resume { .. } | Instruction::InstrumentedResume
                )
                && let Some((loc, _)) = self.code.locations.get(idx)
                && loc.line.get() as u32 != *self.prev_line
            {
                *self.prev_line = loc.line.get() as u32;
                vm.trace_event(crate::protocol::TraceEvent::Line, None)?;
                // Trace callback may have changed lasti via set_f_lineno.
                // Re-read and restart the loop from the new position.
                if self.lasti() != (idx as u32 + 1) {
                    // set_f_lineno defers stack unwinding because we hold
                    // the state mutex.  Perform it now.
                    let pops = self.object.pending_stack_pops();
                    if pops > 0 {
                        let from_stack = self.object.pending_unwind_from_stack();
                        self.unwind_stack_for_lineno(pops as usize, from_stack, vm);
                        self.object.set_pending_stack_pops(0);
                    }
                    arg_state.reset();
                    continue;
                }
            }
            let op = self.code.instructions.read_op(idx);
            let arg = arg_state.extend(self.code.instructions.read_arg(idx));
            let mut do_extend_arg = false;
            let caches = op.cache_entries();

            // Update prev_line only when tracing or monitoring is active.
            // When neither is enabled, prev_line is stale but unused.
            if vm.use_tracing.get() {
                if !matches!(
                    op.into(),
                    Opcode::Resume | Opcode::ExtendedArg | Opcode::InstrumentedLine
                ) && let Some((loc, _)) = self.code.locations.get(idx)
                {
                    *self.prev_line = loc.line.get() as u32;
                }

                // Fire 'opcode' trace event for sys.settrace when f_trace_opcodes
                // is set. Skip RESUME and ExtendedArg
                // (_Py_call_instrumentation_instruction).
                if !vm.is_none(&self.object.trace.lock())
                    && *self.object.trace_opcodes.lock()
                    && !matches!(
                        op.into(),
                        Opcode::Resume | Opcode::InstrumentedResume | Opcode::ExtendedArg
                    )
                {
                    vm.trace_event(crate::protocol::TraceEvent::Opcode, None)?;
                }
            }

            if vm.eval_breaker_tripped()
                && let Err(exception) = vm.check_signals()
            {
                #[cold]
                fn handle_signal_exception(
                    frame: &mut ExecutingFrame<'_>,
                    exception: PyBaseExceptionRef,
                    idx: usize,
                    vm: &VirtualMachine,
                ) -> FrameResult {
                    if let Some((loc, _end_loc)) = frame.code.locations.get(idx) {
                        let next = exception.__traceback__();
                        let new_traceback = PyTraceback::new(
                            next,
                            frame.object.to_owned(),
                            idx as u32 * 2,
                            loc.line,
                        );
                        exception.set_traceback_typed(Some(new_traceback.into_ref(&vm.ctx)));
                    }
                    vm.contextualize_exception(&exception);
                    frame.unwind_blocks(vm, UnwindReason::Raising { exception })
                }
                match handle_signal_exception(self, exception, idx, vm) {
                    Ok(None) => {}
                    Ok(Some(value)) => {
                        break Ok(value);
                    }
                    Err(exception) => {
                        break Err(exception);
                    }
                }
                continue;
            }
            let lasti_before = self.lasti();
            let result = self.execute_instruction(op, arg, &mut do_extend_arg, vm);
            // Skip inline cache entries if instruction fell through (no jump).
            if caches > 0 && self.lasti() == lasti_before {
                self.update_lasti(|i| *i += caches as u32);
            }
            match result {
                Ok(None) => {}
                Ok(Some(value)) => {
                    break Ok(value);
                }
                // Instruction raised an exception
                Err(exception) => {
                    #[cold]
                    fn handle_exception(
                        frame: &mut ExecutingFrame<'_>,
                        exception: PyBaseExceptionRef,
                        idx: usize,
                        is_reraise: bool,
                        is_new_raise: bool,
                        vm: &VirtualMachine,
                    ) -> FrameResult {
                        // 1. Extract traceback from exception's '__traceback__' attr.
                        // 2. Add new entry with current execution position (filename, lineno, code_object) to traceback.
                        // 3. First, try to find handler in exception table

                        // RERAISE instructions should not add traceback entries - they're just
                        // re-raising an already-processed exception
                        if !is_reraise {
                            // Check if the exception already has traceback entries before
                            // we add ours. If it does, it was propagated from a callee
                            // function and we should not re-contextualize it.
                            let had_prior_traceback = exception.__traceback__().is_some();

                            // PyTraceBack_Here always adds a new entry without
                            // checking for duplicates. Each time an exception passes through
                            // a frame (e.g., in a loop with repeated raise statements),
                            // a new traceback entry is added.
                            if let Some((loc, _end_loc)) = frame.code.locations.get(idx) {
                                let next = exception.__traceback__();

                                let new_traceback = PyTraceback::new(
                                    next,
                                    frame.object.to_owned(),
                                    idx as u32 * 2,
                                    loc.line,
                                );
                                vm_trace!(
                                    "Adding to traceback: {:?} {:?}",
                                    new_traceback,
                                    loc.line
                                );
                                exception
                                    .set_traceback_typed(Some(new_traceback.into_ref(&vm.ctx)));
                            }

                            // _PyErr_SetObject sets __context__ only when the exception
                            // is first raised. When an exception propagates through frames,
                            // __context__ must not be overwritten. We contextualize when:
                            // - It's an explicit raise (raise/raise from)
                            // - The exception had no prior traceback (originated here)
                            if is_new_raise || !had_prior_traceback {
                                vm.contextualize_exception(&exception);
                            }
                        }

                        // Use exception table for zero-cost exception handling
                        frame.unwind_blocks(vm, UnwindReason::Raising { exception })
                    }

                    // Check if this is a RERAISE instruction
                    // Both AnyInstruction::Raise { kind: Reraise/ReraiseFromStack } and
                    // AnyInstruction::Reraise are reraise operations that should not add
                    // new traceback entries.
                    // EndAsyncFor and CleanupThrow also re-raise non-matching exceptions.
                    let is_reraise = match op {
                        Instruction::RaiseVarargs { argc: kind } => matches!(
                            kind.get(arg),
                            bytecode::RaiseKind::BareRaise | bytecode::RaiseKind::ReraiseFromStack
                        ),
                        Instruction::Reraise { .. }
                        | Instruction::EndAsyncFor
                        | Instruction::CleanupThrow => true,
                        _ => false,
                    };

                    // Explicit raise instructions (raise/raise from) - these always
                    // need contextualization even if the exception has prior traceback
                    let is_new_raise = matches!(
                        op,
                        Instruction::RaiseVarargs { argc: kind }
                            if matches!(
                                kind.get(arg),
                                bytecode::RaiseKind::Raise | bytecode::RaiseKind::RaiseCause
                            )
                    );

                    // Fire RAISE or RERAISE monitoring event.
                    // If the callback raises, replace the original exception.
                    let exception = {
                        let mon_events = vm.state.monitoring_events.load();
                        if is_reraise {
                            if mon_events & monitoring::EVENT_RERAISE != 0 {
                                let offset = idx as u32 * 2;
                                let exc_obj: PyObjectRef = exception.clone().into();
                                match monitoring::fire_reraise(vm, self.code, offset, &exc_obj) {
                                    Ok(()) => exception,
                                    Err(monitor_exc) => monitor_exc,
                                }
                            } else {
                                exception
                            }
                        } else if mon_events & monitoring::EVENT_RAISE != 0 {
                            let offset = idx as u32 * 2;
                            let exc_obj: PyObjectRef = exception.clone().into();
                            match monitoring::fire_raise(vm, self.code, offset, &exc_obj) {
                                Ok(()) => exception,
                                Err(monitor_exc) => monitor_exc,
                            }
                        } else {
                            exception
                        }
                    };

                    // Fire 'exception' trace event for sys.settrace.
                    // Only for new raises, not re-raises (matching the
                    // `error` label that calls _PyEval_MonitorRaise).
                    if !is_reraise {
                        self.fire_exception_trace(&exception, vm)?;
                    }

                    match handle_exception(self, exception, idx, is_reraise, is_new_raise, vm) {
                        Ok(None) => {}
                        Ok(Some(result)) => break Ok(result),
                        Err(exception) => {
                            // Fire PY_UNWIND: exception escapes this frame
                            let exception = if vm.state.monitoring_events.load()
                                & monitoring::EVENT_PY_UNWIND
                                != 0
                            {
                                let offset = idx as u32 * 2;
                                let exc_obj: PyObjectRef = exception.clone().into();
                                match monitoring::fire_py_unwind(vm, self.code, offset, &exc_obj) {
                                    Ok(()) => exception,
                                    Err(monitor_exc) => monitor_exc,
                                }
                            } else {
                                exception
                            };

                            // Restore lasti from traceback so frame.f_lineno matches tb_lineno
                            // The traceback was created with the correct lasti when exception
                            // was first raised, but frame.lasti may have changed during cleanup
                            if let Some(tb) = exception.__traceback__()
                                && core::ptr::eq::<Py<Frame>>(&*tb.frame, self.object)
                            {
                                // This traceback entry is for this frame - restore its lasti
                                // tb.lasti is in bytes (idx * 2), convert back to instruction index
                                self.update_lasti(|i| *i = tb.lasti / 2);
                            }
                            break Err(exception);
                        }
                    }
                }
            }
            if !do_extend_arg {
                arg_state.reset()
            }
        }
    }

    fn yield_from_target(&self) -> Option<&PyObject> {
        // checks gi_frame_state == FRAME_SUSPENDED_YIELD_FROM
        // which is set when YIELD_VALUE with oparg >= 1 is executed.
        // In RustPython, we check:
        // 1. lasti points to RESUME (after YIELD_VALUE)
        // 2. The previous instruction was YIELD_VALUE with arg >= 1
        // 3. Stack top is the delegate (receiver)
        //
        // First check if stack is empty - if so, we can't be in yield-from
        if self.localsplus.stack_is_empty() {
            return None;
        }
        let lasti = self.lasti() as usize;
        if let Some(unit) = self.code.instructions.get(lasti) {
            match &unit.op {
                Instruction::Send { .. } => return Some(self.top_value()),
                Instruction::Resume { .. } | Instruction::InstrumentedResume => {
                    // Check if previous instruction was YIELD_VALUE with arg >= 1
                    // This indicates yield-from/await context
                    if lasti > 0
                        && let Some(prev_unit) = self.code.instructions.get(lasti - 1)
                        && matches!(
                            &prev_unit.op.into(),
                            Opcode::YieldValue | Opcode::InstrumentedYieldValue
                        )
                    {
                        // YIELD_VALUE arg: 0 = direct yield, >= 1 = yield-from/await
                        // OpArgByte.0 is the raw byte value
                        if u8::from(prev_unit.arg) >= 1 {
                            // In yield-from/await context, delegate is on top of stack
                            return Some(self.top_value());
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Handle throw() on a generator/coroutine.
    fn gen_throw(
        &mut self,
        vm: &VirtualMachine,
        exc_type: PyObjectRef,
        exc_val: PyObjectRef,
        exc_tb: PyObjectRef,
    ) -> PyResult<ExecutionResult> {
        self.monitoring_mask = vm.state.monitoring_events.load();
        if let Some(jen) = self.yield_from_target() {
            // Check if the exception is GeneratorExit (type or instance).
            // For GeneratorExit, close the sub-iterator instead of throwing.
            let is_gen_exit = if let Some(typ) = exc_type.downcast_ref::<PyType>() {
                typ.fast_issubclass(vm.ctx.exceptions.generator_exit)
            } else {
                exc_type.fast_isinstance(vm.ctx.exceptions.generator_exit)
            };

            if is_gen_exit {
                // gen_close_iter: close the sub-iterator
                let close_result = if let Some(coro) = self.builtin_coro(jen) {
                    coro.close(jen, vm).map(|_| ())
                } else if let Some(close_meth) = vm.get_attribute_opt(jen.to_owned(), "close")? {
                    close_meth.call((), vm).map(|_| ())
                } else {
                    Ok(())
                };
                if let Err(err) = close_result {
                    let idx = self.lasti().saturating_sub(1) as usize;
                    if idx < self.code.locations.len() {
                        let (loc, _end_loc) = self.code.locations[idx];
                        let next = err.__traceback__();
                        let new_traceback = PyTraceback::new(
                            next,
                            self.object.to_owned(),
                            idx as u32 * 2,
                            loc.line,
                        );
                        err.set_traceback_typed(Some(new_traceback.into_ref(&vm.ctx)));
                    }

                    self.push_value(vm.ctx.none());
                    vm.contextualize_exception(&err);
                    return match self.unwind_blocks(vm, UnwindReason::Raising { exception: err }) {
                        Ok(None) => {
                            *self.prev_line = 0;
                            self.run(vm)
                        }
                        Ok(Some(result)) => Ok(result),
                        Err(exception) => Err(exception),
                    };
                }
                // Fall through to throw_here to raise GeneratorExit in the generator
            } else {
                // For non-GeneratorExit, delegate throw to sub-iterator
                let thrower = if let Some(coro) = self.builtin_coro(jen) {
                    Some(Either::A(coro))
                } else {
                    vm.get_attribute_opt(jen.to_owned(), "throw")?
                        .map(Either::B)
                };
                if let Some(thrower) = thrower {
                    let ret = match thrower {
                        Either::A(coro) => coro
                            .throw(jen, exc_type, exc_val, exc_tb, vm)
                            .to_pyresult(vm),
                        Either::B(meth) => meth.call((exc_type, exc_val, exc_tb), vm),
                    };
                    return ret.map(ExecutionResult::Yield).or_else(|err| {
                        // Add traceback entry for the yield-from/await point.
                        // gen_send_ex2 resumes the frame with a pending exception,
                        // which goes through error: → PyTraceBack_Here. We add the
                        // entry here before calling unwind_blocks.
                        let idx = self.lasti().saturating_sub(1) as usize;
                        if idx < self.code.locations.len() {
                            let (loc, _end_loc) = self.code.locations[idx];
                            let next = err.__traceback__();
                            let new_traceback = PyTraceback::new(
                                next,
                                self.object.to_owned(),
                                idx as u32 * 2,
                                loc.line,
                            );
                            err.set_traceback_typed(Some(new_traceback.into_ref(&vm.ctx)));
                        }

                        self.push_value(vm.ctx.none());
                        vm.contextualize_exception(&err);
                        match self.unwind_blocks(vm, UnwindReason::Raising { exception: err }) {
                            Ok(None) => {
                                *self.prev_line = 0;
                                self.run(vm)
                            }
                            Ok(Some(result)) => Ok(result),
                            Err(exception) => Err(exception),
                        }
                    });
                }
            }
        }
        // throw_here: no delegate has throw method, or not in yield-from
        // Validate the exception type first. Invalid types propagate directly to
        // the caller. Valid types with failed instantiation (e.g. __new__ returns
        // wrong type) get thrown into the generator via PyErr_SetObject path.
        let ctor = ExceptionCtor::try_from_object(vm, exc_type)?;
        let exception = match ctor.instantiate_value(exc_val, vm) {
            Ok(exc) => {
                if let Some(tb) = Option::<PyRef<PyTraceback>>::try_from_object(vm, exc_tb)? {
                    exc.set_traceback_typed(Some(tb));
                }
                exc
            }
            Err(err) => err,
        };

        // Add traceback entry for the generator frame at the yield site
        let idx = self.lasti().saturating_sub(1) as usize;
        if idx < self.code.locations.len() {
            let (loc, _end_loc) = self.code.locations[idx];
            let next = exception.__traceback__();
            let new_traceback =
                PyTraceback::new(next, self.object.to_owned(), idx as u32 * 2, loc.line);
            exception.set_traceback_typed(Some(new_traceback.into_ref(&vm.ctx)));
        }

        // Fire PY_THROW and RAISE events before raising the exception.
        // If a monitoring callback fails, its exception replaces the original.
        let exception = {
            let mon_events = vm.state.monitoring_events.load();
            let exception = if mon_events & monitoring::EVENT_PY_THROW != 0 {
                let offset = idx as u32 * 2;
                let exc_obj: PyObjectRef = exception.clone().into();
                match monitoring::fire_py_throw(vm, self.code, offset, &exc_obj) {
                    Ok(()) => exception,
                    Err(monitor_exc) => monitor_exc,
                }
            } else {
                exception
            };
            if mon_events & monitoring::EVENT_RAISE != 0 {
                let offset = idx as u32 * 2;
                let exc_obj: PyObjectRef = exception.clone().into();
                match monitoring::fire_raise(vm, self.code, offset, &exc_obj) {
                    Ok(()) => exception,
                    Err(monitor_exc) => monitor_exc,
                }
            } else {
                exception
            }
        };

        // when raising an exception, set __context__ to the current exception
        // This is done in _PyErr_SetObject
        vm.contextualize_exception(&exception);

        // always pushes Py_None before calling gen_send_ex with exc=1
        // This is needed for exception handler to have correct stack state
        self.push_value(vm.ctx.none());

        match self.unwind_blocks(vm, UnwindReason::Raising { exception }) {
            Ok(None) => {
                // Reset prev_line so that the first instruction in the handler
                // fires a LINE event. In CPython, gen_send_ex re-enters the
                // eval loop which reinitializes its local prev_instr tracker.
                *self.prev_line = 0;
                self.run(vm)
            }
            Ok(Some(result)) => Ok(result),
            Err(exception) => {
                // Fire PY_UNWIND: exception escapes the generator frame.
                let exception =
                    if vm.state.monitoring_events.load() & monitoring::EVENT_PY_UNWIND != 0 {
                        let offset = idx as u32 * 2;
                        let exc_obj: PyObjectRef = exception.clone().into();
                        match monitoring::fire_py_unwind(vm, self.code, offset, &exc_obj) {
                            Ok(()) => exception,
                            Err(monitor_exc) => monitor_exc,
                        }
                    } else {
                        exception
                    };
                Err(exception)
            }
        }
    }

    fn unbound_cell_exception(
        &self,
        localsplus_idx: usize,
        vm: &VirtualMachine,
    ) -> PyBaseExceptionRef {
        use rustpython_compiler_core::bytecode::CO_FAST_FREE;
        let kind = self
            .code
            .localspluskinds
            .get(localsplus_idx)
            .copied()
            .unwrap_or(0);
        if kind & CO_FAST_FREE != 0 {
            let name = self.localsplus_name(localsplus_idx);
            vm.new_name_error(
                format!("cannot access free variable '{name}' where it is not associated with a value in enclosing scope"),
                name.to_owned(),
            )
        } else {
            // Both merged cells (LOCAL|CELL) and non-merged cells get unbound local error
            let name = self.localsplus_name(localsplus_idx);
            vm.new_exception_msg(
                vm.ctx.exceptions.unbound_local_error.to_owned(),
                format!("local variable '{name}' referenced before assignment").into(),
            )
        }
    }

    /// Get the variable name for a localsplus index.
    fn localsplus_name(&self, idx: usize) -> &'static PyStrInterned {
        use rustpython_compiler_core::bytecode::{CO_FAST_CELL, CO_FAST_FREE, CO_FAST_LOCAL};
        let nlocals = self.code.varnames.len();
        let kind = self.code.localspluskinds.get(idx).copied().unwrap_or(0);
        if kind & CO_FAST_LOCAL != 0 {
            // Merged cell or regular local: name is in varnames
            self.code.varnames[idx]
        } else if kind & CO_FAST_FREE != 0 {
            // Free var: slots are at the end of localsplus
            let nlocalsplus = self.code.localspluskinds.len();
            let nfrees = self.code.freevars.len();
            let free_start = nlocalsplus - nfrees;
            self.code.freevars[idx - free_start]
        } else if kind & CO_FAST_CELL != 0 {
            // Non-merged cell: count how many non-merged cell slots are before
            // this index to find the corresponding cellvars entry.
            // Non-merged cellvars appear in their original order (skipping merged ones).
            let nonmerged_pos = self.code.localspluskinds[nlocals..idx]
                .iter()
                .filter(|&&k| k == CO_FAST_CELL)
                .count();
            // Skip merged cellvars to find the right one
            let mut cv_idx = 0;
            let mut nonmerged_count = 0;
            for (i, name) in self.code.cellvars.iter().enumerate() {
                let is_merged = self.code.varnames.contains(name);
                if !is_merged {
                    if nonmerged_count == nonmerged_pos {
                        cv_idx = i;
                        break;
                    }
                    nonmerged_count += 1;
                }
            }
            self.code.cellvars[cv_idx]
        } else {
            self.code.varnames[idx]
        }
    }

    /// Execute a single instruction.
    #[inline(always)]
    fn execute_instruction(
        &mut self,
        instruction: Instruction,
        arg: bytecode::OpArg,
        extend_arg: &mut bool,
        vm: &VirtualMachine,
    ) -> FrameResult {
        flame_guard!(format!(
            "Frame::execute_instruction({instruction:?} {arg:?})"
        ));

        #[cfg(feature = "vm-tracing-logging")]
        {
            trace!("=======");
            /* TODO:
            for frame in self.frames.iter() {
                trace!("  {:?}", frame);
            }
            */
            trace!("  {:#?}", self);
            trace!("  Executing opcode: {instruction:?} {arg:?}",);
            trace!("=======");
        }

        #[cold]
        fn name_error(name: &'static PyStrInterned, vm: &VirtualMachine) -> PyBaseExceptionRef {
            vm.new_name_error(format!("name '{name}' is not defined"), name.to_owned())
        }

        match instruction {
            Instruction::BinaryOp { op } => {
                let op_val = op.get(arg);
                self.adaptive(|s, ii, cb| s.specialize_binary_op(vm, op_val, ii, cb));
                self.execute_bin_op(vm, op_val)
            }
            // Super-instruction for BINARY_OP_ADD_UNICODE + STORE_FAST targeting
            // the left local, matching BINARY_OP_INPLACE_ADD_UNICODE shape.
            Instruction::BinaryOpInplaceAddUnicode => {
                let b = self.top_value();
                let a = self.nth_value(1);
                let instr_idx = self.lasti() as usize - 1;
                let cache_base = instr_idx + 1;
                let target_local = self.binary_op_inplace_unicode_target_local(cache_base, a);
                if let (Some(_a_str), Some(_b_str), Some(target_local)) = (
                    a.downcast_ref_if_exact::<PyStr>(vm),
                    b.downcast_ref_if_exact::<PyStr>(vm),
                    target_local,
                ) {
                    let right = self.pop_value();
                    let left = self.pop_value();

                    let local_obj = self.localsplus.fastlocals_mut()[target_local]
                        .take()
                        .expect("BINARY_OP_INPLACE_ADD_UNICODE target local missing");
                    debug_assert!(local_obj.is(&left));
                    let mut local_str = local_obj
                        .downcast_exact::<PyStr>(vm)
                        .expect("BINARY_OP_INPLACE_ADD_UNICODE target local not exact str")
                        .into_pyref();
                    drop(left);
                    let right_str = right
                        .downcast_ref_if_exact::<PyStr>(vm)
                        .expect("BINARY_OP_INPLACE_ADD_UNICODE right operand not exact str");
                    local_str.concat_in_place(right_str.as_wtf8(), vm);

                    self.localsplus.fastlocals_mut()[target_local] = Some(local_str.into());
                    self.jump_relative_forward(
                        1,
                        Instruction::BinaryOpInplaceAddUnicode.cache_entries() as u32,
                    );
                    Ok(None)
                } else {
                    self.execute_bin_op(vm, self.binary_op_from_arg(arg))
                }
            }
            Instruction::BinarySlice => {
                // Stack: [container, start, stop] -> [result]
                let stop = self.pop_value();
                let start = self.pop_value();
                let container = self.pop_value();
                let slice: PyObjectRef = PySlice {
                    start: Some(start),
                    stop,
                    step: None,
                }
                .into_ref(&vm.ctx)
                .into();
                let result = container.get_item(&*slice, vm)?;
                self.push_value(result);
                Ok(None)
            }
            Instruction::BuildList { count: size } => {
                let sz = size.get(arg) as usize;
                let elements = self.pop_multiple(sz).collect();
                let list_obj = vm.ctx.new_list(elements);
                self.push_value(list_obj.into());
                Ok(None)
            }
            Instruction::BuildMap { count: size } => self.execute_build_map(vm, size.get(arg)),
            Instruction::BuildSet { count: size } => {
                let set = PySet::default().into_ref(&vm.ctx);
                for element in self.pop_multiple(size.get(arg) as usize) {
                    set.add(element, vm)?;
                }
                self.push_value(set.into());
                Ok(None)
            }
            Instruction::BuildSlice { argc } => self.execute_build_slice(vm, argc.get(arg)),
            /*
             Instruction::ToBool => {
                 dbg!("Shouldn't be called outside of match statements for now")
                 let value = self.pop_value();
                 // call __bool__
                 let result = value.try_to_bool(vm)?;
                 self.push_value(vm.ctx.new_bool(result).into());
                 Ok(None)
            }
            */
            Instruction::BuildString { count: size } => {
                let s: Wtf8Buf = self
                    .pop_multiple(size.get(arg) as usize)
                    .map(|pyobj| pyobj.downcast::<PyStr>().unwrap())
                    .collect();
                self.push_value(vm.ctx.new_str(s).into());
                Ok(None)
            }
            Instruction::BuildTuple { count: size } => {
                let elements = self.pop_multiple(size.get(arg) as usize).collect();
                let list_obj = vm.ctx.new_tuple(elements);
                self.push_value(list_obj.into());
                Ok(None)
            }
            Instruction::BuildTemplate => {
                // Stack: [strings_tuple, interpolations_tuple] -> [template]
                let interpolations = self.pop_value();
                let strings = self.pop_value();

                let strings = strings
                    .downcast::<PyTuple>()
                    .map_err(|_| vm.new_type_error("BUILD_TEMPLATE expected tuple for strings"))?;
                let interpolations = interpolations.downcast::<PyTuple>().map_err(|_| {
                    vm.new_type_error("BUILD_TEMPLATE expected tuple for interpolations")
                })?;

                let template = PyTemplate::new(strings, interpolations);
                self.push_value(template.into_pyobject(vm));
                Ok(None)
            }
            Instruction::BuildInterpolation { format: oparg } => {
                // oparg encoding: (conversion << 2) | has_format_spec
                // Stack: [value, expression_str, (format_spec)?] -> [interpolation]
                let oparg_val = oparg.get(arg);
                let has_format_spec = (oparg_val & 1) != 0;
                let conversion_code = oparg_val >> 2;

                let format_spec = if has_format_spec {
                    self.pop_value().downcast::<PyStr>().map_err(|_| {
                        vm.new_type_error("BUILD_INTERPOLATION expected str for format_spec")
                    })?
                } else {
                    vm.ctx.empty_str.to_owned()
                };

                let expression = self.pop_value().downcast::<PyStr>().map_err(|_| {
                    vm.new_type_error("BUILD_INTERPOLATION expected str for expression")
                })?;
                let value = self.pop_value();

                // conversion: 0=None, 1=Str, 2=Repr, 3=Ascii
                let conversion: PyObjectRef = match conversion_code {
                    0 => vm.ctx.none(),
                    1 => vm.ctx.new_str("s").into(),
                    2 => vm.ctx.new_str("r").into(),
                    3 => vm.ctx.new_str("a").into(),
                    _ => vm.ctx.none(), // should not happen
                };

                let interpolation =
                    PyInterpolation::new(value, expression, conversion, format_spec, vm)?;
                self.push_value(interpolation.into_pyobject(vm));
                Ok(None)
            }
            Instruction::Call { argc: nargs } => {
                // Stack: [callable, self_or_null, arg1, ..., argN]
                let nargs_val = nargs.get(arg);
                self.adaptive(|s, ii, cb| s.specialize_call(vm, nargs_val, ii, cb));
                self.execute_call_vectorcall(nargs_val, vm)
            }
            Instruction::CallKw { argc: nargs } => {
                let nargs = nargs.get(arg);
                self.adaptive(|s, ii, cb| s.specialize_call_kw(vm, nargs, ii, cb));
                // Stack: [callable, self_or_null, arg1, ..., argN, kwarg_names]
                self.execute_call_kw_vectorcall(nargs, vm)
            }
            Instruction::CallFunctionEx => {
                // Stack: [callable, self_or_null, args_tuple, kwargs_or_null]
                let args = self.collect_ex_args(vm)?;
                self.execute_call(args, vm)
            }
            Instruction::CallIntrinsic1 { func } => {
                let value = self.pop_value();
                let result = self.call_intrinsic_1(func.get(arg), value, vm)?;
                self.push_value(result);
                Ok(None)
            }
            Instruction::CallIntrinsic2 { func } => {
                let value2 = self.pop_value();
                let value1 = self.pop_value();
                let result = self.call_intrinsic_2(func.get(arg), value1, value2, vm)?;
                self.push_value(result);
                Ok(None)
            }
            Instruction::CheckEgMatch => {
                let match_type = self.pop_value();
                let exc_value = self.pop_value();
                let (rest, matched) =
                    crate::exceptions::exception_group_match(&exc_value, &match_type, vm)?;

                // Set matched exception as current exception (if not None)
                // This mirrors CPython's PyErr_SetHandledException(match_o) in CHECK_EG_MATCH
                if !vm.is_none(&matched)
                    && let Some(exc) = matched.downcast_ref::<PyBaseException>()
                {
                    vm.set_exception(Some(exc.to_owned()));
                }

                self.push_value(rest);
                self.push_value(matched);
                Ok(None)
            }
            Instruction::CompareOp { opname: op } => {
                let op_val = op.get(arg);
                self.adaptive(|s, ii, cb| s.specialize_compare_op(vm, op_val, ii, cb));
                self.execute_compare(vm, arg)
            }
            Instruction::ContainsOp { invert } => {
                self.adaptive(|s, ii, cb| s.specialize_contains_op(vm, ii, cb));
                let b = self.pop_value();
                let a = self.pop_value();

                let value = match invert.get(arg) {
                    bytecode::Invert::No => self._in(vm, &a, &b)?,
                    bytecode::Invert::Yes => self._not_in(vm, &a, &b)?,
                };
                self.push_value(vm.ctx.new_bool(value).into());
                Ok(None)
            }
            Instruction::ConvertValue { oparg: conversion } => {
                self.convert_value(conversion.get(arg), vm)
            }
            Instruction::Copy { i: index } => {
                // CopyItem { index: 1 } copies TOS
                // CopyItem { index: 2 } copies second from top
                // This is 1-indexed to match CPython
                let idx = index.get(arg) as usize;
                let stack_len = self.localsplus.stack_len();
                debug_assert!(stack_len >= idx, "CopyItem: stack underflow");
                let value = self.localsplus.stack_index(stack_len - idx).clone();
                self.push_stackref_opt(value);
                Ok(None)
            }
            Instruction::CopyFreeVars { n } => {
                let n = n.get(arg) as usize;
                if n > 0 {
                    let closure = self
                        .object
                        .func_obj
                        .as_ref()
                        .and_then(|f| f.downcast_ref::<PyFunction>())
                        .and_then(|f| f.closure.as_ref());
                    let nlocalsplus = self.code.localspluskinds.len();
                    let freevar_start = nlocalsplus - n;
                    let fastlocals = self.localsplus.fastlocals_mut();
                    if let Some(closure) = closure {
                        for i in 0..n {
                            fastlocals[freevar_start + i] = Some(closure[i].clone().into());
                        }
                    }
                }
                Ok(None)
            }
            Instruction::DeleteAttr { namei: idx } => self.delete_attr(vm, idx.get(arg)),
            Instruction::DeleteDeref { i } => {
                self.cell_ref(i.get(arg).as_usize()).set(None);
                Ok(None)
            }
            Instruction::DeleteFast { var_num } => {
                let fastlocals = self.localsplus.fastlocals_mut();
                let idx = var_num.get(arg);
                if fastlocals[idx].is_none() {
                    return Err(vm.new_exception_msg(
                        vm.ctx.exceptions.unbound_local_error.to_owned(),
                        format!(
                            "local variable '{}' referenced before assignment",
                            self.code.varnames[idx]
                        )
                        .into(),
                    ));
                }
                fastlocals[idx] = None;
                Ok(None)
            }
            Instruction::DeleteGlobal { namei: idx } => {
                let name = self.code.names[idx.get(arg) as usize];
                match self.globals.del_item(name, vm) {
                    Ok(()) => {}
                    Err(e) if e.fast_isinstance(vm.ctx.exceptions.key_error) => {
                        return Err(name_error(name, vm));
                    }
                    Err(e) => return Err(e),
                }
                Ok(None)
            }
            Instruction::DeleteName { namei: idx } => {
                let name = self.code.names[idx.get(arg) as usize];
                let res = self.locals.mapping(vm).ass_subscript(name, None, vm);

                match res {
                    Ok(()) => {}
                    Err(e) if e.fast_isinstance(vm.ctx.exceptions.key_error) => {
                        return Err(name_error(name, vm));
                    }
                    Err(e) => return Err(e),
                }
                Ok(None)
            }
            Instruction::DeleteSubscr => self.execute_delete_subscript(vm),
            Instruction::DictUpdate { i: index } => {
                // Stack before: [..., dict, ..., source]  (source at TOS)
                // Stack after:  [..., dict, ...]  (source consumed)
                // The dict to update is at position TOS-i (before popping source)

                let idx = index.get(arg);

                // Pop the source from TOS
                let source = self.pop_value();

                // Get the dict to update (it's now at TOS-(i-1) after popping source)
                let dict = if idx <= 1 {
                    // DICT_UPDATE 0 or 1: dict is at TOS (after popping source)
                    self.top_value()
                } else {
                    // DICT_UPDATE n: dict is at TOS-(n-1)
                    self.nth_value(idx - 1)
                };

                let dict = dict.downcast_ref::<PyDict>().expect("exact dict expected");

                // For dictionary unpacking {**x}, x must be a mapping
                // Check if the object has the mapping protocol (keys method)
                if vm
                    .get_method(source.clone(), vm.ctx.intern_str("keys"))
                    .is_none()
                {
                    return Err(vm.new_type_error(format!(
                        "'{}' object is not a mapping",
                        source.class().name()
                    )));
                }

                dict.merge_object(source, vm)?;
                Ok(None)
            }
            Instruction::DictMerge { i: index } => {
                let source = self.pop_value();
                let idx = index.get(arg);

                // Get the dict to merge into (same logic as DICT_UPDATE)
                let dict_ref = if idx <= 1 {
                    self.top_value()
                } else {
                    self.nth_value(idx - 1)
                };

                let dict: &Py<PyDict> = unsafe { dict_ref.downcast_unchecked_ref() };

                // Get callable for error messages
                // Stack: [callable, self_or_null, args_tuple, kwargs_dict]
                let callable = self.nth_value(idx + 2);
                let func_str = Self::object_function_str(callable, vm);

                // Check if source is a mapping
                if vm
                    .get_method(source.clone(), vm.ctx.intern_str("keys"))
                    .is_none()
                {
                    return Err(vm.new_type_error(format!(
                        "{} argument after ** must be a mapping, not {}",
                        func_str,
                        source.class().name()
                    )));
                }

                // Merge keys, checking for duplicates
                let keys_iter = vm.call_method(&source, "keys", ())?;
                for key in keys_iter.try_to_value::<Vec<PyObjectRef>>(vm)? {
                    if dict.contains_key(&*key, vm) {
                        let key_str = key.str(vm)?;
                        return Err(vm.new_type_error(format!(
                            "{} got multiple values for keyword argument '{}'",
                            func_str,
                            key_str.as_wtf8()
                        )));
                    }
                    let value = vm.call_method(&source, "__getitem__", (key.clone(),))?;
                    dict.set_item(&*key, value, vm)?;
                }
                Ok(None)
            }
            Instruction::EndAsyncFor => {
                // Pops (awaitable, exc) from stack.
                // If exc is StopAsyncIteration, clears it (normal loop end).
                // Otherwise re-raises.
                let exc = self.pop_value();
                let _awaitable = self.pop_value();

                let exc = exc
                    .downcast::<PyBaseException>()
                    .expect("EndAsyncFor expects exception on stack");

                if exc.fast_isinstance(vm.ctx.exceptions.stop_async_iteration) {
                    // StopAsyncIteration - normal end of async for loop
                    vm.set_exception(None);
                    Ok(None)
                } else {
                    // Other exception - re-raise
                    Err(exc)
                }
            }
            Instruction::ExtendedArg => {
                *extend_arg = true;
                Ok(None)
            }
            Instruction::ForIter { .. } => {
                // Relative forward jump: target = lasti + caches + delta
                let target = bytecode::Label::from_u32(self.lasti() + 1 + u32::from(arg));
                self.adaptive(|s, ii, cb| s.specialize_for_iter(vm, u32::from(arg), ii, cb));
                self.execute_for_iter(vm, target)?;
                Ok(None)
            }
            Instruction::FormatSimple => {
                let value = self.pop_value();
                let formatted = vm.format(&value, vm.ctx.new_str(""))?;
                self.push_value(formatted.into());

                Ok(None)
            }
            Instruction::FormatWithSpec => {
                let spec = self.pop_value();
                let value = self.pop_value();
                let formatted = vm.format(&value, spec.downcast::<PyStr>().unwrap())?;
                self.push_value(formatted.into());

                Ok(None)
            }
            Instruction::GetAIter => {
                let aiterable = self.pop_value();
                let aiter = vm.call_special_method(&aiterable, identifier!(vm, __aiter__), ())?;
                self.push_value(aiter);
                Ok(None)
            }
            Instruction::GetANext => {
                #[cfg(debug_assertions)] // remove when GetANext is fully implemented
                let orig_stack_len = self.localsplus.stack_len();

                let aiter = self.top_value();
                let awaitable = if aiter.class().is(vm.ctx.types.async_generator) {
                    vm.call_special_method(aiter, identifier!(vm, __anext__), ())?
                } else {
                    if !aiter.has_attr("__anext__", vm).unwrap_or(false) {
                        // TODO: __anext__ must be protocol
                        let msg = format!(
                            "'async for' requires an iterator with __anext__ method, got {:.100}",
                            aiter.class().name()
                        );
                        return Err(vm.new_type_error(msg));
                    }
                    let next_iter =
                        vm.call_special_method(aiter, identifier!(vm, __anext__), ())?;

                    // _PyCoro_GetAwaitableIter in CPython
                    fn get_awaitable_iter(next_iter: &PyObject, vm: &VirtualMachine) -> PyResult {
                        let gen_is_coroutine = |_| {
                            // TODO: cpython gen_is_coroutine
                            true
                        };
                        if next_iter.class().is(vm.ctx.types.coroutine_type)
                            || gen_is_coroutine(next_iter)
                        {
                            return Ok(next_iter.to_owned());
                        }
                        // TODO: error handling
                        vm.call_special_method(next_iter, identifier!(vm, __await__), ())
                    }
                    get_awaitable_iter(&next_iter, vm).map_err(|_| {
                        vm.new_type_error(format!(
                            "'async for' received an invalid object from __anext__: {:.200}",
                            next_iter.class().name()
                        ))
                    })?
                };
                self.push_value(awaitable);
                #[cfg(debug_assertions)]
                debug_assert_eq!(orig_stack_len + 1, self.localsplus.stack_len());
                Ok(None)
            }
            Instruction::GetAwaitable { r#where: oparg } => {
                let iterable = self.pop_value();

                let iter = match crate::coroutine::get_awaitable_iter(iterable.clone(), vm) {
                    Ok(iter) => iter,
                    Err(e) => {
                        // _PyEval_FormatAwaitableError: override error for async with
                        // when the type doesn't have __await__
                        let oparg_val = oparg.get(arg);
                        if vm
                            .get_method(iterable.clone(), identifier!(vm, __await__))
                            .is_none()
                        {
                            if oparg_val == 1 {
                                return Err(vm.new_type_error(format!(
                                    "'async with' received an object from __aenter__ \
                                     that does not implement __await__: {}",
                                    iterable.class().name()
                                )));
                            } else if oparg_val == 2 {
                                return Err(vm.new_type_error(format!(
                                    "'async with' received an object from __aexit__ \
                                     that does not implement __await__: {}",
                                    iterable.class().name()
                                )));
                            }
                        }
                        return Err(e);
                    }
                };

                // Check if coroutine is already being awaited
                if let Some(coro) = iter.downcast_ref::<PyCoroutine>()
                    && coro.as_coro().frame().yield_from_target().is_some()
                {
                    return Err(vm.new_runtime_error("coroutine is being awaited already"));
                }

                self.push_value(iter);
                Ok(None)
            }
            Instruction::GetIter => {
                let iterated_obj = self.pop_value();
                let iter_obj = iterated_obj.get_iter(vm)?;
                self.push_value(iter_obj.into());
                Ok(None)
            }
            Instruction::GetYieldFromIter => {
                // GET_YIELD_FROM_ITER: prepare iterator for yield from
                // If iterable is a coroutine, ensure we're in a coroutine context
                // If iterable is a generator, use it directly
                // Otherwise, call iter() on it
                let iterable = self.pop_value();
                let iter = if iterable.class().is(vm.ctx.types.coroutine_type) {
                    // Coroutine requires CO_COROUTINE or CO_ITERABLE_COROUTINE flag
                    if !self.code.flags.intersects(
                        bytecode::CodeFlags::COROUTINE | bytecode::CodeFlags::ITERABLE_COROUTINE,
                    ) {
                        return Err(vm.new_type_error(
                            "cannot 'yield from' a coroutine object in a non-coroutine generator",
                        ));
                    }
                    iterable
                } else if iterable.class().is(vm.ctx.types.generator_type) {
                    // Generator can be used directly
                    iterable
                } else {
                    // Otherwise, get iterator
                    iterable.get_iter(vm)?.into()
                };
                self.push_value(iter);
                Ok(None)
            }
            Instruction::GetLen => {
                // STACK.append(len(STACK[-1]))
                let obj = self.top_value();
                let len = obj.length(vm)?;
                self.push_value(vm.ctx.new_int(len).into());
                Ok(None)
            }
            Instruction::ImportFrom { namei: idx } => {
                let obj = self.import_from(vm, idx.get(arg))?;
                self.push_value(obj);
                Ok(None)
            }
            Instruction::ImportName { namei: idx } => {
                self.import(vm, Some(self.code.names[idx.get(arg) as usize]))?;
                Ok(None)
            }
            Instruction::IsOp { invert } => {
                let b = self.pop_value();
                let a = self.pop_value();
                let res = a.is(&b);

                let value = match invert.get(arg) {
                    bytecode::Invert::No => res,
                    bytecode::Invert::Yes => !res,
                };
                self.push_value(vm.ctx.new_bool(value).into());
                Ok(None)
            }
            Instruction::JumpForward { .. } => {
                self.jump_relative_forward(u32::from(arg), 0);
                Ok(None)
            }
            Instruction::JumpBackward { .. } => {
                // CPython rewrites JUMP_BACKWARD to JUMP_BACKWARD_NO_JIT
                // when JIT is unavailable.
                let instr_idx = self.lasti() as usize - 1;
                unsafe {
                    self.code
                        .instructions
                        .replace_op(instr_idx, Instruction::JumpBackwardNoJit);
                }
                self.jump_relative_backward(u32::from(arg), 1);
                Ok(None)
            }
            Instruction::JumpBackwardJit | Instruction::JumpBackwardNoJit => {
                self.jump_relative_backward(u32::from(arg), 1);
                Ok(None)
            }
            Instruction::JumpBackwardNoInterrupt { .. } => {
                self.jump_relative_backward(u32::from(arg), 0);
                Ok(None)
            }
            Instruction::ListAppend { i } => {
                let item = self.pop_value();
                let obj = self.nth_value(i.get(arg) - 1);
                let list: &Py<PyList> = unsafe {
                    // SAFETY: trust compiler
                    obj.downcast_unchecked_ref()
                };
                list.append(item);
                Ok(None)
            }
            Instruction::ListExtend { i } => {
                let iterable = self.pop_value();
                let obj = self.nth_value(i.get(arg) - 1);
                let list: &Py<PyList> = unsafe {
                    // SAFETY: compiler guarantees correct type
                    obj.downcast_unchecked_ref()
                };
                let type_name = iterable.class().name().to_owned();
                // Only rewrite the error if the type is truly not iterable
                // (no __iter__ and no __getitem__). Preserve original TypeError
                // from custom iterables that raise during iteration.
                let not_iterable = iterable.class().slots.iter.load().is_none()
                    && iterable
                        .get_class_attr(vm.ctx.intern_str("__getitem__"))
                        .is_none();
                list.extend(iterable, vm).map_err(|e| {
                    if not_iterable && e.class().is(vm.ctx.exceptions.type_error) {
                        vm.new_type_error(format!(
                            "Value after * must be an iterable, not {type_name}"
                        ))
                    } else {
                        e
                    }
                })?;
                Ok(None)
            }
            Instruction::LoadAttr { namei: idx } => self.load_attr(vm, idx.get(arg)),
            Instruction::LoadSuperAttr { namei: idx } => {
                let idx_val = idx.get(arg);
                self.adaptive(|s, ii, cb| s.specialize_load_super_attr(vm, idx_val, ii, cb));
                self.load_super_attr(vm, idx_val)
            }
            Instruction::LoadBuildClass => {
                let build_class = if let Some(builtins_dict) = self.builtins_dict {
                    builtins_dict
                        .get_item_opt(identifier!(vm, __build_class__), vm)?
                        .ok_or_else(|| {
                            vm.new_name_error(
                                "__build_class__ not found",
                                identifier!(vm, __build_class__).to_owned(),
                            )
                        })?
                } else {
                    self.builtins
                        .get_item(identifier!(vm, __build_class__), vm)
                        .map_err(|e| {
                            if e.fast_isinstance(vm.ctx.exceptions.key_error) {
                                vm.new_name_error(
                                    "__build_class__ not found",
                                    identifier!(vm, __build_class__).to_owned(),
                                )
                            } else {
                                e
                            }
                        })?
                };
                self.push_value(build_class);
                Ok(None)
            }
            Instruction::LoadLocals => {
                // Push the locals dict onto the stack
                let locals = self.locals.into_object(vm);
                self.push_value(locals);
                Ok(None)
            }
            Instruction::LoadFromDictOrDeref { i } => {
                // Pop dict from stack (locals or classdict depending on context)
                let class_dict = self.pop_value();
                let idx = i.get(arg).as_usize();
                let name = self.localsplus_name(idx);
                let value = self.mapping_get_optional(&class_dict, name, vm)?;
                self.push_value(match value {
                    Some(v) => v,
                    None => self
                        .cell_ref(idx)
                        .get()
                        .ok_or_else(|| self.unbound_cell_exception(idx, vm))?,
                });
                Ok(None)
            }
            Instruction::LoadFromDictOrGlobals { i: idx } => {
                // PEP 649: Pop dict from stack (classdict), check there first, then globals
                let dict = self.pop_value();
                let name = self.code.names[idx.get(arg) as usize];
                let value = self.mapping_get_optional(&dict, name, vm)?;

                self.push_value(match value {
                    Some(v) => v,
                    None => self.load_global_or_builtin(name, vm)?,
                });
                Ok(None)
            }
            Instruction::LoadConst { consti } => {
                self.push_value(self.code.constants[consti.get(arg)].clone().into());
                // Mirror CPython's LOAD_CONST family transition. RustPython does
                // not currently distinguish immortal constants at runtime.
                let instr_idx = self.lasti() as usize - 1;
                unsafe {
                    self.code
                        .instructions
                        .replace_op(instr_idx, Instruction::LoadConstMortal);
                }
                Ok(None)
            }
            Instruction::LoadConstMortal | Instruction::LoadConstImmortal => {
                self.push_value(self.code.constants[u32::from(arg).into()].clone().into());
                Ok(None)
            }
            Instruction::LoadCommonConstant { idx } => {
                use bytecode::CommonConstant;
                let value = match idx.get(arg) {
                    CommonConstant::AssertionError => {
                        vm.ctx.exceptions.assertion_error.to_owned().into()
                    }
                    CommonConstant::NotImplementedError => {
                        vm.ctx.exceptions.not_implemented_error.to_owned().into()
                    }
                    CommonConstant::BuiltinTuple => vm.ctx.types.tuple_type.to_owned().into(),
                    CommonConstant::BuiltinAll => vm
                        .callable_cache
                        .builtin_all
                        .clone()
                        .expect("builtin_all not initialized"),
                    CommonConstant::BuiltinAny => vm
                        .callable_cache
                        .builtin_any
                        .clone()
                        .expect("builtin_any not initialized"),
                    CommonConstant::BuiltinList => vm.ctx.types.list_type.to_owned().into(),
                    CommonConstant::BuiltinSet => vm.ctx.types.set_type.to_owned().into(),
                };
                self.push_value(value);
                Ok(None)
            }
            Instruction::LoadSmallInt { i: idx } => {
                // Push small integer (-5..=256) directly without constant table lookup
                let value = vm.ctx.new_int(idx.get(arg) as i32);
                self.push_value(value.into());
                Ok(None)
            }
            Instruction::LoadDeref { i } => {
                let idx = i.get(arg).as_usize();
                let x = self
                    .cell_ref(idx)
                    .get()
                    .ok_or_else(|| self.unbound_cell_exception(idx, vm))?;
                self.push_value(x);
                Ok(None)
            }
            Instruction::LoadFast { var_num } => {
                #[cold]
                fn reference_error(
                    varname: &'static PyStrInterned,
                    vm: &VirtualMachine,
                ) -> PyBaseExceptionRef {
                    vm.new_exception_msg(
                        vm.ctx.exceptions.unbound_local_error.to_owned(),
                        format!("local variable '{varname}' referenced before assignment").into(),
                    )
                }
                let idx = var_num.get(arg);
                let x = self.localsplus.fastlocals()[idx]
                    .clone()
                    .ok_or_else(|| reference_error(self.code.varnames[idx], vm))?;
                self.push_value(x);
                Ok(None)
            }
            Instruction::LoadFastAndClear { var_num } => {
                // Save current slot value and clear it (for inlined comprehensions).
                // Pushes NULL (None at Option level) if slot was empty, so that
                // StoreFast can restore the empty state after the comprehension.
                let idx = var_num.get(arg);
                let x = self.localsplus.fastlocals_mut()[idx].take();
                self.push_value_opt(x);
                Ok(None)
            }
            Instruction::LoadFastCheck { var_num } => {
                // Same as LoadFast but explicitly checks for unbound locals
                // (LoadFast in RustPython already does this check)
                let idx = var_num.get(arg);
                let x = self.localsplus.fastlocals()[idx].clone().ok_or_else(|| {
                    vm.new_exception_msg(
                        vm.ctx.exceptions.unbound_local_error.to_owned(),
                        format!(
                            "local variable '{}' referenced before assignment",
                            self.code.varnames[idx]
                        )
                        .into(),
                    )
                })?;
                self.push_value(x);
                Ok(None)
            }
            Instruction::LoadFastLoadFast { var_nums } => {
                // Load two local variables at once
                // oparg encoding: (idx1 << 4) | idx2
                let oparg = var_nums.get(arg);
                let (idx1, idx2) = oparg.indexes();
                let fastlocals = self.localsplus.fastlocals();
                let x1 = fastlocals[idx1].clone().ok_or_else(|| {
                    vm.new_exception_msg(
                        vm.ctx.exceptions.unbound_local_error.to_owned(),
                        format!(
                            "local variable '{}' referenced before assignment",
                            self.code.varnames[idx1]
                        )
                        .into(),
                    )
                })?;
                let x2 = fastlocals[idx2].clone().ok_or_else(|| {
                    vm.new_exception_msg(
                        vm.ctx.exceptions.unbound_local_error.to_owned(),
                        format!(
                            "local variable '{}' referenced before assignment",
                            self.code.varnames[idx2]
                        )
                        .into(),
                    )
                })?;
                self.push_value(x1);
                self.push_value(x2);
                Ok(None)
            }
            // Borrow optimization not yet active; falls back to clone.
            // push_borrowed() is available but disabled until stack
            // lifetime issues at yield/exception points are resolved.
            Instruction::LoadFastBorrow { var_num } => {
                let idx = var_num.get(arg);
                let x = self.localsplus.fastlocals()[idx].clone().ok_or_else(|| {
                    vm.new_exception_msg(
                        vm.ctx.exceptions.unbound_local_error.to_owned(),
                        format!(
                            "local variable '{}' referenced before assignment",
                            self.code.varnames[idx]
                        )
                        .into(),
                    )
                })?;
                self.push_value(x);
                Ok(None)
            }
            Instruction::LoadFastBorrowLoadFastBorrow { var_nums } => {
                let oparg = var_nums.get(arg);
                let (idx1, idx2) = oparg.indexes();
                let fastlocals = self.localsplus.fastlocals();
                let x1 = fastlocals[idx1].clone().ok_or_else(|| {
                    vm.new_exception_msg(
                        vm.ctx.exceptions.unbound_local_error.to_owned(),
                        format!(
                            "local variable '{}' referenced before assignment",
                            self.code.varnames[idx1]
                        )
                        .into(),
                    )
                })?;
                let x2 = fastlocals[idx2].clone().ok_or_else(|| {
                    vm.new_exception_msg(
                        vm.ctx.exceptions.unbound_local_error.to_owned(),
                        format!(
                            "local variable '{}' referenced before assignment",
                            self.code.varnames[idx2]
                        )
                        .into(),
                    )
                })?;
                self.push_value(x1);
                self.push_value(x2);
                Ok(None)
            }
            Instruction::LoadGlobal { namei: idx } => {
                let oparg = idx.get(arg);
                self.adaptive(|s, ii, cb| s.specialize_load_global(vm, oparg, ii, cb));
                let name = &self.code.names[(oparg >> 1) as usize];
                let x = self.load_global_or_builtin(name, vm)?;
                self.push_value(x);
                if (oparg & 1) != 0 {
                    self.push_value_opt(None);
                }
                Ok(None)
            }
            Instruction::LoadName { namei: idx } => {
                let name = self.code.names[idx.get(arg) as usize];
                let result = self.locals.mapping(vm).subscript(name, vm);
                match result {
                    Ok(x) => self.push_value(x),
                    Err(e) if e.fast_isinstance(vm.ctx.exceptions.key_error) => {
                        self.push_value(self.load_global_or_builtin(name, vm)?);
                    }
                    Err(e) => return Err(e),
                }
                Ok(None)
            }
            Instruction::LoadSpecial { method } => {
                // Pops obj, pushes (callable, self_or_null) for CALL convention.
                // Push order: callable first (deeper), self_or_null on top.
                use crate::vm::PyMethod;

                let obj = self.pop_value();
                let oparg = method.get(arg);
                let method_name = get_special_method_name(oparg, vm);

                match vm.get_special_method(&obj, method_name)? {
                    Some(PyMethod::Function { target, func }) => {
                        self.push_value(func); // callable (deeper)
                        self.push_value(target); // self (TOS)
                    }
                    Some(PyMethod::Attribute(bound)) => {
                        self.push_value(bound); // callable (deeper)
                        self.push_null(); // NULL (TOS)
                    }
                    None => {
                        return Err(vm.new_type_error(get_special_method_error_msg(
                            oparg,
                            &obj.class().name(),
                            special_method_can_suggest(&obj, oparg, vm)?,
                        )));
                    }
                };
                Ok(None)
            }
            Instruction::MakeFunction => self.execute_make_function(vm),
            Instruction::MakeCell { i } => {
                // Wrap the current slot value (if any) in a new PyCell.
                // For merged cells (LOCAL|CELL), this wraps the argument value.
                // For non-merged cells, this creates an empty cell.
                let idx = i.get(arg).as_usize();
                let fastlocals = self.localsplus.fastlocals_mut();
                let initial = fastlocals[idx].take();
                let cell = PyCell::new(initial).into_ref(&vm.ctx).into();
                fastlocals[idx] = Some(cell);
                Ok(None)
            }
            Instruction::MapAdd { i } => {
                let value = self.pop_value();
                let key = self.pop_value();
                let obj = self.nth_value(i.get(arg) - 1);
                let dict: &Py<PyDict> = unsafe {
                    // SAFETY: trust compiler
                    obj.downcast_unchecked_ref()
                };
                dict.set_item(&*key, value, vm)?;
                Ok(None)
            }
            Instruction::MatchClass { count: nargs } => {
                // STACK[-1] is a tuple of keyword attribute names, STACK[-2] is the class being matched against, and STACK[-3] is the match subject.
                // nargs is the number of positional sub-patterns.
                let kwd_attrs = self.pop_value();
                let kwd_attrs = kwd_attrs.downcast_ref::<PyTuple>().unwrap();
                let cls = self.pop_value();
                let subject = self.pop_value();
                let nargs_val = nargs.get(arg) as usize;

                // Check if subject is an instance of cls
                if subject.is_instance(cls.as_ref(), vm)? {
                    let mut extracted = vec![];

                    // Get __match_args__ for positional arguments if nargs > 0
                    if nargs_val > 0 {
                        // Get __match_args__ from the class
                        let match_args =
                            vm.get_attribute_opt(cls.clone(), identifier!(vm, __match_args__))?;

                        if let Some(match_args) = match_args {
                            // Convert to tuple
                            let match_args = match match_args.downcast_exact::<PyTuple>(vm) {
                                Ok(tuple) => tuple,
                                Err(match_args) => {
                                    // __match_args__ must be a tuple
                                    // Get type names for error message
                                    let type_name = cls
                                        .downcast::<crate::builtins::PyType>()
                                        .ok()
                                        .and_then(|t| t.__name__(vm).to_str().map(str::to_owned))
                                        .unwrap_or_else(|| String::from("?"));
                                    let match_args_type_name = match_args.class().__name__(vm);
                                    return Err(vm.new_type_error(format!(
                                        "{}.__match_args__ must be a tuple (got {})",
                                        type_name, match_args_type_name
                                    )));
                                }
                            };

                            // Check if we have enough match args
                            if match_args.len() < nargs_val {
                                return Err(vm.new_type_error(format!(
                                    "class pattern accepts at most {} positional sub-patterns ({} given)",
                                    match_args.len(),
                                    nargs_val
                                )));
                            }

                            // Extract positional attributes
                            for i in 0..nargs_val {
                                let attr_name = &match_args[i];
                                let attr_name_str = match attr_name.downcast_ref::<PyStr>() {
                                    Some(s) => s,
                                    None => {
                                        return Err(vm.new_type_error(
                                            "__match_args__ elements must be strings",
                                        ));
                                    }
                                };
                                match subject.get_attr(attr_name_str, vm) {
                                    Ok(value) => extracted.push(value),
                                    Err(e)
                                        if e.fast_isinstance(vm.ctx.exceptions.attribute_error) =>
                                    {
                                        // Missing attribute → non-match
                                        self.push_value(vm.ctx.none());
                                        return Ok(None);
                                    }
                                    Err(e) => return Err(e),
                                }
                            }
                        } else {
                            // No __match_args__, check if this is a type with MATCH_SELF behavior
                            // For built-in types like bool, int, str, list, tuple, dict, etc.
                            // they match the subject itself as the single positional argument
                            let is_match_self_type = cls
                                .downcast::<PyType>()
                                .is_ok_and(|t| t.slots.flags.contains(PyTypeFlags::_MATCH_SELF));

                            if is_match_self_type {
                                if nargs_val == 1 {
                                    // Match the subject itself as the single positional argument
                                    extracted.push(subject.clone());
                                } else if nargs_val > 1 {
                                    // Too many positional arguments for MATCH_SELF
                                    return Err(vm.new_type_error(
                                        "class pattern accepts at most 1 positional sub-pattern for MATCH_SELF types",
                                    ));
                                }
                            } else {
                                // No __match_args__ and not a MATCH_SELF type
                                if nargs_val > 0 {
                                    return Err(vm.new_type_error(
                                        "class pattern defines no positional sub-patterns (__match_args__ missing)",
                                    ));
                                }
                            }
                        }
                    }

                    // Extract keyword attributes
                    for name in kwd_attrs {
                        let name_str = name.downcast_ref::<PyStr>().unwrap();
                        match subject.get_attr(name_str, vm) {
                            Ok(value) => extracted.push(value),
                            Err(e) if e.fast_isinstance(vm.ctx.exceptions.attribute_error) => {
                                self.push_value(vm.ctx.none());
                                return Ok(None);
                            }
                            Err(e) => return Err(e),
                        }
                    }

                    self.push_value(vm.ctx.new_tuple(extracted).into());
                } else {
                    // Not an instance, push None
                    self.push_value(vm.ctx.none());
                }
                Ok(None)
            }
            Instruction::MatchKeys => {
                // MATCH_KEYS doesn't pop subject and keys, only reads them
                let keys_tuple = self.top_value(); // stack[-1]
                let subject = self.nth_value(1); // stack[-2]

                // Check if subject is a mapping and extract values for keys
                if subject.class().slots.flags.contains(PyTypeFlags::MAPPING) {
                    let keys = keys_tuple.downcast_ref::<PyTuple>().unwrap();
                    let mut values = Vec::new();
                    let mut all_match = true;

                    // We use the two argument form of map.get(key, default) for two reasons:
                    // - Atomically check for a key and get its value without error handling.
                    // - Don't cause key creation or resizing in dict subclasses like
                    //   collections.defaultdict that define __missing__ (or similar).
                    // See CPython's _PyEval_MatchKeys

                    if let Some(get_method) = vm
                        .get_method(subject.to_owned(), vm.ctx.intern_str("get"))
                        .transpose()?
                    {
                        let dummy = vm
                            .ctx
                            .new_base_object(vm.ctx.types.object_type.to_owned(), None);

                        for key in keys {
                            // value = map.get(key, dummy)
                            match get_method.call((key.as_object(), dummy.clone()), vm) {
                                Ok(value) => {
                                    // if value == dummy: key not in map!
                                    if value.is(&dummy) {
                                        all_match = false;
                                        break;
                                    }
                                    values.push(value);
                                }
                                Err(e) => return Err(e),
                            }
                        }
                    } else {
                        // Fallback if .get() method is not available (shouldn't happen for mappings)
                        for key in keys {
                            match subject.get_item(key.as_object(), vm) {
                                Ok(value) => values.push(value),
                                Err(e) if e.fast_isinstance(vm.ctx.exceptions.key_error) => {
                                    all_match = false;
                                    break;
                                }
                                Err(e) => return Err(e),
                            }
                        }
                    }

                    if all_match {
                        // Push values tuple on successful match
                        self.push_value(vm.ctx.new_tuple(values).into());
                    } else {
                        // No match - push None
                        self.push_value(vm.ctx.none());
                    }
                } else {
                    // Not a mapping - push None
                    self.push_value(vm.ctx.none());
                }
                Ok(None)
            }
            Instruction::MatchMapping => {
                // Pop and push back the subject to keep it on stack
                let subject = self.pop_value();

                // Check if the type has the MAPPING flag
                let is_mapping = subject.class().slots.flags.contains(PyTypeFlags::MAPPING);

                self.push_value(subject);
                self.push_value(vm.ctx.new_bool(is_mapping).into());
                Ok(None)
            }
            Instruction::MatchSequence => {
                // Pop and push back the subject to keep it on stack
                let subject = self.pop_value();

                // Check if the type has the SEQUENCE flag
                let is_sequence = subject.class().slots.flags.contains(PyTypeFlags::SEQUENCE);

                self.push_value(subject);
                self.push_value(vm.ctx.new_bool(is_sequence).into());
                Ok(None)
            }
            Instruction::Nop => Ok(None),
            // NOT_TAKEN is a branch prediction hint - functionally a NOP
            Instruction::NotTaken => Ok(None),
            // CACHE is used by adaptive interpreter for inline caching - NOP for us
            Instruction::Cache => Ok(None),
            Instruction::ReturnGenerator => {
                // In RustPython, generators/coroutines are created in function.rs
                // before the frame starts executing. The RETURN_GENERATOR instruction
                // pushes None so that the following POP_TOP has something to consume.
                // This matches CPython's semantics where the sent value (None for first call)
                // is on the stack when the generator resumes.
                self.push_value(vm.ctx.none());
                Ok(None)
            }
            Instruction::PopExcept => {
                // Pop prev_exc from value stack and restore it
                let prev_exc = self.pop_value();
                if vm.is_none(&prev_exc) {
                    vm.set_exception(None);
                } else if let Ok(exc) = prev_exc.downcast::<PyBaseException>() {
                    vm.set_exception(Some(exc));
                }

                // NOTE: We do NOT clear the traceback of the exception that was just handled.
                // Python preserves exception tracebacks even after the exception is no longer
                // the "current exception". This is important for code that catches an exception,
                // stores it, and later inspects its traceback.
                // Reference cycles (Exception → Traceback → Frame → locals) are handled by
                // Python's garbage collector which can detect and break cycles.

                Ok(None)
            }
            Instruction::PopJumpIfFalse { .. } => self.pop_jump_if_relative(vm, arg, 1, false),
            Instruction::PopJumpIfTrue { .. } => self.pop_jump_if_relative(vm, arg, 1, true),
            Instruction::PopJumpIfNone { .. } => {
                let value = self.pop_value();
                if vm.is_none(&value) {
                    self.jump_relative_forward(u32::from(arg), 1);
                }
                Ok(None)
            }
            Instruction::PopJumpIfNotNone { .. } => {
                let value = self.pop_value();
                if !vm.is_none(&value) {
                    self.jump_relative_forward(u32::from(arg), 1);
                }
                Ok(None)
            }
            Instruction::PopTop => {
                // Pop value from stack and ignore.
                self.pop_value();
                Ok(None)
            }
            Instruction::EndFor => {
                // Pop the next value from stack (cleanup after loop body)
                self.pop_value();
                Ok(None)
            }
            Instruction::PopIter => {
                // Pop the iterator from stack (end of for loop)
                self.pop_value();
                Ok(None)
            }
            Instruction::PushNull => {
                // Push NULL for self_or_null slot in call protocol
                self.push_null();
                Ok(None)
            }
            Instruction::RaiseVarargs { argc: kind } => self.execute_raise(vm, kind.get(arg)),
            Instruction::Resume { .. } | Instruction::ResumeCheck => {
                // Lazy quickening: initialize adaptive counters on first execution
                if !self.code.quickened.swap(true, atomic::Ordering::Relaxed) {
                    self.code.instructions.quicken();
                    atomic::fence(atomic::Ordering::Release);
                }
                if self.monitoring_disabled_for_code(vm) {
                    let global_ver = vm
                        .state
                        .instrumentation_version
                        .load(atomic::Ordering::Acquire);
                    monitoring::instrument_code(self.code, 0);
                    self.code
                        .instrumentation_version
                        .store(global_ver, atomic::Ordering::Release);
                    return Ok(None);
                }
                // Check if bytecode needs re-instrumentation
                let global_ver = vm
                    .state
                    .instrumentation_version
                    .load(atomic::Ordering::Acquire);
                let code_ver = self
                    .code
                    .instrumentation_version
                    .load(atomic::Ordering::Acquire);
                if code_ver != global_ver {
                    let events = {
                        let state = vm.state.monitoring.lock();
                        state.events_for_code(self.code.get_id())
                    };
                    monitoring::instrument_code(self.code, events);
                    self.code
                        .instrumentation_version
                        .store(global_ver, atomic::Ordering::Release);
                    // Re-execute this instruction (it may now be INSTRUMENTED_RESUME)
                    self.update_lasti(|i| *i -= 1);
                }
                Ok(None)
            }
            Instruction::ReturnValue => {
                let value = self.pop_value();
                self.unwind_blocks(vm, UnwindReason::Returning { value })
            }
            Instruction::SetAdd { i } => {
                let item = self.pop_value();
                let obj = self.nth_value(i.get(arg) - 1);
                let set: &Py<PySet> = unsafe {
                    // SAFETY: trust compiler
                    obj.downcast_unchecked_ref()
                };
                set.add(item, vm)?;
                Ok(None)
            }
            Instruction::SetUpdate { i } => {
                let iterable = self.pop_value();
                let obj = self.nth_value(i.get(arg) - 1);
                let set: &Py<PySet> = unsafe {
                    // SAFETY: compiler guarantees correct type
                    obj.downcast_unchecked_ref()
                };
                let iter = PyIter::try_from_object(vm, iterable)?;
                while let PyIterReturn::Return(item) = iter.next(vm)? {
                    set.add(item, vm)?;
                }
                Ok(None)
            }
            Instruction::PushExcInfo => {
                // Stack: [exc] -> [prev_exc, exc]
                let exc = self.pop_value();
                let prev_exc = vm
                    .current_exception()
                    .map(|e| e.into())
                    .unwrap_or_else(|| vm.ctx.none());

                // Set exc as the current exception
                if let Some(exc_ref) = exc.downcast_ref::<PyBaseException>() {
                    vm.set_exception(Some(exc_ref.to_owned()));
                }

                self.push_value(prev_exc);
                self.push_value(exc);
                Ok(None)
            }
            Instruction::CheckExcMatch => {
                // Stack: [exc, type] -> [exc, bool]
                let exc_type = self.pop_value();
                let exc = self.top_value();

                // Validate that exc_type inherits from BaseException
                if let Some(tuple_of_exceptions) = exc_type.downcast_ref::<PyTuple>() {
                    for exception in tuple_of_exceptions {
                        if !exception
                            .is_subclass(vm.ctx.exceptions.base_exception_type.into(), vm)?
                        {
                            return Err(vm.new_type_error(
                                "catching classes that do not inherit from BaseException is not allowed",
                            ));
                        }
                    }
                } else if !exc_type.is_subclass(vm.ctx.exceptions.base_exception_type.into(), vm)? {
                    return Err(vm.new_type_error(
                        "catching classes that do not inherit from BaseException is not allowed",
                    ));
                }

                let result = exc.is_instance(&exc_type, vm)?;
                self.push_value(vm.ctx.new_bool(result).into());
                Ok(None)
            }
            Instruction::Reraise { depth: _ } => {
                // inst(RERAISE, (values[oparg], exc -- values[oparg]))
                //
                // RERAISE pops only `exc` from TOS. The `values` below it
                // (lasti and optional prev_exc) stay on the stack — the
                // outer exception handler's exception-table unwind will
                // pop them down to its configured stack depth.
                //
                // `oparg` encodes how many values are preserved below exc
                // (1 for simple reraise, 2 for with-block reraise where
                // values[0]=lasti). Runtime-wise we don't need oparg since
                // the exception table handles stack layout.
                let exc = self.pop_value();

                if let Some(exc_ref) = exc.downcast_ref::<PyBaseException>() {
                    Err(exc_ref.to_owned())
                } else {
                    // Fallback: use current exception if TOS is not an exception
                    let exc = vm
                        .topmost_exception()
                        .ok_or_else(|| vm.new_runtime_error("No active exception to re-raise"))?;
                    Err(exc)
                }
            }
            Instruction::SetFunctionAttribute { flag: attr } => {
                self.execute_set_function_attribute(vm, attr.get(arg))
            }
            Instruction::SetupAnnotations => self.setup_annotations(vm),
            Instruction::StoreAttr { namei: idx } => {
                let idx_val = idx.get(arg);
                self.adaptive(|s, ii, cb| s.specialize_store_attr(vm, idx_val, ii, cb));
                self.store_attr(vm, idx_val)
            }
            Instruction::StoreDeref { i } => {
                let value = self.pop_value();
                self.cell_ref(i.get(arg).as_usize()).set(Some(value));
                Ok(None)
            }
            Instruction::StoreFast { var_num } => {
                // pop_value_opt: allows NULL from LoadFastAndClear restore path
                let value = self.pop_value_opt();
                let fastlocals = self.localsplus.fastlocals_mut();
                fastlocals[var_num.get(arg)] = value;
                Ok(None)
            }
            Instruction::StoreFastLoadFast { var_nums } => {
                // pop_value_opt: allows NULL from LoadFastAndClear restore paths.
                let value = self.pop_value_opt();
                let oparg = var_nums.get(arg);
                let (store_idx, load_idx) = oparg.indexes();
                let load_value = {
                    let locals = self.localsplus.fastlocals_mut();
                    locals[store_idx] = value;
                    locals[load_idx].clone()
                };
                self.push_value_opt(load_value);
                Ok(None)
            }
            Instruction::StoreFastStoreFast { var_nums } => {
                let oparg = var_nums.get(arg);
                let (idx1, idx2) = oparg.indexes();
                // pop_value_opt: allows NULL from LoadFastAndClear restore path
                let value1 = self.pop_value_opt();
                let value2 = self.pop_value_opt();
                let fastlocals = self.localsplus.fastlocals_mut();
                fastlocals[idx1] = value1;
                fastlocals[idx2] = value2;
                Ok(None)
            }
            Instruction::StoreGlobal { namei: idx } => {
                let value = self.pop_value();
                self.globals
                    .set_item(self.code.names[idx.get(arg) as usize], value, vm)?;
                Ok(None)
            }
            Instruction::StoreName { namei: idx } => {
                let name = self.code.names[idx.get(arg) as usize];
                let value = self.pop_value();
                self.locals
                    .mapping(vm)
                    .ass_subscript(name, Some(value), vm)?;
                Ok(None)
            }
            Instruction::StoreSlice => {
                // Stack: [value, container, start, stop] -> []
                let stop = self.pop_value();
                let start = self.pop_value();
                let container = self.pop_value();
                let value = self.pop_value();
                let slice: PyObjectRef = PySlice {
                    start: Some(start),
                    stop,
                    step: None,
                }
                .into_ref(&vm.ctx)
                .into();
                container.set_item(&*slice, value, vm)?;
                Ok(None)
            }
            Instruction::StoreSubscr => {
                self.adaptive(|s, ii, cb| s.specialize_store_subscr(vm, ii, cb));
                self.execute_store_subscript(vm)
            }
            Instruction::Swap { i: index } => {
                let len = self.localsplus.stack_len();
                debug_assert!(len > 0, "stack underflow in SWAP");
                let i = len - 1; // TOS index
                let index_val = index.get(arg) as usize;
                // CPython: SWAP(n) swaps TOS with PEEK(n) where PEEK(n) = stack_pointer[-n]
                // This means swap TOS with the element at index (len - n)
                debug_assert!(
                    index_val <= len,
                    "SWAP index {} exceeds stack size {}",
                    index_val,
                    len
                );
                let j = len - index_val;
                self.localsplus.stack_swap(i, j);
                Ok(None)
            }
            Instruction::ToBool => {
                self.adaptive(|s, ii, cb| s.specialize_to_bool(vm, ii, cb));
                let obj = self.pop_value();
                let bool_val = obj.try_to_bool(vm)?;
                self.push_value(vm.ctx.new_bool(bool_val).into());
                Ok(None)
            }
            Instruction::UnpackEx { counts: args } => {
                let args = args.get(arg);
                self.execute_unpack_ex(vm, args.before, args.after)
            }
            Instruction::UnpackSequence { count: size } => {
                let expected = size.get(arg);
                self.adaptive(|s, ii, cb| s.specialize_unpack_sequence(vm, expected, ii, cb));
                self.unpack_sequence(expected, vm)
            }
            Instruction::WithExceptStart => {
                // Stack: [..., exit_func, self_or_null, lasti, prev_exc, exc]
                // exit_func at TOS-4, self_or_null at TOS-3
                let exc = vm.current_exception();

                let stack_len = self.localsplus.stack_len();
                let exit_func = expect_unchecked(
                    self.localsplus.stack_index(stack_len - 5).clone(),
                    "WithExceptStart: exit_func is NULL",
                );
                let self_or_null = self.localsplus.stack_index(stack_len - 4).clone();

                let (tp, val, tb) = if let Some(ref exc) = exc {
                    vm.split_exception(exc.clone())
                } else {
                    (vm.ctx.none(), vm.ctx.none(), vm.ctx.none())
                };

                let exit_res = if let Some(self_exit) = self_or_null {
                    exit_func.call((self_exit.to_pyobj(), tp, val, tb), vm)?
                } else {
                    exit_func.call((tp, val, tb), vm)?
                };
                self.push_value(exit_res);

                Ok(None)
            }
            Instruction::YieldValue { .. } => {
                debug_assert!(
                    self.localsplus
                        .stack_as_slice()
                        .iter()
                        .flatten()
                        .all(|sr| !sr.is_borrowed()),
                    "borrowed refs on stack at yield point"
                );
                Ok(Some(ExecutionResult::Yield(self.pop_value())))
            }
            Instruction::Send { .. } => {
                // (receiver, v -- receiver, retval)
                self.adaptive(|s, ii, cb| s.specialize_send(vm, ii, cb));
                let exit_label = bytecode::Label::from_u32(self.lasti() + 1 + u32::from(arg));
                let receiver = self.nth_value(1);
                let can_fast_send = !self.specialization_eval_frame_active(vm)
                    && (receiver.downcast_ref_if_exact::<PyGenerator>(vm).is_some()
                        || receiver.downcast_ref_if_exact::<PyCoroutine>(vm).is_some())
                    && self
                        .builtin_coro(receiver)
                        .is_some_and(|coro| !coro.running() && !coro.closed());
                let val = self.pop_value();
                let receiver = self.top_value();
                let ret = if can_fast_send {
                    let coro = self.builtin_coro(receiver).unwrap();
                    if vm.is_none(&val) {
                        coro.send_none(receiver, vm)?
                    } else {
                        coro.send(receiver, val, vm)?
                    }
                } else {
                    self._send(receiver, val, vm)?
                };
                match ret {
                    PyIterReturn::Return(value) => {
                        self.push_value(value);
                        Ok(None)
                    }
                    PyIterReturn::StopIteration(value) => {
                        if vm.use_tracing.get() && !vm.is_none(&self.object.trace.lock()) {
                            let stop_exc = vm.new_stop_iteration(value.clone());
                            self.fire_exception_trace(&stop_exc, vm)?;
                        }
                        let value = vm.unwrap_or_none(value);
                        self.push_value(value);
                        self.jump(exit_label);
                        Ok(None)
                    }
                }
            }
            Instruction::SendGen => {
                let exit_label = bytecode::Label::from_u32(self.lasti() + 1 + u32::from(arg));
                // Stack: [receiver, val] — peek receiver before popping
                let receiver = self.nth_value(1);
                let can_fast_send = !self.specialization_eval_frame_active(vm)
                    && (receiver.downcast_ref_if_exact::<PyGenerator>(vm).is_some()
                        || receiver.downcast_ref_if_exact::<PyCoroutine>(vm).is_some())
                    && self
                        .builtin_coro(receiver)
                        .is_some_and(|coro| !coro.running() && !coro.closed());
                let val = self.pop_value();

                if can_fast_send {
                    let receiver = self.top_value();
                    let coro = self.builtin_coro(receiver).unwrap();
                    let ret = if vm.is_none(&val) {
                        coro.send_none(receiver, vm)?
                    } else {
                        coro.send(receiver, val, vm)?
                    };
                    match ret {
                        PyIterReturn::Return(value) => {
                            self.push_value(value);
                            return Ok(None);
                        }
                        PyIterReturn::StopIteration(value) => {
                            if vm.use_tracing.get() && !vm.is_none(&self.object.trace.lock()) {
                                let stop_exc = vm.new_stop_iteration(value.clone());
                                self.fire_exception_trace(&stop_exc, vm)?;
                            }
                            let value = vm.unwrap_or_none(value);
                            self.push_value(value);
                            self.jump(exit_label);
                            return Ok(None);
                        }
                    }
                }
                let receiver = self.top_value();
                match self._send(receiver, val, vm)? {
                    PyIterReturn::Return(value) => {
                        self.push_value(value);
                        Ok(None)
                    }
                    PyIterReturn::StopIteration(value) => {
                        if vm.use_tracing.get() && !vm.is_none(&self.object.trace.lock()) {
                            let stop_exc = vm.new_stop_iteration(value.clone());
                            self.fire_exception_trace(&stop_exc, vm)?;
                        }
                        let value = vm.unwrap_or_none(value);
                        self.push_value(value);
                        self.jump(exit_label);
                        Ok(None)
                    }
                }
            }
            Instruction::EndSend => {
                // Stack: (receiver, value) -> (value)
                // Pops receiver, leaves value
                let value = self.pop_value();
                self.pop_value(); // discard receiver
                self.push_value(value);
                Ok(None)
            }
            Instruction::ExitInitCheck => {
                // Check that __init__ returned None
                let should_be_none = self.pop_value();
                if !vm.is_none(&should_be_none) {
                    return Err(vm.new_type_error(format!(
                        "__init__() should return None, not '{}'",
                        should_be_none.class().name()
                    )));
                }
                Ok(None)
            }
            Instruction::CleanupThrow => {
                // CLEANUP_THROW: (sub_iter, last_sent_val, exc) -> (None, value) OR re-raise
                // If StopIteration: pop all 3, extract value, push (None, value)
                // Otherwise: pop all 3, return Err(exc) for unwind_blocks to handle
                //
                // Unlike CPython where exception_unwind pops the triple as part of
                // stack cleanup to handler depth, RustPython pops here explicitly
                // and lets unwind_blocks find outer handlers.
                // Compiler sets handler_depth = base + 2 (before exc is pushed).

                // First peek at exc_value (top of stack) without popping
                let exc = self.top_value();

                // Check if it's a StopIteration
                if let Some(exc_ref) = exc.downcast_ref::<PyBaseException>()
                    && exc_ref.fast_isinstance(vm.ctx.exceptions.stop_iteration)
                {
                    // Extract value from StopIteration
                    let value = exc_ref.get_arg(0).unwrap_or_else(|| vm.ctx.none());
                    // Now pop all three
                    self.pop_value(); // exc
                    self.pop_value(); // last_sent_val
                    self.pop_value(); // sub_iter
                    self.push_value(vm.ctx.none());
                    self.push_value(value);
                    return Ok(None);
                }

                // Re-raise other exceptions: pop all three and return Err(exc)
                let exc = self.pop_value(); // exc
                self.pop_value(); // last_sent_val
                self.pop_value(); // sub_iter

                let exc = exc
                    .downcast::<PyBaseException>()
                    .map_err(|_| vm.new_type_error("exception expected"))?;
                Err(exc)
            }
            Instruction::UnaryInvert => {
                let a = self.pop_value();
                let value = vm._invert(&a)?;
                self.push_value(value);
                Ok(None)
            }
            Instruction::UnaryNegative => {
                let a = self.pop_value();
                let value = vm._neg(&a)?;
                self.push_value(value);
                Ok(None)
            }
            Instruction::UnaryNot => {
                let obj = self.pop_value();
                let value = obj.try_to_bool(vm)?;
                self.push_value(vm.ctx.new_bool(!value).into());
                Ok(None)
            }
            // Specialized LOAD_ATTR opcodes
            Instruction::LoadAttrMethodNoDict => {
                let oparg = LoadAttr::from_u32(u32::from(arg));
                let cache_base = self.lasti() as usize;

                let owner = self.top_value();
                let type_version = self.code.instructions.read_cache_u32(cache_base + 1);

                if type_version != 0
                    && owner.class().tp_version_tag.load(Acquire) == type_version
                    && let Some(func) = self.try_read_cached_descriptor(cache_base, type_version)
                {
                    let owner = self.pop_value();
                    self.push_value(func);
                    self.push_value(owner);
                    Ok(None)
                } else {
                    self.load_attr_slow(vm, oparg)
                }
            }
            Instruction::LoadAttrMethodLazyDict => {
                let oparg = LoadAttr::from_u32(u32::from(arg));
                let cache_base = self.lasti() as usize;

                let owner = self.top_value();
                let type_version = self.code.instructions.read_cache_u32(cache_base + 1);

                if type_version != 0
                    && owner.class().tp_version_tag.load(Acquire) == type_version
                    && owner.dict().is_none()
                    && let Some(func) = self.try_read_cached_descriptor(cache_base, type_version)
                {
                    let owner = self.pop_value();
                    self.push_value(func);
                    self.push_value(owner);
                    Ok(None)
                } else {
                    self.load_attr_slow(vm, oparg)
                }
            }
            Instruction::LoadAttrMethodWithValues => {
                let oparg = LoadAttr::from_u32(u32::from(arg));
                let cache_base = self.lasti() as usize;
                let attr_name = self.code.names[oparg.name_idx() as usize];

                let owner = self.top_value();
                let type_version = self.code.instructions.read_cache_u32(cache_base + 1);

                if type_version != 0 && owner.class().tp_version_tag.load(Acquire) == type_version {
                    // Check instance dict doesn't shadow the method
                    let shadowed = if let Some(dict) = owner.dict() {
                        match dict.get_item_opt(attr_name, vm) {
                            Ok(Some(_)) => true,
                            Ok(None) => false,
                            Err(_) => {
                                // Dict lookup error -> use safe path.
                                return self.load_attr_slow(vm, oparg);
                            }
                        }
                    } else {
                        false
                    };

                    if !shadowed
                        && let Some(func) =
                            self.try_read_cached_descriptor(cache_base, type_version)
                    {
                        let owner = self.pop_value();
                        self.push_value(func);
                        self.push_value(owner);
                        return Ok(None);
                    }
                }
                self.load_attr_slow(vm, oparg)
            }
            Instruction::LoadAttrInstanceValue => {
                let oparg = LoadAttr::from_u32(u32::from(arg));
                let cache_base = self.lasti() as usize;
                let attr_name = self.code.names[oparg.name_idx() as usize];

                let owner = self.top_value();
                let type_version = self.code.instructions.read_cache_u32(cache_base + 1);

                if type_version != 0 && owner.class().tp_version_tag.load(Acquire) == type_version {
                    // Type version matches — no data descriptor for this attr.
                    // Try direct dict lookup, skipping full descriptor protocol.
                    if let Some(dict) = owner.dict()
                        && let Some(value) = dict.get_item_opt(attr_name, vm)?
                    {
                        self.pop_value();
                        self.push_value(value);
                        return Ok(None);
                    }
                    // Not in instance dict — fall through to class lookup via slow path
                }
                self.load_attr_slow(vm, oparg)
            }
            Instruction::LoadAttrWithHint => {
                let oparg = LoadAttr::from_u32(u32::from(arg));
                let cache_base = self.lasti() as usize;
                let attr_name = self.code.names[oparg.name_idx() as usize];

                let owner = self.top_value();
                let type_version = self.code.instructions.read_cache_u32(cache_base + 1);

                if type_version != 0
                    && owner.class().tp_version_tag.load(Acquire) == type_version
                    && let Some(dict) = owner.dict()
                    && let Some(value) = dict.get_item_opt(attr_name, vm)?
                {
                    self.pop_value();
                    if oparg.is_method() {
                        self.push_value(value);
                        self.push_value_opt(None);
                    } else {
                        self.push_value(value);
                    }
                    return Ok(None);
                }

                self.load_attr_slow(vm, oparg)
            }
            Instruction::LoadAttrModule => {
                let oparg = LoadAttr::from_u32(u32::from(arg));
                let cache_base = self.lasti() as usize;
                let attr_name = self.code.names[oparg.name_idx() as usize];

                let owner = self.top_value();
                let type_version = self.code.instructions.read_cache_u32(cache_base + 1);

                if type_version != 0
                    && owner.class().tp_version_tag.load(Acquire) == type_version
                    && let Some(module) = owner.downcast_ref_if_exact::<PyModule>(vm)
                    && let Ok(value) = module.get_attr(attr_name, vm)
                {
                    self.pop_value();
                    if oparg.is_method() {
                        self.push_value(value);
                        self.push_value_opt(None);
                    } else {
                        self.push_value(value);
                    }
                    return Ok(None);
                }
                self.load_attr_slow(vm, oparg)
            }
            Instruction::LoadAttrNondescriptorNoDict => {
                let oparg = LoadAttr::from_u32(u32::from(arg));
                let cache_base = self.lasti() as usize;

                let owner = self.top_value();
                let type_version = self.code.instructions.read_cache_u32(cache_base + 1);

                if type_version != 0
                    && owner.class().tp_version_tag.load(Acquire) == type_version
                    && let Some(attr) = self.try_read_cached_descriptor(cache_base, type_version)
                {
                    self.pop_value();
                    if oparg.is_method() {
                        self.push_value(attr);
                        self.push_value_opt(None);
                    } else {
                        self.push_value(attr);
                    }
                    return Ok(None);
                }
                self.load_attr_slow(vm, oparg)
            }
            Instruction::LoadAttrNondescriptorWithValues => {
                let oparg = LoadAttr::from_u32(u32::from(arg));
                let cache_base = self.lasti() as usize;
                let attr_name = self.code.names[oparg.name_idx() as usize];

                let owner = self.top_value();
                let type_version = self.code.instructions.read_cache_u32(cache_base + 1);

                if type_version != 0 && owner.class().tp_version_tag.load(Acquire) == type_version {
                    // Instance dict has priority — check if attr is shadowed
                    if let Some(dict) = owner.dict()
                        && let Some(value) = dict.get_item_opt(attr_name, vm)?
                    {
                        self.pop_value();
                        if oparg.is_method() {
                            self.push_value(value);
                            self.push_value_opt(None);
                        } else {
                            self.push_value(value);
                        }
                        return Ok(None);
                    }
                    // Not in instance dict — use cached class attr
                    let Some(attr) = self.try_read_cached_descriptor(cache_base, type_version)
                    else {
                        return self.load_attr_slow(vm, oparg);
                    };
                    self.pop_value();
                    if oparg.is_method() {
                        self.push_value(attr);
                        self.push_value_opt(None);
                    } else {
                        self.push_value(attr);
                    }
                    return Ok(None);
                }
                self.load_attr_slow(vm, oparg)
            }
            Instruction::LoadAttrClass => {
                let oparg = LoadAttr::from_u32(u32::from(arg));
                let cache_base = self.lasti() as usize;

                let owner = self.top_value();
                let type_version = self.code.instructions.read_cache_u32(cache_base + 1);

                if type_version != 0
                    && let Some(owner_type) = owner.downcast_ref::<PyType>()
                    && owner_type.tp_version_tag.load(Acquire) == type_version
                    && let Some(attr) = self.try_read_cached_descriptor(cache_base, type_version)
                {
                    self.pop_value();
                    if oparg.is_method() {
                        self.push_value(attr);
                        self.push_value_opt(None);
                    } else {
                        self.push_value(attr);
                    }
                    return Ok(None);
                }
                self.load_attr_slow(vm, oparg)
            }
            Instruction::LoadAttrClassWithMetaclassCheck => {
                let oparg = LoadAttr::from_u32(u32::from(arg));
                let cache_base = self.lasti() as usize;

                let owner = self.top_value();
                let type_version = self.code.instructions.read_cache_u32(cache_base + 1);
                let metaclass_version = self.code.instructions.read_cache_u32(cache_base + 3);

                if type_version != 0
                    && metaclass_version != 0
                    && let Some(owner_type) = owner.downcast_ref::<PyType>()
                    && owner_type.tp_version_tag.load(Acquire) == type_version
                    && owner.class().tp_version_tag.load(Acquire) == metaclass_version
                    && let Some(attr) = self.try_read_cached_descriptor(cache_base, type_version)
                {
                    self.pop_value();
                    if oparg.is_method() {
                        self.push_value(attr);
                        self.push_value_opt(None);
                    } else {
                        self.push_value(attr);
                    }
                    return Ok(None);
                }
                self.load_attr_slow(vm, oparg)
            }
            Instruction::LoadAttrGetattributeOverridden => {
                let oparg = LoadAttr::from_u32(u32::from(arg));
                let cache_base = self.lasti() as usize;
                let owner = self.top_value();
                let type_version = self.code.instructions.read_cache_u32(cache_base + 1);
                let func_version = self.code.instructions.read_cache_u32(cache_base + 3);

                if !oparg.is_method()
                    && !self.specialization_eval_frame_active(vm)
                    && type_version != 0
                    && func_version != 0
                    && owner.class().tp_version_tag.load(Acquire) == type_version
                    && let Some(func_obj) =
                        self.try_read_cached_descriptor(cache_base, type_version)
                    && let Some(func) = func_obj.downcast_ref_if_exact::<PyFunction>(vm)
                    && func.func_version() == func_version
                    && self.specialization_has_datastack_space_for_func(vm, func)
                {
                    debug_assert!(func.has_exact_argcount(2));
                    let owner = self.pop_value();
                    let attr_name = self.code.names[oparg.name_idx() as usize].to_owned().into();
                    let result = func.invoke_exact_args(vec![owner, attr_name], vm)?;
                    self.push_value(result);
                    return Ok(None);
                }
                self.load_attr_slow(vm, oparg)
            }
            Instruction::LoadAttrSlot => {
                let oparg = LoadAttr::from_u32(u32::from(arg));
                let cache_base = self.lasti() as usize;

                let owner = self.top_value();
                let type_version = self.code.instructions.read_cache_u32(cache_base + 1);

                if type_version != 0 && owner.class().tp_version_tag.load(Acquire) == type_version {
                    let slot_offset =
                        self.code.instructions.read_cache_u32(cache_base + 3) as usize;
                    if let Some(value) = owner.get_slot(slot_offset) {
                        self.pop_value();
                        if oparg.is_method() {
                            self.push_value(value);
                            self.push_value_opt(None);
                        } else {
                            self.push_value(value);
                        }
                        return Ok(None);
                    }
                    // Slot is None → AttributeError (fall through to slow path)
                }
                self.load_attr_slow(vm, oparg)
            }
            Instruction::LoadAttrProperty => {
                let oparg = LoadAttr::from_u32(u32::from(arg));
                let cache_base = self.lasti() as usize;

                let owner = self.top_value();
                let type_version = self.code.instructions.read_cache_u32(cache_base + 1);

                if type_version != 0
                    && !self.specialization_eval_frame_active(vm)
                    && owner.class().tp_version_tag.load(Acquire) == type_version
                    && let Some(fget_obj) =
                        self.try_read_cached_descriptor(cache_base, type_version)
                    && let Some(func) = fget_obj.downcast_ref_if_exact::<PyFunction>(vm)
                    && func.can_specialize_call(1)
                    && self.specialization_has_datastack_space_for_func(vm, func)
                {
                    let owner = self.pop_value();
                    let result = func.invoke_exact_args(vec![owner], vm)?;
                    self.push_value(result);
                    return Ok(None);
                }
                self.load_attr_slow(vm, oparg)
            }
            Instruction::StoreAttrInstanceValue => {
                let attr_idx = u32::from(arg);
                let instr_idx = self.lasti() as usize - 1;
                let cache_base = instr_idx + 1;
                let attr_name = self.code.names[attr_idx as usize];
                let owner = self.top_value();
                let type_version = self.code.instructions.read_cache_u32(cache_base + 1);

                if type_version != 0
                    && owner.class().tp_version_tag.load(Acquire) == type_version
                    && let Some(dict) = owner.dict()
                {
                    self.pop_value(); // owner
                    let value = self.pop_value();
                    dict.set_item(attr_name, value, vm)?;
                    return Ok(None);
                }
                self.store_attr(vm, attr_idx)
            }
            Instruction::StoreAttrWithHint => {
                let attr_idx = u32::from(arg);
                let instr_idx = self.lasti() as usize - 1;
                let cache_base = instr_idx + 1;
                let attr_name = self.code.names[attr_idx as usize];
                let owner = self.top_value();
                let type_version = self.code.instructions.read_cache_u32(cache_base + 1);

                if type_version != 0
                    && owner.class().tp_version_tag.load(Acquire) == type_version
                    && let Some(dict) = owner.dict()
                {
                    self.pop_value(); // owner
                    let value = self.pop_value();
                    dict.set_item(attr_name, value, vm)?;
                    return Ok(None);
                }
                self.store_attr(vm, attr_idx)
            }
            Instruction::StoreAttrSlot => {
                let instr_idx = self.lasti() as usize - 1;
                let cache_base = instr_idx + 1;
                let type_version = self.code.instructions.read_cache_u32(cache_base + 1);
                let version_match = type_version != 0 && {
                    let owner = self.top_value();
                    owner.class().tp_version_tag.load(Acquire) == type_version
                };

                if version_match {
                    let slot_offset =
                        self.code.instructions.read_cache_u16(cache_base + 3) as usize;
                    let owner = self.pop_value();
                    let value = self.pop_value();
                    owner.set_slot(slot_offset, Some(value));
                    return Ok(None);
                }
                let attr_idx = u32::from(arg);
                self.store_attr(vm, attr_idx)
            }
            Instruction::StoreSubscrListInt => {
                // Stack: [value, obj, idx] (TOS=idx, TOS1=obj, TOS2=value)
                let idx = self.pop_value();
                let obj = self.pop_value();
                let value = self.pop_value();
                if let Some(list) = obj.downcast_ref_if_exact::<PyList>(vm)
                    && let Some(int_idx) = idx.downcast_ref_if_exact::<PyInt>(vm)
                    && let Some(i) = specialization_nonnegative_compact_index(int_idx, vm)
                {
                    let mut vec = list.borrow_vec_mut();
                    if i < vec.len() {
                        vec[i] = value;
                        return Ok(None);
                    }
                }
                obj.set_item(&*idx, value, vm)?;
                Ok(None)
            }
            Instruction::StoreSubscrDict => {
                // Stack: [value, obj, idx] (TOS=idx, TOS1=obj, TOS2=value)
                let idx = self.pop_value();
                let obj = self.pop_value();
                let value = self.pop_value();
                if let Some(dict) = obj.downcast_ref_if_exact::<PyDict>(vm) {
                    dict.set_item(&*idx, value, vm)?;
                    Ok(None)
                } else {
                    obj.set_item(&*idx, value, vm)?;
                    Ok(None)
                }
            }
            // Specialized BINARY_OP opcodes
            Instruction::BinaryOpAddInt => {
                self.execute_binary_op_int(vm, |a, b| a + b, bytecode::BinaryOperator::Add)
            }
            Instruction::BinaryOpSubtractInt => {
                self.execute_binary_op_int(vm, |a, b| a - b, bytecode::BinaryOperator::Subtract)
            }
            Instruction::BinaryOpMultiplyInt => {
                self.execute_binary_op_int(vm, |a, b| a * b, bytecode::BinaryOperator::Multiply)
            }
            Instruction::BinaryOpAddFloat => {
                self.execute_binary_op_float(vm, |a, b| a + b, bytecode::BinaryOperator::Add)
            }
            Instruction::BinaryOpSubtractFloat => {
                self.execute_binary_op_float(vm, |a, b| a - b, bytecode::BinaryOperator::Subtract)
            }
            Instruction::BinaryOpMultiplyFloat => {
                self.execute_binary_op_float(vm, |a, b| a * b, bytecode::BinaryOperator::Multiply)
            }
            Instruction::BinaryOpAddUnicode => {
                let b = self.top_value();
                let a = self.nth_value(1);
                if let (Some(a_str), Some(b_str)) = (
                    a.downcast_ref_if_exact::<PyStr>(vm),
                    b.downcast_ref_if_exact::<PyStr>(vm),
                ) {
                    let result = a_str.as_wtf8().py_add(b_str.as_wtf8());
                    self.pop_value();
                    self.pop_value();
                    self.push_value(result.to_pyobject(vm));
                    Ok(None)
                } else {
                    self.execute_bin_op(vm, bytecode::BinaryOperator::Add)
                }
            }
            Instruction::BinaryOpSubscrGetitem => {
                let owner = self.nth_value(1);
                if !self.specialization_eval_frame_active(vm)
                    && let Some((func, func_version)) =
                        owner.class().get_cached_getitem_for_specialization()
                    && func.func_version() == func_version
                    && self.specialization_has_datastack_space_for_func(vm, &func)
                {
                    debug_assert!(func.has_exact_argcount(2));
                    let sub = self.pop_value();
                    let owner = self.pop_value();
                    let result = func.invoke_exact_args(vec![owner, sub], vm)?;
                    self.push_value(result);
                    return Ok(None);
                }
                self.execute_bin_op(vm, bytecode::BinaryOperator::Subscr)
            }
            Instruction::BinaryOpExtend => {
                let op = self.binary_op_from_arg(arg);
                let b = self.top_value();
                let a = self.nth_value(1);
                let cache_base = self.lasti() as usize;
                if let Some(descr) = self.read_cached_binary_op_extend_descr(cache_base)
                    && descr.oparg == op
                    && (descr.guard)(a, b, vm)
                    && let Some(result) = (descr.action)(a, b, vm)
                {
                    self.pop_value();
                    self.pop_value();
                    self.push_value(result);
                    Ok(None)
                } else {
                    self.execute_bin_op(vm, op)
                }
            }
            Instruction::BinaryOpSubscrListInt => {
                let b = self.top_value();
                let a = self.nth_value(1);
                if let (Some(list), Some(idx)) = (
                    a.downcast_ref_if_exact::<PyList>(vm),
                    b.downcast_ref_if_exact::<PyInt>(vm),
                ) && let Some(i) = specialization_nonnegative_compact_index(idx, vm)
                {
                    let vec = list.borrow_vec();
                    if i < vec.len() {
                        let value = vec.do_get(i);
                        drop(vec);
                        self.pop_value();
                        self.pop_value();
                        self.push_value(value);
                        return Ok(None);
                    }
                }
                self.execute_bin_op(vm, bytecode::BinaryOperator::Subscr)
            }
            Instruction::BinaryOpSubscrTupleInt => {
                let b = self.top_value();
                let a = self.nth_value(1);
                if let (Some(tuple), Some(idx)) = (
                    a.downcast_ref_if_exact::<PyTuple>(vm),
                    b.downcast_ref_if_exact::<PyInt>(vm),
                ) && let Some(i) = specialization_nonnegative_compact_index(idx, vm)
                {
                    let elements = tuple.as_slice();
                    if i < elements.len() {
                        let value = elements[i].clone();
                        self.pop_value();
                        self.pop_value();
                        self.push_value(value);
                        return Ok(None);
                    }
                }
                self.execute_bin_op(vm, bytecode::BinaryOperator::Subscr)
            }
            Instruction::BinaryOpSubscrDict => {
                let b = self.top_value();
                let a = self.nth_value(1);
                if let Some(dict) = a.downcast_ref_if_exact::<PyDict>(vm) {
                    match dict.get_item_opt(b, vm) {
                        Ok(Some(value)) => {
                            self.pop_value();
                            self.pop_value();
                            self.push_value(value);
                            return Ok(None);
                        }
                        Ok(None) => {
                            let key = self.pop_value();
                            self.pop_value();
                            return Err(vm.new_key_error(key));
                        }
                        Err(e) => {
                            return Err(e);
                        }
                    }
                }
                self.execute_bin_op(vm, bytecode::BinaryOperator::Subscr)
            }
            Instruction::BinaryOpSubscrStrInt => {
                let b = self.top_value();
                let a = self.nth_value(1);
                if let (Some(a_str), Some(b_int)) = (
                    a.downcast_ref_if_exact::<PyStr>(vm),
                    b.downcast_ref_if_exact::<PyInt>(vm),
                ) && let Some(i) = specialization_nonnegative_compact_index(b_int, vm)
                    && let Ok(ch) = a_str.getitem_by_index(vm, i as isize)
                    && ch.is_ascii()
                {
                    let ascii_idx = ch.to_u32() as usize;
                    self.pop_value();
                    self.pop_value();
                    self.push_value(vm.ctx.ascii_char_cache[ascii_idx].clone().into());
                    return Ok(None);
                }
                self.execute_bin_op(vm, bytecode::BinaryOperator::Subscr)
            }
            Instruction::BinaryOpSubscrListSlice => {
                let b = self.top_value();
                let a = self.nth_value(1);
                if a.downcast_ref_if_exact::<PyList>(vm).is_some()
                    && b.downcast_ref::<PySlice>().is_some()
                {
                    let b_owned = self.pop_value();
                    let a_owned = self.pop_value();
                    let result = a_owned.get_item(b_owned.as_object(), vm)?;
                    self.push_value(result);
                    return Ok(None);
                }
                self.execute_bin_op(vm, bytecode::BinaryOperator::Subscr)
            }
            Instruction::CallPyExactArgs => {
                let instr_idx = self.lasti() as usize - 1;
                let cache_base = instr_idx + 1;
                let cached_version = self.code.instructions.read_cache_u32(cache_base + 1);
                let nargs: u32 = arg.into();
                if self.specialization_eval_frame_active(vm) {
                    return self.execute_call_vectorcall(nargs, vm);
                }
                // Stack: [callable, self_or_null, arg1, ..., argN]
                let stack_len = self.localsplus.stack_len();
                let self_or_null_is_some = self
                    .localsplus
                    .stack_index(stack_len - nargs as usize - 1)
                    .is_some();
                let callable = self.nth_value(nargs + 1);
                if let Some(func) = callable.downcast_ref_if_exact::<PyFunction>(vm)
                    && func.func_version() == cached_version
                    && cached_version != 0
                {
                    let effective_nargs = nargs + u32::from(self_or_null_is_some);
                    if !func.has_exact_argcount(effective_nargs) {
                        return self.execute_call_vectorcall(nargs, vm);
                    }
                    if !self.specialization_has_datastack_space_for_func(vm, func) {
                        return self.execute_call_vectorcall(nargs, vm);
                    }
                    if self.specialization_call_recursion_guard(vm) {
                        return self.execute_call_vectorcall(nargs, vm);
                    }
                    let pos_args: Vec<PyObjectRef> = self.pop_multiple(nargs as usize).collect();
                    let self_or_null = self.pop_value_opt();
                    let callable = self.pop_value();
                    let func = callable.downcast_ref_if_exact::<PyFunction>(vm).unwrap();
                    let args = if let Some(self_val) = self_or_null {
                        let mut all_args = Vec::with_capacity(pos_args.len() + 1);
                        all_args.push(self_val);
                        all_args.extend(pos_args);
                        all_args
                    } else {
                        pos_args
                    };
                    let result = func.invoke_exact_args(args, vm)?;
                    self.push_value(result);
                    Ok(None)
                } else {
                    self.execute_call_vectorcall(nargs, vm)
                }
            }
            Instruction::CallBoundMethodExactArgs => {
                let instr_idx = self.lasti() as usize - 1;
                let cache_base = instr_idx + 1;
                let cached_version = self.code.instructions.read_cache_u32(cache_base + 1);
                let nargs: u32 = arg.into();
                if self.specialization_eval_frame_active(vm) {
                    return self.execute_call_vectorcall(nargs, vm);
                }
                // Stack: [callable, self_or_null(NULL), arg1, ..., argN]
                let stack_len = self.localsplus.stack_len();
                let self_or_null_is_some = self
                    .localsplus
                    .stack_index(stack_len - nargs as usize - 1)
                    .is_some();
                let callable = self.nth_value(nargs + 1);
                if !self_or_null_is_some
                    && let Some(bound_method) = callable.downcast_ref_if_exact::<PyBoundMethod>(vm)
                {
                    let bound_function = bound_method.function_obj().clone();
                    let bound_self = bound_method.self_obj().clone();
                    if let Some(func) = bound_function.downcast_ref_if_exact::<PyFunction>(vm)
                        && func.func_version() == cached_version
                        && cached_version != 0
                    {
                        if !func.has_exact_argcount(nargs + 1) {
                            return self.execute_call_vectorcall(nargs, vm);
                        }
                        if !self.specialization_has_datastack_space_for_func(vm, func) {
                            return self.execute_call_vectorcall(nargs, vm);
                        }
                        if self.specialization_call_recursion_guard(vm) {
                            return self.execute_call_vectorcall(nargs, vm);
                        }
                        let pos_args: Vec<PyObjectRef> =
                            self.pop_multiple(nargs as usize).collect();
                        self.pop_value_opt(); // null (self_or_null)
                        self.pop_value(); // callable (bound method)
                        let mut all_args = Vec::with_capacity(pos_args.len() + 1);
                        all_args.push(bound_self);
                        all_args.extend(pos_args);
                        let result = func.invoke_exact_args(all_args, vm)?;
                        self.push_value(result);
                        return Ok(None);
                    }
                }
                self.execute_call_vectorcall(nargs, vm)
            }
            Instruction::CallLen => {
                let nargs: u32 = arg.into();
                if nargs == 1 {
                    // Stack: [callable, null, arg]
                    let obj = self.pop_value(); // arg
                    let null = self.pop_value_opt();
                    let callable = self.pop_value();
                    if null.is_none()
                        && vm
                            .callable_cache
                            .len
                            .as_ref()
                            .is_some_and(|len_callable| callable.is(len_callable))
                    {
                        let len = obj.length(vm)?;
                        self.push_value(vm.ctx.new_int(len).into());
                        return Ok(None);
                    }
                    // Guard failed — re-push and fallback
                    self.push_value(callable);
                    self.push_value_opt(null);
                    self.push_value(obj);
                }
                self.execute_call_vectorcall(nargs, vm)
            }
            Instruction::CallIsinstance => {
                let nargs: u32 = arg.into();
                let stack_len = self.localsplus.stack_len();
                let self_or_null_is_some = self
                    .localsplus
                    .stack_index(stack_len - nargs as usize - 1)
                    .is_some();
                let effective_nargs = nargs + u32::from(self_or_null_is_some);
                if effective_nargs == 2 {
                    let callable = self.nth_value(nargs + 1);
                    if vm
                        .callable_cache
                        .isinstance
                        .as_ref()
                        .is_some_and(|isinstance_callable| callable.is(isinstance_callable))
                    {
                        let nargs_usize = nargs as usize;
                        let pos_args: Vec<PyObjectRef> = self.pop_multiple(nargs_usize).collect();
                        let self_or_null = self.pop_value_opt();
                        self.pop_value(); // callable
                        let mut all_args = Vec::with_capacity(2);
                        if let Some(self_val) = self_or_null {
                            all_args.push(self_val);
                        }
                        all_args.extend(pos_args);
                        let result = all_args[0].is_instance(&all_args[1], vm)?;
                        self.push_value(vm.ctx.new_bool(result).into());
                        return Ok(None);
                    }
                }
                self.execute_call_vectorcall(nargs, vm)
            }
            Instruction::CallType1 => {
                let nargs: u32 = arg.into();
                if nargs == 1 {
                    // Stack: [callable, null, arg]
                    let obj = self.pop_value();
                    let null = self.pop_value_opt();
                    let callable = self.pop_value();
                    if null.is_none() && callable.is(vm.ctx.types.type_type.as_object()) {
                        let tp = obj.class().to_owned().into();
                        self.push_value(tp);
                        return Ok(None);
                    }
                    // Guard failed — re-push and fallback
                    self.push_value(callable);
                    self.push_value_opt(null);
                    self.push_value(obj);
                }
                self.execute_call_vectorcall(nargs, vm)
            }
            Instruction::CallStr1 => {
                let nargs: u32 = arg.into();
                if nargs == 1 {
                    let obj = self.pop_value();
                    let null = self.pop_value_opt();
                    let callable = self.pop_value();
                    if null.is_none() && callable.is(vm.ctx.types.str_type.as_object()) {
                        let result = obj.str(vm)?;
                        self.push_value(result.into());
                        return Ok(None);
                    }
                    self.push_value(callable);
                    self.push_value_opt(null);
                    self.push_value(obj);
                }
                self.execute_call_vectorcall(nargs, vm)
            }
            Instruction::CallTuple1 => {
                let nargs: u32 = arg.into();
                if nargs == 1 {
                    let obj = self.pop_value();
                    let null = self.pop_value_opt();
                    let callable = self.pop_value();
                    if null.is_none() && callable.is(vm.ctx.types.tuple_type.as_object()) {
                        // tuple(x) returns x as-is when x is already an exact tuple
                        if let Ok(tuple) = obj.clone().downcast_exact::<PyTuple>(vm) {
                            self.push_value(tuple.into_pyref().into());
                        } else {
                            let elements: Vec<PyObjectRef> = vm.extract_elements_with(&obj, Ok)?;
                            self.push_value(vm.ctx.new_tuple(elements).into());
                        }
                        return Ok(None);
                    }
                    self.push_value(callable);
                    self.push_value_opt(null);
                    self.push_value(obj);
                }
                self.execute_call_vectorcall(nargs, vm)
            }
            Instruction::CallBuiltinO => {
                let nargs: u32 = arg.into();
                let stack_len = self.localsplus.stack_len();
                let self_or_null_is_some = self
                    .localsplus
                    .stack_index(stack_len - nargs as usize - 1)
                    .is_some();
                let effective_nargs = nargs + u32::from(self_or_null_is_some);
                let callable = self.nth_value(nargs + 1);
                if let Some(native) = callable.downcast_ref_if_exact::<PyNativeFunction>(vm) {
                    let call_conv = native.value.flags
                        & (PyMethodFlags::VARARGS
                            | PyMethodFlags::FASTCALL
                            | PyMethodFlags::NOARGS
                            | PyMethodFlags::O
                            | PyMethodFlags::KEYWORDS);
                    if call_conv == PyMethodFlags::O && effective_nargs == 1 {
                        let nargs_usize = nargs as usize;
                        let pos_args: Vec<PyObjectRef> = self.pop_multiple(nargs_usize).collect();
                        let self_or_null = self.pop_value_opt();
                        let callable = self.pop_value();
                        let mut args_vec = Vec::with_capacity(effective_nargs as usize);
                        if let Some(self_val) = self_or_null {
                            args_vec.push(self_val);
                        }
                        args_vec.extend(pos_args);
                        let result =
                            callable.vectorcall(args_vec, effective_nargs as usize, None, vm)?;
                        self.push_value(result);
                        return Ok(None);
                    }
                }
                self.execute_call_vectorcall(nargs, vm)
            }
            Instruction::CallBuiltinFast => {
                let nargs: u32 = arg.into();
                let stack_len = self.localsplus.stack_len();
                let self_or_null_is_some = self
                    .localsplus
                    .stack_index(stack_len - nargs as usize - 1)
                    .is_some();
                let effective_nargs = nargs + u32::from(self_or_null_is_some);
                let callable = self.nth_value(nargs + 1);
                if let Some(native) = callable.downcast_ref_if_exact::<PyNativeFunction>(vm) {
                    let call_conv = native.value.flags
                        & (PyMethodFlags::VARARGS
                            | PyMethodFlags::FASTCALL
                            | PyMethodFlags::NOARGS
                            | PyMethodFlags::O
                            | PyMethodFlags::KEYWORDS);
                    if call_conv == PyMethodFlags::FASTCALL {
                        let nargs_usize = nargs as usize;
                        let pos_args: Vec<PyObjectRef> = self.pop_multiple(nargs_usize).collect();
                        let self_or_null = self.pop_value_opt();
                        let callable = self.pop_value();
                        let mut args_vec = Vec::with_capacity(effective_nargs as usize);
                        if let Some(self_val) = self_or_null {
                            args_vec.push(self_val);
                        }
                        args_vec.extend(pos_args);
                        let result =
                            callable.vectorcall(args_vec, effective_nargs as usize, None, vm)?;
                        self.push_value(result);
                        return Ok(None);
                    }
                }
                self.execute_call_vectorcall(nargs, vm)
            }
            Instruction::CallPyGeneral => {
                let instr_idx = self.lasti() as usize - 1;
                let cache_base = instr_idx + 1;
                let cached_version = self.code.instructions.read_cache_u32(cache_base + 1);
                let nargs: u32 = arg.into();
                if self.specialization_eval_frame_active(vm) {
                    return self.execute_call_vectorcall(nargs, vm);
                }
                let callable = self.nth_value(nargs + 1);
                if let Some(func) = callable.downcast_ref_if_exact::<PyFunction>(vm)
                    && func.func_version() == cached_version
                    && cached_version != 0
                {
                    if self.specialization_call_recursion_guard(vm) {
                        return self.execute_call_vectorcall(nargs, vm);
                    }
                    let nargs_usize = nargs as usize;
                    let pos_args: Vec<PyObjectRef> = self.pop_multiple(nargs_usize).collect();
                    let self_or_null = self.pop_value_opt();
                    let callable = self.pop_value();
                    let (args_vec, effective_nargs) = if let Some(self_val) = self_or_null {
                        let mut v = Vec::with_capacity(nargs_usize + 1);
                        v.push(self_val);
                        v.extend(pos_args);
                        (v, nargs_usize + 1)
                    } else {
                        (pos_args, nargs_usize)
                    };
                    let result =
                        vectorcall_function(&callable, args_vec, effective_nargs, None, vm)?;
                    self.push_value(result);
                    Ok(None)
                } else {
                    self.execute_call_vectorcall(nargs, vm)
                }
            }
            Instruction::CallBoundMethodGeneral => {
                let instr_idx = self.lasti() as usize - 1;
                let cache_base = instr_idx + 1;
                let cached_version = self.code.instructions.read_cache_u32(cache_base + 1);
                let nargs: u32 = arg.into();
                if self.specialization_eval_frame_active(vm) {
                    return self.execute_call_vectorcall(nargs, vm);
                }
                let stack_len = self.localsplus.stack_len();
                let self_or_null_is_some = self
                    .localsplus
                    .stack_index(stack_len - nargs as usize - 1)
                    .is_some();
                let callable = self.nth_value(nargs + 1);
                if !self_or_null_is_some
                    && let Some(bound_method) = callable.downcast_ref_if_exact::<PyBoundMethod>(vm)
                {
                    let bound_function = bound_method.function_obj().clone();
                    let bound_self = bound_method.self_obj().clone();
                    if let Some(func) = bound_function.downcast_ref_if_exact::<PyFunction>(vm)
                        && func.func_version() == cached_version
                        && cached_version != 0
                    {
                        if self.specialization_call_recursion_guard(vm) {
                            return self.execute_call_vectorcall(nargs, vm);
                        }
                        let nargs_usize = nargs as usize;
                        let pos_args: Vec<PyObjectRef> = self.pop_multiple(nargs_usize).collect();
                        self.pop_value_opt(); // null (self_or_null)
                        self.pop_value(); // callable (bound method)
                        let mut args_vec = Vec::with_capacity(nargs_usize + 1);
                        args_vec.push(bound_self);
                        args_vec.extend(pos_args);
                        let result = vectorcall_function(
                            &bound_function,
                            args_vec,
                            nargs_usize + 1,
                            None,
                            vm,
                        )?;
                        self.push_value(result);
                        return Ok(None);
                    }
                }
                self.execute_call_vectorcall(nargs, vm)
            }
            Instruction::CallListAppend => {
                let nargs: u32 = arg.into();
                if nargs == 1 {
                    // Stack: [callable, self_or_null, item]
                    let stack_len = self.localsplus.stack_len();
                    let self_or_null_is_some = self.localsplus.stack_index(stack_len - 2).is_some();
                    let callable = self.nth_value(2);
                    let self_is_list = self
                        .localsplus
                        .stack_index(stack_len - 2)
                        .as_ref()
                        .is_some_and(|obj| obj.downcast_ref::<PyList>().is_some());
                    if vm
                        .callable_cache
                        .list_append
                        .as_ref()
                        .is_some_and(|list_append| callable.is(list_append))
                        && self_or_null_is_some
                        && self_is_list
                    {
                        let item = self.pop_value();
                        let self_or_null = self.pop_value_opt();
                        let callable = self.pop_value();
                        if let Some(list_obj) = self_or_null.as_ref()
                            && let Some(list) = list_obj.downcast_ref::<PyList>()
                        {
                            list.append(item);
                            // CALL_LIST_APPEND fuses the following POP_TOP.
                            self.jump_relative_forward(
                                1,
                                Instruction::CallListAppend.cache_entries() as u32,
                            );
                            return Ok(None);
                        }
                        self.push_value(callable);
                        self.push_value_opt(self_or_null);
                        self.push_value(item);
                    }
                }
                self.execute_call_vectorcall(nargs, vm)
            }
            Instruction::CallMethodDescriptorNoargs => {
                let nargs: u32 = arg.into();
                let stack_len = self.localsplus.stack_len();
                let self_or_null_is_some = self
                    .localsplus
                    .stack_index(stack_len - nargs as usize - 1)
                    .is_some();
                let total_nargs = nargs + u32::from(self_or_null_is_some);
                if total_nargs == 1 {
                    let callable = self.nth_value(nargs + 1);
                    let self_index =
                        stack_len - nargs as usize - 1 + usize::from(!self_or_null_is_some);
                    if let Some(descr) = callable.downcast_ref_if_exact::<PyMethodDescriptor>(vm)
                        && (descr.method.flags
                            & (PyMethodFlags::VARARGS
                                | PyMethodFlags::FASTCALL
                                | PyMethodFlags::NOARGS
                                | PyMethodFlags::O
                                | PyMethodFlags::KEYWORDS))
                            == PyMethodFlags::NOARGS
                        && self
                            .localsplus
                            .stack_index(self_index)
                            .as_ref()
                            .is_some_and(|self_obj| self_obj.class().is(descr.objclass))
                    {
                        let func = descr.method.func;
                        let positional_args: Vec<PyObjectRef> =
                            self.pop_multiple(nargs as usize).collect();
                        let self_or_null = self.pop_value_opt();
                        self.pop_value(); // callable
                        let mut all_args = Vec::with_capacity(total_nargs as usize);
                        if let Some(self_val) = self_or_null {
                            all_args.push(self_val);
                        }
                        all_args.extend(positional_args);
                        let args = FuncArgs {
                            args: all_args,
                            kwargs: Default::default(),
                        };
                        let result = func(vm, args)?;
                        self.push_value(result);
                        return Ok(None);
                    }
                }
                self.execute_call_vectorcall(nargs, vm)
            }
            Instruction::CallMethodDescriptorO => {
                let nargs: u32 = arg.into();
                let stack_len = self.localsplus.stack_len();
                let self_or_null_is_some = self
                    .localsplus
                    .stack_index(stack_len - nargs as usize - 1)
                    .is_some();
                let total_nargs = nargs + u32::from(self_or_null_is_some);
                if total_nargs == 2 {
                    let callable = self.nth_value(nargs + 1);
                    let self_index =
                        stack_len - nargs as usize - 1 + usize::from(!self_or_null_is_some);
                    if let Some(descr) = callable.downcast_ref_if_exact::<PyMethodDescriptor>(vm)
                        && (descr.method.flags
                            & (PyMethodFlags::VARARGS
                                | PyMethodFlags::FASTCALL
                                | PyMethodFlags::NOARGS
                                | PyMethodFlags::O
                                | PyMethodFlags::KEYWORDS))
                            == PyMethodFlags::O
                        && self
                            .localsplus
                            .stack_index(self_index)
                            .as_ref()
                            .is_some_and(|self_obj| self_obj.class().is(descr.objclass))
                    {
                        let func = descr.method.func;
                        let positional_args: Vec<PyObjectRef> =
                            self.pop_multiple(nargs as usize).collect();
                        let self_or_null = self.pop_value_opt();
                        self.pop_value(); // callable
                        let mut all_args = Vec::with_capacity(total_nargs as usize);
                        if let Some(self_val) = self_or_null {
                            all_args.push(self_val);
                        }
                        all_args.extend(positional_args);
                        let args = FuncArgs {
                            args: all_args,
                            kwargs: Default::default(),
                        };
                        let result = func(vm, args)?;
                        self.push_value(result);
                        return Ok(None);
                    }
                }
                self.execute_call_vectorcall(nargs, vm)
            }
            Instruction::CallMethodDescriptorFast => {
                let nargs: u32 = arg.into();
                let stack_len = self.localsplus.stack_len();
                let self_or_null_is_some = self
                    .localsplus
                    .stack_index(stack_len - nargs as usize - 1)
                    .is_some();
                let total_nargs = nargs + u32::from(self_or_null_is_some);
                let callable = self.nth_value(nargs + 1);
                let self_index =
                    stack_len - nargs as usize - 1 + usize::from(!self_or_null_is_some);
                if total_nargs > 0
                    && let Some(descr) = callable.downcast_ref_if_exact::<PyMethodDescriptor>(vm)
                    && (descr.method.flags
                        & (PyMethodFlags::VARARGS
                            | PyMethodFlags::FASTCALL
                            | PyMethodFlags::NOARGS
                            | PyMethodFlags::O
                            | PyMethodFlags::KEYWORDS))
                        == PyMethodFlags::FASTCALL
                    && self
                        .localsplus
                        .stack_index(self_index)
                        .as_ref()
                        .is_some_and(|self_obj| self_obj.class().is(descr.objclass))
                {
                    let func = descr.method.func;
                    let positional_args: Vec<PyObjectRef> =
                        self.pop_multiple(nargs as usize).collect();
                    let self_or_null = self.pop_value_opt();
                    self.pop_value(); // callable
                    let mut all_args = Vec::with_capacity(total_nargs as usize);
                    if let Some(self_val) = self_or_null {
                        all_args.push(self_val);
                    }
                    all_args.extend(positional_args);
                    let args = FuncArgs {
                        args: all_args,
                        kwargs: Default::default(),
                    };
                    let result = func(vm, args)?;
                    self.push_value(result);
                    return Ok(None);
                }
                self.execute_call_vectorcall(nargs, vm)
            }
            Instruction::CallBuiltinClass => {
                let nargs: u32 = arg.into();
                let callable = self.nth_value(nargs + 1);
                if let Some(cls) = callable.downcast_ref::<PyType>()
                    && cls.slots.vectorcall.load().is_some()
                {
                    let nargs_usize = nargs as usize;
                    let pos_args: Vec<PyObjectRef> = self.pop_multiple(nargs_usize).collect();
                    let self_or_null = self.pop_value_opt();
                    let callable = self.pop_value();
                    let self_is_some = self_or_null.is_some();
                    let mut args_vec = Vec::with_capacity(nargs_usize + usize::from(self_is_some));
                    if let Some(self_val) = self_or_null {
                        args_vec.push(self_val);
                    }
                    args_vec.extend(pos_args);
                    let result = callable.vectorcall(
                        args_vec,
                        nargs_usize + usize::from(self_is_some),
                        None,
                        vm,
                    )?;
                    self.push_value(result);
                    return Ok(None);
                }
                self.execute_call_vectorcall(nargs, vm)
            }
            Instruction::CallAllocAndEnterInit => {
                let instr_idx = self.lasti() as usize - 1;
                let cache_base = instr_idx + 1;
                let cached_version = self.code.instructions.read_cache_u32(cache_base + 1);
                let nargs: u32 = arg.into();
                let callable = self.nth_value(nargs + 1);
                let stack_len = self.localsplus.stack_len();
                let self_or_null_is_some = self
                    .localsplus
                    .stack_index(stack_len - nargs as usize - 1)
                    .is_some();
                if !self.specialization_eval_frame_active(vm)
                    && !self_or_null_is_some
                    && cached_version != 0
                    && let Some(cls) = callable.downcast_ref::<PyType>()
                    && cls.tp_version_tag.load(Acquire) == cached_version
                    && let Some(init_func) = cls.get_cached_init_for_specialization(cached_version)
                    && let Some(cls_alloc) = cls.slots.alloc.load()
                {
                    // Match CPython's `code->co_framesize + _Py_InitCleanup.co_framesize`
                    // shape, using RustPython's datastack-backed frame size
                    // equivalent for the extra shim frame.
                    let init_cleanup_stack_bytes =
                        datastack_frame_size_bytes_for_code(&vm.ctx.init_cleanup_code)
                            .expect("_Py_InitCleanup shim is not a generator/coroutine");
                    if !self.specialization_has_datastack_space_for_func_with_extra(
                        vm,
                        &init_func,
                        init_cleanup_stack_bytes,
                    ) {
                        return self.execute_call_vectorcall(nargs, vm);
                    }
                    // CPython creates `_Py_InitCleanup` + `__init__` frames here.
                    // Keep the guard conservative and deopt when the effective
                    // recursion budget for those two frames is not available.
                    if self.specialization_call_recursion_guard_with_extra_frames(vm, 1) {
                        return self.execute_call_vectorcall(nargs, vm);
                    }
                    // Allocate object directly (tp_new == object.__new__, tp_alloc == generic).
                    let cls_ref = cls.to_owned();
                    let new_obj = cls_alloc(cls_ref, 0, vm)?;

                    // Build args: [new_obj, arg1, ..., argN]
                    let pos_args: Vec<PyObjectRef> = self.pop_multiple(nargs as usize).collect();
                    let _null = self.pop_value_opt(); // self_or_null (None)
                    let _callable = self.pop_value(); // callable (type)
                    let result = self
                        .specialization_run_init_cleanup_shim(new_obj, &init_func, pos_args, vm)?;
                    self.push_value(result);
                    return Ok(None);
                }
                self.execute_call_vectorcall(nargs, vm)
            }
            Instruction::CallMethodDescriptorFastWithKeywords => {
                // Native function interface is uniform regardless of keyword support
                let nargs: u32 = arg.into();
                let stack_len = self.localsplus.stack_len();
                let self_or_null_is_some = self
                    .localsplus
                    .stack_index(stack_len - nargs as usize - 1)
                    .is_some();
                let total_nargs = nargs + u32::from(self_or_null_is_some);
                let callable = self.nth_value(nargs + 1);
                let self_index =
                    stack_len - nargs as usize - 1 + usize::from(!self_or_null_is_some);
                if total_nargs > 0
                    && let Some(descr) = callable.downcast_ref_if_exact::<PyMethodDescriptor>(vm)
                    && (descr.method.flags
                        & (PyMethodFlags::VARARGS
                            | PyMethodFlags::FASTCALL
                            | PyMethodFlags::NOARGS
                            | PyMethodFlags::O
                            | PyMethodFlags::KEYWORDS))
                        == (PyMethodFlags::FASTCALL | PyMethodFlags::KEYWORDS)
                    && self
                        .localsplus
                        .stack_index(self_index)
                        .as_ref()
                        .is_some_and(|self_obj| self_obj.class().is(descr.objclass))
                {
                    let func = descr.method.func;
                    let positional_args: Vec<PyObjectRef> =
                        self.pop_multiple(nargs as usize).collect();
                    let self_or_null = self.pop_value_opt();
                    self.pop_value(); // callable
                    let mut all_args = Vec::with_capacity(total_nargs as usize);
                    if let Some(self_val) = self_or_null {
                        all_args.push(self_val);
                    }
                    all_args.extend(positional_args);
                    let args = FuncArgs {
                        args: all_args,
                        kwargs: Default::default(),
                    };
                    let result = func(vm, args)?;
                    self.push_value(result);
                    return Ok(None);
                }
                self.execute_call_vectorcall(nargs, vm)
            }
            Instruction::CallBuiltinFastWithKeywords => {
                // Native function interface is uniform regardless of keyword support
                let nargs: u32 = arg.into();
                let stack_len = self.localsplus.stack_len();
                let self_or_null_is_some = self
                    .localsplus
                    .stack_index(stack_len - nargs as usize - 1)
                    .is_some();
                let effective_nargs = nargs + u32::from(self_or_null_is_some);
                let callable = self.nth_value(nargs + 1);
                if let Some(native) = callable.downcast_ref_if_exact::<PyNativeFunction>(vm) {
                    let call_conv = native.value.flags
                        & (PyMethodFlags::VARARGS
                            | PyMethodFlags::FASTCALL
                            | PyMethodFlags::NOARGS
                            | PyMethodFlags::O
                            | PyMethodFlags::KEYWORDS);
                    if call_conv == (PyMethodFlags::FASTCALL | PyMethodFlags::KEYWORDS) {
                        let nargs_usize = nargs as usize;
                        let pos_args: Vec<PyObjectRef> = self.pop_multiple(nargs_usize).collect();
                        let self_or_null = self.pop_value_opt();
                        let callable = self.pop_value();
                        let mut args_vec = Vec::with_capacity(effective_nargs as usize);
                        if let Some(self_val) = self_or_null {
                            args_vec.push(self_val);
                        }
                        args_vec.extend(pos_args);
                        let result =
                            callable.vectorcall(args_vec, effective_nargs as usize, None, vm)?;
                        self.push_value(result);
                        return Ok(None);
                    }
                }
                self.execute_call_vectorcall(nargs, vm)
            }
            Instruction::CallNonPyGeneral => {
                let nargs: u32 = arg.into();
                let stack_len = self.localsplus.stack_len();
                let self_or_null_is_some = self
                    .localsplus
                    .stack_index(stack_len - nargs as usize - 1)
                    .is_some();
                let callable = self.nth_value(nargs + 1);
                if callable.downcast_ref_if_exact::<PyFunction>(vm).is_some()
                    || callable
                        .downcast_ref_if_exact::<PyBoundMethod>(vm)
                        .is_some()
                {
                    return self.execute_call_vectorcall(nargs, vm);
                }
                let nargs_usize = nargs as usize;
                let pos_args: Vec<PyObjectRef> = self.pop_multiple(nargs_usize).collect();
                let self_or_null = self.pop_value_opt();
                let callable = self.pop_value();
                let mut args_vec =
                    Vec::with_capacity(nargs_usize + usize::from(self_or_null_is_some));
                if let Some(self_val) = self_or_null {
                    args_vec.push(self_val);
                }
                args_vec.extend(pos_args);
                let result = callable.vectorcall(
                    args_vec,
                    nargs_usize + usize::from(self_or_null_is_some),
                    None,
                    vm,
                )?;
                self.push_value(result);
                Ok(None)
            }
            Instruction::CallKwPy => {
                let instr_idx = self.lasti() as usize - 1;
                let cache_base = instr_idx + 1;
                let cached_version = self.code.instructions.read_cache_u32(cache_base + 1);
                let nargs: u32 = arg.into();
                if self.specialization_eval_frame_active(vm) {
                    return self.execute_call_kw_vectorcall(nargs, vm);
                }
                // Stack: [callable, self_or_null, arg1, ..., argN, kwarg_names]
                let callable = self.nth_value(nargs + 2);
                if let Some(func) = callable.downcast_ref_if_exact::<PyFunction>(vm)
                    && func.func_version() == cached_version
                    && cached_version != 0
                {
                    if self.specialization_call_recursion_guard(vm) {
                        return self.execute_call_kw_vectorcall(nargs, vm);
                    }
                    let nargs_usize = nargs as usize;
                    let kwarg_names_obj = self.pop_value();
                    let kwarg_names_tuple = kwarg_names_obj
                        .downcast_ref::<PyTuple>()
                        .expect("kwarg names should be tuple");
                    let kw_count = kwarg_names_tuple.len();
                    let all_args: Vec<PyObjectRef> = self.pop_multiple(nargs_usize).collect();
                    let self_or_null = self.pop_value_opt();
                    let callable = self.pop_value();
                    let pos_count = nargs_usize - kw_count;
                    let (args_vec, effective_nargs) = if let Some(self_val) = self_or_null {
                        let mut v = Vec::with_capacity(nargs_usize + 1);
                        v.push(self_val);
                        v.extend(all_args);
                        (v, pos_count + 1)
                    } else {
                        (all_args, pos_count)
                    };
                    let kwnames = kwarg_names_tuple.as_slice();
                    let result = vectorcall_function(
                        &callable,
                        args_vec,
                        effective_nargs,
                        Some(kwnames),
                        vm,
                    )?;
                    self.push_value(result);
                    return Ok(None);
                }
                self.execute_call_kw_vectorcall(nargs, vm)
            }
            Instruction::CallKwBoundMethod => {
                let instr_idx = self.lasti() as usize - 1;
                let cache_base = instr_idx + 1;
                let cached_version = self.code.instructions.read_cache_u32(cache_base + 1);
                let nargs: u32 = arg.into();
                if self.specialization_eval_frame_active(vm) {
                    return self.execute_call_kw_vectorcall(nargs, vm);
                }
                // Stack: [callable, self_or_null, arg1, ..., argN, kwarg_names]
                let stack_len = self.localsplus.stack_len();
                let self_or_null_is_some = self
                    .localsplus
                    .stack_index(stack_len - nargs as usize - 2)
                    .is_some();
                let callable = self.nth_value(nargs + 2);
                if !self_or_null_is_some
                    && let Some(bound_method) = callable.downcast_ref_if_exact::<PyBoundMethod>(vm)
                {
                    let bound_function = bound_method.function_obj().clone();
                    let bound_self = bound_method.self_obj().clone();
                    if let Some(func) = bound_function.downcast_ref_if_exact::<PyFunction>(vm)
                        && func.func_version() == cached_version
                        && cached_version != 0
                    {
                        let nargs_usize = nargs as usize;
                        let kwarg_names_obj = self.pop_value();
                        let kwarg_names_tuple = kwarg_names_obj
                            .downcast_ref::<PyTuple>()
                            .expect("kwarg names should be tuple");
                        let kw_count = kwarg_names_tuple.len();
                        let all_args: Vec<PyObjectRef> = self.pop_multiple(nargs_usize).collect();
                        self.pop_value_opt(); // null (self_or_null)
                        self.pop_value(); // callable (bound method)
                        let pos_count = nargs_usize - kw_count;
                        let mut args_vec = Vec::with_capacity(nargs_usize + 1);
                        args_vec.push(bound_self);
                        args_vec.extend(all_args);
                        let kwnames = kwarg_names_tuple.as_slice();
                        let result = vectorcall_function(
                            &bound_function,
                            args_vec,
                            pos_count + 1,
                            Some(kwnames),
                            vm,
                        )?;
                        self.push_value(result);
                        return Ok(None);
                    }
                }
                self.execute_call_kw_vectorcall(nargs, vm)
            }
            Instruction::CallKwNonPy => {
                let nargs: u32 = arg.into();
                let stack_len = self.localsplus.stack_len();
                let self_or_null_is_some = self
                    .localsplus
                    .stack_index(stack_len - nargs as usize - 2)
                    .is_some();
                let callable = self.nth_value(nargs + 2);
                if callable.downcast_ref_if_exact::<PyFunction>(vm).is_some()
                    || callable
                        .downcast_ref_if_exact::<PyBoundMethod>(vm)
                        .is_some()
                {
                    return self.execute_call_kw_vectorcall(nargs, vm);
                }
                let nargs_usize = nargs as usize;
                let kwarg_names_obj = self.pop_value();
                let kwarg_names_tuple = kwarg_names_obj
                    .downcast_ref::<PyTuple>()
                    .expect("kwarg names should be tuple");
                let kw_count = kwarg_names_tuple.len();
                let all_args: Vec<PyObjectRef> = self.pop_multiple(nargs_usize).collect();
                let self_or_null = self.pop_value_opt();
                let callable = self.pop_value();
                let pos_count = nargs_usize - kw_count;
                let mut args_vec =
                    Vec::with_capacity(nargs_usize + usize::from(self_or_null_is_some));
                if let Some(self_val) = self_or_null {
                    args_vec.push(self_val);
                }
                args_vec.extend(all_args);
                let result = callable.vectorcall(
                    args_vec,
                    pos_count + usize::from(self_or_null_is_some),
                    Some(kwarg_names_tuple.as_slice()),
                    vm,
                )?;
                self.push_value(result);
                Ok(None)
            }
            Instruction::LoadSuperAttrAttr => {
                let oparg = u32::from(arg);
                let attr_name = self.code.names[(oparg >> 2) as usize];
                // Stack: [global_super, class, self]
                let self_obj = self.top_value();
                let class_obj = self.nth_value(1);
                let global_super = self.nth_value(2);
                // Guard: global_super is builtin super and class is a type
                if global_super.is(&vm.ctx.types.super_type.as_object())
                    && class_obj.downcast_ref::<PyType>().is_some()
                {
                    let class = class_obj.downcast_ref::<PyType>().unwrap();
                    let start_type = self_obj.class();
                    // MRO lookup: skip classes up to and including `class`, then search
                    let mro: Vec<PyRef<PyType>> = start_type.mro_map_collect(|x| x.to_owned());
                    let mut found = None;
                    let mut past_class = false;
                    for cls in &mro {
                        if !past_class {
                            if cls.is(class) {
                                past_class = true;
                            }
                            continue;
                        }
                        if let Some(descr) = cls.get_direct_attr(attr_name) {
                            // Call descriptor __get__ if available
                            // Pass None for obj when self IS its own type (classmethod)
                            let obj_arg = if self_obj.is(start_type.as_object()) {
                                None
                            } else {
                                Some(self_obj.to_owned())
                            };
                            let result = vm
                                .call_get_descriptor_specific(
                                    &descr,
                                    obj_arg,
                                    Some(start_type.as_object().to_owned()),
                                )
                                .unwrap_or(Ok(descr))?;
                            found = Some(result);
                            break;
                        }
                    }
                    if let Some(attr) = found {
                        self.pop_value(); // self
                        self.pop_value(); // class
                        self.pop_value(); // super
                        self.push_value(attr);
                        return Ok(None);
                    }
                }
                let oparg = LoadSuperAttr::from_u32(oparg);
                self.load_super_attr(vm, oparg)
            }
            Instruction::LoadSuperAttrMethod => {
                let oparg = u32::from(arg);
                let attr_name = self.code.names[(oparg >> 2) as usize];
                // Stack: [global_super, class, self]
                let self_obj = self.top_value();
                let class_obj = self.nth_value(1);
                let global_super = self.nth_value(2);
                // Guard: global_super is builtin super and class is a type
                if global_super.is(&vm.ctx.types.super_type.as_object())
                    && class_obj.downcast_ref::<PyType>().is_some()
                {
                    let class = class_obj.downcast_ref::<PyType>().unwrap();
                    let self_val = self_obj.to_owned();
                    let start_type = self_obj.class();
                    // MRO lookup
                    let mro: Vec<PyRef<PyType>> = start_type.mro_map_collect(|x| x.to_owned());
                    let mut found = None;
                    let mut past_class = false;
                    for cls in &mro {
                        if !past_class {
                            if cls.is(class) {
                                past_class = true;
                            }
                            continue;
                        }
                        if let Some(descr) = cls.get_direct_attr(attr_name) {
                            let descr_cls = descr.class();
                            if descr_cls
                                .slots
                                .flags
                                .has_feature(PyTypeFlags::METHOD_DESCRIPTOR)
                            {
                                // Method descriptor: push unbound func + self
                                // CALL will prepend self as first positional arg
                                found = Some((descr, true));
                            } else if let Some(descr_get) = descr_cls.slots.descr_get.load() {
                                // Has __get__ but not METHOD_DESCRIPTOR: bind it
                                let bound = descr_get(
                                    descr,
                                    Some(self_val.clone()),
                                    Some(start_type.as_object().to_owned()),
                                    vm,
                                )?;
                                found = Some((bound, false));
                            } else {
                                // Plain attribute
                                found = Some((descr, false));
                            }
                            break;
                        }
                    }
                    if let Some((attr, is_method)) = found {
                        self.pop_value(); // self
                        self.pop_value(); // class
                        self.pop_value(); // super
                        self.push_value(attr);
                        if is_method {
                            self.push_value(self_val);
                        } else {
                            self.push_null();
                        }
                        return Ok(None);
                    }
                }
                let oparg = LoadSuperAttr::from_u32(oparg);
                self.load_super_attr(vm, oparg)
            }
            Instruction::CompareOpInt => {
                let b = self.top_value();
                let a = self.nth_value(1);
                if let (Some(a_int), Some(b_int)) = (
                    a.downcast_ref_if_exact::<PyInt>(vm),
                    b.downcast_ref_if_exact::<PyInt>(vm),
                ) && let (Some(a_val), Some(b_val)) = (
                    specialization_compact_int_value(a_int, vm),
                    specialization_compact_int_value(b_int, vm),
                ) {
                    let op = self.compare_op_from_arg(arg);
                    let result = op.eval_ord(a_val.cmp(&b_val));
                    self.pop_value();
                    self.pop_value();
                    self.push_value(vm.ctx.new_bool(result).into());
                    Ok(None)
                } else {
                    self.execute_compare(vm, arg)
                }
            }
            Instruction::CompareOpFloat => {
                let b = self.top_value();
                let a = self.nth_value(1);
                if let (Some(a_f), Some(b_f)) = (
                    a.downcast_ref_if_exact::<PyFloat>(vm),
                    b.downcast_ref_if_exact::<PyFloat>(vm),
                ) {
                    let op = self.compare_op_from_arg(arg);
                    let (a, b) = (a_f.to_f64(), b_f.to_f64());
                    // Use Rust's IEEE 754 float comparison which handles NaN correctly
                    let result = match a.partial_cmp(&b) {
                        Some(ord) => op.eval_ord(ord),
                        None => op == PyComparisonOp::Ne, // NaN != anything is true
                    };
                    self.pop_value();
                    self.pop_value();
                    self.push_value(vm.ctx.new_bool(result).into());
                    Ok(None)
                } else {
                    self.execute_compare(vm, arg)
                }
            }
            Instruction::CompareOpStr => {
                let b = self.top_value();
                let a = self.nth_value(1);
                if let (Some(a_str), Some(b_str)) = (
                    a.downcast_ref_if_exact::<PyStr>(vm),
                    b.downcast_ref_if_exact::<PyStr>(vm),
                ) {
                    let op = self.compare_op_from_arg(arg);
                    if op != PyComparisonOp::Eq && op != PyComparisonOp::Ne {
                        return self.execute_compare(vm, arg);
                    }
                    let result = op.eval_ord(a_str.as_wtf8().cmp(b_str.as_wtf8()));
                    self.pop_value();
                    self.pop_value();
                    self.push_value(vm.ctx.new_bool(result).into());
                    Ok(None)
                } else {
                    self.execute_compare(vm, arg)
                }
            }
            Instruction::ToBoolBool => {
                let obj = self.top_value();
                if obj.class().is(vm.ctx.types.bool_type) {
                    // Already a bool, no-op
                    Ok(None)
                } else {
                    let obj = self.pop_value();
                    let result = obj.try_to_bool(vm)?;
                    self.push_value(vm.ctx.new_bool(result).into());
                    Ok(None)
                }
            }
            Instruction::ToBoolInt => {
                let obj = self.top_value();
                if let Some(int_val) = obj.downcast_ref_if_exact::<PyInt>(vm) {
                    let result = !int_val.as_bigint().is_zero();
                    self.pop_value();
                    self.push_value(vm.ctx.new_bool(result).into());
                    Ok(None)
                } else {
                    let obj = self.pop_value();
                    let result = obj.try_to_bool(vm)?;
                    self.push_value(vm.ctx.new_bool(result).into());
                    Ok(None)
                }
            }
            Instruction::ToBoolNone => {
                let obj = self.top_value();
                if obj.class().is(vm.ctx.types.none_type) {
                    self.pop_value();
                    self.push_value(vm.ctx.new_bool(false).into());
                    Ok(None)
                } else {
                    let obj = self.pop_value();
                    let result = obj.try_to_bool(vm)?;
                    self.push_value(vm.ctx.new_bool(result).into());
                    Ok(None)
                }
            }
            Instruction::ToBoolList => {
                let obj = self.top_value();
                if let Some(list) = obj.downcast_ref_if_exact::<PyList>(vm) {
                    let result = !list.borrow_vec().is_empty();
                    self.pop_value();
                    self.push_value(vm.ctx.new_bool(result).into());
                    Ok(None)
                } else {
                    let obj = self.pop_value();
                    let result = obj.try_to_bool(vm)?;
                    self.push_value(vm.ctx.new_bool(result).into());
                    Ok(None)
                }
            }
            Instruction::ToBoolStr => {
                let obj = self.top_value();
                if let Some(s) = obj.downcast_ref_if_exact::<PyStr>(vm) {
                    let result = !s.is_empty();
                    self.pop_value();
                    self.push_value(vm.ctx.new_bool(result).into());
                    Ok(None)
                } else {
                    let obj = self.pop_value();
                    let result = obj.try_to_bool(vm)?;
                    self.push_value(vm.ctx.new_bool(result).into());
                    Ok(None)
                }
            }
            Instruction::ToBoolAlwaysTrue => {
                // Objects without __bool__ or __len__ are always True.
                // Guard: check type version hasn't changed.
                let instr_idx = self.lasti() as usize - 1;
                let cache_base = instr_idx + 1;
                let obj = self.top_value();
                let cached_version = self.code.instructions.read_cache_u32(cache_base + 1);
                if cached_version != 0 && obj.class().tp_version_tag.load(Acquire) == cached_version
                {
                    self.pop_value();
                    self.push_value(vm.ctx.new_bool(true).into());
                    Ok(None)
                } else {
                    let obj = self.pop_value();
                    let result = obj.try_to_bool(vm)?;
                    self.push_value(vm.ctx.new_bool(result).into());
                    Ok(None)
                }
            }
            Instruction::ContainsOpDict => {
                let b = self.top_value(); // haystack
                if let Some(dict) = b.downcast_ref_if_exact::<PyDict>(vm) {
                    let a = self.nth_value(1); // needle
                    let found = dict.get_item_opt(a, vm)?.is_some();
                    self.pop_value();
                    self.pop_value();
                    let invert = bytecode::Invert::try_from(u32::from(arg) as u8)
                        .unwrap_or(bytecode::Invert::No);
                    let value = match invert {
                        bytecode::Invert::No => found,
                        bytecode::Invert::Yes => !found,
                    };
                    self.push_value(vm.ctx.new_bool(value).into());
                    Ok(None)
                } else {
                    let b = self.pop_value();
                    let a = self.pop_value();
                    let invert = bytecode::Invert::try_from(u32::from(arg) as u8)
                        .unwrap_or(bytecode::Invert::No);
                    let value = match invert {
                        bytecode::Invert::No => self._in(vm, &a, &b)?,
                        bytecode::Invert::Yes => self._not_in(vm, &a, &b)?,
                    };
                    self.push_value(vm.ctx.new_bool(value).into());
                    Ok(None)
                }
            }
            Instruction::ContainsOpSet => {
                let b = self.top_value(); // haystack
                if b.downcast_ref_if_exact::<PySet>(vm).is_some()
                    || b.downcast_ref_if_exact::<PyFrozenSet>(vm).is_some()
                {
                    let a = self.nth_value(1); // needle
                    let found = vm._contains(b, a)?;
                    self.pop_value();
                    self.pop_value();
                    let invert = bytecode::Invert::try_from(u32::from(arg) as u8)
                        .unwrap_or(bytecode::Invert::No);
                    let value = match invert {
                        bytecode::Invert::No => found,
                        bytecode::Invert::Yes => !found,
                    };
                    self.push_value(vm.ctx.new_bool(value).into());
                    Ok(None)
                } else {
                    let b = self.pop_value();
                    let a = self.pop_value();
                    let invert = bytecode::Invert::try_from(u32::from(arg) as u8)
                        .unwrap_or(bytecode::Invert::No);
                    let value = match invert {
                        bytecode::Invert::No => self._in(vm, &a, &b)?,
                        bytecode::Invert::Yes => self._not_in(vm, &a, &b)?,
                    };
                    self.push_value(vm.ctx.new_bool(value).into());
                    Ok(None)
                }
            }
            Instruction::UnpackSequenceTwoTuple => {
                let obj = self.top_value();
                if let Some(tuple) = obj.downcast_ref_if_exact::<PyTuple>(vm) {
                    let elements = tuple.as_slice();
                    if elements.len() == 2 {
                        let e0 = elements[0].clone();
                        let e1 = elements[1].clone();
                        self.pop_value();
                        self.push_value(e1);
                        self.push_value(e0);
                        return Ok(None);
                    }
                }
                let size = u32::from(arg);
                self.unpack_sequence(size, vm)
            }
            Instruction::UnpackSequenceTuple => {
                let size = u32::from(arg) as usize;
                let obj = self.top_value();
                if let Some(tuple) = obj.downcast_ref_if_exact::<PyTuple>(vm) {
                    let elements = tuple.as_slice();
                    if elements.len() == size {
                        let elems: Vec<_> = elements.to_vec();
                        self.pop_value();
                        for elem in elems.into_iter().rev() {
                            self.push_value(elem);
                        }
                        return Ok(None);
                    }
                }
                self.unpack_sequence(size as u32, vm)
            }
            Instruction::UnpackSequenceList => {
                let size = u32::from(arg) as usize;
                let obj = self.top_value();
                if let Some(list) = obj.downcast_ref_if_exact::<PyList>(vm) {
                    let vec = list.borrow_vec();
                    if vec.len() == size {
                        let elems: Vec<_> = vec.to_vec();
                        drop(vec);
                        self.pop_value();
                        for elem in elems.into_iter().rev() {
                            self.push_value(elem);
                        }
                        return Ok(None);
                    }
                }
                self.unpack_sequence(size as u32, vm)
            }
            Instruction::ForIterRange => {
                let target = bytecode::Label::from_u32(self.lasti() + 1 + u32::from(arg));
                let iter = self.top_value();
                if let Some(range_iter) = iter.downcast_ref_if_exact::<PyRangeIterator>(vm) {
                    if let Some(value) = range_iter.fast_next() {
                        self.push_value(vm.ctx.new_int(value).into());
                    } else {
                        self.for_iter_jump_on_exhausted(target);
                    }
                    Ok(None)
                } else {
                    self.execute_for_iter(vm, target)?;
                    Ok(None)
                }
            }
            Instruction::ForIterList => {
                let target = bytecode::Label::from_u32(self.lasti() + 1 + u32::from(arg));
                let iter = self.top_value();
                if let Some(list_iter) = iter.downcast_ref_if_exact::<PyListIterator>(vm) {
                    if let Some(value) = list_iter.fast_next() {
                        self.push_value(value);
                    } else {
                        self.for_iter_jump_on_exhausted(target);
                    }
                    Ok(None)
                } else {
                    self.execute_for_iter(vm, target)?;
                    Ok(None)
                }
            }
            Instruction::ForIterTuple => {
                let target = bytecode::Label::from_u32(self.lasti() + 1 + u32::from(arg));
                let iter = self.top_value();
                if let Some(tuple_iter) = iter.downcast_ref_if_exact::<PyTupleIterator>(vm) {
                    if let Some(value) = tuple_iter.fast_next() {
                        self.push_value(value);
                    } else {
                        self.for_iter_jump_on_exhausted(target);
                    }
                    Ok(None)
                } else {
                    self.execute_for_iter(vm, target)?;
                    Ok(None)
                }
            }
            Instruction::ForIterGen => {
                let target = bytecode::Label::from_u32(self.lasti() + 1 + u32::from(arg));
                let iter = self.top_value();
                if self.specialization_eval_frame_active(vm) {
                    self.execute_for_iter(vm, target)?;
                    return Ok(None);
                }
                if let Some(generator) = iter.downcast_ref_if_exact::<PyGenerator>(vm) {
                    if generator.as_coro().running() || generator.as_coro().closed() {
                        self.execute_for_iter(vm, target)?;
                        return Ok(None);
                    }
                    match generator.as_coro().send_none(iter, vm) {
                        Ok(PyIterReturn::Return(value)) => {
                            self.push_value(value);
                        }
                        Ok(PyIterReturn::StopIteration(value)) => {
                            if vm.use_tracing.get() && !vm.is_none(&self.object.trace.lock()) {
                                let stop_exc = vm.new_stop_iteration(value);
                                self.fire_exception_trace(&stop_exc, vm)?;
                            }
                            self.for_iter_jump_on_exhausted(target);
                        }
                        Err(e) => return Err(e),
                    }
                    Ok(None)
                } else {
                    self.execute_for_iter(vm, target)?;
                    Ok(None)
                }
            }
            Instruction::LoadGlobalModule => {
                let oparg = u32::from(arg);
                let cache_base = self.lasti() as usize;
                // Keep specialized opcode on guard miss (JUMP_TO_PREDICTED behavior).
                let cached_version = self.code.instructions.read_cache_u16(cache_base + 1);
                let cached_index = self.code.instructions.read_cache_u16(cache_base + 3);
                if let Ok(current_version) = u16::try_from(self.globals.version())
                    && cached_version == current_version
                {
                    let name = self.code.names[(oparg >> 1) as usize];
                    if let Some(x) = self.globals.get_item_opt_hint(name, cached_index, vm)? {
                        self.push_value(x);
                        if (oparg & 1) != 0 {
                            self.push_value_opt(None);
                        }
                        return Ok(None);
                    }
                }
                let name = self.code.names[(oparg >> 1) as usize];
                let x = self.load_global_or_builtin(name, vm)?;
                self.push_value(x);
                if (oparg & 1) != 0 {
                    self.push_value_opt(None);
                }
                Ok(None)
            }
            Instruction::LoadGlobalBuiltin => {
                let oparg = u32::from(arg);
                let cache_base = self.lasti() as usize;
                let cached_globals_ver = self.code.instructions.read_cache_u16(cache_base + 1);
                let cached_builtins_ver = self.code.instructions.read_cache_u16(cache_base + 2);
                let cached_index = self.code.instructions.read_cache_u16(cache_base + 3);
                if let Ok(current_globals_ver) = u16::try_from(self.globals.version())
                    && cached_globals_ver == current_globals_ver
                    && let Some(builtins_dict) = self.builtins.downcast_ref_if_exact::<PyDict>(vm)
                    && let Ok(current_builtins_ver) = u16::try_from(builtins_dict.version())
                    && cached_builtins_ver == current_builtins_ver
                {
                    let name = self.code.names[(oparg >> 1) as usize];
                    if let Some(x) = builtins_dict.get_item_opt_hint(name, cached_index, vm)? {
                        self.push_value(x);
                        if (oparg & 1) != 0 {
                            self.push_value_opt(None);
                        }
                        return Ok(None);
                    }
                }
                let name = self.code.names[(oparg >> 1) as usize];
                let x = self.load_global_or_builtin(name, vm)?;
                self.push_value(x);
                if (oparg & 1) != 0 {
                    self.push_value_opt(None);
                }
                Ok(None)
            }
            // All INSTRUMENTED_* opcodes delegate to a cold function to keep
            // the hot instruction loop free of monitoring overhead.
            _ => self.execute_instrumented(instruction, arg, vm),
        }
    }

    /// Handle all INSTRUMENTED_* opcodes. This function is cold — it only
    /// runs when sys.monitoring has rewritten the bytecode.
    #[cold]
    fn execute_instrumented(
        &mut self,
        instruction: Instruction,
        arg: bytecode::OpArg,
        vm: &VirtualMachine,
    ) -> FrameResult {
        debug_assert!(
            instruction.is_instrumented(),
            "execute_instrumented called with non-instrumented opcode {instruction:?}"
        );
        if self.monitoring_disabled_for_code(vm) {
            let global_ver = vm
                .state
                .instrumentation_version
                .load(atomic::Ordering::Acquire);
            monitoring::instrument_code(self.code, 0);
            self.code
                .instrumentation_version
                .store(global_ver, atomic::Ordering::Release);
            self.update_lasti(|i| *i -= 1);
            return Ok(None);
        }
        self.monitoring_mask = vm.state.monitoring_events.load();
        match instruction {
            Instruction::InstrumentedResume => {
                // Version check: re-instrument if stale
                let global_ver = vm
                    .state
                    .instrumentation_version
                    .load(atomic::Ordering::Acquire);
                let code_ver = self
                    .code
                    .instrumentation_version
                    .load(atomic::Ordering::Acquire);
                if code_ver != global_ver {
                    let events = {
                        let state = vm.state.monitoring.lock();
                        state.events_for_code(self.code.get_id())
                    };
                    monitoring::instrument_code(self.code, events);
                    self.code
                        .instrumentation_version
                        .store(global_ver, atomic::Ordering::Release);
                    // Re-execute (may have been de-instrumented to base Resume)
                    self.update_lasti(|i| *i -= 1);
                    return Ok(None);
                }
                let resume_type = u32::from(arg);
                let offset = (self.lasti() - 1) * 2;
                if resume_type == 0 {
                    if self.monitoring_mask & monitoring::EVENT_PY_START != 0 {
                        monitoring::fire_py_start(vm, self.code, offset)?;
                    }
                } else if self.monitoring_mask & monitoring::EVENT_PY_RESUME != 0 {
                    monitoring::fire_py_resume(vm, self.code, offset)?;
                }
                Ok(None)
            }
            Instruction::InstrumentedReturnValue => {
                let value = self.pop_value();
                if self.monitoring_mask & monitoring::EVENT_PY_RETURN != 0 {
                    let offset = (self.lasti() - 1) * 2;
                    monitoring::fire_py_return(vm, self.code, offset, &value)?;
                }
                self.unwind_blocks(vm, UnwindReason::Returning { value })
            }
            Instruction::InstrumentedYieldValue => {
                debug_assert!(
                    self.localsplus
                        .stack_as_slice()
                        .iter()
                        .flatten()
                        .all(|sr| !sr.is_borrowed()),
                    "borrowed refs on stack at yield point"
                );
                let value = self.pop_value();
                if self.monitoring_mask & monitoring::EVENT_PY_YIELD != 0 {
                    let offset = (self.lasti() - 1) * 2;
                    monitoring::fire_py_yield(vm, self.code, offset, &value)?;
                }
                Ok(Some(ExecutionResult::Yield(value)))
            }
            Instruction::InstrumentedCall => {
                let args = self.collect_positional_args(u32::from(arg));
                self.execute_call_instrumented(args, vm)
            }
            Instruction::InstrumentedCallKw => {
                let args = self.collect_keyword_args(u32::from(arg));
                self.execute_call_instrumented(args, vm)
            }
            Instruction::InstrumentedCallFunctionEx => {
                let args = self.collect_ex_args(vm)?;
                self.execute_call_instrumented(args, vm)
            }
            Instruction::InstrumentedLoadSuperAttr => {
                let oparg = bytecode::LoadSuperAttr::from(u32::from(arg));
                let offset = (self.lasti() - 1) * 2;
                // Fire CALL event before super() call
                let call_args = if self.monitoring_mask & monitoring::EVENT_CALL != 0 {
                    let global_super: PyObjectRef = self.nth_value(2).to_owned();
                    let arg0 = if oparg.has_class() {
                        self.nth_value(1).to_owned()
                    } else {
                        monitoring::get_missing(vm)
                    };
                    monitoring::fire_call(vm, self.code, offset, &global_super, arg0.clone())?;
                    Some((global_super, arg0))
                } else {
                    None
                };
                match self.load_super_attr(vm, oparg) {
                    Ok(result) => {
                        // Fire C_RETURN on success
                        if let Some((global_super, arg0)) = call_args {
                            monitoring::fire_c_return(vm, self.code, offset, &global_super, arg0)?;
                        }
                        Ok(result)
                    }
                    Err(exc) => {
                        // Fire C_RAISE on failure
                        let exc = if let Some((global_super, arg0)) = call_args {
                            match monitoring::fire_c_raise(
                                vm,
                                self.code,
                                offset,
                                &global_super,
                                arg0,
                            ) {
                                Ok(()) => exc,
                                Err(monitor_exc) => monitor_exc,
                            }
                        } else {
                            exc
                        };
                        Err(exc)
                    }
                }
            }
            Instruction::InstrumentedJumpForward => {
                let src_offset = (self.lasti() - 1) * 2;
                let target_idx = self.lasti() + u32::from(arg);
                let target = bytecode::Label::from_u32(target_idx);
                self.jump(target);
                if self.monitoring_mask & monitoring::EVENT_JUMP != 0 {
                    monitoring::fire_jump(vm, self.code, src_offset, target.as_u32() * 2)?;
                }
                Ok(None)
            }
            Instruction::InstrumentedJumpBackward => {
                let src_offset = (self.lasti() - 1) * 2;
                let target_idx = self.lasti() + 1 - u32::from(arg);
                let target = bytecode::Label::from_u32(target_idx);
                self.jump(target);
                if self.monitoring_mask & monitoring::EVENT_JUMP != 0 {
                    monitoring::fire_jump(vm, self.code, src_offset, target.as_u32() * 2)?;
                }
                Ok(None)
            }
            Instruction::InstrumentedForIter => {
                let src_offset = (self.lasti() - 1) * 2;
                let target = bytecode::Label::from_u32(self.lasti() + 1 + u32::from(arg));
                let continued = self.execute_for_iter(vm, target)?;
                if continued {
                    if self.monitoring_mask & monitoring::EVENT_BRANCH_LEFT != 0 {
                        let dest_offset = (self.lasti() + 1) * 2; // after caches
                        monitoring::fire_branch_left(vm, self.code, src_offset, dest_offset)?;
                    }
                } else if self.monitoring_mask & monitoring::EVENT_BRANCH_RIGHT != 0 {
                    let dest_offset = self.lasti() * 2;
                    monitoring::fire_branch_right(vm, self.code, src_offset, dest_offset)?;
                }
                Ok(None)
            }
            Instruction::InstrumentedEndFor => {
                // Stack: [value, receiver(iter), ...]
                // PyGen_Check: only fire STOP_ITERATION for generators
                let is_gen = self
                    .nth_value(1)
                    .downcast_ref::<crate::builtins::PyGenerator>()
                    .is_some();
                let value = self.pop_value();
                if is_gen && self.monitoring_mask & monitoring::EVENT_STOP_ITERATION != 0 {
                    let offset = (self.lasti() - 1) * 2;
                    monitoring::fire_stop_iteration(vm, self.code, offset, &value)?;
                }
                Ok(None)
            }
            Instruction::InstrumentedEndSend => {
                let value = self.pop_value();
                let receiver = self.pop_value();
                // PyGen_Check || PyCoro_CheckExact
                let is_gen_or_coro = receiver
                    .downcast_ref::<crate::builtins::PyGenerator>()
                    .is_some()
                    || receiver
                        .downcast_ref::<crate::builtins::PyCoroutine>()
                        .is_some();
                if is_gen_or_coro && self.monitoring_mask & monitoring::EVENT_STOP_ITERATION != 0 {
                    let offset = (self.lasti() - 1) * 2;
                    monitoring::fire_stop_iteration(vm, self.code, offset, &value)?;
                }
                self.push_value(value);
                Ok(None)
            }
            Instruction::InstrumentedPopJumpIfTrue => {
                let src_offset = (self.lasti() - 1) * 2;
                let target_idx = self.lasti() + 1 + u32::from(arg);
                let obj = self.pop_value();
                let value = obj.try_to_bool(vm)?;
                if value {
                    self.jump(bytecode::Label::from_u32(target_idx));
                    if self.monitoring_mask & monitoring::EVENT_BRANCH_RIGHT != 0 {
                        monitoring::fire_branch_right(vm, self.code, src_offset, target_idx * 2)?;
                    }
                }
                Ok(None)
            }
            Instruction::InstrumentedPopJumpIfFalse => {
                let src_offset = (self.lasti() - 1) * 2;
                let target_idx = self.lasti() + 1 + u32::from(arg);
                let obj = self.pop_value();
                let value = obj.try_to_bool(vm)?;
                if !value {
                    self.jump(bytecode::Label::from_u32(target_idx));
                    if self.monitoring_mask & monitoring::EVENT_BRANCH_RIGHT != 0 {
                        monitoring::fire_branch_right(vm, self.code, src_offset, target_idx * 2)?;
                    }
                }
                Ok(None)
            }
            Instruction::InstrumentedPopJumpIfNone => {
                let src_offset = (self.lasti() - 1) * 2;
                let target_idx = self.lasti() + 1 + u32::from(arg);
                let value = self.pop_value();
                if vm.is_none(&value) {
                    self.jump(bytecode::Label::from_u32(target_idx));
                    if self.monitoring_mask & monitoring::EVENT_BRANCH_RIGHT != 0 {
                        monitoring::fire_branch_right(vm, self.code, src_offset, target_idx * 2)?;
                    }
                }
                Ok(None)
            }
            Instruction::InstrumentedPopJumpIfNotNone => {
                let src_offset = (self.lasti() - 1) * 2;
                let target_idx = self.lasti() + 1 + u32::from(arg);
                let value = self.pop_value();
                if !vm.is_none(&value) {
                    self.jump(bytecode::Label::from_u32(target_idx));
                    if self.monitoring_mask & monitoring::EVENT_BRANCH_RIGHT != 0 {
                        monitoring::fire_branch_right(vm, self.code, src_offset, target_idx * 2)?;
                    }
                }
                Ok(None)
            }
            Instruction::InstrumentedNotTaken => {
                if self.monitoring_mask & monitoring::EVENT_BRANCH_LEFT != 0 {
                    let not_taken_idx = self.lasti() as usize - 1;
                    // Scan backwards past CACHE entries to find the branch instruction
                    let mut branch_idx = not_taken_idx.saturating_sub(1);
                    while branch_idx > 0
                        && matches!(
                            self.code.instructions.read_op(branch_idx),
                            Instruction::Cache
                        )
                    {
                        branch_idx -= 1;
                    }
                    let src_offset = (branch_idx as u32) * 2;
                    let dest_offset = self.lasti() * 2;
                    monitoring::fire_branch_left(vm, self.code, src_offset, dest_offset)?;
                }
                Ok(None)
            }
            Instruction::InstrumentedPopIter => {
                // BRANCH_RIGHT is fired by InstrumentedForIter, not here.
                self.pop_value();
                Ok(None)
            }
            Instruction::InstrumentedEndAsyncFor => {
                if self.monitoring_mask & monitoring::EVENT_BRANCH_RIGHT != 0 {
                    let oparg_val = u32::from(arg);
                    // src = next_instr - oparg (END_SEND position)
                    let src_offset = (self.lasti() - oparg_val) * 2;
                    // dest = this_instr + 1
                    let dest_offset = self.lasti() * 2;
                    monitoring::fire_branch_right(vm, self.code, src_offset, dest_offset)?;
                }
                let exc = self.pop_value();
                let _awaitable = self.pop_value();
                let exc = exc
                    .downcast::<PyBaseException>()
                    .expect("EndAsyncFor expects exception on stack");
                if exc.fast_isinstance(vm.ctx.exceptions.stop_async_iteration) {
                    vm.set_exception(None);
                    Ok(None)
                } else {
                    Err(exc)
                }
            }
            Instruction::InstrumentedLine => {
                let idx = self.lasti() as usize - 1;
                let offset = idx as u32 * 2;

                // Read the full side-table chain before firing any events,
                // because a callback may de-instrument and clear the tables.
                let (real_op_byte, also_instruction) = {
                    let data = self.code.monitoring_data.lock();
                    let line_op = data.as_ref().map(|d| d.line_opcodes[idx]).unwrap_or(0);
                    if line_op == u8::from(Instruction::InstrumentedInstruction) {
                        // LINE wraps INSTRUCTION: resolve the INSTRUCTION side-table too
                        let inst_op = data
                            .as_ref()
                            .map(|d| d.per_instruction_opcodes[idx])
                            .unwrap_or(0);
                        (inst_op, true)
                    } else {
                        (line_op, false)
                    }
                };
                debug_assert!(
                    real_op_byte != 0,
                    "INSTRUMENTED_LINE at {idx} without stored opcode"
                );

                // Fire LINE event only if line changed
                if let Some((loc, _)) = self.code.locations.get(idx) {
                    let line = loc.line.get() as u32;
                    if line != *self.prev_line && line > 0 {
                        *self.prev_line = line;
                        monitoring::fire_line(vm, self.code, offset, line)?;
                    }
                }

                // If the LINE position also had INSTRUCTION, fire that event too
                if also_instruction {
                    monitoring::fire_instruction(vm, self.code, offset)?;
                }

                // Re-dispatch to the real original opcode
                let original_op = Instruction::try_from(real_op_byte)
                    .expect("invalid opcode in side-table chain");
                let lasti_before_dispatch = self.lasti();
                let result = if original_op.to_base().is_some() {
                    self.execute_instrumented(original_op, arg, vm)
                } else {
                    let mut do_extend_arg = false;
                    self.execute_instruction(original_op, arg, &mut do_extend_arg, vm)
                };
                let orig_caches = original_op.to_base().unwrap_or(original_op).cache_entries();
                if orig_caches > 0 && self.lasti() == lasti_before_dispatch {
                    self.update_lasti(|i| *i += orig_caches as u32);
                }
                result
            }
            Instruction::InstrumentedInstruction => {
                let idx = self.lasti() as usize - 1;
                let offset = idx as u32 * 2;

                // Get original opcode from side-table
                let original_op_byte = {
                    let data = self.code.monitoring_data.lock();
                    data.as_ref()
                        .map(|d| d.per_instruction_opcodes[idx])
                        .unwrap_or(0)
                };
                debug_assert!(
                    original_op_byte != 0,
                    "INSTRUMENTED_INSTRUCTION at {idx} without stored opcode"
                );

                // Fire INSTRUCTION event
                monitoring::fire_instruction(vm, self.code, offset)?;

                // Re-dispatch to original opcode
                let original_op = Instruction::try_from(original_op_byte)
                    .expect("invalid opcode in instruction side-table");
                let lasti_before_dispatch = self.lasti();
                let result = if original_op.to_base().is_some() {
                    self.execute_instrumented(original_op, arg, vm)
                } else {
                    let mut do_extend_arg = false;
                    self.execute_instruction(original_op, arg, &mut do_extend_arg, vm)
                };
                let orig_caches = original_op.to_base().unwrap_or(original_op).cache_entries();
                if orig_caches > 0 && self.lasti() == lasti_before_dispatch {
                    self.update_lasti(|i| *i += orig_caches as u32);
                }
                result
            }
            _ => {
                unreachable!("{instruction:?} instruction should not be executed")
            }
        }
    }

    #[inline]
    fn mapping_get_optional(
        &self,
        mapping: &PyObjectRef,
        name: &Py<PyStr>,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        if mapping.class().is(vm.ctx.types.dict_type) {
            let dict = mapping
                .downcast_ref::<PyDict>()
                .expect("exact dict must have a PyDict payload");
            dict.get_item_opt(name, vm)
        } else {
            match mapping.get_item(name, vm) {
                Ok(value) => Ok(Some(value)),
                Err(err) if err.fast_isinstance(vm.ctx.exceptions.key_error) => Ok(None),
                Err(err) => Err(err),
            }
        }
    }

    #[inline]
    fn load_global_or_builtin(&self, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
        if let Some(builtins_dict) = self.builtins_dict {
            // Fast path: both globals and builtins are exact dicts
            // SAFETY: builtins_dict is only set when globals is also exact dict
            let globals_exact = unsafe { PyExact::ref_unchecked(self.globals.as_ref()) };
            globals_exact
                .get_chain_exact(builtins_dict, name, vm)?
                .ok_or_else(|| {
                    vm.new_name_error(format!("name '{name}' is not defined"), name.to_owned())
                })
        } else {
            // Slow path: builtins is not a dict, use generic __getitem__
            if let Some(value) = self.globals.get_item_opt(name, vm)? {
                return Ok(value);
            }
            self.builtins.get_item(name, vm).map_err(|e| {
                if e.fast_isinstance(vm.ctx.exceptions.key_error) {
                    vm.new_name_error(format!("name '{name}' is not defined"), name.to_owned())
                } else {
                    e
                }
            })
        }
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn import(&mut self, vm: &VirtualMachine, module_name: Option<&Py<PyStr>>) -> PyResult<()> {
        let module_name = module_name.unwrap_or(vm.ctx.empty_str);
        let top = self.pop_value();
        let from_list = match <Option<PyTupleRef>>::try_from_object(vm, top)? {
            Some(from_list) => from_list.try_into_typed::<PyStr>(vm)?,
            None => vm.ctx.empty_tuple_typed().to_owned(),
        };
        let level = usize::try_from_object(vm, self.pop_value())?;

        let module = vm.import_from(module_name, &from_list, level)?;

        self.push_value(module);
        Ok(())
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn import_from(&mut self, vm: &VirtualMachine, idx: bytecode::NameIdx) -> PyResult {
        let module = self.top_value();
        let name = self.code.names[idx as usize];

        // Load attribute, and transform any error into import error.
        if let Some(obj) = vm.get_attribute_opt(module.to_owned(), name)? {
            return Ok(obj);
        }
        // fallback to importing '{module.__name__}.{name}' from sys.modules
        let fallback_module = (|| {
            let mod_name = module.get_attr(identifier!(vm, __name__), vm).ok()?;
            let mod_name = mod_name.downcast_ref::<PyStr>()?;
            let full_mod_name = format!("{mod_name}.{name}");
            let sys_modules = vm.sys_module.get_attr("modules", vm).ok()?;
            sys_modules.get_item(&full_mod_name, vm).ok()
        })();

        if let Some(sub_module) = fallback_module {
            return Ok(sub_module);
        }

        use crate::import::{
            get_spec_file_origin, is_possibly_shadowing_path, is_stdlib_module_name,
        };

        // Get module name for the error message
        let mod_name_obj = module.get_attr(identifier!(vm, __name__), vm).ok();
        let mod_name_str = mod_name_obj
            .as_ref()
            .and_then(|n| n.downcast_ref::<PyUtf8Str>().map(|s| s.as_str().to_owned()));
        let module_name = mod_name_str.as_deref().unwrap_or("<unknown module name>");

        let spec = module
            .get_attr("__spec__", vm)
            .ok()
            .filter(|s| !vm.is_none(s));

        let origin = get_spec_file_origin(&spec, vm);

        let is_possibly_shadowing = origin
            .as_ref()
            .map(|o| is_possibly_shadowing_path(o, vm))
            .unwrap_or(false);
        let is_possibly_shadowing_stdlib = if is_possibly_shadowing {
            if let Some(ref mod_name) = mod_name_obj {
                is_stdlib_module_name(mod_name, vm)?
            } else {
                false
            }
        } else {
            false
        };

        let msg = if is_possibly_shadowing_stdlib {
            let origin = origin.as_ref().unwrap();
            format!(
                "cannot import name '{name}' from '{module_name}' \
                 (consider renaming '{origin}' since it has the same \
                 name as the standard library module named '{module_name}' \
                 and prevents importing that standard library module)"
            )
        } else {
            let is_init = is_module_initializing(module, vm);
            if is_init {
                if is_possibly_shadowing {
                    let origin = origin.as_ref().unwrap();
                    format!(
                        "cannot import name '{name}' from '{module_name}' \
                         (consider renaming '{origin}' if it has the same name \
                         as a library you intended to import)"
                    )
                } else if let Some(ref path) = origin {
                    format!(
                        "cannot import name '{name}' from partially initialized module \
                         '{module_name}' (most likely due to a circular import) ({path})"
                    )
                } else {
                    format!(
                        "cannot import name '{name}' from partially initialized module \
                         '{module_name}' (most likely due to a circular import)"
                    )
                }
            } else if let Some(ref path) = origin {
                format!("cannot import name '{name}' from '{module_name}' ({path})")
            } else {
                format!("cannot import name '{name}' from '{module_name}' (unknown location)")
            }
        };
        let err = vm.new_import_error(msg, vm.ctx.new_utf8_str(module_name));

        if let Some(ref path) = origin {
            let _ignore = err
                .as_object()
                .set_attr("path", vm.ctx.new_str(path.as_str()), vm);
        }

        // name_from = the attribute name that failed to import (best-effort metadata)
        let _ignore = err.as_object().set_attr("name_from", name.to_owned(), vm);

        Err(err)
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn import_star(&mut self, vm: &VirtualMachine) -> PyResult<()> {
        let module = self.pop_value();

        let Some(dict) = module.dict() else {
            return Ok(());
        };

        let mod_name = module
            .get_attr(identifier!(vm, __name__), vm)
            .ok()
            .and_then(|n| n.downcast::<PyStr>().ok());

        let require_str = |obj: PyObjectRef, attr: &str| -> PyResult<PyRef<PyStr>> {
            obj.downcast().map_err(|obj: PyObjectRef| {
                let source = if let Some(ref mod_name) = mod_name {
                    format!("{}.{attr}", mod_name.as_wtf8())
                } else {
                    attr.to_owned()
                };
                let repr = obj.repr(vm).unwrap_or_else(|_| vm.ctx.new_str("?"));
                vm.new_type_error(format!(
                    "{} in {} must be str, not {}",
                    repr.as_wtf8(),
                    source,
                    obj.class().name()
                ))
            })
        };

        let locals_map = self.locals.mapping(vm);
        if let Ok(all) = dict.get_item(identifier!(vm, __all__), vm) {
            let items: Vec<PyObjectRef> = all.try_to_value(vm)?;
            for item in items {
                let name = require_str(item, "__all__")?;
                let value = module.get_attr(&*name, vm)?;
                locals_map.ass_subscript(&name, Some(value), vm)?;
            }
        } else {
            for (k, v) in dict {
                let k = require_str(k, "__dict__")?;
                if !k.as_bytes().starts_with(b"_") {
                    locals_map.ass_subscript(&k, Some(v), vm)?;
                }
            }
        }
        Ok(())
    }

    /// Unwind blocks.
    /// The reason for unwinding gives a hint on what to do when
    /// unwinding a block.
    /// Optionally returns an exception.
    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn unwind_blocks(&mut self, vm: &VirtualMachine, reason: UnwindReason) -> FrameResult {
        // use exception table for exception handling
        match reason {
            UnwindReason::Raising { exception } => {
                // Look up handler in exception table
                // lasti points to NEXT instruction (already incremented in run loop)
                // The exception occurred at the previous instruction
                // Python uses signed int where INSTR_OFFSET() - 1 = -1 before first instruction.
                // We use u32, so check for 0 explicitly.
                if self.lasti() == 0 {
                    // No instruction executed yet, no handler can match
                    return Err(exception);
                }
                let offset = self.lasti() - 1;
                if let Some(entry) =
                    bytecode::find_exception_handler(&self.code.exceptiontable, offset)
                {
                    // Fire EXCEPTION_HANDLED before setting up handler.
                    // If the callback raises, the handler is NOT set up and the
                    // new exception propagates instead.
                    if vm.state.monitoring_events.load() & monitoring::EVENT_EXCEPTION_HANDLED != 0
                    {
                        let byte_offset = offset * 2;
                        let exc_obj: PyObjectRef = exception.clone().into();
                        monitoring::fire_exception_handled(vm, self.code, byte_offset, &exc_obj)?;
                    }

                    // 1. Pop stack to entry.depth
                    while self.localsplus.stack_len() > entry.depth as usize {
                        let _ = self.localsplus.stack_pop();
                    }

                    // 2. If push_lasti=true (SETUP_CLEANUP), push lasti before exception
                    // pushes lasti as PyLong
                    if entry.push_lasti {
                        self.push_value(vm.ctx.new_int(offset as i32).into());
                    }

                    // 3. Push exception onto stack
                    // always push exception, PUSH_EXC_INFO transforms [exc] -> [prev_exc, exc]
                    // Do NOT call vm.set_exception here! PUSH_EXC_INFO will do it.
                    // PUSH_EXC_INFO needs to get prev_exc from vm.current_exception() BEFORE setting the new one.
                    self.push_value(exception.into());

                    // 4. Jump to handler
                    self.jump(bytecode::Label::from_u32(entry.target));

                    Ok(None)
                } else {
                    // No handler found, propagate exception
                    Err(exception)
                }
            }
            UnwindReason::Returning { value } => Ok(Some(ExecutionResult::Return(value))),
        }
    }

    fn execute_store_subscript(&mut self, vm: &VirtualMachine) -> FrameResult {
        let idx = self.pop_value();
        let obj = self.pop_value();
        let value = self.pop_value();
        obj.set_item(&*idx, value, vm)?;
        Ok(None)
    }

    fn execute_delete_subscript(&mut self, vm: &VirtualMachine) -> FrameResult {
        let idx = self.pop_value();
        let obj = self.pop_value();
        obj.del_item(&*idx, vm)?;
        Ok(None)
    }

    fn execute_build_map(&mut self, vm: &VirtualMachine, size: u32) -> FrameResult {
        let size = size as usize;
        let map_obj = vm.ctx.new_dict();
        for (key, value) in self.pop_multiple(2 * size).tuples() {
            map_obj.set_item(&*key, value, vm)?;
        }

        self.push_value(map_obj.into());
        Ok(None)
    }

    fn execute_build_slice(
        &mut self,
        vm: &VirtualMachine,
        argc: bytecode::BuildSliceArgCount,
    ) -> FrameResult {
        let step = match argc {
            bytecode::BuildSliceArgCount::Two => None,
            bytecode::BuildSliceArgCount::Three => Some(self.pop_value()),
        };
        let stop = self.pop_value();
        let start = self.pop_value();

        let obj = PySlice {
            start: Some(start),
            stop,
            step,
        }
        .into_ref(&vm.ctx);
        self.push_value(obj.into());
        Ok(None)
    }

    fn collect_positional_args(&mut self, nargs: u32) -> FuncArgs {
        FuncArgs {
            args: self.pop_multiple(nargs as usize).collect(),
            kwargs: IndexMap::new(),
        }
    }

    fn collect_keyword_args(&mut self, nargs: u32) -> FuncArgs {
        let kwarg_names = self
            .pop_value()
            .downcast::<PyTuple>()
            .expect("kwarg names should be tuple of strings");
        let args = self.pop_multiple(nargs as usize);

        let kwarg_names = kwarg_names.as_slice().iter().map(|pyobj| {
            pyobj
                .downcast_ref::<PyUtf8Str>()
                .unwrap()
                .as_str()
                .to_owned()
        });
        FuncArgs::with_kwargs_names(args, kwarg_names)
    }

    fn collect_ex_args(&mut self, vm: &VirtualMachine) -> PyResult<FuncArgs> {
        let kwargs_or_null = self.pop_value_opt();
        let kwargs = if let Some(kw_obj) = kwargs_or_null {
            let mut kwargs = IndexMap::new();

            // Stack: [callable, self_or_null, args_tuple]
            let callable = self.nth_value(2);
            let func_str = Self::object_function_str(callable, vm);

            Self::iterate_mapping_keys(vm, &kw_obj, &func_str, |key| {
                let key_str = key
                    .downcast_ref::<PyUtf8Str>()
                    .ok_or_else(|| vm.new_type_error("keywords must be strings"))?;
                let value = kw_obj.get_item(&*key, vm)?;
                kwargs.insert(key_str.as_str().to_owned(), value);
                Ok(())
            })?;
            kwargs
        } else {
            IndexMap::new()
        };
        let args_obj = self.pop_value();
        let args = if let Some(tuple) = args_obj.downcast_ref::<PyTuple>() {
            tuple.as_slice().to_vec()
        } else {
            // Single *arg passed directly; convert to sequence at runtime.
            // Stack: [callable, self_or_null]
            let callable = self.nth_value(1);
            let func_str = Self::object_function_str(callable, vm);
            let not_iterable = args_obj.class().slots.iter.load().is_none()
                && args_obj
                    .get_class_attr(vm.ctx.intern_str("__getitem__"))
                    .is_none();
            args_obj.try_to_value::<Vec<PyObjectRef>>(vm).map_err(|e| {
                if not_iterable && e.class().is(vm.ctx.exceptions.type_error) {
                    vm.new_type_error(format!(
                        "{} argument after * must be an iterable, not {}",
                        func_str,
                        args_obj.class().name()
                    ))
                } else {
                    e
                }
            })?
        };
        Ok(FuncArgs { args, kwargs })
    }

    /// Returns a display string for a callable object for use in error messages.
    /// For objects with `__qualname__`, returns "module.qualname()" or "qualname()".
    /// For other objects, returns repr(obj).
    fn object_function_str(obj: &PyObject, vm: &VirtualMachine) -> Wtf8Buf {
        let repr_fallback = || {
            obj.repr(vm)
                .as_ref()
                .map_or_else(|_| "?".as_ref(), |s| s.as_wtf8())
                .to_owned()
        };
        let Ok(qualname) = obj.get_attr(vm.ctx.intern_str("__qualname__"), vm) else {
            return repr_fallback();
        };
        let Some(qualname_str) = qualname.downcast_ref::<PyStr>() else {
            return repr_fallback();
        };
        if let Ok(module) = obj.get_attr(vm.ctx.intern_str("__module__"), vm)
            && let Some(module_str) = module.downcast_ref::<PyStr>()
            && module_str.as_bytes() != b"builtins"
        {
            return wtf8_concat!(module_str.as_wtf8(), ".", qualname_str.as_wtf8(), "()");
        }
        wtf8_concat!(qualname_str.as_wtf8(), "()")
    }

    /// Helper function to iterate over mapping keys using the keys() method.
    /// This ensures proper order preservation for OrderedDict and other custom mappings.
    fn iterate_mapping_keys<F>(
        vm: &VirtualMachine,
        mapping: &PyObject,
        func_str: &Wtf8,
        mut key_handler: F,
    ) -> PyResult<()>
    where
        F: FnMut(PyObjectRef) -> PyResult<()>,
    {
        let Some(keys_method) = vm.get_method(mapping.to_owned(), vm.ctx.intern_str("keys")) else {
            return Err(vm.new_type_error(format!(
                "{} argument after ** must be a mapping, not {}",
                func_str,
                mapping.class().name()
            )));
        };

        let keys = keys_method?.call((), vm)?.get_iter(vm)?;
        while let PyIterReturn::Return(key) = keys.next(vm)? {
            key_handler(key)?;
        }
        Ok(())
    }

    /// Vectorcall dispatch for Instruction::Call (positional args only).
    /// Uses vectorcall slot if available, otherwise falls back to FuncArgs.
    #[inline]
    fn execute_call_vectorcall(&mut self, nargs: u32, vm: &VirtualMachine) -> FrameResult {
        let nargs_usize = nargs as usize;
        let stack_len = self.localsplus.stack_len();
        debug_assert!(
            stack_len >= nargs_usize + 2,
            "CALL stack underflow: need callable + self_or_null + {nargs_usize} args, have {stack_len}"
        );
        let callable_idx = stack_len - nargs_usize - 2;
        let self_or_null_idx = stack_len - nargs_usize - 1;
        let args_start = stack_len - nargs_usize;

        // Build args: [self?, arg1, ..., argN]
        let self_or_null = self
            .localsplus
            .stack_index_mut(self_or_null_idx)
            .take()
            .map(|sr| sr.to_pyobj());
        let has_self = self_or_null.is_some();

        let effective_nargs = if has_self {
            nargs_usize + 1
        } else {
            nargs_usize
        };
        let mut args_vec = Vec::with_capacity(effective_nargs);
        if let Some(self_val) = self_or_null {
            args_vec.push(self_val);
        }
        for stack_idx in args_start..stack_len {
            let val = self
                .localsplus
                .stack_index_mut(stack_idx)
                .take()
                .unwrap()
                .to_pyobj();
            args_vec.push(val);
        }

        let callable_obj = self
            .localsplus
            .stack_index_mut(callable_idx)
            .take()
            .unwrap()
            .to_pyobj();
        self.localsplus.stack_truncate(callable_idx);

        // invoke_vectorcall falls back to FuncArgs if no vectorcall slot
        let result = callable_obj.vectorcall(args_vec, effective_nargs, None, vm)?;
        self.push_value(result);
        Ok(None)
    }

    /// Vectorcall dispatch for Instruction::CallKw (positional + keyword args).
    #[inline]
    fn execute_call_kw_vectorcall(&mut self, nargs: u32, vm: &VirtualMachine) -> FrameResult {
        let nargs_usize = nargs as usize;

        // Pop kwarg_names tuple from top of stack
        let kwarg_names_obj = self.pop_value();
        let kwarg_names_tuple = kwarg_names_obj
            .downcast_ref::<PyTuple>()
            .expect("kwarg names should be tuple");
        let kw_count = kwarg_names_tuple.len();
        debug_assert!(kw_count <= nargs_usize, "CALL_KW kw_count exceeds nargs");

        let stack_len = self.localsplus.stack_len();
        debug_assert!(
            stack_len >= nargs_usize + 2,
            "CALL_KW stack underflow: need callable + self_or_null + {nargs_usize} args, have {stack_len}"
        );
        let callable_idx = stack_len - nargs_usize - 2;
        let self_or_null_idx = stack_len - nargs_usize - 1;
        let args_start = stack_len - nargs_usize;

        // Build args: [self?, pos_arg1, ..., pos_argM, kw_val1, ..., kw_valK]
        let self_or_null = self
            .localsplus
            .stack_index_mut(self_or_null_idx)
            .take()
            .map(|sr| sr.to_pyobj());
        let has_self = self_or_null.is_some();

        let pos_count = nargs_usize
            .checked_sub(kw_count)
            .expect("CALL_KW: kw_count exceeds nargs");
        let effective_nargs = if has_self { pos_count + 1 } else { pos_count };

        // Build the full args slice: positional (including self) + kwarg values
        let total_args = effective_nargs + kw_count;
        let mut args_vec = Vec::with_capacity(total_args);
        if let Some(self_val) = self_or_null {
            args_vec.push(self_val);
        }
        for stack_idx in args_start..stack_len {
            let val = self
                .localsplus
                .stack_index_mut(stack_idx)
                .take()
                .unwrap()
                .to_pyobj();
            args_vec.push(val);
        }

        let callable_obj = self
            .localsplus
            .stack_index_mut(callable_idx)
            .take()
            .unwrap()
            .to_pyobj();
        self.localsplus.stack_truncate(callable_idx);

        // invoke_vectorcall falls back to FuncArgs if no vectorcall slot
        let kwnames = kwarg_names_tuple.as_slice();
        let result = callable_obj.vectorcall(args_vec, effective_nargs, Some(kwnames), vm)?;
        self.push_value(result);
        Ok(None)
    }

    #[inline]
    fn execute_call(&mut self, args: FuncArgs, vm: &VirtualMachine) -> FrameResult {
        // Stack: [callable, self_or_null, ...]
        let self_or_null = self.pop_value_opt(); // Option<PyObjectRef>
        let callable = self.pop_value();

        let final_args = if let Some(self_val) = self_or_null {
            let mut args = args;
            args.prepend_arg(self_val);
            args
        } else {
            args
        };

        let value = callable.call(final_args, vm)?;
        self.push_value(value);
        Ok(None)
    }

    /// Instrumented version of execute_call: fires CALL, C_RETURN, and C_RAISE events.
    fn execute_call_instrumented(&mut self, args: FuncArgs, vm: &VirtualMachine) -> FrameResult {
        let self_or_null = self.pop_value_opt();
        let callable = self.pop_value();

        let final_args = if let Some(self_val) = self_or_null {
            let mut args = args;
            args.prepend_arg(self_val);
            args
        } else {
            args
        };

        let is_python_call = callable.downcast_ref_if_exact::<PyFunction>(vm).is_some();

        // Fire CALL event
        let call_arg0 = if self.monitoring_mask & monitoring::EVENT_CALL != 0 {
            let arg0 = final_args
                .args
                .first()
                .cloned()
                .unwrap_or_else(|| monitoring::get_missing(vm));
            let offset = (self.lasti() - 1) * 2;
            monitoring::fire_call(vm, self.code, offset, &callable, arg0.clone())?;
            Some(arg0)
        } else {
            None
        };

        match callable.call(final_args, vm) {
            Ok(value) => {
                if let Some(arg0) = call_arg0
                    && !is_python_call
                {
                    let offset = (self.lasti() - 1) * 2;
                    monitoring::fire_c_return(vm, self.code, offset, &callable, arg0)?;
                }
                self.push_value(value);
                Ok(None)
            }
            Err(exc) => {
                let exc = if let Some(arg0) = call_arg0
                    && !is_python_call
                {
                    let offset = (self.lasti() - 1) * 2;
                    match monitoring::fire_c_raise(vm, self.code, offset, &callable, arg0) {
                        Ok(()) => exc,
                        Err(monitor_exc) => monitor_exc,
                    }
                } else {
                    exc
                };
                Err(exc)
            }
        }
    }

    fn execute_raise(&mut self, vm: &VirtualMachine, kind: bytecode::RaiseKind) -> FrameResult {
        let cause = match kind {
            bytecode::RaiseKind::RaiseCause => {
                let val = self.pop_value();
                Some(if vm.is_none(&val) {
                    // if the cause arg is none, we clear the cause
                    None
                } else {
                    // if the cause arg is an exception, we overwrite it
                    let ctor = ExceptionCtor::try_from_object(vm, val).map_err(|_| {
                        vm.new_type_error("exception causes must derive from BaseException")
                    })?;
                    Some(ctor.instantiate(vm)?)
                })
            }
            // if there's no cause arg, we keep the cause as is
            _ => None,
        };
        let exception = match kind {
            bytecode::RaiseKind::RaiseCause | bytecode::RaiseKind::Raise => {
                ExceptionCtor::try_from_object(vm, self.pop_value())?.instantiate(vm)?
            }
            bytecode::RaiseKind::BareRaise => {
                // RAISE_VARARGS 0: bare `raise` gets exception from VM state
                vm.topmost_exception()
                    .ok_or_else(|| vm.new_runtime_error("No active exception to reraise"))?
            }
            bytecode::RaiseKind::ReraiseFromStack => {
                // RERAISE: gets exception from stack top
                // Used in cleanup blocks where exception is on stack after COPY 3
                let exc = self.pop_value();
                exc.downcast::<PyBaseException>().map_err(|obj| {
                    vm.new_type_error(format!(
                        "exceptions must derive from BaseException, not {}",
                        obj.class().name()
                    ))
                })?
            }
        };
        #[cfg(debug_assertions)]
        debug!("Exception raised: {exception:?} with cause: {cause:?}");
        if let Some(cause) = cause {
            exception.set___cause__(cause);
        }
        Err(exception)
    }

    fn builtin_coro<'a>(&self, coro: &'a PyObject) -> Option<&'a Coro> {
        match_class!(match coro {
            ref g @ PyGenerator => Some(g.as_coro()),
            ref c @ PyCoroutine => Some(c.as_coro()),
            _ => None,
        })
    }

    fn _send(
        &self,
        jen: &PyObject,
        val: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyIterReturn> {
        match self.builtin_coro(jen) {
            Some(coro) => coro.send(jen, val, vm),
            // TODO: turn return type to PyResult<PyIterReturn> then ExecutionResult will be simplified
            None if vm.is_none(&val) => PyIter::new(jen).next(vm),
            None => {
                let meth = jen.get_attr("send", vm)?;
                PyIterReturn::from_pyresult(meth.call((val,), vm), vm)
            }
        }
    }

    fn execute_unpack_ex(&mut self, vm: &VirtualMachine, before: u8, after: u8) -> FrameResult {
        let (before, after) = (before as usize, after as usize);
        let value = self.pop_value();
        let not_iterable = value.class().slots.iter.load().is_none()
            && value
                .get_class_attr(vm.ctx.intern_str("__getitem__"))
                .is_none();
        let elements: Vec<_> = value.try_to_value(vm).map_err(|e| {
            if not_iterable && e.class().is(vm.ctx.exceptions.type_error) {
                vm.new_type_error(format!(
                    "cannot unpack non-iterable {} object",
                    value.class().name()
                ))
            } else {
                e
            }
        })?;
        let min_expected = before + after;

        let middle = elements.len().checked_sub(min_expected).ok_or_else(|| {
            vm.new_value_error(format!(
                "not enough values to unpack (expected at least {}, got {})",
                min_expected,
                elements.len()
            ))
        })?;

        let mut elements = elements;
        // Elements on stack from right-to-left:
        self.localsplus.stack_extend(
            elements
                .drain(before + middle..)
                .rev()
                .map(|e| Some(PyStackRef::new_owned(e))),
        );

        let middle_elements = elements.drain(before..).collect();
        let t = vm.ctx.new_list(middle_elements);
        self.push_value(t.into());

        // Lastly the first reversed values:
        self.localsplus.stack_extend(
            elements
                .into_iter()
                .rev()
                .map(|e| Some(PyStackRef::new_owned(e))),
        );

        Ok(None)
    }

    #[inline]
    fn jump(&mut self, label: bytecode::Label) {
        let target_pc = label.as_u32();
        vm_trace!("jump from {:?} to {:?}", self.lasti(), target_pc);
        self.update_lasti(|i| *i = target_pc);
    }

    /// Jump forward by `delta` code units from after instruction + caches.
    /// lasti is already at instruction_index + 1, so after = lasti + caches.
    ///
    /// Unchecked arithmetic is intentional: the compiler guarantees valid
    /// targets, and debug builds will catch overflow via Rust's default checks.
    #[inline]
    fn jump_relative_forward(&mut self, delta: u32, caches: u32) {
        let target = self.lasti() + caches + delta;
        self.update_lasti(|i| *i = target);
    }

    /// Jump backward by `delta` code units from after instruction + caches.
    ///
    /// Unchecked arithmetic is intentional: the compiler guarantees valid
    /// targets, and debug builds will catch underflow via Rust's default checks.
    #[inline]
    fn jump_relative_backward(&mut self, delta: u32, caches: u32) {
        let target = self.lasti() + caches - delta;
        self.update_lasti(|i| *i = target);
    }

    #[inline]
    fn pop_jump_if_relative(
        &mut self,
        vm: &VirtualMachine,
        arg: bytecode::OpArg,
        caches: u32,
        flag: bool,
    ) -> FrameResult {
        let obj = self.pop_value();
        let value = obj.try_to_bool(vm)?;
        if value == flag {
            self.jump_relative_forward(u32::from(arg), caches);
        }
        Ok(None)
    }

    /// Advance the iterator on top of stack.
    /// Returns `true` if iteration continued (item pushed), `false` if exhausted (jumped).
    fn execute_for_iter(
        &mut self,
        vm: &VirtualMachine,
        target: bytecode::Label,
    ) -> Result<bool, PyBaseExceptionRef> {
        let top = self.top_value();

        // FOR_ITER_RANGE: bypass generic iterator protocol for range iterators
        if let Some(range_iter) = top.downcast_ref_if_exact::<PyRangeIterator>(vm) {
            if let Some(value) = range_iter.fast_next() {
                self.push_value(vm.ctx.new_int(value).into());
                return Ok(true);
            }
            if vm.use_tracing.get() && !vm.is_none(&self.object.trace.lock()) {
                let stop_exc = vm.new_stop_iteration(None);
                self.fire_exception_trace(&stop_exc, vm)?;
            }
            self.jump(self.for_iter_jump_target(target));
            return Ok(false);
        }

        let top_of_stack = PyIter::new(top);
        let next_obj = top_of_stack.next(vm);

        match next_obj {
            Ok(PyIterReturn::Return(value)) => {
                self.push_value(value);
                Ok(true)
            }
            Ok(PyIterReturn::StopIteration(value)) => {
                // Fire 'exception' trace event for StopIteration, matching
                // FOR_ITER's inline call to _PyEval_MonitorRaise.
                if vm.use_tracing.get() && !vm.is_none(&self.object.trace.lock()) {
                    let stop_exc = vm.new_stop_iteration(value);
                    self.fire_exception_trace(&stop_exc, vm)?;
                }
                self.jump(self.for_iter_jump_target(target));
                Ok(false)
            }
            Err(next_error) => {
                self.pop_value();
                Err(next_error)
            }
        }
    }

    /// Compute the jump target for FOR_ITER exhaustion: skip END_FOR and jump to POP_ITER.
    fn for_iter_jump_target(&self, target: bytecode::Label) -> bytecode::Label {
        let target_idx = target.as_usize();
        if let Some(unit) = self.code.instructions.get(target_idx)
            && matches!(
                unit.op,
                bytecode::Instruction::EndFor | bytecode::Instruction::InstrumentedEndFor
            )
        {
            return bytecode::Label::from_u32(target.as_u32() + 1);
        }
        target
    }
    fn execute_make_function(&mut self, vm: &VirtualMachine) -> FrameResult {
        // MakeFunction only takes code object, no flags
        let code_obj: PyRef<PyCode> = self
            .pop_value()
            .downcast()
            .expect("Stack value should be code object");

        // Create function with minimal attributes
        let func_obj = PyFunction::new(code_obj, self.globals.clone(), vm)?.into_pyobject(vm);

        self.push_value(func_obj);
        Ok(None)
    }

    fn execute_set_function_attribute(
        &mut self,
        vm: &VirtualMachine,
        attr: bytecode::MakeFunctionFlag,
    ) -> FrameResult {
        // SET_FUNCTION_ATTRIBUTE sets attributes on a function
        // Stack: [..., attr_value, func] -> [..., func]
        // Stack order: func is at -1, attr_value is at -2

        let func = self.pop_value_opt();
        let attr_value = expect_unchecked(self.replace_top(func), "attr_value must not be null");

        let func = self.top_value();
        // Get the function reference and call the new method
        let func_ref = func
            .downcast_ref_if_exact::<PyFunction>(vm)
            .expect("SET_FUNCTION_ATTRIBUTE expects function on stack");

        let payload: &PyFunction = func_ref.payload();
        // SetFunctionAttribute always follows MakeFunction, so at this point
        // there are no other references to func. It is therefore safe to treat it as mutable.
        unsafe {
            let payload_ptr = payload as *const PyFunction as *mut PyFunction;
            (*payload_ptr).set_function_attribute(attr, attr_value, vm)?;
        };

        Ok(None)
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn execute_bin_op(&mut self, vm: &VirtualMachine, op: bytecode::BinaryOperator) -> FrameResult {
        let b_ref = &self.pop_value();
        let a_ref = &self.pop_value();
        let value = match op {
            // BINARY_OP_ADD_INT / BINARY_OP_SUBTRACT_INT fast paths:
            // bypass binary_op1 dispatch for exact int types, use i64 arithmetic
            // when possible to avoid BigInt heap allocation.
            bytecode::BinaryOperator::Add | bytecode::BinaryOperator::InplaceAdd => {
                if let (Some(a), Some(b)) = (
                    a_ref.downcast_ref_if_exact::<PyInt>(vm),
                    b_ref.downcast_ref_if_exact::<PyInt>(vm),
                ) {
                    Ok(self.int_add(a.as_bigint(), b.as_bigint(), vm))
                } else if matches!(op, bytecode::BinaryOperator::Add) {
                    vm._add(a_ref, b_ref)
                } else {
                    vm._iadd(a_ref, b_ref)
                }
            }
            bytecode::BinaryOperator::Subtract | bytecode::BinaryOperator::InplaceSubtract => {
                if let (Some(a), Some(b)) = (
                    a_ref.downcast_ref_if_exact::<PyInt>(vm),
                    b_ref.downcast_ref_if_exact::<PyInt>(vm),
                ) {
                    Ok(self.int_sub(a.as_bigint(), b.as_bigint(), vm))
                } else if matches!(op, bytecode::BinaryOperator::Subtract) {
                    vm._sub(a_ref, b_ref)
                } else {
                    vm._isub(a_ref, b_ref)
                }
            }
            bytecode::BinaryOperator::Multiply => vm._mul(a_ref, b_ref),
            bytecode::BinaryOperator::MatrixMultiply => vm._matmul(a_ref, b_ref),
            bytecode::BinaryOperator::Power => vm._pow(a_ref, b_ref, vm.ctx.none.as_object()),
            bytecode::BinaryOperator::TrueDivide => vm._truediv(a_ref, b_ref),
            bytecode::BinaryOperator::FloorDivide => vm._floordiv(a_ref, b_ref),
            bytecode::BinaryOperator::Remainder => vm._mod(a_ref, b_ref),
            bytecode::BinaryOperator::Lshift => vm._lshift(a_ref, b_ref),
            bytecode::BinaryOperator::Rshift => vm._rshift(a_ref, b_ref),
            bytecode::BinaryOperator::Xor => vm._xor(a_ref, b_ref),
            bytecode::BinaryOperator::Or => vm._or(a_ref, b_ref),
            bytecode::BinaryOperator::And => vm._and(a_ref, b_ref),
            bytecode::BinaryOperator::InplaceMultiply => vm._imul(a_ref, b_ref),
            bytecode::BinaryOperator::InplaceMatrixMultiply => vm._imatmul(a_ref, b_ref),
            bytecode::BinaryOperator::InplacePower => {
                vm._ipow(a_ref, b_ref, vm.ctx.none.as_object())
            }
            bytecode::BinaryOperator::InplaceTrueDivide => vm._itruediv(a_ref, b_ref),
            bytecode::BinaryOperator::InplaceFloorDivide => vm._ifloordiv(a_ref, b_ref),
            bytecode::BinaryOperator::InplaceRemainder => vm._imod(a_ref, b_ref),
            bytecode::BinaryOperator::InplaceLshift => vm._ilshift(a_ref, b_ref),
            bytecode::BinaryOperator::InplaceRshift => vm._irshift(a_ref, b_ref),
            bytecode::BinaryOperator::InplaceXor => vm._ixor(a_ref, b_ref),
            bytecode::BinaryOperator::InplaceOr => vm._ior(a_ref, b_ref),
            bytecode::BinaryOperator::InplaceAnd => vm._iand(a_ref, b_ref),
            bytecode::BinaryOperator::Subscr => a_ref.get_item(b_ref.as_object(), vm),
        }?;

        self.push_value(value);
        Ok(None)
    }

    /// Int addition with i64 fast path to avoid BigInt heap allocation.
    #[inline]
    fn int_add(&self, a: &BigInt, b: &BigInt, vm: &VirtualMachine) -> PyObjectRef {
        use num_traits::ToPrimitive;
        if let (Some(av), Some(bv)) = (a.to_i64(), b.to_i64())
            && let Some(result) = av.checked_add(bv)
        {
            return vm.ctx.new_int(result).into();
        }
        vm.ctx.new_int(a + b).into()
    }

    /// Int subtraction with i64 fast path to avoid BigInt heap allocation.
    #[inline]
    fn int_sub(&self, a: &BigInt, b: &BigInt, vm: &VirtualMachine) -> PyObjectRef {
        use num_traits::ToPrimitive;
        if let (Some(av), Some(bv)) = (a.to_i64(), b.to_i64())
            && let Some(result) = av.checked_sub(bv)
        {
            return vm.ctx.new_int(result).into();
        }
        vm.ctx.new_int(a - b).into()
    }

    #[cold]
    fn setup_annotations(&mut self, vm: &VirtualMachine) -> FrameResult {
        let __annotations__ = identifier!(vm, __annotations__);
        let locals_obj = self.locals.as_object(vm);
        // Try using locals as dict first, if not, fallback to generic method.
        let has_annotations = if let Some(d) = locals_obj.downcast_ref_if_exact::<PyDict>(vm) {
            d.contains_key(__annotations__, vm)
        } else {
            self._in(vm, __annotations__.as_object(), locals_obj)?
        };
        if !has_annotations {
            locals_obj.set_item(__annotations__, vm.ctx.new_dict().into(), vm)?;
        }
        Ok(None)
    }

    /// _PyEval_UnpackIterableStackRef
    fn unpack_sequence(&mut self, size: u32, vm: &VirtualMachine) -> FrameResult {
        let value = self.pop_value();
        let size = size as usize;

        // Fast path for exact tuple/list types (not subclasses) — push
        // elements directly from the slice without intermediate Vec allocation,
        // matching UNPACK_SEQUENCE_TUPLE / UNPACK_SEQUENCE_LIST specializations.
        let cls = value.class();
        if cls.is(vm.ctx.types.tuple_type) {
            let tuple = value.downcast_ref::<PyTuple>().unwrap();
            return self.unpack_fast(tuple.as_slice(), size, vm);
        }
        if cls.is(vm.ctx.types.list_type) {
            let list = value.downcast_ref::<PyList>().unwrap();
            let borrowed = list.borrow_vec();
            return self.unpack_fast(&borrowed, size, vm);
        }

        // General path — iterate up to `size + 1` elements to avoid
        // consuming the entire iterator (fixes hang on infinite sequences).
        let not_iterable = value.class().slots.iter.load().is_none()
            && value
                .get_class_attr(vm.ctx.intern_str("__getitem__"))
                .is_none();
        let iter = PyIter::try_from_object(vm, value.clone()).map_err(|e| {
            if not_iterable && e.class().is(vm.ctx.exceptions.type_error) {
                vm.new_type_error(format!(
                    "cannot unpack non-iterable {} object",
                    value.class().name()
                ))
            } else {
                e
            }
        })?;

        let mut elements = Vec::with_capacity(size);
        for _ in 0..size {
            match iter.next(vm)? {
                PyIterReturn::Return(item) => elements.push(item),
                PyIterReturn::StopIteration(_) => {
                    return Err(vm.new_value_error(format!(
                        "not enough values to unpack (expected {size}, got {})",
                        elements.len()
                    )));
                }
            }
        }

        // Check that the iterator is exhausted.
        match iter.next(vm)? {
            PyIterReturn::Return(_) => {
                // For exact dict types, show "got N" using the container's
                // size (PyDict_Size). Exact tuple/list are handled by the
                // fast path above and never reach here.
                let msg = if value.class().is(vm.ctx.types.dict_type) {
                    if let Ok(got) = value.length(vm) {
                        if got > size {
                            format!("too many values to unpack (expected {size}, got {got})")
                        } else {
                            format!("too many values to unpack (expected {size})")
                        }
                    } else {
                        format!("too many values to unpack (expected {size})")
                    }
                } else {
                    format!("too many values to unpack (expected {size})")
                };
                Err(vm.new_value_error(msg))
            }
            PyIterReturn::StopIteration(_) => {
                self.localsplus.stack_extend(
                    elements
                        .into_iter()
                        .rev()
                        .map(|e| Some(PyStackRef::new_owned(e))),
                );
                Ok(None)
            }
        }
    }

    fn unpack_fast(
        &mut self,
        elements: &[PyObjectRef],
        size: usize,
        vm: &VirtualMachine,
    ) -> FrameResult {
        match elements.len().cmp(&size) {
            core::cmp::Ordering::Equal => {
                for elem in elements.iter().rev() {
                    self.push_value(elem.clone());
                }
                Ok(None)
            }
            core::cmp::Ordering::Greater => Err(vm.new_value_error(format!(
                "too many values to unpack (expected {size}, got {})",
                elements.len()
            ))),
            core::cmp::Ordering::Less => Err(vm.new_value_error(format!(
                "not enough values to unpack (expected {size}, got {})",
                elements.len()
            ))),
        }
    }

    fn convert_value(
        &mut self,
        conversion: bytecode::ConvertValueOparg,
        vm: &VirtualMachine,
    ) -> FrameResult {
        use bytecode::ConvertValueOparg;
        let value = self.pop_value();
        let value = match conversion {
            ConvertValueOparg::Str => value.str(vm)?.into(),
            ConvertValueOparg::Repr => value.repr(vm)?.into(),
            ConvertValueOparg::Ascii => builtins::ascii(value, vm)?.into(),
            ConvertValueOparg::None => value,
        };

        self.push_value(value);
        Ok(None)
    }

    fn _in(&self, vm: &VirtualMachine, needle: &PyObject, haystack: &PyObject) -> PyResult<bool> {
        let found = vm._contains(haystack, needle)?;
        Ok(found)
    }

    #[inline(always)]
    fn _not_in(
        &self,
        vm: &VirtualMachine,
        needle: &PyObject,
        haystack: &PyObject,
    ) -> PyResult<bool> {
        Ok(!self._in(vm, needle, haystack)?)
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn execute_compare(&mut self, vm: &VirtualMachine, arg: bytecode::OpArg) -> FrameResult {
        let op = bytecode::ComparisonOperator::try_from(u32::from(arg))
            .unwrap_or(bytecode::ComparisonOperator::Equal);
        let b = self.pop_value();
        let a = self.pop_value();
        let cmp_op: PyComparisonOp = op.into();
        let force_bool = u32::from(arg) & bytecode::oparg::COMPARE_OP_BOOL_MASK != 0;

        // COMPARE_OP_INT: leaf type, cannot recurse — skip rich_compare dispatch
        if let (Some(a_int), Some(b_int)) = (
            a.downcast_ref_if_exact::<PyInt>(vm),
            b.downcast_ref_if_exact::<PyInt>(vm),
        ) {
            let result = cmp_op.eval_ord(a_int.as_bigint().cmp(b_int.as_bigint()));
            self.push_value(vm.ctx.new_bool(result).into());
            return Ok(None);
        }
        // COMPARE_OP_FLOAT: leaf type, cannot recurse — skip rich_compare dispatch.
        // Falls through on NaN (partial_cmp returns None) for correct != semantics.
        if let (Some(a_f), Some(b_f)) = (
            a.downcast_ref_if_exact::<PyFloat>(vm),
            b.downcast_ref_if_exact::<PyFloat>(vm),
        ) && let Some(ord) = a_f.to_f64().partial_cmp(&b_f.to_f64())
        {
            let result = cmp_op.eval_ord(ord);
            self.push_value(vm.ctx.new_bool(result).into());
            return Ok(None);
        }

        let value = a.rich_compare(b, cmp_op, vm)?;
        let value = if force_bool {
            let bool_val = value.try_to_bool(vm)?;
            vm.ctx.new_bool(bool_val).into()
        } else {
            value
        };
        self.push_value(value);
        Ok(None)
    }

    /// Read a cached descriptor pointer and validate it against the expected
    /// type version, using a lock-free double-check pattern:
    ///   1. read pointer  →  incref (try_to_owned)
    ///   2. re-read version + pointer and confirm they still match
    ///
    /// This matches the read-side pattern used in LOAD_ATTR_METHOD_WITH_VALUES
    /// and friends: no read-side lock, relying on the write side to invalidate
    /// the version tag before swapping the pointer.
    #[inline]
    fn try_read_cached_descriptor(
        &self,
        cache_base: usize,
        expected_type_version: u32,
    ) -> Option<PyObjectRef> {
        let descr_ptr = self.code.instructions.read_cache_ptr(cache_base + 5);
        if descr_ptr == 0 {
            return None;
        }
        // SAFETY: `descr_ptr` was a valid `*mut PyObject` when the writer
        // stored it, and the writer keeps a strong reference alive in
        // `InlineCacheEntry`.  `try_to_owned_from_ptr` performs a
        // conditional incref that fails if the object is already freed.
        let cloned = unsafe { PyObject::try_to_owned_from_ptr(descr_ptr as *mut PyObject) }?;
        // Double-check: version tag still matches AND pointer unchanged.
        if self.code.instructions.read_cache_u32(cache_base + 1) == expected_type_version
            && self.code.instructions.read_cache_ptr(cache_base + 5) == descr_ptr
        {
            Some(cloned)
        } else {
            drop(cloned);
            None
        }
    }

    #[inline]
    unsafe fn write_cached_descriptor(
        &self,
        cache_base: usize,
        type_version: u32,
        descr_ptr: usize,
    ) {
        // Publish descriptor cache with version-invalidation protocol:
        // invalidate version first, then write payload, then publish version.
        // Reader double-checks version+ptr after incref, so no writer lock needed.
        unsafe {
            self.code.instructions.write_cache_u32(cache_base + 1, 0);
            self.code
                .instructions
                .write_cache_ptr(cache_base + 5, descr_ptr);
            self.code
                .instructions
                .write_cache_u32(cache_base + 1, type_version);
        }
    }

    #[inline]
    unsafe fn write_cached_descriptor_with_metaclass(
        &self,
        cache_base: usize,
        type_version: u32,
        metaclass_version: u32,
        descr_ptr: usize,
    ) {
        unsafe {
            self.code.instructions.write_cache_u32(cache_base + 1, 0);
            self.code
                .instructions
                .write_cache_u32(cache_base + 3, metaclass_version);
            self.code
                .instructions
                .write_cache_ptr(cache_base + 5, descr_ptr);
            self.code
                .instructions
                .write_cache_u32(cache_base + 1, type_version);
        }
    }

    #[inline]
    unsafe fn write_cached_binary_op_extend_descr(
        &self,
        cache_base: usize,
        descr: Option<&'static BinaryOpExtendSpecializationDescr>,
    ) {
        let ptr = descr.map_or(0, |d| {
            d as *const BinaryOpExtendSpecializationDescr as usize
        });
        unsafe {
            self.code
                .instructions
                .write_cache_ptr(cache_base + BINARY_OP_EXTEND_EXTERNAL_CACHE_OFFSET, ptr);
        }
    }

    #[inline]
    fn read_cached_binary_op_extend_descr(
        &self,
        cache_base: usize,
    ) -> Option<&'static BinaryOpExtendSpecializationDescr> {
        let ptr = self
            .code
            .instructions
            .read_cache_ptr(cache_base + BINARY_OP_EXTEND_EXTERNAL_CACHE_OFFSET);
        if ptr == 0 {
            return None;
        }
        // SAFETY: We only store pointers to entries in `BINARY_OP_EXTEND_DESCRIPTORS`.
        Some(unsafe { &*(ptr as *const BinaryOpExtendSpecializationDescr) })
    }

    #[inline]
    fn binary_op_extended_specialization(
        &self,
        op: bytecode::BinaryOperator,
        lhs: &PyObject,
        rhs: &PyObject,
        vm: &VirtualMachine,
    ) -> Option<&'static BinaryOpExtendSpecializationDescr> {
        BINARY_OP_EXTEND_DESCRIPTORS
            .iter()
            .find(|d| d.oparg == op && (d.guard)(lhs, rhs, vm))
    }

    fn load_attr(&mut self, vm: &VirtualMachine, oparg: LoadAttr) -> FrameResult {
        self.adaptive(|s, ii, cb| s.specialize_load_attr(vm, oparg, ii, cb));
        self.load_attr_slow(vm, oparg)
    }

    fn specialize_load_attr(
        &mut self,
        _vm: &VirtualMachine,
        oparg: LoadAttr,
        instr_idx: usize,
        cache_base: usize,
    ) {
        // Pre-check: bail if already specialized by another thread
        if !matches!(
            self.code.instructions.read_op(instr_idx),
            Instruction::LoadAttr { .. }
        ) {
            return;
        }
        let obj = self.top_value();
        let cls = obj.class();

        // Check if this is a type object (class attribute access)
        if obj.downcast_ref::<PyType>().is_some() {
            self.specialize_class_load_attr(_vm, oparg, instr_idx, cache_base);
            return;
        }

        // Only specialize if getattro is the default (PyBaseObject::getattro)
        let is_default_getattro = cls
            .slots
            .getattro
            .load()
            .is_some_and(|f| f as usize == PyBaseObject::getattro as *const () as usize);
        if !is_default_getattro {
            let mut type_version = cls.tp_version_tag.load(Acquire);
            if type_version == 0 {
                type_version = cls.assign_version_tag();
            }
            if type_version != 0
                && !oparg.is_method()
                && !self.specialization_eval_frame_active(_vm)
                && cls.get_attr(identifier!(_vm, __getattr__)).is_none()
                && let Some(getattribute) = cls.get_attr(identifier!(_vm, __getattribute__))
                && let Some(func) = getattribute.downcast_ref_if_exact::<PyFunction>(_vm)
                && func.can_specialize_call(2)
            {
                let func_version = func.get_version_for_current_state();
                if func_version != 0 {
                    let func_ptr = &*getattribute as *const PyObject as usize;
                    unsafe {
                        self.code
                            .instructions
                            .write_cache_u32(cache_base + 3, func_version);
                        self.write_cached_descriptor(cache_base, type_version, func_ptr);
                    }
                    self.specialize_at(
                        instr_idx,
                        cache_base,
                        Instruction::LoadAttrGetattributeOverridden,
                    );
                    return;
                }
            }
            unsafe {
                self.code.instructions.write_adaptive_counter(
                    cache_base,
                    bytecode::adaptive_counter_backoff(
                        self.code.instructions.read_adaptive_counter(cache_base),
                    ),
                );
            }
            return;
        }

        // Get or assign type version
        let mut type_version = cls.tp_version_tag.load(Acquire);
        if type_version == 0 {
            type_version = cls.assign_version_tag();
        }
        if type_version == 0 {
            // Version counter overflow — backoff to avoid re-attempting every execution
            unsafe {
                self.code.instructions.write_adaptive_counter(
                    cache_base,
                    bytecode::adaptive_counter_backoff(
                        self.code.instructions.read_adaptive_counter(cache_base),
                    ),
                );
            }
            return;
        }

        let attr_name = self.code.names[oparg.name_idx() as usize];

        // Match CPython: only specialize module attribute loads when the
        // current module dict has no __getattr__ override and the attribute is
        // already present.
        if let Some(module) = obj.downcast_ref_if_exact::<PyModule>(_vm) {
            let module_dict = module.dict();
            match (
                module_dict.get_item_opt(identifier!(_vm, __getattr__), _vm),
                module_dict.get_item_opt(attr_name, _vm),
            ) {
                (Ok(None), Ok(Some(_))) => {
                    unsafe {
                        self.code
                            .instructions
                            .write_cache_u32(cache_base + 1, type_version);
                    }
                    self.specialize_at(instr_idx, cache_base, Instruction::LoadAttrModule);
                }
                (Ok(_), Ok(_)) => self.cooldown_adaptive_at(cache_base),
                _ => unsafe {
                    self.code.instructions.write_adaptive_counter(
                        cache_base,
                        bytecode::adaptive_counter_backoff(
                            self.code.instructions.read_adaptive_counter(cache_base),
                        ),
                    );
                },
            }
            return;
        }

        // Look up attr in class via MRO
        let cls_attr = cls.get_attr(attr_name);
        let class_has_dict = cls.slots.flags.has_feature(PyTypeFlags::HAS_DICT);

        if oparg.is_method() {
            // Method specialization
            if let Some(ref descr) = cls_attr
                && descr
                    .class()
                    .slots
                    .flags
                    .has_feature(PyTypeFlags::METHOD_DESCRIPTOR)
            {
                let descr_ptr = &**descr as *const PyObject as usize;
                unsafe {
                    self.write_cached_descriptor(cache_base, type_version, descr_ptr);
                }

                let new_op = if !class_has_dict {
                    Instruction::LoadAttrMethodNoDict
                } else if obj.dict().is_none() {
                    Instruction::LoadAttrMethodLazyDict
                } else {
                    Instruction::LoadAttrMethodWithValues
                };
                self.specialize_at(instr_idx, cache_base, new_op);
                return;
            }
            // Can't specialize this method call
            unsafe {
                self.code.instructions.write_adaptive_counter(
                    cache_base,
                    bytecode::adaptive_counter_backoff(
                        self.code.instructions.read_adaptive_counter(cache_base),
                    ),
                );
            }
        } else {
            // Regular attribute access
            let has_data_descr = cls_attr.as_ref().is_some_and(|descr| {
                let descr_cls = descr.class();
                descr_cls.slots.descr_get.load().is_some()
                    && descr_cls.slots.descr_set.load().is_some()
            });
            let has_descr_get = cls_attr
                .as_ref()
                .is_some_and(|descr| descr.class().slots.descr_get.load().is_some());

            if has_data_descr {
                // Check for member descriptor (slot access)
                if let Some(ref descr) = cls_attr
                    && let Some(member_descr) = descr.downcast_ref::<PyMemberDescriptor>()
                    && let MemberGetter::Offset(offset) = member_descr.member.getter
                {
                    unsafe {
                        self.code
                            .instructions
                            .write_cache_u32(cache_base + 1, type_version);
                        self.code
                            .instructions
                            .write_cache_u32(cache_base + 3, offset as u32);
                    }
                    self.specialize_at(instr_idx, cache_base, Instruction::LoadAttrSlot);
                } else if let Some(ref descr) = cls_attr
                    && let Some(prop) = descr.downcast_ref::<PyProperty>()
                    && let Some(fget) = prop.get_fget()
                    && let Some(func) = fget.downcast_ref_if_exact::<PyFunction>(_vm)
                    && func.can_specialize_call(1)
                    && !self.specialization_eval_frame_active(_vm)
                {
                    // Property specialization caches fget directly.
                    let fget_ptr = &*fget as *const PyObject as usize;
                    unsafe {
                        self.write_cached_descriptor(cache_base, type_version, fget_ptr);
                    }
                    self.specialize_at(instr_idx, cache_base, Instruction::LoadAttrProperty);
                } else {
                    unsafe {
                        self.code.instructions.write_adaptive_counter(
                            cache_base,
                            bytecode::adaptive_counter_backoff(
                                self.code.instructions.read_adaptive_counter(cache_base),
                            ),
                        );
                    }
                }
            } else if has_descr_get {
                // Non-data descriptor with __get__ — can't specialize
                unsafe {
                    self.code.instructions.write_adaptive_counter(
                        cache_base,
                        bytecode::adaptive_counter_backoff(
                            self.code.instructions.read_adaptive_counter(cache_base),
                        ),
                    );
                }
            } else if class_has_dict {
                if let Some(ref descr) = cls_attr {
                    // Plain class attr + class supports dict — check dict first, fallback
                    let descr_ptr = &**descr as *const PyObject as usize;
                    unsafe {
                        self.write_cached_descriptor(cache_base, type_version, descr_ptr);
                    }
                    self.specialize_at(
                        instr_idx,
                        cache_base,
                        Instruction::LoadAttrNondescriptorWithValues,
                    );
                } else {
                    // Match CPython ABSENT/no-shadow behavior: if the
                    // attribute is missing on both the class and the current
                    // instance, keep the generic opcode and just enter
                    // cooldown instead of specializing a repeated miss path.
                    let has_instance_attr = if let Some(dict) = obj.dict() {
                        match dict.get_item_opt(attr_name, _vm) {
                            Ok(Some(_)) => true,
                            Ok(None) => false,
                            Err(_) => {
                                unsafe {
                                    self.code.instructions.write_adaptive_counter(
                                        cache_base,
                                        bytecode::adaptive_counter_backoff(
                                            self.code
                                                .instructions
                                                .read_adaptive_counter(cache_base),
                                        ),
                                    );
                                }
                                return;
                            }
                        }
                    } else {
                        false
                    };
                    if has_instance_attr {
                        unsafe {
                            self.code
                                .instructions
                                .write_cache_u32(cache_base + 1, type_version);
                        }
                        self.specialize_at(instr_idx, cache_base, Instruction::LoadAttrWithHint);
                    } else {
                        self.cooldown_adaptive_at(cache_base);
                    }
                }
            } else if let Some(ref descr) = cls_attr {
                // No dict support, plain class attr — cache directly
                let descr_ptr = &**descr as *const PyObject as usize;
                unsafe {
                    self.write_cached_descriptor(cache_base, type_version, descr_ptr);
                }
                self.specialize_at(
                    instr_idx,
                    cache_base,
                    Instruction::LoadAttrNondescriptorNoDict,
                );
            } else {
                // No dict and no class attr: repeated miss path, so cooldown.
                self.cooldown_adaptive_at(cache_base);
            }
        }
    }

    fn specialize_class_load_attr(
        &mut self,
        _vm: &VirtualMachine,
        oparg: LoadAttr,
        instr_idx: usize,
        cache_base: usize,
    ) {
        let obj = self.top_value();
        let owner_type = obj.downcast_ref::<PyType>().unwrap();

        // Get or assign type version for the type object itself
        let mut type_version = owner_type.tp_version_tag.load(Acquire);
        if type_version == 0 {
            type_version = owner_type.assign_version_tag();
        }
        if type_version == 0 {
            unsafe {
                self.code.instructions.write_adaptive_counter(
                    cache_base,
                    bytecode::adaptive_counter_backoff(
                        self.code.instructions.read_adaptive_counter(cache_base),
                    ),
                );
            }
            return;
        }

        let attr_name = self.code.names[oparg.name_idx() as usize];

        // Check metaclass: ensure no data descriptor on metaclass for this name
        let mcl = obj.class();
        let mcl_attr = mcl.get_attr(attr_name);
        if let Some(ref attr) = mcl_attr {
            let attr_class = attr.class();
            if attr_class.slots.descr_set.load().is_some() {
                // Data descriptor on metaclass — can't specialize
                unsafe {
                    self.code.instructions.write_adaptive_counter(
                        cache_base,
                        bytecode::adaptive_counter_backoff(
                            self.code.instructions.read_adaptive_counter(cache_base),
                        ),
                    );
                }
                return;
            }
        }
        let mut metaclass_version = 0;
        if !mcl.slots.flags.has_feature(PyTypeFlags::IMMUTABLETYPE) {
            metaclass_version = mcl.tp_version_tag.load(Acquire);
            if metaclass_version == 0 {
                metaclass_version = mcl.assign_version_tag();
            }
            if metaclass_version == 0 {
                unsafe {
                    self.code.instructions.write_adaptive_counter(
                        cache_base,
                        bytecode::adaptive_counter_backoff(
                            self.code.instructions.read_adaptive_counter(cache_base),
                        ),
                    );
                }
                return;
            }
        }

        // Look up attr in the type's own MRO
        let cls_attr = owner_type.get_attr(attr_name);
        if let Some(ref descr) = cls_attr {
            let descr_class = descr.class();
            let has_descr_get = descr_class.slots.descr_get.load().is_some();
            if !has_descr_get {
                // METHOD or NON_DESCRIPTOR — can cache directly
                let descr_ptr = &**descr as *const PyObject as usize;
                let new_op = if metaclass_version == 0 {
                    Instruction::LoadAttrClass
                } else {
                    Instruction::LoadAttrClassWithMetaclassCheck
                };
                unsafe {
                    if metaclass_version == 0 {
                        self.write_cached_descriptor(cache_base, type_version, descr_ptr);
                    } else {
                        self.write_cached_descriptor_with_metaclass(
                            cache_base,
                            type_version,
                            metaclass_version,
                            descr_ptr,
                        );
                    }
                }
                self.specialize_at(instr_idx, cache_base, new_op);
                return;
            }
        }

        // Can't specialize
        unsafe {
            self.code.instructions.write_adaptive_counter(
                cache_base,
                bytecode::adaptive_counter_backoff(
                    self.code.instructions.read_adaptive_counter(cache_base),
                ),
            );
        }
    }

    fn load_attr_slow(&mut self, vm: &VirtualMachine, oparg: LoadAttr) -> FrameResult {
        let attr_name = self.code.names[oparg.name_idx() as usize];
        let parent = self.pop_value();

        if oparg.is_method() {
            // Method call: push [method, self_or_null]
            let method = PyMethod::get(parent.clone(), attr_name, vm)?;
            match method {
                PyMethod::Function { target: _, func } => {
                    self.push_value(func);
                    self.push_value(parent);
                }
                PyMethod::Attribute(val) => {
                    self.push_value(val);
                    self.push_null();
                }
            }
        } else {
            // Regular attribute access
            let obj = parent.get_attr(attr_name, vm)?;
            self.push_value(obj);
        }
        Ok(None)
    }

    fn specialize_binary_op(
        &mut self,
        vm: &VirtualMachine,
        op: bytecode::BinaryOperator,
        instr_idx: usize,
        cache_base: usize,
    ) {
        if !matches!(
            self.code.instructions.read_op(instr_idx),
            Instruction::BinaryOp { .. }
        ) {
            return;
        }
        let b = self.top_value();
        let a = self.nth_value(1);
        // `external_cache` in _PyBinaryOpCache is used only by BINARY_OP_EXTEND.
        unsafe {
            self.write_cached_binary_op_extend_descr(cache_base, None);
        }
        let mut cached_extend_descr = None;

        let new_op = match op {
            bytecode::BinaryOperator::Add => {
                if a.downcast_ref_if_exact::<PyInt>(vm).is_some()
                    && b.downcast_ref_if_exact::<PyInt>(vm).is_some()
                {
                    Some(Instruction::BinaryOpAddInt)
                } else if a.downcast_ref_if_exact::<PyFloat>(vm).is_some()
                    && b.downcast_ref_if_exact::<PyFloat>(vm).is_some()
                {
                    Some(Instruction::BinaryOpAddFloat)
                } else if a.downcast_ref_if_exact::<PyStr>(vm).is_some()
                    && b.downcast_ref_if_exact::<PyStr>(vm).is_some()
                {
                    if self
                        .binary_op_inplace_unicode_target_local(cache_base, a)
                        .is_some()
                    {
                        Some(Instruction::BinaryOpInplaceAddUnicode)
                    } else {
                        Some(Instruction::BinaryOpAddUnicode)
                    }
                } else if let Some(descr) = self.binary_op_extended_specialization(op, a, b, vm) {
                    cached_extend_descr = Some(descr);
                    Some(Instruction::BinaryOpExtend)
                } else {
                    None
                }
            }
            bytecode::BinaryOperator::Subtract => {
                if a.downcast_ref_if_exact::<PyInt>(vm).is_some()
                    && b.downcast_ref_if_exact::<PyInt>(vm).is_some()
                {
                    Some(Instruction::BinaryOpSubtractInt)
                } else if a.downcast_ref_if_exact::<PyFloat>(vm).is_some()
                    && b.downcast_ref_if_exact::<PyFloat>(vm).is_some()
                {
                    Some(Instruction::BinaryOpSubtractFloat)
                } else if let Some(descr) = self.binary_op_extended_specialization(op, a, b, vm) {
                    cached_extend_descr = Some(descr);
                    Some(Instruction::BinaryOpExtend)
                } else {
                    None
                }
            }
            bytecode::BinaryOperator::Multiply => {
                if a.downcast_ref_if_exact::<PyInt>(vm).is_some()
                    && b.downcast_ref_if_exact::<PyInt>(vm).is_some()
                {
                    Some(Instruction::BinaryOpMultiplyInt)
                } else if a.downcast_ref_if_exact::<PyFloat>(vm).is_some()
                    && b.downcast_ref_if_exact::<PyFloat>(vm).is_some()
                {
                    Some(Instruction::BinaryOpMultiplyFloat)
                } else if let Some(descr) = self.binary_op_extended_specialization(op, a, b, vm) {
                    cached_extend_descr = Some(descr);
                    Some(Instruction::BinaryOpExtend)
                } else {
                    None
                }
            }
            bytecode::BinaryOperator::TrueDivide => {
                if let Some(descr) = self.binary_op_extended_specialization(op, a, b, vm) {
                    cached_extend_descr = Some(descr);
                    Some(Instruction::BinaryOpExtend)
                } else {
                    None
                }
            }
            bytecode::BinaryOperator::Subscr => {
                let b_is_nonnegative_int = b
                    .downcast_ref_if_exact::<PyInt>(vm)
                    .is_some_and(|i| specialization_nonnegative_compact_index(i, vm).is_some());
                if a.downcast_ref_if_exact::<PyList>(vm).is_some() && b_is_nonnegative_int {
                    Some(Instruction::BinaryOpSubscrListInt)
                } else if a.downcast_ref_if_exact::<PyTuple>(vm).is_some() && b_is_nonnegative_int {
                    Some(Instruction::BinaryOpSubscrTupleInt)
                } else if a.downcast_ref_if_exact::<PyDict>(vm).is_some() {
                    Some(Instruction::BinaryOpSubscrDict)
                } else if a.downcast_ref_if_exact::<PyStr>(vm).is_some() && b_is_nonnegative_int {
                    Some(Instruction::BinaryOpSubscrStrInt)
                } else if a.downcast_ref_if_exact::<PyList>(vm).is_some()
                    && b.downcast_ref::<PySlice>().is_some()
                {
                    Some(Instruction::BinaryOpSubscrListSlice)
                } else {
                    let cls = a.class();
                    if cls.slots.flags.has_feature(PyTypeFlags::HEAPTYPE)
                        && !self.specialization_eval_frame_active(vm)
                        && let Some(_getitem) = cls.get_attr(identifier!(vm, __getitem__))
                        && let Some(func) = _getitem.downcast_ref_if_exact::<PyFunction>(vm)
                        && func.can_specialize_call(2)
                    {
                        let mut type_version = cls.tp_version_tag.load(Acquire);
                        if type_version == 0 {
                            type_version = cls.assign_version_tag();
                        }
                        if type_version != 0 {
                            if cls.cache_getitem_for_specialization(
                                func.to_owned(),
                                type_version,
                                vm,
                            ) {
                                Some(Instruction::BinaryOpSubscrGetitem)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
            }
            bytecode::BinaryOperator::InplaceAdd => {
                if a.downcast_ref_if_exact::<PyStr>(vm).is_some()
                    && b.downcast_ref_if_exact::<PyStr>(vm).is_some()
                {
                    if self
                        .binary_op_inplace_unicode_target_local(cache_base, a)
                        .is_some()
                    {
                        Some(Instruction::BinaryOpInplaceAddUnicode)
                    } else {
                        Some(Instruction::BinaryOpAddUnicode)
                    }
                } else if a.downcast_ref_if_exact::<PyInt>(vm).is_some()
                    && b.downcast_ref_if_exact::<PyInt>(vm).is_some()
                {
                    Some(Instruction::BinaryOpAddInt)
                } else if a.downcast_ref_if_exact::<PyFloat>(vm).is_some()
                    && b.downcast_ref_if_exact::<PyFloat>(vm).is_some()
                {
                    Some(Instruction::BinaryOpAddFloat)
                } else {
                    None
                }
            }
            bytecode::BinaryOperator::InplaceSubtract => {
                if a.downcast_ref_if_exact::<PyInt>(vm).is_some()
                    && b.downcast_ref_if_exact::<PyInt>(vm).is_some()
                {
                    Some(Instruction::BinaryOpSubtractInt)
                } else if a.downcast_ref_if_exact::<PyFloat>(vm).is_some()
                    && b.downcast_ref_if_exact::<PyFloat>(vm).is_some()
                {
                    Some(Instruction::BinaryOpSubtractFloat)
                } else {
                    None
                }
            }
            bytecode::BinaryOperator::InplaceMultiply => {
                if a.downcast_ref_if_exact::<PyInt>(vm).is_some()
                    && b.downcast_ref_if_exact::<PyInt>(vm).is_some()
                {
                    Some(Instruction::BinaryOpMultiplyInt)
                } else if a.downcast_ref_if_exact::<PyFloat>(vm).is_some()
                    && b.downcast_ref_if_exact::<PyFloat>(vm).is_some()
                {
                    Some(Instruction::BinaryOpMultiplyFloat)
                } else {
                    None
                }
            }
            bytecode::BinaryOperator::And
            | bytecode::BinaryOperator::Or
            | bytecode::BinaryOperator::Xor
            | bytecode::BinaryOperator::InplaceAnd
            | bytecode::BinaryOperator::InplaceOr
            | bytecode::BinaryOperator::InplaceXor => {
                if let Some(descr) = self.binary_op_extended_specialization(op, a, b, vm) {
                    cached_extend_descr = Some(descr);
                    Some(Instruction::BinaryOpExtend)
                } else {
                    None
                }
            }
            _ => None,
        };

        if matches!(new_op, Some(Instruction::BinaryOpExtend)) {
            unsafe {
                self.write_cached_binary_op_extend_descr(cache_base, cached_extend_descr);
            }
        }
        self.commit_specialization(instr_idx, cache_base, new_op);
    }

    #[inline]
    fn binary_op_inplace_unicode_target_local(
        &self,
        cache_base: usize,
        left: &PyObject,
    ) -> Option<usize> {
        let next_idx = cache_base + Instruction::from(Opcode::BinaryOp).cache_entries();
        let unit = self.code.instructions.get(next_idx)?;
        let next_op = unit.op.to_base().unwrap_or(unit.op);
        if !matches!(next_op, Instruction::StoreFast { .. }) {
            return None;
        }
        let local_idx = usize::from(u8::from(unit.arg));
        self.localsplus
            .fastlocals()
            .get(local_idx)
            .and_then(|slot| slot.as_ref())
            .filter(|local| local.is(left))
            .map(|_| local_idx)
    }

    /// Adaptive counter: trigger specialization at zero, otherwise advance countdown.
    #[inline]
    fn adaptive(&mut self, specialize: impl FnOnce(&mut Self, usize, usize)) {
        let instr_idx = self.lasti() as usize - 1;
        let cache_base = instr_idx + 1;
        let counter = self.code.instructions.read_adaptive_counter(cache_base);
        if bytecode::adaptive_counter_triggers(counter) {
            specialize(self, instr_idx, cache_base);
        } else {
            unsafe {
                self.code.instructions.write_adaptive_counter(
                    cache_base,
                    bytecode::advance_adaptive_counter(counter),
                );
            }
        }
    }

    /// Install a specialized opcode and set adaptive cooldown bits.
    #[inline]
    fn specialize_at(&mut self, instr_idx: usize, cache_base: usize, new_op: Instruction) {
        unsafe {
            self.code
                .instructions
                .write_adaptive_counter(cache_base, ADAPTIVE_COOLDOWN_VALUE);
            self.code.instructions.replace_op(instr_idx, new_op);
        }
    }

    #[inline]
    fn cooldown_adaptive_at(&mut self, cache_base: usize) {
        unsafe {
            self.code
                .instructions
                .write_adaptive_counter(cache_base, ADAPTIVE_COOLDOWN_VALUE);
        }
    }

    /// Commit a specialization result: replace op on success, backoff on failure.
    #[inline]
    fn commit_specialization(
        &mut self,
        instr_idx: usize,
        cache_base: usize,
        new_op: Option<Instruction>,
    ) {
        if let Some(new_op) = new_op {
            self.specialize_at(instr_idx, cache_base, new_op);
        } else {
            unsafe {
                self.code.instructions.write_adaptive_counter(
                    cache_base,
                    bytecode::adaptive_counter_backoff(
                        self.code.instructions.read_adaptive_counter(cache_base),
                    ),
                );
            }
        }
    }

    /// Execute a specialized binary op on two int operands.
    /// Fallback to generic binary op if either operand is not an exact int.
    #[inline]
    fn execute_binary_op_int(
        &mut self,
        vm: &VirtualMachine,
        op: impl FnOnce(&BigInt, &BigInt) -> BigInt,
        deopt_op: bytecode::BinaryOperator,
    ) -> FrameResult {
        let b = self.top_value();
        let a = self.nth_value(1);
        if let (Some(a_int), Some(b_int)) = (
            a.downcast_ref_if_exact::<PyInt>(vm),
            b.downcast_ref_if_exact::<PyInt>(vm),
        ) {
            let result = op(a_int.as_bigint(), b_int.as_bigint());
            self.pop_value();
            self.pop_value();
            self.push_value(vm.ctx.new_bigint(&result).into());
            Ok(None)
        } else {
            self.execute_bin_op(vm, deopt_op)
        }
    }

    /// Execute a specialized binary op on two float operands.
    /// Fallback to generic binary op if either operand is not an exact float.
    #[inline]
    fn execute_binary_op_float(
        &mut self,
        vm: &VirtualMachine,
        op: impl FnOnce(f64, f64) -> f64,
        deopt_op: bytecode::BinaryOperator,
    ) -> FrameResult {
        let b = self.top_value();
        let a = self.nth_value(1);
        if let (Some(a_f), Some(b_f)) = (
            a.downcast_ref_if_exact::<PyFloat>(vm),
            b.downcast_ref_if_exact::<PyFloat>(vm),
        ) {
            let result = op(a_f.to_f64(), b_f.to_f64());
            self.pop_value();
            self.pop_value();
            self.push_value(vm.ctx.new_float(result).into());
            Ok(None)
        } else {
            self.execute_bin_op(vm, deopt_op)
        }
    }

    fn specialize_call(
        &mut self,
        vm: &VirtualMachine,
        nargs: u32,
        instr_idx: usize,
        cache_base: usize,
    ) {
        if !matches!(
            self.code.instructions.read_op(instr_idx),
            Instruction::Call { .. }
        ) {
            return;
        }
        // Stack: [callable, self_or_null, arg1, ..., argN]
        // callable is at position nargs + 1 from top
        // self_or_null is at position nargs from top
        let stack_len = self.localsplus.stack_len();
        let self_or_null_is_some = self
            .localsplus
            .stack_index(stack_len - nargs as usize - 1)
            .is_some();
        let callable = self.nth_value(nargs + 1);

        if let Some(func) = callable.downcast_ref_if_exact::<PyFunction>(vm) {
            if self.specialization_eval_frame_active(vm) {
                unsafe {
                    self.code.instructions.write_adaptive_counter(
                        cache_base,
                        bytecode::adaptive_counter_backoff(
                            self.code.instructions.read_adaptive_counter(cache_base),
                        ),
                    );
                }
                return;
            }
            if !func.is_optimized_for_call_specialization() {
                unsafe {
                    self.code.instructions.write_adaptive_counter(
                        cache_base,
                        bytecode::adaptive_counter_backoff(
                            self.code.instructions.read_adaptive_counter(cache_base),
                        ),
                    );
                }
                return;
            }
            let version = func.get_version_for_current_state();
            if version == 0 {
                unsafe {
                    self.code.instructions.write_adaptive_counter(
                        cache_base,
                        bytecode::adaptive_counter_backoff(
                            self.code.instructions.read_adaptive_counter(cache_base),
                        ),
                    );
                }
                return;
            }

            let effective_nargs = if self_or_null_is_some {
                nargs + 1
            } else {
                nargs
            };

            let new_op = if func.can_specialize_call(effective_nargs) {
                Instruction::CallPyExactArgs
            } else {
                Instruction::CallPyGeneral
            };
            unsafe {
                self.code
                    .instructions
                    .write_cache_u32(cache_base + 1, version);
            }
            self.specialize_at(instr_idx, cache_base, new_op);
            return;
        }

        // Bound Python method object (`method`) specialization.
        if !self_or_null_is_some
            && let Some(bound_method) = callable.downcast_ref_if_exact::<PyBoundMethod>(vm)
        {
            if let Some(func) = bound_method
                .function_obj()
                .downcast_ref_if_exact::<PyFunction>(vm)
            {
                if self.specialization_eval_frame_active(vm) {
                    unsafe {
                        self.code.instructions.write_adaptive_counter(
                            cache_base,
                            bytecode::adaptive_counter_backoff(
                                self.code.instructions.read_adaptive_counter(cache_base),
                            ),
                        );
                    }
                    return;
                }
                if !func.is_optimized_for_call_specialization() {
                    unsafe {
                        self.code.instructions.write_adaptive_counter(
                            cache_base,
                            bytecode::adaptive_counter_backoff(
                                self.code.instructions.read_adaptive_counter(cache_base),
                            ),
                        );
                    }
                    return;
                }
                let version = func.get_version_for_current_state();
                if version == 0 {
                    unsafe {
                        self.code.instructions.write_adaptive_counter(
                            cache_base,
                            bytecode::adaptive_counter_backoff(
                                self.code.instructions.read_adaptive_counter(cache_base),
                            ),
                        );
                    }
                    return;
                }

                let new_op = if func.can_specialize_call(nargs + 1) {
                    Instruction::CallBoundMethodExactArgs
                } else {
                    Instruction::CallBoundMethodGeneral
                };
                unsafe {
                    self.code
                        .instructions
                        .write_cache_u32(cache_base + 1, version);
                }
                self.specialize_at(instr_idx, cache_base, new_op);
            } else {
                // Match CPython: bound methods wrapping non-Python callables
                // are not specialized as CALL_NON_PY_GENERAL.
                unsafe {
                    self.code.instructions.write_adaptive_counter(
                        cache_base,
                        bytecode::adaptive_counter_backoff(
                            self.code.instructions.read_adaptive_counter(cache_base),
                        ),
                    );
                }
            }
            return;
        }

        // Try to specialize method descriptor calls
        if let Some(descr) = callable.downcast_ref_if_exact::<PyMethodDescriptor>(vm) {
            let call_cache_entries = Instruction::CallListAppend.cache_entries();
            let next_idx = cache_base + call_cache_entries;
            let next_is_pop_top = if next_idx < self.code.instructions.len() {
                let next_op = self.code.instructions.read_op(next_idx);
                matches!(next_op.to_base().unwrap_or(next_op), Instruction::PopTop)
            } else {
                false
            };

            let call_conv = descr.method.flags
                & (PyMethodFlags::VARARGS
                    | PyMethodFlags::FASTCALL
                    | PyMethodFlags::NOARGS
                    | PyMethodFlags::O
                    | PyMethodFlags::KEYWORDS);
            let total_nargs = nargs + u32::from(self_or_null_is_some);

            let new_op = if call_conv == PyMethodFlags::NOARGS {
                if total_nargs != 1 {
                    unsafe {
                        self.code.instructions.write_adaptive_counter(
                            cache_base,
                            bytecode::adaptive_counter_backoff(
                                self.code.instructions.read_adaptive_counter(cache_base),
                            ),
                        );
                    }
                    return;
                }
                Instruction::CallMethodDescriptorNoargs
            } else if call_conv == PyMethodFlags::O {
                if total_nargs != 2 {
                    unsafe {
                        self.code.instructions.write_adaptive_counter(
                            cache_base,
                            bytecode::adaptive_counter_backoff(
                                self.code.instructions.read_adaptive_counter(cache_base),
                            ),
                        );
                    }
                    return;
                }
                if self_or_null_is_some
                    && nargs == 1
                    && next_is_pop_top
                    && vm
                        .callable_cache
                        .list_append
                        .as_ref()
                        .is_some_and(|list_append| callable.is(list_append))
                {
                    Instruction::CallListAppend
                } else {
                    Instruction::CallMethodDescriptorO
                }
            } else if call_conv == PyMethodFlags::FASTCALL {
                Instruction::CallMethodDescriptorFast
            } else if call_conv == (PyMethodFlags::FASTCALL | PyMethodFlags::KEYWORDS) {
                Instruction::CallMethodDescriptorFastWithKeywords
            } else {
                Instruction::CallNonPyGeneral
            };
            self.specialize_at(instr_idx, cache_base, new_op);
            return;
        }

        // Try to specialize builtin calls
        if let Some(native) = callable.downcast_ref_if_exact::<PyNativeFunction>(vm) {
            let effective_nargs = nargs + u32::from(self_or_null_is_some);
            let call_conv = native.value.flags
                & (PyMethodFlags::VARARGS
                    | PyMethodFlags::FASTCALL
                    | PyMethodFlags::NOARGS
                    | PyMethodFlags::O
                    | PyMethodFlags::KEYWORDS);
            let new_op = if call_conv == PyMethodFlags::O {
                if effective_nargs != 1 {
                    unsafe {
                        self.code.instructions.write_adaptive_counter(
                            cache_base,
                            bytecode::adaptive_counter_backoff(
                                self.code.instructions.read_adaptive_counter(cache_base),
                            ),
                        );
                    }
                    return;
                }
                if native.zelf.is_none()
                    && nargs == 1
                    && vm
                        .callable_cache
                        .len
                        .as_ref()
                        .is_some_and(|len_callable| callable.is(len_callable))
                {
                    Instruction::CallLen
                } else {
                    Instruction::CallBuiltinO
                }
            } else if call_conv == PyMethodFlags::FASTCALL {
                if native.zelf.is_none()
                    && effective_nargs == 2
                    && vm
                        .callable_cache
                        .isinstance
                        .as_ref()
                        .is_some_and(|isinstance_callable| callable.is(isinstance_callable))
                {
                    Instruction::CallIsinstance
                } else {
                    Instruction::CallBuiltinFast
                }
            } else if call_conv == (PyMethodFlags::FASTCALL | PyMethodFlags::KEYWORDS) {
                Instruction::CallBuiltinFastWithKeywords
            } else {
                Instruction::CallNonPyGeneral
            };
            self.specialize_at(instr_idx, cache_base, new_op);
            return;
        }

        // type/str/tuple(x) and class-call specializations
        if let Some(cls) = callable.downcast_ref::<PyType>() {
            if cls.slots.flags.has_feature(PyTypeFlags::IMMUTABLETYPE) {
                if !self_or_null_is_some && nargs == 1 {
                    let new_op = if callable.is(&vm.ctx.types.type_type.as_object()) {
                        Some(Instruction::CallType1)
                    } else if callable.is(&vm.ctx.types.str_type.as_object()) {
                        Some(Instruction::CallStr1)
                    } else if callable.is(&vm.ctx.types.tuple_type.as_object()) {
                        Some(Instruction::CallTuple1)
                    } else {
                        None
                    };
                    if let Some(new_op) = new_op {
                        self.specialize_at(instr_idx, cache_base, new_op);
                        return;
                    }
                }
                if cls.slots.vectorcall.load().is_some() {
                    self.specialize_at(instr_idx, cache_base, Instruction::CallBuiltinClass);
                    return;
                }
                self.specialize_at(instr_idx, cache_base, Instruction::CallNonPyGeneral);
                return;
            }

            // CPython only considers CALL_ALLOC_AND_ENTER_INIT for types whose
            // metaclass is exactly `type`.
            if !callable.class().is(vm.ctx.types.type_type) {
                self.specialize_at(instr_idx, cache_base, Instruction::CallNonPyGeneral);
                return;
            }

            // CallAllocAndEnterInit: heap type with default __new__
            if !self_or_null_is_some && cls.slots.flags.has_feature(PyTypeFlags::HEAPTYPE) {
                let object_new = vm.ctx.types.object_type.slots.new.load();
                let cls_new = cls.slots.new.load();
                let object_alloc = vm.ctx.types.object_type.slots.alloc.load();
                let cls_alloc = cls.slots.alloc.load();
                if let (Some(cls_new_fn), Some(obj_new_fn), Some(cls_alloc_fn), Some(obj_alloc_fn)) =
                    (cls_new, object_new, cls_alloc, object_alloc)
                    && cls_new_fn as usize == obj_new_fn as usize
                    && cls_alloc_fn as usize == obj_alloc_fn as usize
                {
                    let init = cls.get_attr(identifier!(vm, __init__));
                    let mut version = cls.tp_version_tag.load(Acquire);
                    if version == 0 {
                        version = cls.assign_version_tag();
                    }
                    if version == 0 {
                        unsafe {
                            self.code.instructions.write_adaptive_counter(
                                cache_base,
                                bytecode::adaptive_counter_backoff(
                                    self.code.instructions.read_adaptive_counter(cache_base),
                                ),
                            );
                        }
                        return;
                    }
                    if let Some(init) = init
                        && let Some(init_func) = init.downcast_ref_if_exact::<PyFunction>(vm)
                        && init_func.is_simple_for_call_specialization()
                        && cls.cache_init_for_specialization(init_func.to_owned(), version, vm)
                    {
                        unsafe {
                            self.code
                                .instructions
                                .write_cache_u32(cache_base + 1, version);
                        }
                        self.specialize_at(
                            instr_idx,
                            cache_base,
                            Instruction::CallAllocAndEnterInit,
                        );
                        return;
                    }
                }
            }
            self.specialize_at(instr_idx, cache_base, Instruction::CallNonPyGeneral);
            return;
        }

        // General fallback: specialized non-Python callable path
        self.specialize_at(instr_idx, cache_base, Instruction::CallNonPyGeneral);
    }

    fn specialize_call_kw(
        &mut self,
        vm: &VirtualMachine,
        nargs: u32,
        instr_idx: usize,
        cache_base: usize,
    ) {
        if !matches!(
            self.code.instructions.read_op(instr_idx),
            Instruction::CallKw { .. }
        ) {
            return;
        }
        // Stack: [callable, self_or_null, arg1, ..., argN, kwarg_names]
        // callable is at position nargs + 2 from top
        let stack_len = self.localsplus.stack_len();
        let self_or_null_is_some = self
            .localsplus
            .stack_index(stack_len - nargs as usize - 2)
            .is_some();
        let callable = self.nth_value(nargs + 2);

        if let Some(func) = callable.downcast_ref_if_exact::<PyFunction>(vm) {
            if self.specialization_eval_frame_active(vm) {
                unsafe {
                    self.code.instructions.write_adaptive_counter(
                        cache_base,
                        bytecode::adaptive_counter_backoff(
                            self.code.instructions.read_adaptive_counter(cache_base),
                        ),
                    );
                }
                return;
            }
            if !func.is_optimized_for_call_specialization() {
                unsafe {
                    self.code.instructions.write_adaptive_counter(
                        cache_base,
                        bytecode::adaptive_counter_backoff(
                            self.code.instructions.read_adaptive_counter(cache_base),
                        ),
                    );
                }
                return;
            }
            let version = func.get_version_for_current_state();
            if version == 0 {
                unsafe {
                    self.code.instructions.write_adaptive_counter(
                        cache_base,
                        bytecode::adaptive_counter_backoff(
                            self.code.instructions.read_adaptive_counter(cache_base),
                        ),
                    );
                }
                return;
            }

            unsafe {
                self.code
                    .instructions
                    .write_cache_u32(cache_base + 1, version);
            }
            self.specialize_at(instr_idx, cache_base, Instruction::CallKwPy);
            return;
        }

        if !self_or_null_is_some
            && let Some(bound_method) = callable.downcast_ref_if_exact::<PyBoundMethod>(vm)
        {
            if let Some(func) = bound_method
                .function_obj()
                .downcast_ref_if_exact::<PyFunction>(vm)
            {
                if self.specialization_eval_frame_active(vm) {
                    unsafe {
                        self.code.instructions.write_adaptive_counter(
                            cache_base,
                            bytecode::adaptive_counter_backoff(
                                self.code.instructions.read_adaptive_counter(cache_base),
                            ),
                        );
                    }
                    return;
                }
                if !func.is_optimized_for_call_specialization() {
                    unsafe {
                        self.code.instructions.write_adaptive_counter(
                            cache_base,
                            bytecode::adaptive_counter_backoff(
                                self.code.instructions.read_adaptive_counter(cache_base),
                            ),
                        );
                    }
                    return;
                }
                let version = func.get_version_for_current_state();
                if version == 0 {
                    unsafe {
                        self.code.instructions.write_adaptive_counter(
                            cache_base,
                            bytecode::adaptive_counter_backoff(
                                self.code.instructions.read_adaptive_counter(cache_base),
                            ),
                        );
                    }
                    return;
                }
                unsafe {
                    self.code
                        .instructions
                        .write_cache_u32(cache_base + 1, version);
                }
                self.specialize_at(instr_idx, cache_base, Instruction::CallKwBoundMethod);
            } else {
                // Match CPython: bound methods wrapping non-Python callables
                // are not specialized as CALL_KW_NON_PY.
                unsafe {
                    self.code.instructions.write_adaptive_counter(
                        cache_base,
                        bytecode::adaptive_counter_backoff(
                            self.code.instructions.read_adaptive_counter(cache_base),
                        ),
                    );
                }
            }
            return;
        }

        // General fallback: specialized non-Python callable path
        self.specialize_at(instr_idx, cache_base, Instruction::CallKwNonPy);
    }

    fn specialize_send(&mut self, vm: &VirtualMachine, instr_idx: usize, cache_base: usize) {
        if !matches!(
            self.code.instructions.read_op(instr_idx),
            Instruction::Send { .. }
        ) {
            return;
        }
        // Stack: [receiver, val] — receiver is at position 1
        let receiver = self.nth_value(1);
        let is_exact_gen_or_coro = receiver.downcast_ref_if_exact::<PyGenerator>(vm).is_some()
            || receiver.downcast_ref_if_exact::<PyCoroutine>(vm).is_some();
        if is_exact_gen_or_coro && !self.specialization_eval_frame_active(vm) {
            self.specialize_at(instr_idx, cache_base, Instruction::SendGen);
        } else {
            unsafe {
                self.code.instructions.write_adaptive_counter(
                    cache_base,
                    bytecode::adaptive_counter_backoff(
                        self.code.instructions.read_adaptive_counter(cache_base),
                    ),
                );
            }
        }
    }

    fn specialize_load_super_attr(
        &mut self,
        vm: &VirtualMachine,
        oparg: LoadSuperAttr,
        instr_idx: usize,
        cache_base: usize,
    ) {
        if !matches!(
            self.code.instructions.read_op(instr_idx),
            Instruction::LoadSuperAttr { .. }
        ) {
            return;
        }
        // Stack: [global_super, class, self]
        let global_super = self.nth_value(2);
        let class = self.nth_value(1);

        if !global_super.is(&vm.ctx.types.super_type.as_object())
            || class.downcast_ref::<PyType>().is_none()
        {
            unsafe {
                self.code.instructions.write_adaptive_counter(
                    cache_base,
                    bytecode::adaptive_counter_backoff(
                        self.code.instructions.read_adaptive_counter(cache_base),
                    ),
                );
            }
            return;
        }

        let new_op = if oparg.is_load_method() {
            Instruction::LoadSuperAttrMethod
        } else {
            Instruction::LoadSuperAttrAttr
        };
        self.specialize_at(instr_idx, cache_base, new_op);
    }

    fn specialize_compare_op(
        &mut self,
        vm: &VirtualMachine,
        op: bytecode::ComparisonOperator,
        instr_idx: usize,
        cache_base: usize,
    ) {
        if !matches!(
            self.code.instructions.read_op(instr_idx),
            Instruction::CompareOp { .. }
        ) {
            return;
        }
        let b = self.top_value();
        let a = self.nth_value(1);

        let new_op = if let (Some(a_int), Some(b_int)) = (
            a.downcast_ref_if_exact::<PyInt>(vm),
            b.downcast_ref_if_exact::<PyInt>(vm),
        ) {
            if specialization_compact_int_value(a_int, vm).is_some()
                && specialization_compact_int_value(b_int, vm).is_some()
            {
                Some(Instruction::CompareOpInt)
            } else {
                None
            }
        } else if a.downcast_ref_if_exact::<PyFloat>(vm).is_some()
            && b.downcast_ref_if_exact::<PyFloat>(vm).is_some()
        {
            Some(Instruction::CompareOpFloat)
        } else if a.downcast_ref_if_exact::<PyStr>(vm).is_some()
            && b.downcast_ref_if_exact::<PyStr>(vm).is_some()
            && (op == bytecode::ComparisonOperator::Equal
                || op == bytecode::ComparisonOperator::NotEqual)
        {
            Some(Instruction::CompareOpStr)
        } else {
            None
        };

        self.commit_specialization(instr_idx, cache_base, new_op);
    }

    /// Recover the ComparisonOperator from the instruction arg byte.
    /// `replace_op` preserves the arg byte, so the original op remains accessible.
    fn compare_op_from_arg(&self, arg: bytecode::OpArg) -> PyComparisonOp {
        bytecode::ComparisonOperator::try_from(u32::from(arg))
            .unwrap_or(bytecode::ComparisonOperator::Equal)
            .into()
    }

    /// Recover the BinaryOperator from the instruction arg byte.
    /// `replace_op` preserves the arg byte, so the original op remains accessible.
    fn binary_op_from_arg(&self, arg: bytecode::OpArg) -> bytecode::BinaryOperator {
        bytecode::BinaryOperator::try_from(u32::from(arg)).unwrap_or(bytecode::BinaryOperator::Add)
    }

    fn specialize_to_bool(&mut self, vm: &VirtualMachine, instr_idx: usize, cache_base: usize) {
        if !matches!(
            self.code.instructions.read_op(instr_idx),
            Instruction::ToBool
        ) {
            return;
        }
        let obj = self.top_value();
        let cls = obj.class();

        let new_op = if cls.is(vm.ctx.types.bool_type) {
            Some(Instruction::ToBoolBool)
        } else if cls.is(PyInt::class(&vm.ctx)) {
            Some(Instruction::ToBoolInt)
        } else if cls.is(vm.ctx.types.none_type) {
            Some(Instruction::ToBoolNone)
        } else if cls.is(PyList::class(&vm.ctx)) {
            Some(Instruction::ToBoolList)
        } else if cls.is(PyStr::class(&vm.ctx)) {
            Some(Instruction::ToBoolStr)
        } else if cls.slots.flags.has_feature(PyTypeFlags::HEAPTYPE)
            && cls.slots.as_number.boolean.load().is_none()
            && cls.slots.as_mapping.length.load().is_none()
            && cls.slots.as_sequence.length.load().is_none()
        {
            // Cache type version for ToBoolAlwaysTrue guard
            let mut type_version = cls.tp_version_tag.load(Acquire);
            if type_version == 0 {
                type_version = cls.assign_version_tag();
            }
            if type_version != 0 {
                unsafe {
                    self.code
                        .instructions
                        .write_cache_u32(cache_base + 1, type_version);
                }
                self.specialize_at(instr_idx, cache_base, Instruction::ToBoolAlwaysTrue);
            } else {
                unsafe {
                    self.code.instructions.write_adaptive_counter(
                        cache_base,
                        bytecode::adaptive_counter_backoff(
                            self.code.instructions.read_adaptive_counter(cache_base),
                        ),
                    );
                }
            }
            return;
        } else {
            None
        };

        self.commit_specialization(instr_idx, cache_base, new_op);
    }

    fn specialize_for_iter(
        &mut self,
        vm: &VirtualMachine,
        jump_delta: u32,
        instr_idx: usize,
        cache_base: usize,
    ) {
        if !matches!(
            self.code.instructions.read_op(instr_idx),
            Instruction::ForIter { .. }
        ) {
            return;
        }
        let iter = self.top_value();

        let new_op = if iter.downcast_ref_if_exact::<PyRangeIterator>(vm).is_some() {
            Some(Instruction::ForIterRange)
        } else if iter.downcast_ref_if_exact::<PyListIterator>(vm).is_some() {
            Some(Instruction::ForIterList)
        } else if iter.downcast_ref_if_exact::<PyTupleIterator>(vm).is_some() {
            Some(Instruction::ForIterTuple)
        } else if iter.downcast_ref_if_exact::<PyGenerator>(vm).is_some()
            && jump_delta <= i16::MAX as u32
            && self.for_iter_has_end_for_shape(instr_idx, jump_delta)
            && !self.specialization_eval_frame_active(vm)
        {
            Some(Instruction::ForIterGen)
        } else {
            None
        };

        self.commit_specialization(instr_idx, cache_base, new_op);
    }

    #[inline]
    fn specialization_eval_frame_active(&self, vm: &VirtualMachine) -> bool {
        vm.use_tracing.get()
    }

    #[inline]
    fn specialization_has_datastack_space_for_func(
        &self,
        vm: &VirtualMachine,
        func: &Py<PyFunction>,
    ) -> bool {
        self.specialization_has_datastack_space_for_func_with_extra(vm, func, 0)
    }

    #[inline]
    fn specialization_has_datastack_space_for_func_with_extra(
        &self,
        vm: &VirtualMachine,
        func: &Py<PyFunction>,
        extra_bytes: usize,
    ) -> bool {
        match func.datastack_frame_size_bytes() {
            Some(frame_size) => frame_size
                .checked_add(extra_bytes)
                .is_some_and(|size| vm.datastack_has_space(size)),
            None => extra_bytes == 0 || vm.datastack_has_space(extra_bytes),
        }
    }

    #[inline]
    fn specialization_call_recursion_guard(&self, vm: &VirtualMachine) -> bool {
        self.specialization_call_recursion_guard_with_extra_frames(vm, 0)
    }

    #[inline]
    fn specialization_call_recursion_guard_with_extra_frames(
        &self,
        vm: &VirtualMachine,
        extra_frames: usize,
    ) -> bool {
        vm.current_recursion_depth()
            .saturating_add(1)
            .saturating_add(extra_frames)
            >= vm.recursion_limit.get()
    }

    #[inline]
    fn for_iter_has_end_for_shape(&self, instr_idx: usize, jump_delta: u32) -> bool {
        let target_idx = instr_idx
            + 1
            + Instruction::from(Opcode::ForIter).cache_entries()
            + jump_delta as usize;
        self.code.instructions.get(target_idx).is_some_and(|unit| {
            matches!(
                unit.op,
                Instruction::EndFor | Instruction::InstrumentedEndFor
            )
        })
    }

    /// Handle iterator exhaustion in specialized FOR_ITER handlers.
    /// Skips END_FOR if present at target and jumps.
    fn for_iter_jump_on_exhausted(&mut self, target: bytecode::Label) {
        let target_idx = target.as_usize();
        let jump_target = if let Some(unit) = self.code.instructions.get(target_idx) {
            if matches!(
                unit.op,
                bytecode::Instruction::EndFor | bytecode::Instruction::InstrumentedEndFor
            ) {
                bytecode::Label::from_u32(target.as_u32() + 1)
            } else {
                target
            }
        } else {
            target
        };
        self.jump(jump_target);
    }

    fn specialize_load_global(
        &mut self,
        vm: &VirtualMachine,
        oparg: u32,
        instr_idx: usize,
        cache_base: usize,
    ) {
        if !matches!(
            self.code.instructions.read_op(instr_idx),
            Instruction::LoadGlobal { .. }
        ) {
            return;
        }
        let name = self.code.names[(oparg >> 1) as usize];
        let Ok(globals_version) = u16::try_from(self.globals.version()) else {
            unsafe {
                self.code.instructions.write_adaptive_counter(
                    cache_base,
                    bytecode::adaptive_counter_backoff(
                        self.code.instructions.read_adaptive_counter(cache_base),
                    ),
                );
            }
            return;
        };

        if let Ok(Some(globals_hint)) = self.globals.hint_for_key(name, vm) {
            unsafe {
                self.code
                    .instructions
                    .write_cache_u16(cache_base + 1, globals_version);
                self.code.instructions.write_cache_u16(cache_base + 2, 0);
                self.code
                    .instructions
                    .write_cache_u16(cache_base + 3, globals_hint);
            }
            self.specialize_at(instr_idx, cache_base, Instruction::LoadGlobalModule);
            return;
        }

        if let Some(builtins_dict) = self.builtins.downcast_ref_if_exact::<PyDict>(vm)
            && let Ok(Some(builtins_hint)) = builtins_dict.hint_for_key(name, vm)
            && let Ok(builtins_version) = u16::try_from(builtins_dict.version())
        {
            unsafe {
                self.code
                    .instructions
                    .write_cache_u16(cache_base + 1, globals_version);
                self.code
                    .instructions
                    .write_cache_u16(cache_base + 2, builtins_version);
                self.code
                    .instructions
                    .write_cache_u16(cache_base + 3, builtins_hint);
            }
            self.specialize_at(instr_idx, cache_base, Instruction::LoadGlobalBuiltin);
            return;
        }

        unsafe {
            self.code.instructions.write_adaptive_counter(
                cache_base,
                bytecode::adaptive_counter_backoff(
                    self.code.instructions.read_adaptive_counter(cache_base),
                ),
            );
        }
    }

    fn specialize_store_subscr(
        &mut self,
        vm: &VirtualMachine,
        instr_idx: usize,
        cache_base: usize,
    ) {
        if !matches!(
            self.code.instructions.read_op(instr_idx),
            Instruction::StoreSubscr
        ) {
            return;
        }
        // Stack: [value, obj, idx] — obj is TOS-1
        let obj = self.nth_value(1);
        let idx = self.top_value();

        let new_op = if let (Some(list), Some(int_idx)) = (
            obj.downcast_ref_if_exact::<PyList>(vm),
            idx.downcast_ref_if_exact::<PyInt>(vm),
        ) {
            let list_len = list.borrow_vec().len();
            if specialization_nonnegative_compact_index(int_idx, vm).is_some_and(|i| i < list_len) {
                Some(Instruction::StoreSubscrListInt)
            } else {
                None
            }
        } else if obj.downcast_ref_if_exact::<PyDict>(vm).is_some() {
            Some(Instruction::StoreSubscrDict)
        } else {
            None
        };

        self.commit_specialization(instr_idx, cache_base, new_op);
    }

    fn specialize_contains_op(&mut self, vm: &VirtualMachine, instr_idx: usize, cache_base: usize) {
        if !matches!(
            self.code.instructions.read_op(instr_idx),
            Instruction::ContainsOp { .. }
        ) {
            return;
        }
        let haystack = self.top_value(); // b = TOS = haystack
        let new_op = if haystack.downcast_ref_if_exact::<PyDict>(vm).is_some() {
            Some(Instruction::ContainsOpDict)
        } else if haystack.downcast_ref_if_exact::<PySet>(vm).is_some()
            || haystack.downcast_ref_if_exact::<PyFrozenSet>(vm).is_some()
        {
            Some(Instruction::ContainsOpSet)
        } else {
            None
        };

        self.commit_specialization(instr_idx, cache_base, new_op);
    }

    fn specialize_unpack_sequence(
        &mut self,
        vm: &VirtualMachine,
        expected_count: u32,
        instr_idx: usize,
        cache_base: usize,
    ) {
        if !matches!(
            self.code.instructions.read_op(instr_idx),
            Instruction::UnpackSequence { .. }
        ) {
            return;
        }
        let obj = self.top_value();
        let new_op = if let Some(tuple) = obj.downcast_ref_if_exact::<PyTuple>(vm) {
            if tuple.len() != expected_count as usize {
                None
            } else if expected_count == 2 {
                Some(Instruction::UnpackSequenceTwoTuple)
            } else {
                Some(Instruction::UnpackSequenceTuple)
            }
        } else if let Some(list) = obj.downcast_ref_if_exact::<PyList>(vm) {
            if list.borrow_vec().len() == expected_count as usize {
                Some(Instruction::UnpackSequenceList)
            } else {
                None
            }
        } else {
            None
        };

        self.commit_specialization(instr_idx, cache_base, new_op);
    }

    fn specialize_store_attr(
        &mut self,
        vm: &VirtualMachine,
        attr_idx: bytecode::NameIdx,
        instr_idx: usize,
        cache_base: usize,
    ) {
        if !matches!(
            self.code.instructions.read_op(instr_idx),
            Instruction::StoreAttr { .. }
        ) {
            return;
        }
        // TOS = owner (the object being assigned to)
        let owner = self.top_value();
        let cls = owner.class();

        // Only specialize if setattr is the default (generic_setattr)
        let is_default_setattr = cls
            .slots
            .setattro
            .load()
            .is_some_and(|f| f as usize == PyBaseObject::slot_setattro as *const () as usize);
        if !is_default_setattr {
            unsafe {
                self.code.instructions.write_adaptive_counter(
                    cache_base,
                    bytecode::adaptive_counter_backoff(
                        self.code.instructions.read_adaptive_counter(cache_base),
                    ),
                );
            }
            return;
        }

        // Get or assign type version
        let mut type_version = cls.tp_version_tag.load(Acquire);
        if type_version == 0 {
            type_version = cls.assign_version_tag();
        }
        if type_version == 0 {
            unsafe {
                self.code.instructions.write_adaptive_counter(
                    cache_base,
                    bytecode::adaptive_counter_backoff(
                        self.code.instructions.read_adaptive_counter(cache_base),
                    ),
                );
            }
            return;
        }

        // Check for data descriptor
        let attr_name = self.code.names[attr_idx as usize];
        let cls_attr = cls.get_attr(attr_name);
        let has_data_descr = cls_attr.as_ref().is_some_and(|descr| {
            let descr_cls = descr.class();
            descr_cls.slots.descr_get.load().is_some() && descr_cls.slots.descr_set.load().is_some()
        });

        if has_data_descr {
            // Check for member descriptor (slot access)
            if let Some(ref descr) = cls_attr
                && let Some(member_descr) = descr.downcast_ref::<PyMemberDescriptor>()
                && let MemberGetter::Offset(offset) = member_descr.member.getter
            {
                unsafe {
                    self.code
                        .instructions
                        .write_cache_u32(cache_base + 1, type_version);
                    self.code
                        .instructions
                        .write_cache_u16(cache_base + 3, offset as u16);
                }
                self.specialize_at(instr_idx, cache_base, Instruction::StoreAttrSlot);
            } else {
                unsafe {
                    self.code.instructions.write_adaptive_counter(
                        cache_base,
                        bytecode::adaptive_counter_backoff(
                            self.code.instructions.read_adaptive_counter(cache_base),
                        ),
                    );
                }
            }
        } else if let Some(dict) = owner.dict() {
            let use_hint = match dict.get_item_opt(attr_name, vm) {
                Ok(Some(_)) => true,
                Ok(None) => false,
                Err(_) => {
                    unsafe {
                        self.code.instructions.write_adaptive_counter(
                            cache_base,
                            bytecode::adaptive_counter_backoff(
                                self.code.instructions.read_adaptive_counter(cache_base),
                            ),
                        );
                    }
                    return;
                }
            };
            unsafe {
                self.code
                    .instructions
                    .write_cache_u32(cache_base + 1, type_version);
            }
            self.specialize_at(
                instr_idx,
                cache_base,
                if use_hint {
                    Instruction::StoreAttrWithHint
                } else {
                    Instruction::StoreAttrInstanceValue
                },
            );
        } else {
            unsafe {
                self.code.instructions.write_adaptive_counter(
                    cache_base,
                    bytecode::adaptive_counter_backoff(
                        self.code.instructions.read_adaptive_counter(cache_base),
                    ),
                );
            }
        }
    }

    fn load_super_attr(&mut self, vm: &VirtualMachine, oparg: LoadSuperAttr) -> FrameResult {
        let attr_name = self.code.names[oparg.name_idx() as usize];

        // Stack layout (bottom to top): [super, class, self]
        // Pop in LIFO order: self, class, super
        let self_obj = self.pop_value();
        let class = self.pop_value();
        let global_super = self.pop_value();

        // Create super object - pass args based on has_class flag
        // When super is shadowed, has_class=false means call with 0 args
        let super_obj = if oparg.has_class() {
            global_super.call((class, self_obj.clone()), vm)?
        } else {
            global_super.call((), vm)?
        };

        if oparg.is_load_method() {
            // Method load: push [method, self_or_null]
            let method = PyMethod::get(super_obj, attr_name, vm)?;
            match method {
                PyMethod::Function { target: _, func } => {
                    self.push_value(func);
                    self.push_value(self_obj);
                }
                PyMethod::Attribute(val) => {
                    self.push_value(val);
                    self.push_null();
                }
            }
        } else {
            // Regular attribute access
            let obj = super_obj.get_attr(attr_name, vm)?;
            self.push_value(obj);
        }
        Ok(None)
    }

    fn store_attr(&mut self, vm: &VirtualMachine, attr: bytecode::NameIdx) -> FrameResult {
        let attr_name = self.code.names[attr as usize];
        let parent = self.pop_value();
        let value = self.pop_value();
        parent.set_attr(attr_name, value, vm)?;
        Ok(None)
    }

    fn delete_attr(&mut self, vm: &VirtualMachine, attr: bytecode::NameIdx) -> FrameResult {
        let attr_name = self.code.names[attr as usize];
        let parent = self.pop_value();
        parent.del_attr(attr_name, vm)?;
        Ok(None)
    }

    // Block stack functions removed - exception table handles all exception/cleanup

    #[inline]
    #[track_caller]
    fn push_stackref_opt(&mut self, obj: Option<PyStackRef>) {
        match self.localsplus.stack_try_push(obj) {
            Ok(()) => {}
            Err(_e) => self.fatal("tried to push value onto stack but overflowed max_stackdepth"),
        }
    }

    #[inline]
    #[track_caller] // not a real track_caller but push_value is less useful for debugging
    fn push_value_opt(&mut self, obj: Option<PyObjectRef>) {
        self.push_stackref_opt(obj.map(PyStackRef::new_owned));
    }

    #[inline]
    #[track_caller]
    fn push_value(&mut self, obj: PyObjectRef) {
        self.push_stackref_opt(Some(PyStackRef::new_owned(obj)));
    }

    /// Push a borrowed reference onto the stack (no refcount increment).
    ///
    /// # Safety
    /// The object must remain alive until the borrowed ref is consumed.
    /// The compiler guarantees consumption within the same basic block.
    #[inline]
    #[track_caller]
    #[allow(dead_code)]
    unsafe fn push_borrowed(&mut self, obj: &PyObject) {
        self.push_stackref_opt(Some(unsafe { PyStackRef::new_borrowed(obj) }));
    }

    #[inline]
    fn push_null(&mut self) {
        self.push_stackref_opt(None);
    }

    /// Pop a raw stackref from the stack, returning None if the stack slot is NULL.
    #[inline]
    fn pop_stackref_opt(&mut self) -> Option<PyStackRef> {
        if self.localsplus.stack_is_empty() {
            self.fatal("tried to pop from empty stack");
        }
        self.localsplus.stack_pop()
    }

    /// Pop a raw stackref from the stack. Panics if NULL.
    #[inline]
    #[track_caller]
    fn pop_stackref(&mut self) -> PyStackRef {
        expect_unchecked(
            self.pop_stackref_opt(),
            "pop stackref but null found. This is a compiler bug.",
        )
    }

    /// Pop a value from the stack, returning None if the stack slot is NULL.
    /// Automatically promotes borrowed refs to owned.
    #[inline]
    fn pop_value_opt(&mut self) -> Option<PyObjectRef> {
        self.pop_stackref_opt().map(|sr| sr.to_pyobj())
    }

    #[inline]
    #[track_caller]
    fn pop_value(&mut self) -> PyObjectRef {
        self.pop_stackref().to_pyobj()
    }

    fn call_intrinsic_1(
        &mut self,
        func: bytecode::IntrinsicFunction1,
        arg: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        match func {
            bytecode::IntrinsicFunction1::Invalid => {
                unreachable!("This is a bug in RustPython compiler")
            }
            bytecode::IntrinsicFunction1::Print => {
                let displayhook = vm
                    .sys_module
                    .get_attr("displayhook", vm)
                    .map_err(|_| vm.new_runtime_error("lost sys.displayhook"))?;
                displayhook.call((arg,), vm)
            }
            bytecode::IntrinsicFunction1::ImportStar => {
                // arg is the module object
                self.push_value(arg); // Push module back on stack for import_star
                self.import_star(vm)?;
                Ok(vm.ctx.none())
            }
            bytecode::IntrinsicFunction1::UnaryPositive => vm._pos(&arg),
            bytecode::IntrinsicFunction1::SubscriptGeneric => {
                // Used for PEP 695: Generic[*type_params]
                crate::builtins::genericalias::subscript_generic(arg, vm)
            }
            bytecode::IntrinsicFunction1::TypeVar => {
                let type_var: PyObjectRef =
                    _typing::TypeVar::new(vm, arg, vm.ctx.none(), vm.ctx.none())
                        .into_ref(&vm.ctx)
                        .into();
                Ok(type_var)
            }
            bytecode::IntrinsicFunction1::ParamSpec => {
                let param_spec: PyObjectRef =
                    _typing::ParamSpec::new(arg, vm).into_ref(&vm.ctx).into();
                Ok(param_spec)
            }
            bytecode::IntrinsicFunction1::TypeVarTuple => {
                let type_var_tuple: PyObjectRef =
                    _typing::TypeVarTuple::new(arg, vm).into_ref(&vm.ctx).into();
                Ok(type_var_tuple)
            }
            bytecode::IntrinsicFunction1::TypeAlias => {
                // TypeAlias receives a tuple of (name, type_params, value)
                let tuple: PyTupleRef = arg
                    .downcast()
                    .map_err(|_| vm.new_type_error("TypeAlias expects a tuple argument"))?;

                if tuple.len() != 3 {
                    return Err(vm.new_type_error(format!(
                        "TypeAlias expects exactly 3 arguments, got {}",
                        tuple.len()
                    )));
                }

                let name = tuple.as_slice()[0].clone();
                let type_params_obj = tuple.as_slice()[1].clone();
                let compute_value = tuple.as_slice()[2].clone();

                let type_params: PyTupleRef = if vm.is_none(&type_params_obj) {
                    vm.ctx.empty_tuple.clone()
                } else {
                    type_params_obj
                        .downcast()
                        .map_err(|_| vm.new_type_error("Type params must be a tuple."))?
                };

                let name = name
                    .downcast::<crate::builtins::PyStr>()
                    .map_err(|_| vm.new_type_error("TypeAliasType name must be a string"))?;
                let type_alias = _typing::TypeAliasType::new(name, type_params, compute_value);
                Ok(type_alias.into_ref(&vm.ctx).into())
            }
            bytecode::IntrinsicFunction1::ListToTuple => {
                // Convert list to tuple
                let list = arg
                    .downcast::<PyList>()
                    .map_err(|_| vm.new_type_error("LIST_TO_TUPLE expects a list"))?;
                Ok(vm.ctx.new_tuple(list.borrow_vec().to_vec()).into())
            }
            bytecode::IntrinsicFunction1::StopIterationError => {
                // Convert StopIteration to RuntimeError (PEP 479)
                // Returns the exception object; RERAISE will re-raise it
                if arg.fast_isinstance(vm.ctx.exceptions.stop_iteration) {
                    let flags = &self.code.flags;
                    let msg = if flags
                        .contains(bytecode::CodeFlags::COROUTINE | bytecode::CodeFlags::GENERATOR)
                    {
                        "async generator raised StopIteration"
                    } else if flags.contains(bytecode::CodeFlags::COROUTINE) {
                        "coroutine raised StopIteration"
                    } else {
                        "generator raised StopIteration"
                    };
                    let err = vm.new_runtime_error(msg);
                    // PEP 479 chains both __cause__ and __context__ to the
                    // original StopIteration; the explicit cause is what users
                    // see in tracebacks (suppress_context becomes true), but
                    // assertions that inspect __context__ also expect it set.
                    let cause: Option<PyBaseExceptionRef> = arg.downcast().ok();
                    err.set___context__(cause.clone());
                    err.set___cause__(cause);
                    Ok(err.into())
                } else {
                    // Not StopIteration, pass through for RERAISE
                    Ok(arg)
                }
            }
            bytecode::IntrinsicFunction1::AsyncGenWrap => {
                // Wrap value for async generator
                // Creates an AsyncGenWrappedValue
                Ok(crate::builtins::asyncgenerator::PyAsyncGenWrappedValue(arg)
                    .into_ref(&vm.ctx)
                    .into())
            }
        }
    }

    fn call_intrinsic_2(
        &mut self,
        func: bytecode::IntrinsicFunction2,
        arg1: PyObjectRef,
        arg2: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        match func {
            bytecode::IntrinsicFunction2::Invalid => {
                unreachable!("This is a bug in RustPython compiler")
            }
            bytecode::IntrinsicFunction2::SetTypeparamDefault => {
                crate::stdlib::_typing::set_typeparam_default(arg1, arg2, vm)
            }
            bytecode::IntrinsicFunction2::SetFunctionTypeParams => {
                // arg1 is the function, arg2 is the type params tuple
                // Set __type_params__ attribute on the function
                arg1.set_attr("__type_params__", arg2, vm)?;
                Ok(arg1)
            }
            bytecode::IntrinsicFunction2::TypeVarWithBound => {
                let type_var: PyObjectRef = _typing::TypeVar::new(vm, arg1, arg2, vm.ctx.none())
                    .into_ref(&vm.ctx)
                    .into();
                Ok(type_var)
            }
            bytecode::IntrinsicFunction2::TypeVarWithConstraint => {
                let type_var: PyObjectRef = _typing::TypeVar::new(vm, arg1, vm.ctx.none(), arg2)
                    .into_ref(&vm.ctx)
                    .into();
                Ok(type_var)
            }
            bytecode::IntrinsicFunction2::PrepReraiseStar => {
                // arg1 = orig (original exception)
                // arg2 = excs (list of exceptions raised/reraised in except* blocks)
                // Returns: exception to reraise, or None if nothing to reraise
                crate::exceptions::prep_reraise_star(arg1, arg2, vm)
            }
        }
    }

    /// Pop multiple values from the stack. Panics if any slot is NULL.
    fn pop_multiple(&mut self, count: usize) -> impl ExactSizeIterator<Item = PyObjectRef> + '_ {
        let stack_len = self.localsplus.stack_len();
        if count > stack_len {
            let instr = self.code.instructions.get(self.lasti() as usize);
            let op_name = instr
                .map(|i| format!("{:?}", i.op))
                .unwrap_or_else(|| "None".to_string());
            panic!(
                "Stack underflow in pop_multiple: trying to pop {} elements from stack with {} elements. lasti={}, code={}, op={}, source_path={}",
                count,
                stack_len,
                self.lasti(),
                self.code.obj_name,
                op_name,
                self.code.source_path()
            );
        }
        self.localsplus.stack_drain(stack_len - count).map(|obj| {
            expect_unchecked(obj, "pop_multiple but null found. This is a compiler bug.").to_pyobj()
        })
    }

    #[inline]
    fn replace_top(&mut self, top: Option<PyObjectRef>) -> Option<PyObjectRef> {
        let mut slot = top.map(PyStackRef::new_owned);
        let last = self.localsplus.stack_last_mut().unwrap();
        core::mem::swap(last, &mut slot);
        slot.map(|sr| sr.to_pyobj())
    }

    #[inline]
    #[track_caller]
    fn top_value(&self) -> &PyObject {
        match self.localsplus.stack_last() {
            Some(Some(last)) => last.as_object(),
            Some(None) => self.fatal("tried to get top of stack but got NULL"),
            None => self.fatal("tried to get top of stack but stack is empty"),
        }
    }

    #[inline]
    #[track_caller]
    fn nth_value(&self, depth: u32) -> &PyObject {
        let idx = self.localsplus.stack_len() - depth as usize - 1;
        match self.localsplus.stack_index(idx) {
            Some(obj) => obj.as_object(),
            None => unsafe { core::hint::unreachable_unchecked() },
        }
    }

    #[cold]
    #[inline(never)]
    #[track_caller]
    fn fatal(&self, msg: &'static str) -> ! {
        dbg!(self);
        panic!("{msg}")
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // SAFETY: Debug is best-effort; concurrent mutation is unlikely
        // and would only affect debug output.
        let iframe = unsafe { &*self.iframe.get() };
        let stack_str =
            iframe
                .localsplus
                .stack_as_slice()
                .iter()
                .fold(String::new(), |mut s, slot| {
                    match slot {
                        Some(elem) if elem.downcastable::<Self>() => {
                            s.push_str("\n  > {frame}");
                        }
                        Some(elem) => {
                            core::fmt::write(&mut s, format_args!("\n  > {elem:?}")).unwrap();
                        }
                        None => {
                            s.push_str("\n  > NULL");
                        }
                    }
                    s
                });
        // TODO: fix this up
        write!(
            f,
            "Frame Object {{ \n Stack:{}\n Locals initialized:{}\n}}",
            stack_str,
            self.locals.get().is_some()
        )
    }
}

/// _PyEval_SpecialMethodCanSuggest
fn special_method_can_suggest(
    obj: &PyObjectRef,
    oparg: SpecialMethod,
    vm: &VirtualMachine,
) -> PyResult<bool> {
    Ok(match oparg {
        SpecialMethod::Enter | SpecialMethod::Exit => {
            vm.get_special_method(obj, get_special_method_name(SpecialMethod::AEnter, vm))?
                .is_some()
                && vm
                    .get_special_method(obj, get_special_method_name(SpecialMethod::AExit, vm))?
                    .is_some()
        }
        SpecialMethod::AEnter | SpecialMethod::AExit => {
            vm.get_special_method(obj, get_special_method_name(SpecialMethod::Enter, vm))?
                .is_some()
                && vm
                    .get_special_method(obj, get_special_method_name(SpecialMethod::Exit, vm))?
                    .is_some()
        }
    })
}

fn get_special_method_name(oparg: SpecialMethod, vm: &VirtualMachine) -> &'static PyStrInterned {
    match oparg {
        SpecialMethod::Enter => identifier!(vm, __enter__),
        SpecialMethod::Exit => identifier!(vm, __exit__),
        SpecialMethod::AEnter => identifier!(vm, __aenter__),
        SpecialMethod::AExit => identifier!(vm, __aexit__),
    }
}

/// _Py_SpecialMethod _Py_SpecialMethods
fn get_special_method_error_msg(
    oparg: SpecialMethod,
    class_name: &str,
    can_suggest: bool,
) -> String {
    if can_suggest {
        match oparg {
            SpecialMethod::Enter => format!(
                "'{class_name}' object does not support the context manager protocol (missed __enter__ method) but it supports the asynchronous context manager protocol. Did you mean to use 'async with'?"
            ),
            SpecialMethod::Exit => format!(
                "'{class_name}' object does not support the context manager protocol (missed __exit__ method) but it supports the asynchronous context manager protocol. Did you mean to use 'async with'?"
            ),
            SpecialMethod::AEnter => format!(
                "'{class_name}' object does not support the asynchronous context manager protocol (missed __aenter__ method) but it supports the context manager protocol. Did you mean to use 'with'?"
            ),
            SpecialMethod::AExit => format!(
                "'{class_name}' object does not support the asynchronous context manager protocol (missed __aexit__ method) but it supports the context manager protocol. Did you mean to use 'with'?"
            ),
        }
    } else {
        match oparg {
            SpecialMethod::Enter => format!(
                "'{class_name}' object does not support the context manager protocol (missed __enter__ method)"
            ),
            SpecialMethod::Exit => format!(
                "'{class_name}' object does not support the context manager protocol (missed __exit__ method)"
            ),
            SpecialMethod::AEnter => format!(
                "'{class_name}' object does not support the asynchronous context manager protocol (missed __aenter__ method)"
            ),
            SpecialMethod::AExit => format!(
                "'{class_name}' object does not support the asynchronous context manager protocol (missed __aexit__ method)"
            ),
        }
    }
}

fn is_module_initializing(module: &PyObject, vm: &VirtualMachine) -> bool {
    let Ok(spec) = module.get_attr(&vm.ctx.new_str("__spec__"), vm) else {
        return false;
    };
    if vm.is_none(&spec) {
        return false;
    }
    let Ok(initializing_attr) = spec.get_attr(&vm.ctx.new_str("_initializing"), vm) else {
        return false;
    };
    initializing_attr.try_to_bool(vm).unwrap_or(false)
}

fn expect_unchecked<T: fmt::Debug>(optional: Option<T>, err_msg: &'static str) -> T {
    if cfg!(debug_assertions) {
        optional.expect(err_msg)
    } else {
        unsafe { optional.unwrap_unchecked() }
    }
}
