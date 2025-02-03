// This file is a modified copy of src/sync/mod.rs from the dumpster crate
// Original source: https://github.com/claytonwramsey/dumpster/blob/bc197d2f875aadae086e1ba3dd7da6d29ffee6fa/dumpster/src/sync/mod.rs
/*
    dumpster, acycle-tracking garbage collector for Rust.    Copyright (C) 2023 Clayton Ramsey.

    This Source Code Form is subject to the terms of the Mozilla Public
    License, v. 2.0. If a copy of the MPL was not distributed with this
    file, You can obtain one at http://mozilla.org/MPL/2.0/.
*/

//! Thread-safe shared garbage collection.
//!
//! Most users of this module will be interested in using [`Gc`] directly out of the box - this will
//! just work.
//! Those with more particular needs (such as benchmarking) should turn toward
//! [`set_collect_condition`] in order to tune exactly when the garbage collector does cleanups.

use std::{
    fmt::Debug,
    sync::atomic::AtomicUsize,
};

/// The tag of the current sweep operation.
/// All new allocations are minted with the current tag.
pub(crate) static CURRENT_TAG: AtomicUsize = AtomicUsize::new(0);

/// Begin a collection operation of the allocations on the heap.
///
/// Due to concurrency issues, this might not collect every single unreachable allocation that
/// currently exists, but often calling `collect()` will get allocations made by this thread.
pub fn collect() {
    collect_all_await();
}

#[derive(Debug)]
/// Information passed to a [`CollectCondition`] used to determine whether the garbage collector
/// should start collecting.
///
/// A `CollectInfo` is exclusively created by being passed as an argument to the collection
/// condition.
/// To set a custom collection condition, refer to [`set_collect_condition`].
/// ```
pub struct CollectInfo {
    /// Dummy value so this is a private structure.
    pub(super) _private: (),
}

/// A function which determines whether the garbage collector should start collecting.
/// This type primarily exists so that it can be used with [`set_collect_condition`].
pub type CollectCondition = fn(&CollectInfo) -> bool;

#[must_use]
/// The default collection condition used by the garbage collector.
///
/// There are no guarantees about what this function returns, other than that it will return `true`
/// with sufficient frequency to ensure that all `Gc` operations are amortized _O(1)_ in runtime.
///
/// This function isn't really meant to be called by users, but rather it's supposed to be handed
/// off to [`set_collect_condition`] to return to the default operating mode of the library.
///
/// This collection condition applies globally, i.e. to every thread.

pub fn default_collect_condition(info: &CollectInfo) -> bool {
    info.n_gcs_dropped_since_last_collect() > info.n_gcs_existing()
}

use crate::object::gc::collect::{collect_all_await, n_gcs_dropped, n_gcs_existing};

// TODO: unused
// notify_created_gc();


impl CollectInfo {
    #[must_use]
    /// Get the number of times that a [`Gc`] has been dropped since the last time a collection
    /// operation was performed.
    pub fn n_gcs_dropped_since_last_collect(&self) -> usize {
        n_gcs_dropped()
    }

    #[must_use]
    /// Get the total number of [`Gc`]s which currently exist.
    pub fn n_gcs_existing(&self) -> usize {
        n_gcs_existing()
    }
}
