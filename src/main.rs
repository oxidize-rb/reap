#[macro_use]
extern crate serde;
extern crate serde_json;

mod dominator;

use std::borrow::Cow;
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
enum Object {
    Root {
        references: Vec<usize>,
        label: String,
    },
    Module {
        address: usize,
        references: Vec<usize>,
        memsize: usize,
        kind: String,
        name: Option<String>,
    },
    Instance {
        address: usize,
        references: Vec<usize>,
        memsize: usize,
        module: usize,
        kind: String,
        label: String,
    },
    Other {
        address: usize,
        references: Vec<usize>,
        memsize: usize,
        kind: String,
        label: String,
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
        let kind = self.object_type;

        let result = match kind.as_str() {
            "ROOT" => Object::Root {
                references,
                label: self.root.unwrap(),
            },
            "CLASS" | "MODULE" | "ICLASS" => Object::Module {
                address: address?,
                references,
                memsize,
                kind,
                name: self.name,
            },
            "ARRAY" => Object::Other {
                address: address?,
                references,
                memsize,
                kind,
                label: format!("Array[len={}]", self.length?),
            },
            "HASH" => Object::Other {
                address: address?,
                references,
                memsize,
                kind,
                label: format!("Hash[size={}]", self.size?),
            },
            "STRING" => Object::Other {
                address: address?,
                references,
                memsize,
                kind,
                label: if let Some(v) = self.value {
                    if v.len() > 40 {
                        format!("'{}...'", v.chars().take(37).collect::<String>())
                    } else {
                        v.to_owned()
                    }
                } else {
                    "".to_string()
                },
            },
            other => {
                let label = format!("{}[{}]", other, address?);
                if let Some(c) = self.class {
                    Object::Instance {
                        address: address?,
                        references,
                        memsize,
                        module: Line::parse_address(c.as_str()),
                        kind,
                        label,
                    }
                } else {
                    Object::Other {
                        address: address?,
                        references,
                        memsize,
                        kind,
                        label,
                    }
                }
            }
        };

        Some(result)
    }

    fn parse_address(addr: &str) -> usize {
        usize::from_str_radix(&addr[2..], 16).unwrap()
    }
}

impl Object {
    pub fn kind(&self) -> &str {
        match self {
            Object::Root { .. } => "ROOT",
            Object::Module { kind, .. } => kind.as_str(),
            Object::Instance { kind, .. } => kind.as_str(),
            Object::Other { kind, .. } => kind.as_str(),
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

impl Display for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let label: Cow<str> = match self {
            Object::Root { label, .. } => Cow::Borrowed(label.as_str()),
            Object::Module {
                name,
                kind,
                address,
                ..
            } => {
                if let Some(n) = name {
                    Cow::Borrowed(n.as_str())
                } else {
                    Cow::Owned(format!("{}[{}]", kind, address))
                }
            }
            Object::Instance { label, .. } => Cow::Borrowed(label.as_str()),
            Object::Other { label, .. } => Cow::Borrowed(label.as_str()),
        };
        write!(f, "{}", label)
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

    // TODO Is there a way to satisfy the borrow checker without this copying?
    let module_names = {
        let mut module_names: HashMap<usize, String> = HashMap::new();
        for obj in objects_by_addr.values() {
            if let Object::Module { address, name, .. } = obj {
                if let Some(n) = name {
                    module_names.insert(*address, n.to_string());
                }
            }
        }
        module_names
    };

    for obj in objects_by_addr.values_mut() {
        if let Object::Instance {
            module,
            kind,
            label,
            address,
            ..
        } = obj
        {
            if let Some(name) = module_names.get(module) {
                *kind = name.to_owned();
                *label = format!("{}[{}]", name, address);
            }
        }
    }

    Ok((roots, objects_by_addr))
}

fn print_basic_stats(objects_by_addr: &BTreeMap<usize, Object>) {
    let mut by_kind: HashMap<&str, Stats> = HashMap::new();
    for obj in objects_by_addr.values() {
        by_kind
            .entry(obj.kind())
            .and_modify(|c| *c = (*c).add(obj.stats()))
            .or_insert_with(|| obj.stats());
    }
    print_largest(&by_kind, 10);
}

fn print_dominators(roots: &[Object], objects_by_addr: &BTreeMap<usize, Object>) {
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

    let mut subtree_sizes: HashMap<&Object, Stats> = HashMap::new();

    // Assign each node's stats to itself
    for obj in &objects {
        subtree_sizes.insert(obj, obj.stats());
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

            subtree_sizes
                .entry(objects[i])
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

    println!("\nObjects retaining the most memory:");
    print_dominators(roots.as_slice(), &objects_by_addr);

    Ok(())
}
