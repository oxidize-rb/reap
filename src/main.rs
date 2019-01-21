#[macro_use]
extern crate serde;
extern crate serde_json;

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::env;
use std::fs::File;
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
struct Contents {
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

    pub fn stats(&self) -> Contents {
        Contents {
            count: 1,
            bytes: self.bytes(),
        }
    }
}

impl Contents {
    pub fn add(&mut self, other: &Contents) {
        self.count += other.count;
        self.bytes += other.bytes;
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

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();
    assert!(
        args.len() == 2,
        "Expected exactly one argument (filename), got {:?}",
        args
    );

    let (roots, objects_by_addr) = parse(&args[1])?;

    let mut by_kind: HashMap<&str, Contents, _> = HashMap::new();
    for obj in objects_by_addr.values() {
        let kind = obj.kind(&objects_by_addr);
        by_kind
            .entry(kind)
            .and_modify(|c| (*c).add(&obj.stats()))
            .or_insert_with(|| obj.stats());
    }
    let sorted = {
        let mut vec: Vec<(&&str, &Contents)> = by_kind.iter().collect();
        vec.sort_unstable_by_key(|(_, c)| c.bytes);
        vec
    };
    for (kind, contents) in sorted.iter().rev().take(10) {
        println!(
            "{}: {} bytes ({} objects)",
            kind, contents.bytes, contents.count
        );
    }
    let rest = sorted
        .iter()
        .rev()
        .skip(10)
        .fold(Contents::default(), |mut acc, (_, c)| {
            acc.add(c);
            acc
        });
    println!("...: {} bytes ({} objects)", rest.bytes, rest.count);

    Ok(())
}
