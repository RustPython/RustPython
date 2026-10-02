//! A collection snapshot: compact indices, with no object access during analysis.

use super::*;

struct Node {
    object: PyObjectRef,
    refs: usize,
    end: usize,
}

pub(super) struct Graph {
    nodes: Vec<Node>,
    edges: Vec<u32>,
}

struct ResetIndices<'a>(&'a [Node]);

impl Drop for ResetIndices<'_> {
    fn drop(&mut self) {
        for node in self.0 {
            node.object.end_gc_refs();
        }
    }
}

impl Graph {
    /// All candidates must have exactly one collector-owned reference in
    /// `objects`, and belong to one heap. Its payloads must be stable under
    /// the owner's stop-the-world or permanent retirement. Each child's
    /// temporary header index is valid only for this capture. Duplicate owned edges are
    /// retained, since each contributes one strong reference.
    pub(super) unsafe fn capture(objects: Vec<PyObjectRef>) -> Self {
        let nodes: Vec<_> = objects
            .into_iter()
            .map(|object| Node {
                refs: object.strong_count().saturating_sub(1),
                object,
                end: 0,
            })
            .collect();
        let mut graph = Self {
            nodes,
            edges: Vec::new(),
        };
        let heap = graph
            .nodes
            .first()
            .map(|node| core::ptr::from_ref(node.object.gc_handle().heap().unwrap()));
        debug_assert!(
            graph
                .nodes
                .iter()
                .all(|node| { node.object.gc_handle().heap().map(core::ptr::from_ref) == heap })
        );
        if graph.nodes.len() > u32::MAX as usize {
            // More nodes than the compact index can represent: conservatively
            // retain them, rather than wrap and subtract from another object.
            for node in &mut graph.nodes {
                node.refs = usize::MAX;
            }
            return graph;
        }
        {
            let reset = ResetIndices(&graph.nodes);
            for (index, node) in reset.0.iter().enumerate() {
                node.object.start_gc_index(index as u32);
            }
            // Offsets live separately while ResetIndices borrows all nodes.
            let mut ends = Vec::with_capacity(graph.nodes.len());
            for node in reset.0 {
                unsafe {
                    node.object.gc_visit_referents(&mut |child| {
                        if child.gc_handle().heap().map(core::ptr::from_ref) == heap
                            && child.is_gc_collecting()
                        {
                            graph.edges.push(child.gc_refs());
                        }
                    });
                }
                ends.push(graph.edges.len());
            }
            drop(reset);
            for (node, end) in graph.nodes.iter_mut().zip(ends) {
                node.end = end;
            }
        }
        graph
    }

    /// Only integer arrays are read here. Python threads may run concurrently:
    /// object lifetimes are pinned, and later revalidation handles mutations.
    pub(super) fn analyze(&mut self) {
        for &child in &self.edges {
            let refs = &mut self.nodes[child as usize].refs;
            if *refs != usize::MAX {
                *refs = refs.saturating_sub(1);
            }
        }
        let mut work = Vec::new();
        for (index, node) in self.nodes.iter_mut().enumerate() {
            if node.refs != 0 {
                node.refs = usize::MAX;
                work.push(index);
            }
        }
        while let Some(index) = work.pop() {
            let start = if index == 0 {
                0
            } else {
                self.nodes[index - 1].end
            };
            let end = self.nodes[index].end;
            for &child in &self.edges[start..end] {
                let child = child as usize;
                if self.nodes[child].refs != usize::MAX {
                    self.nodes[child].refs = usize::MAX;
                    work.push(child);
                }
            }
        }
    }

    pub(super) fn partition(self) -> (Vec<PyObjectRef>, Vec<PyObjectRef>) {
        let mut live = Vec::new();
        let mut dead = Vec::new();
        for node in self.nodes {
            if node.refs == usize::MAX {
                live.push(node.object);
            } else {
                dead.push(node.object);
            }
        }
        (live, dead)
    }
}
