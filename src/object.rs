use bytesize::ByteSize;
use petgraph::{Directed, Graph};
use std::fmt::Display;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone)]
pub struct Object {
    pub address: usize,
    pub bytes: usize,
    pub kind: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Stats {
    pub count: usize,
    pub bytes: usize,
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

    pub fn with_dominator_stats(&self, stats: Stats) -> Object {
        let mut clone = self.clone();
        clone.label = Some(format!(
            "{}: {} self, {} refs, {} objects",
            self,
            ByteSize(self.bytes as u64),
            ByteSize((stats.bytes - self.bytes) as u64),
            stats.count
        ));
        clone
    }

    pub fn format(&self, class_name_only: bool) -> String {
        if let Some(ref label) = self.label {
            return format!("{}", label);
        } else {
            if class_name_only {
                self.kind.clone()
            } else {
                format!("{}[{:#x}]", self.kind, self.address)
            }
        }
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

pub type ReferenceGraph = Graph<Object, &'static str, Directed, usize>;

pub const EDGE_WEIGHT: &str = "";
