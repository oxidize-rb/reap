use crate::object::*;
use petgraph::graph::NodeIndex;
use petgraph::Graph;
use std::collections::HashMap;
use std::fs::File;
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

impl Line {
    pub fn parse(self) -> Option<ParsedLine> {
        let mut object = Object {
            address: self
                .address
                .as_ref()
                .and_then(|a| parse_address(a.as_str()).ok())
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
                .flat_map(|r| parse_address(r.as_str()).ok())
                .collect(),
            module: self.class.and_then(|c| parse_address(c.as_str()).ok()),
            name: self.name,
            object,
        })
    }
}

pub fn parse_address(addr: &str) -> Result<usize, std::num::ParseIntError> {
    usize::from_str_radix(&addr[2..], 16)
}

pub fn parse(file: &str) -> std::io::Result<(NodeIndex<usize>, ReferenceGraph)> {
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
                graph.add_edge(*i, *j, EDGE_WEIGHT);
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
