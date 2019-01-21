// Adapted from https://github.com/CraneStation/cranelift/blob/990e1a427691002ebeeaa06ce433970894608b27/lib/codegen/src/dominator_tree.rs
//
// Code from Cranelift copyright Cranelift devs & available under the Apache 2.0 license.

use std::cmp::Ordering;

/// We assume the node at index zero in the adjacency list is the root
const ROOT: usize = 0;

/// RPO numbers are not first assigned in a contiguous way but as multiples of STRIDE, to leave
/// room for modifications of the dominator tree.
const STRIDE: usize = 4;

/// Special RPO numbers used during `compute_postorder`.
const DONE: usize = 1;
const SEEN: usize = 2;

#[derive(Clone, Default)]
struct DomNode {
    /// Predecessors in the input graph (stored here for convenience).
    predecessors: Vec<usize>,

    /// Number of this node in a reverse post-order traversal of the input graph,
    /// starting from 1.
    ///
    /// This number is monotonic in the reverse postorder but not contiguous,
    /// since we leave holes for later localized modifications of the dominator
    /// tree.
    ///
    /// Unreachable nodes get number 0, all others are positive.
    rpo_number: usize,

    /// The immediate dominator of this node.
    ///
    /// This is `None` for unreachable nodes and root.
    idom: Option<usize>,
}

pub struct DominatorTree {
    nodes: Vec<DomNode>,
    postorder: Vec<usize>,
}

impl DominatorTree {
    /// Allocate and compute a dominator tree, given an adjacency list.
    ///
    /// Assumes that the root node is at index zero.
    pub fn from_graph(adj_list: &[Vec<usize>]) -> Self {
        let mut domtree = Self::new();
        domtree.compute(adj_list);
        domtree
    }

    /// Allocate a new blank dominator tree. Use `compute` to compute the dominator tree for a
    /// function.
    fn new() -> Self {
        Self {
            nodes: Vec::new(),
            postorder: Vec::new(),
        }
    }

    /// Compute post-order and dominator tree.
    fn compute(&mut self, adj_list: &[Vec<usize>]) {
        self.nodes.resize(adj_list.len(), DomNode::default());
        self.compute_postorder(adj_list);
        self.compute_domtree();
    }

    /// Compute a post-order of the input graph.
    ///
    /// This leaves `rpo_number == 1` for all reachable nodes, 0 for unreachable ones.
    fn compute_postorder(&mut self, adj_list: &[Vec<usize>]) {
        let mut stack = Vec::new();

        // This algorithm is a depth first traversal (DFT) of the graph, computing a
        // post-order of the nodes that are reachable. A DFT post-order is not
        // unique. The specific order we get is controlled by two factors:
        //
        // During this algorithm only, use `rpo_number` to hold the following state:
        //
        //   0:    Node has not yet been reached in the pre-order.
        //   SEEN: Node has been pushed on the stack but successors not yet pushed.
        //   DONE: Successors pushed.
        stack.push(ROOT);
        self.nodes[ROOT].rpo_number = SEEN;

        while let Some(node) = stack.pop() {
            match self.nodes[node].rpo_number {
                SEEN => {
                    // This is the first time we pop the node, so we need to scan its successors and
                    // then revisit it.
                    self.nodes[node].rpo_number = DONE;
                    stack.push(node);

                    // Push each successor onto `stack` if it has not already been seen.
                    for succ in adj_list[node].clone() {
                        if self.nodes[succ].rpo_number == 0 {
                            self.nodes[succ].rpo_number = SEEN;
                            stack.push(succ);
                        }

                        self.nodes[succ].predecessors.push(node);
                    }
                }
                DONE => {
                    // This is the second time we pop the node, so all successors have been
                    // processed.
                    self.postorder.push(node);
                }
                _ => unreachable!(),
            }
        }
    }

    /// Build a dominator tree from an adjacency list using Keith D. Cooper's
    /// "Simple, Fast Dominator Algorithm."
    fn compute_domtree(&mut self) {
        // During this algorithm, `rpo_number` has the following values:
        //
        // 0: Node is not reachable.
        // 1: Node is reachable, but has not yet been visited during the first pass. This is set by
        // `compute_postorder`.
        // 2+: Node is reachable and has an assigned RPO number.

        // We'll be iterating over a reverse post-order of the input graph, skipping the root.
        debug_assert_eq!(Some(ROOT), self.postorder.pop());

        // Do a first pass where we assign RPO numbers to all reachable nodes.
        self.nodes[ROOT].rpo_number = 2 * STRIDE;
        for (rpo_idx, &node) in self.postorder.iter().rev().enumerate() {
            // Update the current node and give it an RPO number.
            // The root gets 2, the rest start at 3 by multiples of STRIDE to leave
            // room for future dominator tree modifications.
            //
            // Since `compute_idom` will only look at nodes with an assigned RPO number, the
            // function will never see an uninitialized predecessor.
            //
            // Due to the nature of the post-order traversal, every node we visit will have at
            // least one predecessor that has previously been visited during this RPO.
            self.nodes[node].idom = Some(self.compute_idom(node));
            self.nodes[node].rpo_number = (rpo_idx + 3) * STRIDE;
        }

        // Now that we have RPO numbers for everything and initial immediate dominator estimates,
        // iterate until convergence.
        let mut changed = true;
        while changed {
            changed = false;
            for &node in self.postorder.iter().rev() {
                let idom = Some(self.compute_idom(node));
                if self.nodes[node].idom != idom {
                    self.nodes[node].idom = idom;
                    changed = true;
                }
            }
        }
    }

    /// Compute the immediate dominator for `node` using the current `idom` states
    /// for the reachable nodes.
    fn compute_idom(&self, node: usize) -> usize {
        // Get an iterator with just the reachable, already visited predecessors to `node`.
        // Note that during the first pass, `rpo_number` is 1 for reachable blocks that haven't
        // been visited yet, 0 for unreachable blocks.
        let mut reachable_preds = self.nodes[node]
            .predecessors
            .iter()
            .filter(|pred| self.nodes[**pred].rpo_number > 1);

        // The RPO must visit at least one predecessor before this node.
        let mut idom = *reachable_preds
            .next()
            .expect("Node must have one reachable predecessor");

        for pred in reachable_preds {
            idom = self.common_dominator(idom, *pred);
        }

        idom
    }

    /*
    /// Is `node` reachable from the entry block?
    pub fn is_reachable(&self, node: usize) -> bool {
        self.nodes[node].rpo_number != 0
    }*/

    /// Returns the immediate dominator of `node`.
    ///
    /// The *immediate dominator* is the dominator that is closest to `node`. All other dominators
    /// also dominate the immediate dominator.
    ///
    /// This returns `None` if `node` is not reachable from the root, or is a root.
    pub fn idom(&self, node: usize) -> Option<usize> {
        self.nodes[node].idom
    }

    /// Compare two nodes relative to the reverse post-order.
    fn rpo_cmp(&self, a: usize, b: usize) -> Ordering {
        self.nodes[a].rpo_number.cmp(&self.nodes[b].rpo_number)
    }

    /*
    /// Returns `true` if `a` dominates `b`.
    ///
    /// A node is considered to dominate itself.
    pub fn dominates(&self, a: usize, mut b: usize) -> bool {
        let rpo_a = self.nodes[a].rpo_number;

        // Run a finger up the dominator tree from b until we see a.
        // Do nothing if b is unreachable.
        while rpo_a < self.nodes[b].rpo_number {
            b = match self.idom(b) {
                Some(idom) => idom,
                None => return false, // a is unreachable, so we climbed past the entry
            };
        }
        a == b
    }*/

    /// Compute the common dominator of two nodes.
    ///
    /// Both nodes are assumed to be reachable.
    fn common_dominator(&self, mut a: usize, mut b: usize) -> usize {
        loop {
            match self.rpo_cmp(a, b) {
                Ordering::Less => {
                    // `a` comes before `b` in the RPO. Move `b` up.
                    b = self.nodes[b].idom.expect("Unreachable node?");
                }
                Ordering::Greater => {
                    // `b` comes before `a` in the RPO. Move `a` up.
                    a = self.nodes[a].idom.expect("Unreachable node?");
                }
                Ordering::Equal => break,
            }
        }

        debug_assert_eq!(a, b, "Unreachable node passed to common_dominator?");

        a
    }
}
