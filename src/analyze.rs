use crate::object::*;
use petgraph::algo::dominators;
use petgraph::graph::NodeIndex;
use petgraph::visit::Dfs;
use petgraph::Graph;
use std::collections::{HashMap, HashSet};
use std::iter::Iterator;
use timed_function::timed;

type Index = NodeIndex<usize>;

#[derive(Debug)]
pub struct Analysis {
    // Root (of full graph, or of subgraph).
    root: Index,

    // All nodes dominated by root (this means all reachable nodes if root
    // is the root of the full graph).
    dominated_subgraph: ReferenceGraph,

    // If root is the original root:
    //  All unreachable nodes.
    // If root is a subgraph root:
    //  All nodes from the original graph that _are_ reachable from this
    //  root, but are _not_ dominated by it.
    rest: Vec<Object>,

    // Dominator index for each node in the dominated subgraph.
    dominators: HashMap<Index, Index>,

    // Size of each dominator subtree.
    subtree_sizes: HashMap<Index, Stats>,
}

#[timed]
pub fn analyze(orig_root: Index, subgraph_root: Index, graph: ReferenceGraph) -> Analysis {
    let dominators = find_dominators(orig_root, &graph);

    let (root, dominated_subgraph, rest, dominators) = if subgraph_root == orig_root {
        remove_unreachable(orig_root, &graph, &dominators)
    } else {
        extract_dominated_subgraph(subgraph_root, &graph, &dominators)
    };

    let subtree_sizes = dominator_subtree_sizes(&dominated_subgraph, &dominators);

    Analysis {
        root,
        dominated_subgraph,
        rest,
        dominators,
        subtree_sizes,
    }
}

#[timed]
fn find_dominators(root: Index, graph: &ReferenceGraph) -> HashMap<Index, Index> {
    let dominators = dominators::simple_fast(&graph, root);

    // Convert dominators to map because we need a more flexible data structure;
    // this would be unnecessary if the Dominators struct exposed its internals.
    let mut map = HashMap::new();
    for i in graph.node_indices() {
        if let Some(d) = dominators.immediate_dominator(i) {
            map.insert(i, d);
        }
    }
    map
}

#[timed]
fn remove_unreachable(
    root: Index,
    graph: &ReferenceGraph,
    dominators: &HashMap<Index, Index>,
) -> (Index, ReferenceGraph, Vec<Object>, HashMap<Index, Index>) {
    // We take advantage of the fact that all reachable nodes have a dominator
    // to traverse the graph just once while both sorting reachable/unreachable
    // and translating domination edges into address terms
    let (reachable, unreachable, dominator_addrs) = {
        let mut unreachable: Vec<Object> = Vec::new();
        let mut dominator_addrs: HashMap<usize, usize> = HashMap::new();

        let reachable = graph.filter_map(
            |i, w| {
                if i == root {
                    Some(w.clone())
                } else if let Some(&d) = dominators.get(&i) {
                    dominator_addrs.insert(w.address, graph[d].address);
                    Some(w.clone())
                } else {
                    unreachable.push(w.clone());
                    None
                }
            },
            |_, e| Some(*e),
        );

        (reachable, unreachable, dominator_addrs)
    };

    // Cheap sanity checks
    assert_eq!(
        reachable.node_count() + unreachable.len(),
        graph.node_count()
    );
    assert!(dominator_addrs.len() <= reachable.node_count());

    // Prove that our optimization above does not change results vs checking reachability
    // separately
    debug_assert!(reachable.node_count() == find_reachable_indices(root, graph).len());

    let (root, dominators) = map_indices(&reachable, &dominator_addrs, graph[root].address);
    (root, reachable, unreachable, dominators)
}

#[timed]
fn extract_dominated_subgraph(
    root: Index,
    graph: &ReferenceGraph,
    dominators: &HashMap<Index, Index>,
) -> (Index, ReferenceGraph, Vec<Object>, HashMap<Index, Index>) {
    let reachable = find_reachable_indices(root, graph);
    let dominator_addrs = find_addrs_of_filtered_edges(root, &reachable, dominators, graph);

    let (dominated, rest) = {
        let mut not_dominated: Vec<Object> = Vec::new();

        let dominated = graph.filter_map(
            |i, w| {
                if i == root || dominator_addrs.contains_key(&w.address) {
                    Some(w.clone())
                } else if reachable.contains(&i) {
                    not_dominated.push(w.clone());
                    None
                } else {
                    None
                }
            },
            |_, e| Some(*e),
        );

        (dominated, not_dominated)
    };

    // Cheap sanity checks
    assert!(reachable.len() <= graph.node_count());
    assert!(dominator_addrs.len() <= graph.node_count());
    assert!(dominator_addrs.len() <= dominators.len());
    assert!(dominator_addrs.len() <= reachable.len());
    assert_eq!(dominated.node_count() + rest.len(), reachable.len());
    assert!(dominated.node_count() <= dominator_addrs.len() + 1);

    // Prove that the optimization of passing the reachable set to `find_addrs_of_filtered_edges`
    // does not change results
    debug_assert_eq!(
        dominator_addrs.len(),
        find_addrs_of_filtered_edges(root, &graph.node_indices().collect(), dominators, graph)
            .len()
    );

    let (root, dominators) = map_indices(&dominated, &dominator_addrs, graph[root].address);
    (root, dominated, rest, dominators)
}

#[timed]
fn find_addrs_of_filtered_edges(
    root: Index,
    reachable: &HashSet<Index>,
    tree_edges: &HashMap<Index, Index>,
    graph: &ReferenceGraph,
) -> HashMap<usize, usize> {
    let mut result: HashMap<usize, usize> = HashMap::new();

    // Re-usable buffer
    let mut descendents: Vec<Index> = Vec::new();

    for (c, p) in tree_edges.iter() {
        let mut child = *c;
        let mut parent = *p;

        loop {
            if !reachable.contains(&parent) {
                // We've proved this subtree is _not_ rooted at this root
                // (this an optimization; we'll get the same results if we
                // never hit this case)
                break;
            } else if parent == root || result.contains_key(&graph[parent].address) {
                // We've proved this subtree _is_ rooted at this root
                result.insert(graph[child].address, graph[parent].address);
                parent = child;
                for &child in descendents.iter().rev() {
                    result.insert(graph[child].address, graph[parent].address);
                    parent = child;
                }
                break;
            } else if let Some(&grandparent) = tree_edges.get(&parent) {
                // We need to keep checking
                descendents.push(child);
                child = parent;
                parent = grandparent;
            } else {
                // We've proved this subtree is _not_ rooted at this root
                break;
            }
        }

        descendents.clear();
    }

    result
}

#[timed]
fn find_reachable_indices(root: Index, graph: &ReferenceGraph) -> HashSet<Index> {
    let mut reachable: HashSet<Index> = HashSet::new();
    reachable.insert(root);

    let mut dfs = Dfs::new(&graph, root);
    while let Some(i) = dfs.next(&graph) {
        reachable.insert(i);
    }

    reachable
}

fn map_indices(
    graph: &ReferenceGraph,
    addr_edges: &HashMap<usize, usize>,
    root: usize,
) -> (Index, HashMap<Index, Index>) {
    let index_by_addr = {
        let mut index_by_addr: HashMap<usize, Index> = HashMap::with_capacity(graph.node_count());
        for i in graph.node_indices() {
            index_by_addr.insert(graph[i].address, i);
        }
        index_by_addr
    };

    let mapped_edges = {
        let mut mapped_edges: HashMap<Index, Index> = HashMap::new();
        for (a, d) in addr_edges {
            let i = index_by_addr[&a];
            let j = index_by_addr[&d];
            mapped_edges.insert(i, j);
        }
        mapped_edges
    };

    (index_by_addr[&root], mapped_edges)
}

fn dominator_subtree_sizes(
    graph: &ReferenceGraph,
    dominators: &HashMap<Index, Index>,
) -> HashMap<Index, Stats> {
    let mut subtree_sizes: HashMap<Index, Stats> = HashMap::new();

    // Assign each node's stats to itself
    for i in graph.node_indices() {
        subtree_sizes.insert(i, graph[i].stats());
    }

    // Assign each node's stats to all of its dominators
    for mut i in graph.node_indices() {
        let stats = graph[i].stats();
        while let Some(&d) = dominators.get(&i) {
            subtree_sizes.entry(d).and_modify(|e| *e = (*e).add(stats));
            i = d;
        }
    }

    subtree_sizes
}

fn by_kind<'a, I: Iterator<Item = (&'a Object, Stats)>>(objs: I) -> HashMap<&'a String, Stats> {
    objs.fold(HashMap::new(), |mut by_kind, (obj, stats)| {
        by_kind
            .entry(&obj.kind)
            .and_modify(|c| *c = (*c).add(stats))
            .or_insert(stats);
        by_kind
    })
}

fn largest_and_rest<'a, K, I: Iterator<Item = (&'a K, Stats)>>(
    iter: I,
    count: usize,
) -> (Vec<(&'a K, Stats)>, Stats) {
    let sorted = {
        let mut vec: Vec<(&'a K, Stats)> = iter.collect();
        vec.sort_unstable_by_key(|(_, c)| usize::max_value() - c.bytes);
        vec
    };

    if count >= sorted.len() {
        (sorted, Stats::default())
    } else {
        (
            sorted[0..count].iter().cloned().collect(),
            sorted[count..]
                .iter()
                .fold(Stats::default(), |mut acc, (_, c)| acc.add(*c)),
        )
    }
}

impl Analysis {
    pub fn live_stats_by_kind(&self, top_n: usize) -> (Vec<(&String, Stats)>, Stats) {
        let stats = by_kind(
            self.dominated_subgraph
                .node_indices()
                .map(|i| {
                    let obj = &self.dominated_subgraph[i];
                    (obj, obj.stats())
                }),
        );
        largest_and_rest(stats.iter().map(|(k, v)| (*k, *v)), top_n)
    }

    pub fn retained_stats_by_kind(&self, top_n: usize) -> (Vec<(&String, Stats)>, Stats) {
        let stats = by_kind(
            self.dominated_subgraph
                .node_indices()
                .map(|i| {
                    let obj = &self.dominated_subgraph[i];
                    (obj, self.subtree_sizes[&i])
                }),
        );
        largest_and_rest(stats.iter().map(|(k, v)| (*k, *v)), top_n)
    }

    pub fn unreachable_stats_by_kind(&self, top_n: usize) -> (Vec<(&String, Stats)>, Stats) {
        let stats = by_kind(self.rest.iter().map(|o| (o, o.stats())));
        largest_and_rest(stats.iter().map(|(k, v)| (*k, *v)), top_n)
    }

    pub fn dominator_subtree_stats(&self, top_n: usize) -> (Vec<(&Object, Stats)>, Stats) {
        let (largest, rest) =
            largest_and_rest(self.subtree_sizes.iter().map(|(k, v)| (k, *v)), top_n);
        (
            largest
                .into_iter()
                .map(|(i, stats)| (&self.dominated_subgraph[*i], stats))
                .collect(),
            rest,
        )
    }

    pub fn relevant_dominator_subgraph(&self, relevance_threshold: f64) -> ReferenceGraph {
        let threshold_bytes =
            (self.dominated_totals().bytes as f64 * relevance_threshold).floor() as usize;

        let mut subgraph: ReferenceGraph = Graph::default();
        let mut old_to_new: HashMap<Index, Index> = HashMap::new();

        for (i, stats) in self
            .subtree_sizes
            .iter()
            .filter(|(_, stats)| stats.bytes >= threshold_bytes)
        {
            let obj = &self.dominated_subgraph[*i];
            let added = subgraph.add_node(obj.with_dominator_stats(*stats));
            old_to_new.insert(*i, added);
        }

        for (old, new) in old_to_new.iter() {
            if let Some(d) = self.dominators.get(old) {
                subgraph.add_edge(old_to_new[&d], *new, EDGE_WEIGHT);
            }
        }

        subgraph
    }

    pub fn dominated_totals(&self) -> Stats {
        self.subtree_sizes[&self.root]
    }
}
