use std::sync::atomic::Ordering;

use crate::object::gc::{Collector, GLOBAL_COLLECTOR};
#[cfg(not(feature = "threading"))]
use rustpython_common::atomic::Radium;
use rustpython_common::lock::PyMutexGuard;
use rustpython_common::{
    atomic::PyAtomic,
    lock::{PyMutex, PyRwLockReadGuard},
    rc::PyRc,
};

#[derive(Debug)]
struct State {
    inner: u8,
}

impl Default for State {
    fn default() -> Self {
        let mut new = Self {
            inner: Default::default(),
        };
        new.set_color(Color::Black);
        new.set_in_cycle(false);
        new.set_buffered(false);
        new.set_drop(false);
        new.set_dealloc(false);
        new.set_leak(false);
        new
    }
}

type StateBytes = u8;

macro_rules! getset {
    ($GETTER: ident, $SETTER: ident, $MASK: path, $OFFSET: path) => {
        fn $GETTER(&self) -> bool {
            ((self.inner & $MASK) >> $OFFSET) != 0
        }
        fn $SETTER(&mut self, b: bool) {
            self.inner = (self.inner & !$MASK) | ((b as StateBytes) << $OFFSET)
        }
    };
}

impl State {
    const COLOR_MASK: StateBytes = 0b0000_0011;
    const CYCLE_MASK: StateBytes = 0b0000_0100;
    const CYCLE_OFFSET: StateBytes = 2;
    const BUF_MASK: StateBytes = 0b0000_1000;
    const BUF_OFFSET: StateBytes = 3;
    const DROP_MASK: StateBytes = 0b0001_0000;
    const DROP_OFFSET: StateBytes = 4;
    const DEALLOC_MASK: StateBytes = 0b0010_0000;
    const DEALLOC_OFFSET: StateBytes = 5;
    const LEAK_MASK: StateBytes = 0b0100_0000;
    const LEAK_OFFSET: StateBytes = 6;
    const DONE_MAKS: StateBytes = 0b1000_0000;
    const DONE_OFFSET: StateBytes = 7;
    fn color(&self) -> Color {
        let color = self.inner & Self::COLOR_MASK;
        match color {
            0 => Color::Black,
            1 => Color::Gray,
            2 => Color::White,
            3 => Color::Purple,
            _ => unreachable!(),
        }
    }
    fn set_color(&mut self, color: Color) {
        let color = match color {
            Color::Black => 0,
            Color::Gray => 1,
            Color::White => 2,
            Color::Purple => 3,
        };
        self.inner = (self.inner & !Self::COLOR_MASK) | color;
    }
    getset! {in_cycle, set_in_cycle, Self::CYCLE_MASK, Self::CYCLE_OFFSET}
    getset! {buffered, set_buffered, Self::BUF_MASK, Self::BUF_OFFSET}
    getset! {is_drop, set_drop, Self::DROP_MASK, Self::DROP_OFFSET}
    getset! {is_dealloc, set_dealloc, Self::DEALLOC_MASK, Self::DEALLOC_OFFSET}
    getset! {is_leak, set_leak, Self::LEAK_MASK, Self::LEAK_OFFSET}
    getset! {is_done_drop, set_done_drop, Self::DONE_MAKS, Self::DONE_OFFSET}
}

#[test]
fn test_state() {
    let mut state = State::default();
    assert!(!state.is_dealloc());
    state.set_drop(true);
    assert!(state.inner == 16);
    assert!(!state.is_dealloc() && state.is_drop());
    // color
    state.set_color(Color::Gray);
    assert_eq!(state.color(), Color::Gray);
    state.set_color(Color::White);
    assert_eq!(state.color(), Color::White);
    state.set_color(Color::Black);
    assert_eq!(state.color(), Color::Black);
    state.set_color(Color::Purple);
    assert_eq!(state.color(), Color::Purple);
}

/// Garbage collect header, containing ref count and other info, using repr(C) to stay consistent with PyInner 's repr
#[repr(C)]
#[derive(Debug)]
pub struct GcHeader {
    ref_cnt: PyAtomic<usize>,
    state: PyMutex<State>,
    exclusive: PyMutex<()>,
    gc: PyRc<Collector>,
}

impl Default for GcHeader {
    fn default() -> Self {
        Self {
            ref_cnt: 1.into(),
            state: Default::default(),
            exclusive: PyMutex::new(()),
            /// when threading, using a global GC
            #[cfg(feature = "threading")]
            gc: GLOBAL_COLLECTOR.clone(),
            /// when not threading, using a gc per thread
            #[cfg(not(feature = "threading"))]
            gc: GLOBAL_COLLECTOR.with(|v| v.clone()),
        }
    }
}

// TODO: use macro for getter/setter
impl GcHeader {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn get(&self) -> usize {
        self.ref_cnt.load(Ordering::Relaxed)
    }

    /// gain a exclusive lock to header
    pub fn exclusive(&self) -> PyMutexGuard<()> {
        self.exclusive.lock()
    }

    pub fn gc(&self) -> PyRc<Collector> {
        self.gc.clone()
    }

    pub fn is_done_drop(&self) -> bool {
        self.state.lock().is_done_drop()
    }

    pub fn set_done_drop(&self, b: bool) {
        self.state.lock().set_done_drop(b)
    }

    pub fn is_in_cycle(&self) -> bool {
        self.state.lock().in_cycle()
    }

    pub fn set_in_cycle(&self, b: bool) {
        self.state.lock().set_in_cycle(b)
    }

    pub fn is_drop(&self) -> bool {
        self.state.lock().is_drop()
    }

    pub fn set_drop(&self) {
        self.state.lock().set_drop(true)
    }

    pub fn is_dealloc(&self) -> bool {
        #[cfg(feature = "threading")]
        {
            self.state
                .try_lock_for(std::time::Duration::from_secs(1))
                .expect("Dead lock happen when should not, probably already deallocated")
                .is_dealloc()
        }
        #[cfg(not(feature = "threading"))]
        {
            self.state
                .try_lock()
                .expect("Dead lock happen when should not, probably already deallocated")
                .is_dealloc()
        }
    }

    pub fn set_dealloc(&self) {
        self.state.lock().set_dealloc(true)
    }

    pub(crate) fn check_set_drop_dealloc(&self) -> bool {
        let is_dealloc = self.is_dealloc();
        if is_dealloc {
            warn!("Call a function inside a already deallocated object!");
            return false;
        }
        if !self.is_drop() && !is_dealloc {
            self.set_drop();
            self.set_dealloc();
            true
        } else {
            false
        }
    }

    /// return true if can drop(also mark object as dropped)
    pub(crate) fn check_set_drop_only(&self) -> bool {
        let is_dealloc = self.is_dealloc();
        if is_dealloc {
            warn!("Call a function inside a already deallocated object.");
            return false;
        }
        if !self.is_drop() && !is_dealloc {
            self.set_drop();
            true
        } else {
            false
        }
    }

    /// return true if can dealloc(that is already drop)
    pub(crate) fn check_set_dealloc_only(&self) -> bool {
        let is_drop = self.state.lock().is_drop();
        let is_dealloc = self.is_dealloc();
        if !is_drop {
            warn!("Try to dealloc a object that haven't drop.");
            return false;
        }
        if is_drop && !is_dealloc {
            self.set_dealloc();
            true
        } else {
            false
        }
    }

    pub fn try_pausing(&self) -> Option<PyRwLockReadGuard<()>> {
        if self.is_dealloc() {
            // could be false alarm for PyWeak, like set is_dealloc, then block by guard and havn't drop&dealloc
            debug!("Try to pausing a already deallocated object: {:?}", self);
        }
        self.gc.try_pausing()
    }

    /// This function will block if is a garbage collect is happening
    pub fn do_pausing(&self) {
        if self.is_dealloc() {
            // could be false alarm for PyWeak, like set is_dealloc, then block by guard and havn't drop&dealloc
            debug!("Try to pausing a already deallocated object: {:?}", self);
        }
        self.gc.do_pausing();
    }
    pub fn color(&self) -> Color {
        self.state.lock().color()
    }
    pub fn set_color(&self, new_color: Color) {
        self.state.lock().set_color(new_color)
    }
    pub fn buffered(&self) -> bool {
        self.state.lock().buffered()
    }
    pub fn set_buffered(&self, buffered: bool) {
        self.state.lock().set_buffered(buffered)
    }
    /// simple RC += 1
    pub fn inc(&self) -> usize {
        self.ref_cnt.fetch_add(1, Ordering::Relaxed) + 1
    }
    /// only inc if non-zero(and return true if success)
    #[inline]
    pub fn safe_inc(&self) -> bool {
        let ret = self
            .ref_cnt
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |prev| {
                (prev != 0).then_some(prev + 1)
            })
            .is_ok();
        if ret {
            self.set_color(Color::Black)
        }
        ret
    }
    /// simple RC -= 1
    pub fn dec(&self) -> usize {
        self.ref_cnt.fetch_sub(1, Ordering::Relaxed) - 1
    }
    pub fn rc(&self) -> usize {
        self.ref_cnt.load(Ordering::Relaxed)
    }
}

impl GcHeader {
    // move these functions out and give separated type once type range is stabilized

    pub fn leak(&self) {
        if self.is_leaked() {
            // warn!("Try to leak a already leaked object!");
            return;
        }
        self.state.lock().set_leak(true);
        /*
        const BIT_MARKER: usize = (std::isize::MAX as usize) + 1;
        debug_assert_eq!(BIT_MARKER.count_ones(), 1);
        debug_assert_eq!(BIT_MARKER.leading_zeros(), 0);
        self.ref_cnt.fetch_add(BIT_MARKER, Ordering::Relaxed);
        */
    }

    pub fn is_leaked(&self) -> bool {
        // (self.ref_cnt.load(Ordering::Acquire) as isize) < 0
        self.state.lock().is_leak()
    }
}

/// other color(Green, Red, Orange) in the paper is not in use for now, so remove them in this enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub enum Color {
    /// In use(or free)
    Black,
    /// Possible member of cycle
    Gray,
    /// Member of garbage cycle
    White,
    /// Possible root of cycle
    Purple,
}

#[derive(Debug, Default)]
pub struct GcResult {
    acyclic_cnt: usize,
    cyclic_cnt: usize,
}

impl GcResult {
    fn new(tuple: (usize, usize)) -> Self {
        Self {
            acyclic_cnt: tuple.0,
            cyclic_cnt: tuple.1,
        }
    }
}

impl From<(usize, usize)> for GcResult {
    fn from(t: (usize, usize)) -> Self {
        Self::new(t)
    }
}

impl From<GcResult> for (usize, usize) {
    fn from(g: GcResult) -> Self {
        (g.acyclic_cnt, g.cyclic_cnt)
    }
}

impl From<GcResult> for usize {
    fn from(g: GcResult) -> Self {
        g.acyclic_cnt + g.cyclic_cnt
    }
}
