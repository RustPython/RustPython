// This file has been modified from its original version in the dumpster crate.
// Source: https://github.com/claytonwramsey/dumpster/blob/bc197d2f875aadae086e1ba3dd7da6d29ffee6fa/dumpster/src/sync/collect.rs
/*
    dumpster, acycle-tracking garbage collector for Rust.    Copyright (C) 2023 Clayton Ramsey.

    This Source Code Form is subject to the terms of the Mozilla Public
    License, v. 2.0. If a copy of the MPL was not distributed with this
    file, You can obtain one at http://mozilla.org/MPL/2.0/.
*/

//! A synchronized collection algorithm.

use std::{
    alloc::{dealloc, Layout},
    cell::{Cell, RefCell},
    collections::{hash_map::Entry, HashMap},
    mem::{replace, swap, take, transmute},
    ptr::{drop_in_place, NonNull},
    sync::{
        atomic::{AtomicPtr, AtomicUsize, Ordering},
        LazyLock,
    },
};

use parking_lot::{Mutex, RwLock};
use crate::object::core::PyInner;
use crate::object::Traverse;
use crate::PyObjectRef;
use super::{default_collect_condition, CollectCondition, CollectInfo, Visitor, CURRENT_TAG};
use super::erased::Erased;

/// The garbage truck, which is a global data structure containing information about allocations
/// which might need to be collected.
struct GarbageTruck {
    /// The contents of the garbage truck, containing all the allocations which need to be
    /// collected and have already been delivered by a [`Dumpster`].
    contents: Mutex<HashMap<AllocationId, TrashCan>>,
    /// A lock used for synchronizing threads that are awaiting completion of a collection process.
    /// This lock should be acquired for reads by threads running a collection and for writes by
    /// threads awaiting collection completion.
    collecting_lock: RwLock<()>,
    /// The number of [`Gc`]s dropped since the last time [`Dumpster::collect_all()`] was called.
    n_gcs_dropped: AtomicUsize,
    /// The number of [`Gc`]s currently existing (which have not had their internals replaced with
    /// `None`).
    n_gcs_existing: AtomicUsize,
    /// The function which determines whether a collection should be triggered.
    /// This pointer value should always be cast to a [`CollectCondition`], but since `AtomicPtr`
    /// doesn't handle function pointers correctly, we just cast to `*mut ()`.
    collect_condition: AtomicPtr<()>,
}

/// A structure containing the global information for the garbage collector.
struct Dumpster {
    /// A lookup table for the allocations which may need to be cleaned up later.
    contents: RefCell<HashMap<AllocationId, TrashCan>>,
    /// The number of times an allocation on this thread has been dropped.
    n_drops: Cell<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
/// A unique identifier for an allocation.
struct AllocationId(NonNull<PyInner<()>>);

#[derive(Debug)]
/// The information which describes an allocation that may need to be cleaned up later.
struct TrashCan {
    /// A pointer to the allocation to be cleaned up.
    ptr: Erased,
    /// The function which can be used to build a reference graph.
    /// This function is safe to call on `ptr`.
    dfs_fn: unsafe fn(Erased, &mut HashMap<AllocationId, AllocationInfo>),
}

#[derive(Debug)]
/// A node in the reference graph, which is constructed while searching for unreachable allocations.
struct AllocationInfo {
    /// An erased pointer to the allocation.
    ptr: Erased,
    /// Function for dropping the allocation when its weak and strong count hits zero.
    /// Should have the same behavior as dropping a Gc normally to a reference count of zero.
    weak_drop_fn: unsafe fn(Erased),
    /// Information about this allocation's reachability.
    reachability: Reachability,
}

#[derive(Debug)]
/// The state of whether an allocation is reachable or of unknown reachability.
enum Reachability {
    /// The information describing an allocation whose accessibility is unknown.
    Unknown {
        /// The IDs for the allocations directly accessible from this allocation.
        children: Vec<AllocationId>,
        /// The number of references in the reference count for this allocation which are
        /// "unaccounted," which have not been found while constructing the graph.
        /// It is the difference between the allocations indegree in the "true" reference graph vs
        /// the one we are currently building.
        n_unaccounted: usize,
        /// A function used to destroy the allocation.
        destroy_fn: unsafe fn(Erased, &HashMap<AllocationId, AllocationInfo>),
    },
    /// The allocation here is reachable.
    /// No further information is needed.
    Reachable,
}

/// The global garbage truck.
/// All [`TrashCans`] should eventually end up in here.
static GARBAGE_TRUCK: LazyLock<GarbageTruck> = LazyLock::new(|| GarbageTruck {
    contents: Mutex::new(HashMap::new()),
    collecting_lock: RwLock::new(()),
    n_gcs_dropped: AtomicUsize::new(0),
    n_gcs_existing: AtomicUsize::new(0),
    collect_condition: AtomicPtr::new(default_collect_condition as *mut ()),
});

thread_local! {
    /// The dumpster for this thread.
    /// Allocations which are "dirty" will be transferred to this dumpster before being moved into
    /// the garbage truck for final collection.
    static DUMPSTER: Dumpster = Dumpster {
        contents: RefCell::new(HashMap::new()),
        n_drops: Cell::new(0),
    };

    /// Whether the currently-running thread is doing a cleanup.
    /// This cannot be stored in `DUMPSTER` because otherwise it would cause weird use-after-drop
    /// behavior.
    static CLEANING: Cell<bool> = const { Cell::new(false) };
}

#[allow(clippy::module_name_repetitions)]
/// Collect all allocations in the garbage truck (but not necessarily the dumpster), then await
/// completion of the collection.
/// Ensures that all allocations dropped on the calling thread are cleaned up
pub fn collect_all_await() {
    DUMPSTER.with(|d| d.deliver_to(&GARBAGE_TRUCK));
    GARBAGE_TRUCK.collect_all();
    drop(GARBAGE_TRUCK.collecting_lock.read());
}

/// Notify that a `Gc` was destroyed, and update the tracking count for the number of dropped and
/// existing `Gc`s.
///
/// This may trigger a linear-time cleanup of all allocations, but this will be guaranteed to
/// occur with less-than-linear frequency, so it's always O(1).
pub fn notify_dropped_gc() {
    GARBAGE_TRUCK.n_gcs_existing.fetch_sub(1, Ordering::Relaxed);
    GARBAGE_TRUCK.n_gcs_dropped.fetch_add(1, Ordering::Relaxed);
    DUMPSTER.with(|dumpster| {
        dumpster.n_drops.set(dumpster.n_drops.get() + 1);
        if dumpster.is_full() {
            dumpster.deliver_to(&GARBAGE_TRUCK);
        }
    });

    let collect_cond = unsafe {
        // SAFETY: we only ever store collection conditions in the collect-condition box
        transmute::<*mut (), CollectCondition>(
            GARBAGE_TRUCK.collect_condition.load(Ordering::Relaxed),
        )
    };
    if collect_cond(&CollectInfo { _private: () }) {
        GARBAGE_TRUCK.collect_all();
    }
}

/// Notify that a [`Gc`] was created, and increment the number of total existing `Gc`s.
pub fn notify_created_gc() {
    GARBAGE_TRUCK.n_gcs_existing.fetch_add(1, Ordering::Relaxed);
}

/// Mark an allocation as "dirty," implying that it may or may not be inaccessible and need to
/// be cleaned up.
pub(super) fn mark_dirty<T>(allocation: &PyInner<T>)
where
    T: Traverse + Send + Sync
{
    DUMPSTER.with(|dumpster| {
        if dumpster
            .contents
            .borrow_mut()
            .insert(
                AllocationId::from(allocation),
                TrashCan {
                    ptr: Erased::new(NonNull::from(allocation)),
                    dfs_fn: dfs::<T>,
                },
            )
            .is_none()
        {
            allocation.ref_count.weak.fetch_add(1, Ordering::Acquire);
        }
    });
}

/// Mark an allocation as "clean," implying that it has already been cleaned up and does not
/// need to be cleaned again.
pub(super) fn mark_clean<T>(allocation: &PyInner<T>)
where
    T: Traverse + Send + Sync + ?Sized,
{
    DUMPSTER.with(|dumpster| {
        if dumpster
            .contents
            .borrow_mut()
            .remove(&AllocationId::from(allocation))
            .is_some()
        {
            allocation.ref_count.weak.fetch_sub(1, Ordering::Release);
        }
    });
}

#[allow(clippy::missing_panics_doc)]
/// Set the function which determines whether the garbage collector should be run.
///
/// `f` will be periodically called by the garbage collector to determine whether it should perform
/// a full traversal of the heap.
/// When `f` returns true, a traversal will begin.
pub fn set_collect_condition(f: CollectCondition) {
    GARBAGE_TRUCK
        .collect_condition
        .store(f as *mut (), Ordering::Relaxed);
}

/// Determine whether this thread is currently cleaning.
pub fn currently_cleaning() -> bool {
    CLEANING.get()
}

/// Get the number of `[Gc]`s dropped since the last collection.
pub fn n_gcs_dropped() -> usize {
    GARBAGE_TRUCK.n_gcs_dropped.load(Ordering::Relaxed)
}

/// Get the number of `[Gc]`s currently existing in the entire program.
pub fn n_gcs_existing() -> usize {
    GARBAGE_TRUCK.n_gcs_existing.load(Ordering::Relaxed)
}

impl Dumpster {
    /// Deliver all [`TrashCans`] contained by this dumpster to the garbage collect, removing them
    /// from the local dumpster storage and adding them to the global truck.
    fn deliver_to(&self, garbage_truck: &GarbageTruck) {
        self.n_drops.set(0);
        let mut guard = garbage_truck.contents.lock();
        for (id, can) in self.contents.borrow_mut().drain() {
            if guard.insert(id, can).is_some() {
                unsafe {
                    // SAFETY: an allocation can only be in the dumpster if it still exists and its
                    // header is valid
                    id.0.as_ref()
                }
                    .ref_count
                    .weak
                    .fetch_sub(1, Ordering::Release);
            }
        }
    }

    /// Determine whether this dumpster is full (and therefore should have its contents delivered to
    /// the garbage truck).
    fn is_full(&self) -> bool {
        self.contents.borrow().len() > 100_000 || self.n_drops.get() > 100_000
    }
}

impl GarbageTruck {
    #[allow(clippy::module_name_repetitions)]
    /// Search through the set of existing allocations which have been marked inaccessible, and see
    /// if they are inaccessible.
    /// If so, drop those allocations.
    fn collect_all(&self) {
        let collecting_guard = self.collecting_lock.write();
        self.n_gcs_dropped.store(0, Ordering::Relaxed);
        let to_collect = take(&mut *self.contents.lock());
        let mut ref_graph = HashMap::with_capacity(to_collect.len());

        CURRENT_TAG.fetch_add(1, Ordering::Release);

        for (_, TrashCan { ptr, dfs_fn }) in to_collect {
            unsafe {
                // SAFETY: `ptr` may only be in `to_collect` if it was a valid pointer
                // and `dfs_fn` must have been created with the intent of referring to
                // the erased type of `ptr`.
                dfs_fn(ptr, &mut ref_graph);
            }
        }

        let root_ids = ref_graph
            .iter()
            .filter_map(|(&k, v)| match v.reachability {
                Reachability::Reachable => Some(k),
                Reachability::Unknown { n_unaccounted, .. } => (n_unaccounted > 0
                    || unsafe {
                    // SAFETY: we found `k` in the reference graph,
                    // so it must still be an extant allocation
                    k.0.as_ref().ref_count.weak.load(Ordering::Acquire) > 1
                })
                    .then_some(k),
            })
            .collect::<Vec<_>>();
        for root_id in root_ids {
            mark(root_id, &mut ref_graph);
        }

        CLEANING.set(true);
        // set of allocations which must be destroyed because we were the last weak pointer to it
        let mut weak_destroys = Vec::new();
        for (id, node) in &ref_graph {
            let header_ref = unsafe { id.0.as_ref() };
            match node.reachability {
                Reachability::Unknown { destroy_fn, .. } => unsafe {
                    // SAFETY: `destroy_fn` must have been created with `node.ptr` in mind,
                    // and we have proven that no other references to `node.ptr` exist
                    destroy_fn(node.ptr, &ref_graph);
                },
                Reachability::Reachable => {
                    if header_ref.ref_count.weak.fetch_sub(1, Ordering::Release) == 1
                        && header_ref.ref_count.strong.load(Ordering::Acquire) == 0
                    {
                        // we are the last reference to the allocation.
                        // mark to be cleaned up later
                        // no real synchronization loss to storing the guard because we had the last
                        // reference anyway
                        weak_destroys.push((node.weak_drop_fn, node.ptr));
                    }
                }
            };
        }
        CLEANING.set(false);
        for (drop_fn, ptr) in weak_destroys {
            unsafe {
                // SAFETY: we have proven (via header_ref.weak = 1) that the cleaning
                // process had the last reference to the allocation.
                // `drop_fn` must have been created with the true value of `ptr` in mind.
                drop_fn(ptr);
            };
        }
        drop(collecting_guard);
    }
}

/// Build out a part of the reference graph, making note of all allocations which are reachable from
/// the one described in `ptr`.
///
/// # Inputs
///
/// - `ptr`: A pointer to the allocation that we should start constructing from.
/// - `ref_graph`: A lookup from allocation IDs to node information about that allocation.
///
/// # Effects
///
/// `ref_graph` will be expanded to include all allocations reachable from `ptr`.
///
/// # Safety
///
/// `ptr` must have been created as a pointer to a `PyInner<T>`.
unsafe fn dfs<T: Traverse + Send + Sync + ?Sized>(
    ptr: Erased,
    ref_graph: &mut HashMap<AllocationId, AllocationInfo>,
) {
    let box_ref = unsafe {
        // SAFETY: We require `ptr` to be a an erased pointer to `PyInner<T>`.
        ptr.specify::<PyInner<T>>().as_ref()
    };
    let starting_id = AllocationId::from(box_ref);
    let Entry::Vacant(v) = ref_graph.entry(starting_id) else {
        // the weak count was incremented by another DFS operation elsewhere.
        // Decrement it to have only one from us.
        box_ref.ref_count.weak.fetch_sub(1, Ordering::Release);
        return;
    };
    let strong_count = box_ref.ref_count.strong.load(Ordering::Acquire);
    v.insert(AllocationInfo {
        ptr,
        weak_drop_fn: drop_weak_zero::<T>,
        reachability: Reachability::Unknown {
            children: Vec::new(),
            n_unaccounted: strong_count,
            destroy_fn: destroy_erased::<T>,
        },
    });

    if box_ref
        .value
        .accept(&mut Dfs {
            ref_graph,
            current_id: starting_id,
        })
        .is_err()
        || box_ref.ref_count.generation.load(Ordering::Acquire) >= CURRENT_TAG.load(Ordering::Relaxed)
    {
        // box_ref.value was accessed while we worked
        // mark this allocation as reachable
        mark(starting_id, ref_graph);
    }
}

#[derive(Debug)]
/// The visitor structure used for building the found-reference-graph of allocations.
struct Dfs<'a> {
    /// The reference graph.
    /// Each allocation is assigned a node.
    ref_graph: &'a mut HashMap<AllocationId, AllocationInfo>,
    /// The allocation ID currently being visited.
    /// Used for knowing which node is the parent of another.
    current_id: AllocationId,
}

impl<'a> Visitor for Dfs<'a> {
    fn visit_sync<T>(&mut self, gc: &PyObjectRef)
    where
        T: Traverse + Send + Sync + ?Sized,
    {
        // must not use deref operators since we don't want to update the generation
        let ptr = unsafe {
            // SAFETY: This is the same as the deref implementation, but avoids
            // incrementing the generation count.
            (*gc.ptr.get()).unwrap()
        };
        let box_ref = unsafe {
            // SAFETY: same as above.
            ptr.as_ref()
        };
        let current_tag = CURRENT_TAG.load(Ordering::Relaxed);
        if gc.tag.swap(current_tag, Ordering::Relaxed) >= current_tag
            || box_ref.generation.load(Ordering::Acquire) >= current_tag
        {
            // This pointer was already tagged by this sweep, so it must have been moved by
            mark(self.current_id, self.ref_graph);
            return;
        }

        let mut new_id = AllocationId::from(box_ref);

        let Reachability::Unknown {
            ref mut children, ..
        } = self
            .ref_graph
            .get_mut(&self.current_id)
            .unwrap()
            .reachability
        else {
            // this node has been proven reachable by something higher up. No need to keep building
            // its ref graph
            return;
        };
        children.push(new_id);

        match self.ref_graph.entry(new_id) {
            Entry::Occupied(mut o) => match o.get_mut().reachability {
                Reachability::Unknown {
                    ref mut n_unaccounted,
                    ..
                } => {
                    *n_unaccounted -= 1;
                }
                Reachability::Reachable => (),
            },
            Entry::Vacant(v) => {
                // This allocation has never been visited by the reference graph builder
                let strong_count = box_ref.strong.load(Ordering::Acquire);
                box_ref.weak.fetch_add(1, Ordering::Acquire);
                v.insert(AllocationInfo {
                    ptr: Erased::new(ptr),
                    weak_drop_fn: drop_weak_zero::<T>,
                    reachability: Reachability::Unknown {
                        children: Vec::new(),
                        n_unaccounted: strong_count - 1,
                        destroy_fn: destroy_erased::<T>,
                    },
                });

                // Save the previously visited ID, then carry on to the next one
                swap(&mut new_id, &mut self.current_id);

                if box_ref.value.accept(self).is_err()
                    || box_ref.generation.load(Ordering::Acquire) >= current_tag
                {
                    // On failure, this means `**gc` is accessible, and should be marked
                    // as such
                    mark(self.current_id, self.ref_graph);
                }

                // Restore current_id and carry on
                swap(&mut new_id, &mut self.current_id);
            }
        };
    }
}

/// Traverse the reference graph, marking `root` and any allocations reachable from `root` as
/// reachable.
fn mark(root: AllocationId, graph: &mut HashMap<AllocationId, AllocationInfo>) {
    let node = graph.get_mut(&root).unwrap();
    if let Reachability::Unknown { children, .. } =
        replace(&mut node.reachability, Reachability::Reachable)
    {
        for child in children {
            mark(child, graph);
        }
    }
}

/// Destroy an allocation, obliterating its GCs, dropping it, and deallocating it.
///
/// # Safety
///
/// `ptr` must have been created from a pointer to a `PyInner<T>`.
unsafe fn destroy_erased<T: Traverse + Send + Sync + ?Sized>(
    ptr: Erased,
    graph: &HashMap<AllocationId, AllocationInfo>,
) {
    /// A visitor for decrementing the reference count of pointees.
    struct PrepareForDestruction<'a> {
        /// The reference graph.
        /// Must have been populated with reachability already.
        graph: &'a HashMap<AllocationId, AllocationInfo>,
    }

    impl Visitor for PrepareForDestruction<'_> {
        fn visit_sync<T>(&mut self, gc: &PyObjectRef)
        where
            T: Traverse + Send + Sync + ?Sized,
        {
            let id = AllocationId::from(unsafe {
                // SAFETY: This is the same as dereferencing the GC.
                (*gc.ptr.get()).unwrap()
            });
            if matches!(self.graph[&id].reachability, Reachability::Reachable) {
                unsafe {
                    // SAFETY: This is the same as dereferencing the GC.
                    id.0.as_ref().ref_count.strong.fetch_sub(1, Ordering::Release);
                }
            } else {
                unsafe {
                    // SAFETY: The GC is unreachable,
                    // so the GC will never be dereferenced again.
                    gc.ptr.get().write((*gc.ptr.get()).as_null());
                }
            }
        }
    }

    let specified = ptr.specify::<PyInner<T>>().as_mut();
    specified
        .value
        .accept(&mut PrepareForDestruction { graph })
        .expect("allocation assumed to be unreachable but somehow was accessed");
    let layout = Layout::for_value(specified);
    drop_in_place(specified);
    dealloc(std::ptr::from_mut::<PyInner<T>>(specified).cast(), layout);
}

/// Function for handling dropping an allocation when its weak and strong reference count reach
/// zero.
///
/// # Safety
///
/// `ptr` must have been created as a pointer to a `PyInner<T>`.
unsafe fn drop_weak_zero<T: Traverse + Send + Sync + ?Sized>(ptr: Erased) {
    let mut specified = ptr.specify::<PyInner<T>>();
    assert_eq!(specified.as_ref().ref_count.weak.load(Ordering::Relaxed), 0);
    assert_eq!(specified.as_ref().ref_count.strong.load(Ordering::Relaxed), 0);

    let layout = Layout::for_value(specified.as_ref());
    drop_in_place(specified.as_mut());
    dealloc(specified.as_ptr().cast(), layout);
}

unsafe impl Send for AllocationId {}
unsafe impl Sync for AllocationId {}

impl<T> From<&PyInner<T>> for AllocationId
where
    T: Traverse + Send + Sync + ?Sized,
{
    fn from(value: &PyInner<T>) -> Self {
        AllocationId(NonNull::from(value).cast())
    }
}

impl<T> From<NonNull<PyInner<T>>> for AllocationId
where
    T: Traverse + Send + Sync + ?Sized,
{
    fn from(value: NonNull<PyInner<T>>) -> Self {
        AllocationId(value.cast())
    }
}

impl Drop for Dumpster {
    fn drop(&mut self) {
        self.deliver_to(&GARBAGE_TRUCK);
        // collect_all();
    }
}

impl Drop for GarbageTruck {
    fn drop(&mut self) {
        GARBAGE_TRUCK.collect_all();
    }
}
