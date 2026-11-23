// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::mem::{MemFault, Perm};

#[test]
fn isolates_snapshots_from_updates_and_reused_slots() {
    let mut memory = GuestMemory::new();
    assert!(memory.map(0x1000, 20, Perm::RW, 0x55));
    assert!(memory.map(0x2000, 12, Perm::READ, 0xaa));
    let mut resources = Resources::default();
    let first = resources.create(&memory, 0x1000, 20, 1).unwrap();
    memory.write_bytes(0x1004, &[0x99; 12]).unwrap();
    resources.queue(Operation::Xor, first, 4, 12).unwrap();
    resources.update(&memory, 0x2000, first, 4, 12).unwrap();
    let slot = &resources.slots[resources.index(first).unwrap()];
    let bytes = &slot.resource.as_ref().unwrap().bytes;
    assert_eq!(&bytes[..4], [0x55; 4]);
    assert_eq!(&bytes[4..16], [0xaa; 12]);
    assert_eq!(&bytes[16..], [0x55; 4]);

    resources.release(first).unwrap();
    let replacement = resources.create(&memory, 0x2000, 12, 1).unwrap();
    assert_eq!(replacement & 7, first & 7);
    assert_ne!(replacement, first);
    assert_eq!(resources.index(first), Err(FaultKind::StaleResource));

    let pending = resources.pending.as_ref().unwrap();
    assert_eq!(&pending.bytes[pending.range.clone()], [0x55; 12]);
}

#[test]
fn guest_teardown_drops_live_slots_and_queued_snapshots() {
    let mut memory = GuestMemory::new();
    assert!(memory.map(0x1000, 12, Perm::READ, 0x5a));
    let (retired_handle, queued, live) = {
        let mut resources = Resources::default();
        let handle = resources.create(&memory, 0x1000, 12, 1).unwrap();
        resources.queue(Operation::Xor, handle, 0, 12).unwrap();
        let queued = Arc::downgrade(&resources.pending.as_ref().unwrap().bytes);
        resources.release(handle).unwrap();
        assert!(queued.upgrade().is_some());

        let live_handle = resources.create(&memory, 0x1000, 12, 1).unwrap();
        let slot = &resources.slots[resources.index(live_handle).unwrap()];
        let live = Arc::downgrade(&slot.resource.as_ref().unwrap().bytes);
        (live_handle, queued, live)
    };
    assert!(queued.upgrade().is_none());
    assert!(live.upgrade().is_none());

    let mut next_guest = Resources::default();
    assert_eq!(
        next_guest.index(retired_handle),
        Err(FaultKind::InvalidResource)
    );
    next_guest.create(&memory, 0x1000, 12, 1).unwrap();
    assert_eq!(
        next_guest.index(retired_handle),
        Err(FaultKind::StaleResource)
    );
    assert!(next_guest.pending.is_none());
}

#[test]
fn rejects_invalid_spans_without_mutating_resources_or_guest_output() {
    let mut memory = GuestMemory::new();
    assert!(memory.map(0x1000, 12, Perm::READ, 0x11));
    assert!(memory.map(0x2000, 8, Perm::RW, 0x22));
    assert!(memory.map(0x2008, 4, Perm::RW, 0x33));
    let mut resources = Resources::default();
    let handle = resources.create(&memory, 0x1000, 12, 1).unwrap();

    for (offset, length) in [(4, 12), (u32::MAX - 3, 12), (1, 4), (0, 0)] {
        assert_eq!(
            resources.queue(Operation::Xor, handle, offset, length),
            Err(FaultKind::InvalidResourceView)
        );
    }
    assert_eq!(
        resources.queue(Operation::Draw(draw::Program::Constant), handle, 0, 12),
        Err(FaultKind::InvalidResourceView)
    );
    assert_eq!(
        resources.update(&memory, 0x2000, handle, 0, 12),
        Err(FaultKind::MemFault(MemFault::CrossBoundary))
    );
    resources.queue(Operation::Xor, handle, 0, 12).unwrap();

    for (address, fault) in [
        (0x1000, MemFault::NotWritable),
        (0x2000, MemFault::CrossBoundary),
    ] {
        assert_eq!(
            resources.finish(&mut memory, address, 12),
            Err(FaultKind::MemFault(fault))
        );
    }
    assert_eq!(memory.read_bytes(0x1000, 12).unwrap(), [0x11; 12]);
    assert_eq!(memory.read_bytes(0x2000, 8).unwrap(), [0x22; 8]);
    assert_eq!(memory.read_bytes(0x2008, 4).unwrap(), [0x33; 4]);
    assert_eq!(&*resources.pending.as_ref().unwrap().bytes, [0x11; 12]);
    assert!(resources.readbacks.is_empty());
}

#[test]
fn rejects_allocations_past_the_slot_limit() {
    let mut memory = GuestMemory::new();
    assert!(memory.map(0x1000, 4, Perm::READ, 0));
    let mut resources = Resources::default();
    for i in 0..SLOT_COUNT as u32 {
        assert_eq!(resources.create(&memory, 0x1000, 4, 1).unwrap() & 7, i);
    }
    assert_eq!(
        resources.create(&memory, 0x1000, 4, 1),
        Err(FaultKind::ResourceLimit)
    );
}
