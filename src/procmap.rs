use crate::object::*;
use petgraph::graph::NodeIndex;
use petgraph::Graph;
use proc_maps::{get_process_maps, MapRange};
use read_process_memory::{copy_address, CopyAddress, Pid, ProcessHandle, TryIntoProcessHandle};
use std::collections::HashMap;
use timed_function::timed;

type VALUE = u64;
const POINTER_BYTES: usize = 8;
const MAX_FLAGS: VALUE = u32::max_value() as VALUE;
const HEAP_PAGE_BYTES: usize = 16384;
const RVALUE_WIDTH: usize = 5;
const RVALUE_BYTES: usize = RVALUE_WIDTH * POINTER_BYTES;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
enum Type {
    None = 0x00,
    Object = 0x01,
    Class = 0x02,
    Module = 0x03,
    Float = 0x04,
    String = 0x05,
    Regexp = 0x06,
    Array = 0x07,
    Hash = 0x08,
    Struct = 0x09,
    Bignum = 0x0a,
    File = 0x0b,
    Data = 0x0c,
    Match = 0x0d,
    Complex = 0x0e,
    Rational = 0x0f,
    // Nil = 0x11,
    // True = 0x12,
    // False = 0x13,
    Symbol = 0x14,
    // Fixnum = 0x15,
    // Undef = 0x16,
    IMemo = 0x1a,
    Node = 0x1b,
    IClass = 0x1c,
    Zombie = 0x1d,
}

impl Type {
    #[inline]
    pub fn from_heap_flags(flags: VALUE) -> Result<Type, ()> {
        match flags & 0x1f {
            0x00 => Ok(Type::None),
            0x01 => Ok(Type::Object),
            0x02 => Ok(Type::Class),
            0x03 => Ok(Type::Module),
            0x04 => Ok(Type::Float),
            0x05 => Ok(Type::String),
            0x06 => Ok(Type::Regexp),
            0x07 => Ok(Type::Array),
            0x08 => Ok(Type::Hash),
            0x09 => Ok(Type::Struct),
            0x0a => Ok(Type::Bignum),
            0x0b => Ok(Type::File),
            0x0c => Ok(Type::Data),
            0x0d => Ok(Type::Match),
            0x0e => Ok(Type::Complex),
            0x0f => Ok(Type::Rational),
            0x14 => Ok(Type::Symbol),
            0x1a => Ok(Type::IMemo),
            0x1b => Ok(Type::Node),
            0x1c => Ok(Type::IClass),
            0x1d => Ok(Type::Zombie),
            _ => Err(()),
        }
    }
}

#[derive(Debug)]
enum ArrayData {
    Embedded { len: usize, values: [VALUE; 3] },
    Heap { len: usize, ptr: usize }, // TODO Special treatment for `shared`
}

impl ArrayData {
    #[inline]
    pub fn from_rvalue(flags: VALUE, data: &[VALUE]) -> ArrayData {
        debug_assert!(data.len() == RVALUE_WIDTH);

        let embedded = ((1 << 13) & flags) > 0; // See RARRAY_EMBED_FLAG
        if embedded {
            let len = ((flags >> 15) & 0b11) as usize; // See RARRAY_EMBED_LEN_MASK
            let mut values = [0; 3];
            values[0..len].copy_from_slice(&data[2..2 + len]);
            ArrayData::Embedded { len, values }
        } else {
            let len = data[2] as usize;
            let ptr = data[4] as usize;
            ArrayData::Heap { len, ptr }
        }
    }

    #[inline]
    pub fn references(&self, heap: &[HeapPage], proc: &ProcessHandle) -> Vec<usize> {
        let mut refs: Vec<usize> = Vec::new();
        let mut with_values = |values: &[VALUE]| {
            for v in values {
                let addr = *v as usize;
                if addr % RVALUE_BYTES == 0 && heap.iter().any(|p| p.deref(addr).is_some()) {
                    refs.push(addr)
                }
            }
        };
        match self {
            ArrayData::Embedded { len, values } => with_values(&values[0..*len]),
            ArrayData::Heap { len, ptr } => {
                if let Ok(bytes) = copy_address(*ptr, *len * POINTER_BYTES, proc) {
                    with_values(bytes_to_values(&bytes))
                } else {
                    dbg!(("Read failed", ptr, len));
                }
            }
        };
        refs
    }
}

const STRING_EMBED_BYTES: usize = RVALUE_BYTES - 2 * POINTER_BYTES;

#[derive(Debug)]
enum StringData {
    Embedded {
        len: usize,
        bytes: [u8; STRING_EMBED_BYTES],
    },
    Heap {
        len: usize,
        ptr: usize,
    }, // TODO Special treatment for `shared`
}

impl StringData {
    #[inline]
    pub fn from_rvalue(flags: VALUE, data: &[VALUE]) -> Result<StringData, ()> {
        debug_assert!(data.len() == RVALUE_WIDTH);

        let embedded = ((1 << 13) & flags) == 0; // See RSTRING_NOEMBED
        if embedded {
            let len = ((flags >> 14) & 0b11111) as usize; // See RSTRING_EMBED_LEN_MASK
            if len > STRING_EMBED_BYTES {
                return Err(());
            }

            let available_bytes = values_to_bytes(&data[2..]);
            let mut bytes = [0; STRING_EMBED_BYTES];
            bytes[0..len].copy_from_slice(&available_bytes[0..len]);
            Ok(StringData::Embedded { len, bytes })
        } else {
            let len = data[2] as usize;
            let ptr = data[3] as usize;
            Ok(StringData::Heap { len, ptr })
        }
    }
}

#[derive(Debug)]
enum ObjectData {
    Embedded { ivars: [VALUE; 3] },
    Heap { len: u32, ptr: usize },
}

impl ObjectData {
    #[inline]
    pub fn from_rvalue(flags: VALUE, data: &[VALUE]) -> ObjectData {
        debug_assert!(data.len() == RVALUE_WIDTH);

        let embedded = ((1 << 13) & flags) > 0; // See ROBJECT_EMBED
        if embedded {
            let mut ivars = [0; 3];
            ivars.copy_from_slice(&data[2..RVALUE_WIDTH]);
            ObjectData::Embedded { ivars }
        } else {
            let len = data[2] as u32;
            let ptr = data[3] as usize;
            ObjectData::Heap { len, ptr }
        }
    }

    #[inline]
    pub fn references(&self, heap: &[HeapPage], proc: &ProcessHandle) -> Vec<usize> {
        let mut refs: Vec<usize> = Vec::new();
        let mut with_values = |values: &[VALUE]| {
            for v in values {
                let addr = *v as usize;
                if addr % RVALUE_BYTES == 0 && heap.iter().any(|p| p.deref(addr).is_some()) {
                    refs.push(addr)
                }
            }
        };
        match self {
            ObjectData::Embedded { ivars } => with_values(&ivars[..]),
            ObjectData::Heap { len, ptr } => {
                if let Ok(bytes) = copy_address(*ptr, (*len as usize) * POINTER_BYTES, proc) {
                    with_values(bytes_to_values(&bytes))
                } else {
                    dbg!(("Read failed", ptr, len));
                }
            }
        };
        refs
    }
}

#[derive(Debug)]
enum RValue {
    Free { next: usize },
    Object { klass: usize, data: ObjectData },
    Class { klass: usize },
    Module { klass: usize },
    String { klass: usize, data: StringData },
    Array { klass: usize, data: ArrayData },
    Hash { klass: usize },
    Data { klass: usize },
    IMemo,
    Other { rbtype: Type, klass: usize },
    Invalid,
}

impl RValue {
    #[inline]
    pub fn from_data(heap_page: usize, _offset: usize, data: &[VALUE]) -> RValue {
        debug_assert!(data.len() == RVALUE_WIDTH);

        let flags = data[0];
        if flags > MAX_FLAGS {
            return RValue::Invalid;
        }

        let pointer = data[1] as usize;
        if pointer % RVALUE_BYTES != 0 {
            return match Type::from_heap_flags(flags) {
                Ok(Type::IMemo) => RValue::IMemo,
                _ => RValue::Invalid,
            };
        }

        match Type::from_heap_flags(flags) {
            Ok(Type::None) => {
                if pointer == 0 || (pointer >= heap_page && pointer < heap_page + HEAP_PAGE_BYTES) {
                    RValue::Free { next: pointer }
                } else {
                    RValue::Invalid
                }
            }
            Ok(Type::Object) => RValue::Object {
                klass: pointer,
                data: ObjectData::from_rvalue(flags, data),
            },
            Ok(Type::Class) => RValue::Class { klass: pointer },
            Ok(Type::Module) => RValue::Module { klass: pointer },
            Ok(Type::String) => {
                if let Ok(strdata) = StringData::from_rvalue(flags, data) {
                    RValue::String {
                        klass: pointer,
                        data: strdata,
                    }
                } else {
                    RValue::Invalid
                }
            }
            Ok(Type::Array) => RValue::Array {
                klass: pointer,
                data: ArrayData::from_rvalue(flags, data),
            },
            Ok(Type::Hash) => RValue::Hash { klass: pointer },
            Ok(Type::Data) => RValue::Data { klass: pointer },
            Ok(Type::IMemo) => RValue::IMemo,
            Ok(t) => RValue::Other {
                klass: pointer,
                rbtype: t,
            },
            Err(_) => RValue::Invalid,
        }
    }

    #[inline]
    pub fn is_last_free_value(&self) -> bool {
        match self {
            RValue::Free { next: 0 } => true,
            _ => false,
        }
    }

    #[inline]
    pub fn references(&self, heap: &[HeapPage], proc: &ProcessHandle) -> Vec<usize> {
        let mut refs = match self {
            RValue::Free { .. } => Vec::new(),
            RValue::Object { klass, data } => {
                let mut refs: Vec<usize> = data.references(heap, proc);
                if *klass > 0 {
                    refs.push(*klass);
                }
                refs
            }
            RValue::Class { .. } => Vec::new(),
            RValue::Module { .. } => Vec::new(),
            RValue::String { klass, .. } => {
                let mut refs: Vec<usize> = Vec::new();
                if *klass > 0 {
                    refs.push(*klass);
                }
                refs
            }
            RValue::Array { klass, data } => {
                let mut refs: Vec<usize> = data.references(heap, proc);
                if *klass > 0 {
                    refs.push(*klass);
                }
                refs
            }
            RValue::Hash { .. } => Vec::new(),
            RValue::Data { .. } => Vec::new(),
            RValue::IMemo => Vec::new(),
            RValue::Other { .. } => Vec::new(),
            RValue::Invalid => Vec::new(),
        };

        refs.sort();
        refs.dedup();
        refs
    }

    #[inline]
    pub fn valid(&self, heap: &[HeapPage]) -> bool {
        let on_heap = |a| heap.iter().any(|p| p.deref(a).is_some());

        match self {
            RValue::Free { next, .. } => *next == 0 || on_heap(*next),
            RValue::Object { klass, .. } => on_heap(*klass),
            RValue::Class { klass, .. } => *klass == 0 || on_heap(*klass), // There's exactly one class with a null `klass`, which I suspect is BasicObject
            RValue::Module { klass, .. } => on_heap(*klass),
            // TODO Understand the zero special case
            RValue::String { klass, .. } => *klass == 0 || on_heap(*klass),
            RValue::Array { klass, .. } => *klass == 0 || on_heap(*klass),
            RValue::Hash { klass, .. } => *klass == 0 || on_heap(*klass),
            RValue::Data { klass, .. } => *klass == 0 || on_heap(*klass),
            RValue::IMemo => true,
            RValue::Other { klass, .. } => on_heap(*klass),
            RValue::Invalid => false,
        }
    }

    #[inline]
    pub fn free(&self) -> bool {
        match self {
            RValue::Free { .. } => true,
            _ => false,
        }
    }

    #[inline]
    pub fn kind(&self) -> String {
        match self {
            RValue::Object { .. } => "Object".to_string(),
            RValue::Class { .. } => "Class".to_string(),
            RValue::Module { .. } => "Module".to_string(),
            RValue::String { .. } => "String".to_string(),
            RValue::Array { .. } => "Array".to_string(),
            RValue::Hash { .. } => "Hash".to_string(),
            RValue::Data { .. } => "Data".to_string(),
            RValue::IMemo => "IMemo".to_string(),
            RValue::Other { rbtype, .. } => format!("{:?}", rbtype),
            _ => panic!(),
        }
    }

    #[inline]
    pub fn bytesize(&self) -> usize {
        match self {
            RValue::Array {
                data: ArrayData::Heap { len, .. },
                ..
            } => RVALUE_BYTES + POINTER_BYTES * *len,
            RValue::Object {
                data: ObjectData::Heap { len, .. },
                ..
            } => RVALUE_BYTES + POINTER_BYTES * (*len as usize),
            RValue::String {
                data: StringData::Heap { len, .. },
                ..
            } => RVALUE_BYTES + *len,
            _ => RVALUE_BYTES,
        }
    }
}

#[derive(Debug)]
struct HeapPage {
    addr: usize,
    rvalues: Vec<RValue>,
}

impl HeapPage {
    pub fn from_data(addr: usize, data: &[VALUE]) -> Result<HeapPage, ()> {
        let rvalues = data
            .chunks_exact(RVALUE_WIDTH)
            .enumerate()
            .map(|(i, v)| RValue::from_data(addr, i, v))
            .collect::<Vec<_>>();

        if rvalues
            .iter()
            .filter(|v| match v {
                RValue::Invalid => true,
                _ => false,
            })
            .count()
            >= 2
        {
            Err(())
        } else if rvalues.iter().filter(|v| v.is_last_free_value()).count() >= 3 {
            Err(())
        } else {
            Ok(HeapPage { addr, rvalues })
        }
    }

    #[inline]
    pub fn address(&self, offset: usize) -> usize {
        self.addr + offset * RVALUE_BYTES
    }

    #[inline]
    pub fn contents(&self) -> &[RValue] {
        self.rvalues.as_slice()
    }

    #[inline]
    pub fn deref(&self, addr: usize) -> Option<&RValue> {
        if addr < self.addr {
            None
        } else {
            self.rvalues.get((addr - self.addr) / RVALUE_BYTES)
        }
    }
}

#[timed]
pub fn parse(pid: Pid) -> std::io::Result<(NodeIndex<usize>, ReferenceGraph)> {
    let handle = pid.try_into_process_handle()?;

    let procmaps: Vec<MapRange> = get_process_maps(pid)?
        .into_iter()
        .filter(|m| m.is_read())
        .collect();

    // TODO Darwin specific
    let maybe_heap = procmaps
        .iter()
        .filter(|m| m.filename().iter().all(|n| n.contains("dyld")));

    let mut pages: Vec<HeapPage> = Vec::new();
    let mut buffer = vec![0u8; HEAP_PAGE_BYTES];

    for m in maybe_heap {
        let mut addr: usize = next_aligned(m.start(), HEAP_PAGE_BYTES);

        let last_valid = m.start() + m.size() - buffer.len();

        while addr < last_valid {
            if !handle.copy_address(addr, &mut buffer).is_ok() {
                break;
            }

            let first_rvalue = next_aligned(addr, RVALUE_BYTES);
            let data = bytes_to_values(&buffer[first_rvalue - addr..]);
            if let Ok(page) = HeapPage::from_data(first_rvalue, data) {
                pages.push(page);
            }

            addr += HEAP_PAGE_BYTES;
        }
    }

    let invalid_rvalues = (&pages)
        .iter()
        .flat_map(|p| p.contents())
        .filter(|r| !r.valid(&pages))
        .count();
    dbg!(invalid_rvalues);

    let mut graph = Graph::default();
    let root = graph.add_node(Object::root());
    let mut indices: HashMap<usize, NodeIndex<usize>> = HashMap::new();

    for p in &pages {
        for (i, r) in p.contents().iter().enumerate() {
            if r.valid(&pages) && !r.free() {
                let addr = p.address(i);
                indices.insert(
                    addr,
                    graph.add_node(Object {
                        address: addr,
                        bytes: r.bytesize(),
                        kind: r.kind(),
                        label: None,
                    }),
                );
            }
        }
    }

    let ruby_maps = procmaps.iter().filter(|m| {
        m.filename().iter().all(|n| n.contains("bin/ruby"))
            || m.filename().iter().all(|n| n.contains("libruby"))
    });

    for m in ruby_maps {
        let mut addr: usize = next_aligned(m.start(), POINTER_BYTES);
        let end = m.start() + m.size();
        let buf_len = buffer.len();

        while addr < end {
            let mut slice = &mut buffer[0..std::cmp::min(buf_len, end - addr)];
            if !handle.copy_address(addr, &mut slice).is_ok() {
                break;
            }

            let data = bytes_to_values(slice);
            for d in data {
                let addr = *d as usize;
                if addr % RVALUE_BYTES == 0 {
                    for p in &pages {
                        if p.deref(addr).is_some() {
                            if let Some(n) = indices.get(&addr) {
                                graph.add_edge(root, *n, EDGE_WEIGHT);
                            }
                        }
                    }
                }
            }

            addr += buf_len;
        }
    }

    for p in &pages {
        for (i, v) in p.contents().iter().enumerate() {
            if v.valid(&pages) && !v.free() {
                let addr = p.address(i);
                let n = indices[&addr];
                for r in v.references(&pages, &handle) {
                    if let Some(m) = indices.get(&r) {
                        graph.add_edge(n, *m, EDGE_WEIGHT);
                    }
                }
            }
        }
    }

    dbg!(graph.node_count());
    dbg!(graph.edge_count());
    Ok((root, graph))
}

#[inline]
fn bytes_to_values(data: &[u8]) -> &[VALUE] {
    // TODO Clippy seems right to warn about alignment here
    unsafe { std::slice::from_raw_parts(data.as_ptr() as *const VALUE, data.len() / POINTER_BYTES) }
}

#[inline]
fn values_to_bytes(data: &[VALUE]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * POINTER_BYTES) }
}

// Next address after `addr` that has given alignment
#[inline]
fn next_aligned(addr: usize, alignment: usize) -> usize {
    let offset = alignment - (addr % alignment);
    addr + offset
}