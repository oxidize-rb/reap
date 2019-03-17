use crate::object::*;
use petgraph::graph::NodeIndex;
use petgraph::Graph;
use proc_maps::{get_process_maps, MapRange};
use read_process_memory::{Pid, TryIntoProcessHandle, CopyAddress, copy_address};
use regex::bytes::Regex;
use std::fs::File;
use std::io::prelude::*;
use std::io::Cursor;
use timed_function::timed;
use std::collections::HashMap;
use byteorder::{NativeEndian, ReadBytesExt};

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
    Nil = 0x11,
    True = 0x12,
    False = 0x13,
    Symbol = 0x14,
    Fixnum = 0x15,
    Undef = 0x16,
    IMemo = 0x1a,
    Node = 0x1b,
    IClass = 0x1c,
    Zombie = 0x1d,
}

impl Type {
    #[inline]
    pub fn from_heap_flags(flags: u64) -> Result<Type, ()> {
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
enum RValue {
    Free { next: usize },
    Object { klass: usize },
    Class { klass: usize },
    Module { klass: usize },
    String { klass: usize },
    Array { klass: usize },
    Hash { klass: usize },
    Data { klass: usize },
    IMemo { data: [u64; 5] },
    Other { rbtype: Type, klass: usize },
    Invalid { data: [u64; 5] },
}

impl RValue {

    #[inline]
    pub fn from_data(heap_page: usize, offset: usize, data: &[u64]) -> RValue {
        debug_assert!(data.len() == RVALUE_WIDTH);

        let mut copy = [0u64; 5];
        copy.copy_from_slice(data);

        let flags = data[0];
        if flags > MAX_FLAGS {
            return RValue::Invalid { data: copy };
        }

        let pointer = data[1] as usize;
        if pointer % RVALUE_BYTES != 0 {
            return match Type::from_heap_flags(flags) {
                Ok(Type::IMemo) => RValue::IMemo { data: copy },
                _ => RValue::Invalid { data: copy },
            };
        }

        match Type::from_heap_flags(flags) {
            Ok(Type::None) => {
                if pointer == 0 || (pointer >= heap_page && pointer < heap_page + HEAP_PAGE_BYTES) {
                    RValue::Free { next: pointer }
                } else {
                    RValue::Invalid { data: copy }
                }
            },
            Ok(Type::Object) => RValue::Object { klass: pointer },
            Ok(Type::Class) => RValue::Class { klass: pointer },
            Ok(Type::Module) => RValue::Module { klass: pointer },
            Ok(Type::String) => RValue::String { klass: pointer },
            Ok(Type::Array) => RValue::Array { klass: pointer },
            Ok(Type::Hash) => RValue::Hash { klass: pointer },
            Ok(Type::Data) => RValue::Data { klass: pointer },
            Ok(Type::IMemo) => RValue::IMemo { data: copy },
            Ok(t) =>  RValue::Other { klass: pointer, rbtype: t },
            Err(_) => RValue::Invalid { data: copy },
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
    pub fn rb_type(&self) -> Type {
        match self {
            RValue::Free { .. } => Type::None,
            RValue::Object { .. } => Type::Object,
            RValue::Class { .. } => Type::Class,
            RValue::Module { .. } => Type::Module,
            RValue::String { .. } => Type::String,
            RValue::Array { .. } => Type::Array,
            RValue::Hash { .. } => Type::Hash,
            RValue::Data { .. } => Type::Data,
            RValue::IMemo { .. } => Type::IMemo,
            RValue::Other { rbtype, .. } => *rbtype,
            RValue::Invalid { .. }=> Type::Undef,
        }
    }

    #[inline]
    pub fn valid(&self, heap: &[HeapPage]) -> bool {
        /*
        // TODO Handle USE_FLONUM false for old versions?
        //
        const Qfalse: usize = 0x00;		/* ...0000 0000 */
        const Qtrue: usize  = 0x14;		/* ...0001 0100 */
        const Qnil: usize   = 0x08;		/* ...0000 1000 */
        const Qundef: usize = 0x34;		/* ...0011 0100 */

        const ImmediateMask: usize = 0x07;
        const FixnumFlag: usize    = 0x01;	/* ...xxxx xxx1 */
        const FlonumMask: usize    = 0x03;
        const FlonumFlag: usize    = 0x02;	/* ...xxxx xx10 */
        const SymbolFlag: usize    = 0x0c;	/* ...0000 1100 */
        const SpecialShift: usize  = 8;
        */

        let on_heap = |a| heap.iter().any(|p| p.deref(a).is_some());

        match self {
            RValue::Free { next, .. } => *next == 0 || on_heap(*next),
            RValue::Object { klass, .. } => on_heap(*klass),
            RValue::Class { klass, .. } => *klass == 0 || on_heap(*klass),
            RValue::Module { klass, .. } => on_heap(*klass),
            // TODO Understand this special case
            RValue::String { klass, .. } => *klass == 0 || on_heap(*klass),
            RValue::Array { klass, .. } => *klass == 0 || on_heap(*klass),
            RValue::Hash { klass, .. } => *klass == 0 || on_heap(*klass),
            RValue::Data { klass, .. } => *klass == 0 || on_heap(*klass),
            RValue::IMemo { .. } => true,
            RValue::Other { klass, .. } => on_heap(*klass),
            RValue::Invalid { .. } => false,
        }
    }
}

#[derive(Debug)]
struct HeapPage {
    addr: usize,
    rvalues: Vec<RValue>,
}

impl HeapPage {
    pub fn from_data(addr: usize, data: &[u64]) -> Result<HeapPage, ()> {
        let rvalues = data.chunks_exact(RVALUE_WIDTH).enumerate().map(|(i,v)| {
            RValue::from_data(addr, i, v)
        }).collect::<Vec<_>>();

        if rvalues.iter().filter(|v| match v { RValue::Invalid { .. } => true, _ => false }).count() >= 2 {
            Err(())
        } else if rvalues.iter().filter(|v| v.is_last_free_value()).count() >= 3 {
            Err(())
        } else {
            Ok(HeapPage { addr, rvalues })
        }
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

    let procmaps: Vec<MapRange> = get_process_maps(pid)?.into_iter().filter(|m| {
        m.is_read()// && !m.is_exec()// && m.is_write()
    }).collect();

    let mut pages: Vec<HeapPage> = Vec::new();
    let mut buffer = vec![0u8; HEAP_PAGE_BYTES];

    for m in procmaps {
        let mut addr: usize = next_aligned(m.start(), HEAP_PAGE_BYTES);

        let last_valid = m.start() + m.size() - buffer.len();
        let prev_pages = pages.len();

        while addr < last_valid {
            if !handle.copy_address(addr, &mut buffer).is_ok() {
                break;
            }

            let first_rvalue = next_aligned(addr, RVALUE_BYTES);
            let data = as_u64(&buffer[first_rvalue - addr..]);
            if let Ok(page) = HeapPage::from_data(first_rvalue, data) {
                pages.push(page);
            }

            addr += HEAP_PAGE_BYTES;
        }

        if pages.len() > prev_pages {
            dbg!((m.filename(), m.size(), pages.len() - prev_pages));
        }
    }

    let invalid_rvalues = (&pages).iter().flat_map(|p| p.contents()).filter(|r| !r.valid(&pages)).count();
    dbg!(invalid_rvalues);

    let mut g = Graph::default();
    let i = g.add_node(Object::root());
    Ok((i, g))
}

const POINTER_BYTES: usize = 8;
const MAX_FLAGS: u64 = u32::max_value() as u64;
const HEAP_PAGE_BYTES: usize = 16384;
const HEAP_PAGE_LEN: usize = HEAP_PAGE_BYTES / POINTER_BYTES;
const RVALUE_WIDTH: usize = 5;
const RVALUE_BYTES: usize = RVALUE_WIDTH * POINTER_BYTES;

#[inline]
fn as_u64(data: &[u8]) -> &[u64] {
    unsafe {
        std::slice::from_raw_parts(
            data.as_ptr() as *const u64,
            data.len() / 8
        )
    }
}

// Next address after `addr` that has given alignment
#[inline]
fn next_aligned(addr: usize, alignment: usize) -> usize {
    let offset = alignment - (addr % alignment);
    addr + offset
}
