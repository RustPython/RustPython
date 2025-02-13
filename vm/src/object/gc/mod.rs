mod collect;
mod dumpster;
pub mod erased;
pub mod refcount;
mod visitor;

pub(crate) use dumpster::{default_collect_condition, CollectCondition, CollectInfo, CURRENT_TAG};
pub(crate) use visitor::Visitor;

pub fn try_gc() {
    // TODO: conditionally collect
    dumpster::collect();
}