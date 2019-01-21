#[macro_use]
extern crate serde;
extern crate petgraph;
extern crate serde_json;

use petgraph::algo::dominators;
use petgraph::graph::NodeIndex;
use petgraph::{Directed, Graph};
use std::collections::HashMap;
use std::env;
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

#[derive(Debug)]
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

impl Line {
    pub fn parse(self) -> Option<ParsedLine> {
        let mut object = Object {
            address: self
                .address
                .as_ref()
                .map(|a| Line::parse_address(a.as_str()))
                .unwrap_or(0),
            bytes: self.memsize.unwrap_or(0),
            kind: self.object_type,
            label: None,
        };

        if object.address == 0 && object.kind != "ROOT" {
            return None;
        }

        object.label = match object.kind.as_str() {
            "CLASS" | "MODULE" | "ICLASS" => self.name.clone(),
            "ARRAY" => Some(format!("Array[len={}]", self.length?)),
            "HASH" => Some(format!("Hash[size={}]", self.size?)),
            "STRING" => self.value.map(|v| {
                if v.len() > 40 {
                    format!("'{}...'", v.chars().take(37).collect::<String>())
                } else {
                    v.to_owned()
                }
            }),
            _ => None,
        };

        Some(ParsedLine {
            references: self
                .references
                .iter()
                .map(|r| Line::parse_address(r.as_str()))
                .collect(),
            module: self.class.map(|c| Line::parse_address(c.as_str())),
            name: self.name,
            object,
        })
    }

    fn parse_address(addr: &str) -> usize {
        usize::from_str_radix(&addr[2..], 16).unwrap()
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
            write!(f, "{}[{}]", self.kind, self.address)
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

type ReferenceGraph = Graph<Object, (), Directed, usize>;

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
                graph.add_edge(*i, *j, ());
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

fn print_basic_stats(graph: &ReferenceGraph) {
    let mut by_kind: HashMap<&str, Stats> = HashMap::new();
    for i in graph.node_indices() {
        let obj = graph.node_weight(i).unwrap();
        by_kind
            .entry(&obj.kind)
            .and_modify(|c| *c = (*c).add(obj.stats()))
            .or_insert_with(|| obj.stats());
    }
    print_largest(&by_kind, 10);
}

fn print_dominators(root: NodeIndex<usize>, graph: &ReferenceGraph) {
    let tree = dominators::simple_fast(graph, root);

    let mut subtree_sizes: HashMap<&Object, Stats> = HashMap::new();

    // Assign each node's stats to itself
    for i in graph.node_indices() {
        let obj = graph.node_weight(i).unwrap();
        subtree_sizes.insert(obj, obj.stats());
    }

    // Assign each node's stats to all of its dominators
    for mut i in graph.node_indices() {
        let obj = graph.node_weight(i).unwrap();
        let stats = obj.stats();

        while let Some(dom) = tree.immediate_dominator(i) {
            i = dom;

            subtree_sizes
                .entry(graph.node_weight(i).unwrap())
                .and_modify(|e| *e = (*e).add(stats));
        }
    }

    print_largest(&subtree_sizes, 25);
}

fn print_largest<K: Display + Eq + Hash>(map: &HashMap<K, Stats>, count: usize) {
    let sorted = {
        let mut vec: Vec<(&K, &Stats)> = map.iter().collect();
        vec.sort_unstable_by_key(|(_, c)| c.bytes);
        vec
    };
    for (k, stats) in sorted.iter().rev().take(count) {
        println!("{}: {} bytes ({} objects)", k, stats.bytes, stats.count);
    }
    let rest = sorted
        .iter()
        .rev()
        .skip(count)
        .fold(Stats::default(), |mut acc, (_, c)| acc.add(**c));
    println!("...: {} bytes ({} objects)", rest.bytes, rest.count);
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();
    assert!(
        args.len() == 2,
        "Expected exactly one argument (filename), got {:?}",
        args
    );

    let (root, graph) = parse(&args[1])?;
    print_basic_stats(&graph);

    println!("\nObjects retaining the most memory:");
    print_dominators(root, &graph);

    Ok(())
}
