#[macro_use]
extern crate serde;
extern crate serde_json;

mod dominator;

use std::collections::{BTreeMap, HashMap};
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

    #[serde(rename = "type")]
    object_type: String,
    class: Option<String>,

    name: Option<String>,

    #[serde(default)]
    references: Vec<String>,
}

#[derive(Debug)]
enum Object {
    Root {
        references: Vec<usize>,
    },
    Module {
        address: usize,
        references: Vec<usize>,
        memsize: usize,
        name: Option<String>,
        object_type: String,
    },
    Instance {
        address: usize,
        references: Vec<usize>,
        memsize: usize,
        module: usize,
        object_type: String,
    },
    Other {
        address: usize,
        references: Vec<usize>,
        memsize: usize,
        object_type: String,
    },
}

#[derive(Debug, Clone, Copy, Default)]
struct Stats {
    count: usize,
    bytes: usize,
}

impl Line {
    pub fn parse(self) -> Option<Object> {
        let address = self
            .address
            .as_ref()
            .map(|a| Line::parse_address(a.as_str()));
        let references = self
            .references
            .iter()
            .map(|r| Line::parse_address(r.as_str()))
            .collect();
        let memsize = self.memsize.unwrap_or(0);
        let object_type = self.object_type;

        let result = match object_type.as_str() {
            "ROOT" => Object::Root { references },
            "CLASS" | "MODULE" | "ICLASS" => Object::Module {
                address: address?,
                references,
                memsize,
                name: self.name,
                object_type,
            },
            _ if self.class.is_some() => Object::Instance {
                address: address?,
                references,
                memsize,
                module: Line::parse_address(self.class?.as_str()),
                object_type,
            },
            _ => Object::Other {
                address: address?,
                references,
                memsize,
                object_type,
            },
        };

        Some(result)
    }

    fn parse_address(addr: &str) -> usize {
        usize::from_str_radix(&addr[2..], 16).unwrap()
    }
}

impl Object {
    pub fn label(&self, objects_by_addr: &BTreeMap<usize, Object>) -> String {
        format!("{}[{}]", self.kind(objects_by_addr), self.address())
    }

    pub fn kind<'a>(&'a self, objects_by_addr: &'a BTreeMap<usize, Object>) -> &'a str {
        match self {
            Object::Root { .. } => "ROOT",
            Object::Module { object_type, .. } => object_type,
            Object::Instance {
                module,
                object_type,
                ..
            } => objects_by_addr
                .get(module)
                .and_then(|m| m.name())
                .unwrap_or(object_type),
            Object::Other { object_type, .. } => object_type,
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Object::Module { name, .. } => name.as_ref().map(|n| n.as_str()),
            _ => None,
        }
    }

    pub fn bytes(&self) -> usize {
        match self {
            Object::Root { .. } => 0,
            Object::Module { memsize, .. } => *memsize,
            Object::Instance { memsize, .. } => *memsize,
            Object::Other { memsize, .. } => *memsize,
        }
    }

    pub fn address(&self) -> usize {
        match self {
            Object::Root { .. } => 0,
            Object::Module { address, .. } => *address,
            Object::Instance { address, .. } => *address,
            Object::Other { address, .. } => *address,
        }
    }

    pub fn stats(&self) -> Stats {
        Stats {
            count: 1,
            bytes: self.bytes(),
        }
    }

    pub fn references(&self) -> &[usize] {
        match self {
            Object::Root { references, .. } => &references,
            Object::Module { references, .. } => &references,
            Object::Instance { references, .. } => &references,
            Object::Other { references, .. } => &references,
        }
    }
}

impl PartialEq for Object {
    fn eq(&self, other: &Object) -> bool {
        self.address() == other.address()
    }
}
impl Eq for Object {}

impl Hash for Object {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.address().hash(state);
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

fn parse(file: &str) -> std::io::Result<(Vec<Object>, BTreeMap<usize, Object>)> {
    let file = File::open(file)?;
    let reader = BufReader::new(file);

    let mut objects_by_addr: BTreeMap<usize, Object> = BTreeMap::new();
    let mut roots: Vec<Object> = Vec::new();

    for line in reader.lines().map(|l| l.unwrap()) {
        let obj = serde_json::from_str::<Line>(&line)
            .expect(&line)
            .parse()
            .expect(&line);
        if obj.address() > 0 {
            objects_by_addr.insert(obj.address(), obj);
        } else {
            roots.push(obj);
        }
    }

    Ok((roots, objects_by_addr))
}

fn print_basic_stats(objects_by_addr: &BTreeMap<usize, Object>) {
    let mut by_kind: HashMap<&str, Stats> = HashMap::new();
    for obj in objects_by_addr.values() {
        let kind = obj.kind(objects_by_addr);
        by_kind
            .entry(kind)
            .and_modify(|c| *c = (*c).add(obj.stats()))
            .or_insert_with(|| obj.stats());
    }
    print_largest(&by_kind, 10);
}

fn print_largest_objects(roots: &[Object], objects_by_addr: &BTreeMap<usize, Object>) {
    let objects = {
        let mut objects: Vec<&Object> = Vec::new();
        objects.extend(roots.iter());
        objects.extend(objects_by_addr.values());
        objects
    };

    let index_by_obj = {
        let mut index_by_obj: HashMap<&Object, usize> = HashMap::with_capacity(objects.len());
        for (i, obj) in objects.iter().enumerate() {
            index_by_obj.insert(obj, i + 1);
        }
        index_by_obj
    };

    let adj_list = {
        let mut adj_list: Vec<Vec<usize>> = objects
            .iter()
            .map(|obj| {
                obj.references()
                    .iter()
                    .flat_map(|r| objects_by_addr.get(r).and_then(|o| index_by_obj.get(o)))
                    .cloned()
                    .collect()
            })
            .collect();
        adj_list.insert(0, (1..=roots.len()).collect());
        adj_list
    };

    let tree = dominator::DominatorTree::from_graph(&adj_list);

    let mut subtree_sizes: HashMap<String, Stats> = HashMap::new();

    // Assign each node's stats to itself
    for obj in &objects {
        subtree_sizes.insert(obj.label(objects_by_addr), obj.stats());
    }

    // Assign each node's stats to all of its dominators
    for (mut i, obj) in objects.iter().enumerate() {
        let stats = obj.stats();

        // Correct for artificial root-of-roots
        while let Some(dom) = tree.idom(i + 1) {
            if dom == 0 {
                break;
            } else {
                i = dom - 1;
            }

            let key = objects[i].label(objects_by_addr);
            subtree_sizes
                .entry(key)
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

    let (roots, objects_by_addr) = parse(&args[1])?;
    print_basic_stats(&objects_by_addr);
    print_largest_objects(roots.as_slice(), &objects_by_addr);

    Ok(())
}
