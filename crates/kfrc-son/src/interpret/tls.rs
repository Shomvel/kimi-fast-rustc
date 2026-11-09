//! Explicit interpreter thread identity shared across interpreted functions.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use kfrc_body::DataObject;

use super::InterpretError;

const ARENA_SIZE: u64 = 1 << 32;
static NEXT_THREAD_ARENA: AtomicU64 = AtomicU64::new(0x1000_0000_0000);

#[derive(Debug)]
struct ThreadObject {
    address: u64,
    template: DataObject,
}

/// One logical thread's TLS instances. Reuse this context across functions to
/// preserve values; a distinct context owns independent storage and addresses.
#[derive(Debug)]
pub struct InterpreterThread {
    objects: HashMap<String, ThreadObject>,
    memory: HashMap<u64, u8>,
    next_address: u64,
    limit: u64,
}

impl Default for InterpreterThread {
    fn default() -> Self {
        let base = NEXT_THREAD_ARENA.fetch_add(ARENA_SIZE, Ordering::Relaxed);
        Self {
            objects: HashMap::new(),
            memory: HashMap::new(),
            next_address: base,
            limit: base + ARENA_SIZE,
        }
    }
}

impl InterpreterThread {
    pub fn new() -> Self {
        Self::default()
    }

    pub(super) fn ensure(&mut self, object: &DataObject) -> Result<(u64, bool), InterpretError> {
        let size = object.bytes.len() as u64;
        if let Some(existing) = self.objects.get(&object.symbol) {
            if existing.template != *object {
                return Err(InterpretError::Malformed(format!(
                    "conflicting TLS template for {}",
                    object.symbol,
                )));
            }
            return Ok((existing.address, false));
        }
        if !object.align.is_power_of_two() {
            return Err(InterpretError::Malformed(
                "invalid TLS alignment".to_owned(),
            ));
        }
        let address = self.next_address.next_multiple_of(object.align);
        self.next_address = address
            .checked_add(size.max(1))
            .and_then(|end| end.checked_add(16))
            .filter(|end| *end <= self.limit)
            .ok_or_else(|| {
                InterpretError::Malformed("TLS interpreter arena exhausted".to_owned())
            })?;
        self.objects.insert(
            object.symbol.clone(),
            ThreadObject {
                address,
                template: object.clone(),
            },
        );
        Ok((address, true))
    }

    pub(super) fn address(&self, symbol: &str) -> Option<u64> {
        self.objects.get(symbol).map(|object| object.address)
    }

    pub(super) fn restore(&self, memory: &mut HashMap<u64, u8>) {
        memory.extend(self.memory.iter().map(|(address, byte)| (*address, *byte)));
    }

    pub(super) fn save(&mut self, memory: &HashMap<u64, u8>) {
        for object in self.objects.values() {
            for offset in 0..object.template.bytes.len() as u64 {
                let address = object.address + offset;
                if let Some(byte) = memory.get(&address) {
                    self.memory.insert(address, *byte);
                } else {
                    self.memory.remove(&address);
                }
            }
        }
    }
}
