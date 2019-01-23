extern crate bytesize;
#[macro_use]
extern crate clap;
#[macro_use]
extern crate serde;
extern crate petgraph;
extern crate serde_json;

use bytesize::ByteSize;
use petgraph::algo::dominators;
use petgraph::algo::dominators::Dominators;
use petgraph::dot;
use petgraph::graph::NodeIndex;
use petgraph::{Directed, Graph};
use std::collections::HashMap;
use std::fmt::Display;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::prelude::*;
use std::io::BufReader;

#[derive(Debug, Deserialize)]
struct Line {
    address: Option<String>,
    memsize: Option<usize>,

    #[serde(default)]
    references: Vec<String>,

    #[serde(rename = "type")]
    object_type: String,

    class: Option<String>,
    root: Option<String>,
    name: Option<String>,
    length: Option<usize>,
    size: Option<usize>,
    value: Option<String>,
}

#[derive(Debug)]
struct ParsedLine {
    object: Object,
    references: Vec<usize>,
    module: Option<usize>,
    name: Option<String>,
}

#[derive(Debug, Clone)]
struct Object {
    address: usize,
    bytes: usize,
    kind: String,
    label: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
struct Stats {
    count: usize,
    bytes: usize,
}

const DEFAULT_RELEVANCE_THRESHOLD: f64 = 0.005;

impl Line {
    pub fn parse(self) -> Option<ParsedLine> {
        let mut object = Object {
            address: self
                .address
                .as_ref()
                .and_then(|a| Line::parse_address(a.as_str()).ok())
                .unwrap_or(0),
            bytes: self.memsize.unwrap_or(0),
            kind: self.object_type,
            label: None,
        };

        if object.address == 0 && object.kind != "ROOT" {
            return None;
        }

        object.label = match object.kind.as_str() {
            "CLASS" | "MODULE" | "ICLASS" => self
                .name
                .clone()
                .map(|n| format!("{}[{:#x}][{}]", n, object.address, object.kind)),
            "ARRAY" => Some(format!(
                "Array[{:#x}][len={}]",
                object.address, self.length?
            )),
            "HASH" => Some(format!("Hash[{:#x}][size={}]", object.address, self.size?)),
            "STRING" => self.value.map(|v| {
                let prefix = v
                    .chars()
                    .take(40)
                    .flat_map(|c| {
                        // Hacky escape to prevent dot format from breaking
                        if c.is_control() {
                            None
                        } else if c == '\\' {
                            Some('﹨')
                        } else {
                            Some(c)
                        }
                    })
                    .collect::<String>();
                let ellipsis = if v.chars().nth(41).is_some() {
                    "…"
                } else {
                    ""
                };
                format!("String[{:#x}][{}{}]", object.address, prefix, ellipsis)
            }),
            _ => None,
        };

        Some(ParsedLine {
            references: self
                .references
                .iter()
                .flat_map(|r| Line::parse_address(r.as_str()).ok())
                .collect(),
            module: self
                .class
                .and_then(|c| Line::parse_address(c.as_str()).ok()),
            name: self.name,
            object,
        })
    }

    pub fn parse_address(addr: &str) -> Result<usize, std::num::ParseIntError> {
        usize::from_str_radix(&addr[2..], 16)
    }
}

impl Object {
    pub fn stats(&self) -> Stats {
        Stats {
            count: 1,
            bytes: self.bytes,
        }
    }

    pub fn root() -> Object {
        Object {
            address: 0,
            bytes: 0,
            kind: "ROOT".to_string(),
            label: Some("root".to_string()),
        }
    }

    pub fn is_root(&self) -> bool {
        self.address == 0
    }
}

impl PartialEq for Object {
    fn eq(&self, other: &Object) -> bool {
        self.address == other.address
    }
}
impl Eq for Object {}

impl Hash for Object {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.address.hash(state);
    }
}

impl Display for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        if let Some(ref label) = self.label {
            write!(f, "{}", label)
        } else {
            write!(f, "{}[{:#x}]", self.kind, self.address)
        }
    }
}

impl Stats {
    pub fn add(&mut self, other: Stats) -> Stats {
        Stats {
            count: self.count + other.count,
            bytes: self.bytes + other.bytes,
        }
    }
}

type ReferenceGraph = Graph<Object, &'static str, Directed, usize>;

fn parse(file: &str) -> std::io::Result<(NodeIndex<usize>, ReferenceGraph)> {
    let file = File::open(file)?;
    let reader = BufReader::new(file);

    let mut graph: ReferenceGraph = Graph::default();
    let mut indices: HashMap<usize, NodeIndex<usize>> = HashMap::new();
    let mut references: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut instances: HashMap<usize, usize> = HashMap::new();
    let mut names: HashMap<usize, String> = HashMap::new();

    let root = Object::root();
    let root_address = root.address;
    let root_index = graph.add_node(root);
    indices.insert(root_address, root_index);
    references.insert(root_address, Vec::new());

    for line in reader.lines().map(|l| l.unwrap()) {
        let parsed = serde_json::from_str::<Line>(&line)
            .expect(&line)
            .parse()
            .expect(&line);

        if parsed.object.is_root() {
            let refs = references.get_mut(&root_address).unwrap();
            refs.extend_from_slice(parsed.references.as_slice());
        } else {
            let address = parsed.object.address;
            indices.insert(address, graph.add_node(parsed.object));

            if !parsed.references.is_empty() {
                references.insert(address, parsed.references);
            }
            if let Some(module) = parsed.module {
                instances.insert(address, module);
            }
            if let Some(name) = parsed.name {
                names.insert(address, name);
            }
        }
    }

    for (node, successors) in references {
        let i = &indices[&node];
        for s in successors {
            if let Some(j) = indices.get(&s) {
                graph.add_edge(*i, *j, "");
            }
        }
    }

    for mut obj in graph.node_weights_mut() {
        if let Some(module) = instances.get(&obj.address) {
            if let Some(name) = names.get(module) {
                obj.kind = name.to_owned();
            }
        }
    }

    Ok((root_index, graph))
}

fn stats_by_kind<'a>(
    graph: &'a ReferenceGraph,
    dominators: &'a Dominators<NodeIndex<usize>>,
) -> (HashMap<&'a str, Stats>, HashMap<&'a str, Stats>) {
    let mut live_by_kind: HashMap<&'a str, Stats> = HashMap::new();
    let mut garbage_by_kind: HashMap<&'a str, Stats> = HashMap::new();

    for i in graph.node_indices() {
        let obj = graph.node_weight(i).unwrap();
        let by_kind = if dominators.immediate_dominator(i).is_some() {
            &mut live_by_kind
        } else {
            &mut garbage_by_kind
        };
        by_kind
            .entry(&obj.kind)
            .and_modify(|c| *c = (*c).add(obj.stats()))
            .or_insert_with(|| obj.stats());
    }

    (live_by_kind, garbage_by_kind)
}

fn dominator_subtree_sizes<'a>(
    graph: &'a ReferenceGraph,
    dominators: &'a Dominators<NodeIndex<usize>>,
) -> HashMap<&'a Object, Stats> {
    let mut subtree_sizes: HashMap<&Object, Stats> = HashMap::new();

    // Assign each node's stats to itself
    for i in graph.node_indices() {
        let obj = graph.node_weight(i).unwrap();
        subtree_sizes.insert(obj, obj.stats());
    }

    // Assign each node's stats to all of its dominators, if it's reachable
    for mut i in graph.node_indices() {
        let obj = graph.node_weight(i).unwrap();
        let stats = obj.stats();

        if dominators
            .dominators(i)
            .and_then(|mut ds| ds.next())
            .is_none()
        {
            subtree_sizes.remove(obj);
        } else {
            while let Some(dom) = dominators.immediate_dominator(i) {
                i = dom;

                subtree_sizes
                    .entry(graph.node_weight(i).unwrap())
                    .and_modify(|e| *e = (*e).add(stats));
            }
        }
    }

    subtree_sizes
}

fn dominator_graph<'a>(
    root: NodeIndex<usize>,
    graph: &'a ReferenceGraph,
    dominators: &Dominators<NodeIndex<usize>>,
    subtree_sizes: &HashMap<&'a Object, Stats>,
    relevance_threshold: f64,
) -> ReferenceGraph {
    let threshold_bytes = (subtree_sizes[graph.node_weight(root).unwrap()].bytes as f64
        * relevance_threshold)
        .floor() as usize;

    let mut subgraph: ReferenceGraph = Graph::default();
    let mut old_to_new: HashMap<NodeIndex<usize>, NodeIndex<usize>> = HashMap::new();
    for i in graph.node_indices() {
        let obj = graph.node_weight(i).unwrap();
        if let Some(Stats { count, bytes }) = subtree_sizes.get(obj) {
            if *bytes >= threshold_bytes {
                let mut clone = obj.clone();
                clone.label = Some(format!(
                    "{}: {} self, {} refs, {} objects",
                    obj,
                    ByteSize(obj.bytes as u64),
                    ByteSize((bytes - obj.bytes) as u64),
                    count
                ));
                let added = subgraph.add_node(clone);
                old_to_new.insert(i, added);
            }
        }
    }

    for (old, new) in old_to_new.iter() {
        if let Some(idom) = dominators.immediate_dominator(*old) {
            subgraph.add_edge(old_to_new[&idom], *new, "");
        } else {
            assert!(old == &root);
        }
    }

    subgraph
}

fn subgraph_dominated_by(
    graph: &ReferenceGraph,
    dominators: &Dominators<NodeIndex<usize>>,
    address: usize,
) -> (NodeIndex<usize>, ReferenceGraph) {
    let subtree_root = graph
        .node_indices()
        .find(|i| graph.node_weight(*i).unwrap().address == address)
        .expect("Given subtree root address not found");

    let mut subgraph = graph.clone();

    subgraph.retain_nodes(|_, n| {
        if let Some(mut ds) = dominators.dominators(n) {
            ds.any(|d| d == subtree_root)
        } else {
            false
        }
    });

    let new_root = subgraph
        .node_indices()
        .find(|i| subgraph.node_weight(*i).unwrap().address == address)
        .unwrap();

    (new_root, subgraph)
}

fn write_dot_file(graph: &ReferenceGraph, filename: &str) -> std::io::Result<()> {
    let mut file = File::create(filename)?;
    write!(
        file,
        "{}",
        dot::Dot::with_config(&graph, &[dot::Config::EdgeNoLabel])
    )?;
    Ok(())
}

fn print_largest<K: Display + Eq + Hash>(map: &HashMap<K, Stats>, count: usize) {
    let sorted = {
        let mut vec: Vec<(&K, &Stats)> = map.iter().collect();
        vec.sort_unstable_by_key(|(_, c)| c.bytes);
        vec
    };
    for (k, stats) in sorted.iter().rev().take(count) {
        println!(
            "{}: {} ({} objects)",
            k,
            ByteSize(stats.bytes as u64),
            stats.count
        );
    }

    let rest = sorted
        .iter()
        .rev()
        .skip(count)
        .fold(Stats::default(), |mut acc, (_, c)| acc.add(**c));
    if rest.count > 0 {
        println!(
            "...: {} ({} objects)",
            ByteSize(rest.bytes as u64),
            rest.count
        );
    }
}

fn filter_and_find_dominators(
    root: NodeIndex<usize>,
    graph: ReferenceGraph,
    subtree_root: Option<usize>,
) -> (
    NodeIndex<usize>,
    ReferenceGraph,
    Dominators<NodeIndex<usize>>,
) {
    let dominators = dominators::simple_fast(&graph, root);

    if let Some(address) = subtree_root {
        let (root, graph) = subgraph_dominated_by(&graph, &dominators, address);
        let dominators = dominators::simple_fast(&graph, root);
        (root, graph, dominators)
    } else {
        (root, graph, dominators)
    }
}

fn main() -> std::io::Result<()> {
    let args = clap_app!(reap =>
        (version: "0.1")
        (about: "A tool for parsing Ruby heap dumps.")
        (@arg INPUT: +required "Path to JSON heap dump file")
        (@arg DOT: -d --dot +takes_value "Dot file output")
        (@arg ROOT: -r --root +takes_value "Filter to subtree rooted at object with this address")
        (@arg THRESHOLD: -t --threshold +takes_value "Include nodes retaining at least this fraction of memory in dot output (defaults to 0.005)")
        (@arg COUNT: -n --top-n +takes_value "Print this many of the types & objects retaining the most memory")
    )
    .get_matches();

    let input = args.value_of("INPUT").unwrap();
    let dot_output = args.value_of("DOT");
    let subtree_root = args
        .value_of("ROOT")
        .map(|r| Line::parse_address(r).expect("Invalid subtree root address"));
    let relevance_threshold: f64 = args
        .value_of("THRESHOLD")
        .map(|t| t.parse().expect("Invalid relevance threshold"))
        .unwrap_or(DEFAULT_RELEVANCE_THRESHOLD);
    let top_n: usize = args
        .value_of("COUNT")
        .map(|t| t.parse().expect("Invalid top-n count"))
        .unwrap_or(10);

    let (root, graph, dominators) = {
        let (root, graph) = parse(&input)?;
        filter_and_find_dominators(root, graph, subtree_root)
    };

    let (live_by_kind, garbage_by_kind) = stats_by_kind(&graph, &dominators);
    let subtree_sizes = dominator_subtree_sizes(&graph, &dominators);

    println!("Object types using the most live memory:");
    print_largest(&live_by_kind, top_n);

    println!("\nObjects retaining the most live memory:");
    print_largest(&subtree_sizes, top_n);

    println!("\nObjects unreachable from root");
    print_largest(&garbage_by_kind, top_n);

    if let Some(output) = dot_output {
        let dom_graph = dominator_graph(
            root,
            &graph,
            &dominators,
            &subtree_sizes,
            relevance_threshold,
        );
        write_dot_file(&dom_graph, &output)?;
        println!(
            "\nWrote {} nodes & {} edges to {}",
            dom_graph.node_count(),
            dom_graph.edge_count(),
            &output
        );
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn integration() {
        let (root, graph, dominators) = {
            let (root, graph) = parse("test/heap.json").unwrap();
            filter_and_find_dominators(root, graph, None)
        };

        assert_eq!(18982, graph.node_count());
        assert_eq!(28436, graph.edge_count());

        let (live_by_kind, garbage_by_kind) = stats_by_kind(&graph, &dominators);
        assert_eq!(
            10409,
            live_by_kind["String"].count + garbage_by_kind["String"].count
        );
        assert_eq!(
            544382,
            live_by_kind["String"].bytes + garbage_by_kind["String"].bytes
        );

        let subtree_sizes = dominator_subtree_sizes(&graph, &dominators);
        let root_obj = graph.node_weight(root).unwrap();
        assert_eq!(15472, subtree_sizes[root_obj].count);
        assert_eq!(3439119, subtree_sizes[root_obj].bytes);

        let (_, subgraph) = subgraph_dominated_by(&graph, &dominators, 140204367666240);
        assert_eq!(25, subgraph.node_count());
        assert_eq!(28, subgraph.edge_count());

        let dom_graph = dominator_graph(
            root,
            &graph,
            &dominators,
            &subtree_sizes,
            DEFAULT_RELEVANCE_THRESHOLD,
        );
        assert_eq!(33, dom_graph.node_count());
        assert_eq!(32, dom_graph.edge_count());
    }
}
