//! Discovery of the StarCraft Remastered command-panel root callback.
//!
//! This module only reads through caller-supplied bounded readers. It never
//! follows child controls, changes protection, patches code, invokes callbacks,
//! or writes a game object. Discovery is not a lifetime reservation: callers
//! must run on the game's UI thread and recheck the current root immediately
//! before an exact compare-and-exchange of its callback slot. Never cache a
//! heap root for a later worker-thread write or teardown restore.
//!
//! The x64 SCR Control/BwString layout and two-argument u32 callback contract:
//! https://github.com/neivv/aise/blob/da4fed681dc09bda7a21c0051f60cdad98cc7226/bw_dat/src/bw/structs.rs
//! The distinct console roots, including StatBtn (command panel) and StatData:
//! https://github.com/ShieldBattery/ShieldBattery/blob/master/game/src/bw_scr/console.rs
//! Saved event_filter_entry.asm independently shows [control+0x60] called with
//! RCX=control and RDX=event, and its u32 return used as the consumed flag.

use std::collections::HashSet;

pub const CALLBACK_OFFSET: usize = 0x60;
pub const MAX_ROOTS: usize = 128;
const CONTROL_READ_SIZE: usize = 0x68;
const STRING_DATA_OFFSET: usize = 0x20;
const STRING_LENGTH_OFFSET: usize = 0x28;
const STRING_CAPACITY_OFFSET: usize = 0x30;
const STRING_INLINE_OFFSET: usize = 0x38;
const STRING_INLINE_SIZE: usize = 16;
const PANEL_NAME: &[u8; 8] = b"StatBtn\0";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Target {
    pub control: usize,
    pub slot_address: usize,
    /// Current callback. May equal the caller's already-installed replacement;
    /// never use replacement itself as an original to forward to.
    pub callback: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscoveryError {
    InvalidPointer,
    ReadFailed,
    CyclicRoots,
    TooManyRoots,
    InvalidName,
    DuplicatePanel,
    UnknownCallback,
    ChangedRoot,
}
impl std::fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Command panel discovery: {}", match self {
            Self::InvalidPointer => "invalid or unaligned root pointer",
            Self::ReadFailed => "root or name read failed",
            Self::CyclicRoots => "cyclic root list",
            Self::TooManyRoots => "root count exceeded limit",
            Self::InvalidName => "invalid command-panel name storage",
            Self::DuplicatePanel => "multiple StatBtn roots",
            Self::UnknownCallback => "callback is outside verified game code",
            Self::ChangedRoot => "root identity or callback changed",
        })
    }
}

fn pointer_ok(pointer: usize, len: usize) -> bool {
    pointer >= 0x10000 && pointer % 8 == 0 && pointer.checked_add(len).is_some()
}
fn word(bytes: &[u8], offset: usize) -> usize {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap()) as usize
}

fn panel_header<F>(control: usize, read: &mut F) -> Result<Option<Target>, DiscoveryError>
where F: FnMut(usize, &mut [u8]) -> bool {
    if !pointer_ok(control, CONTROL_READ_SIZE) {
        return Err(DiscoveryError::InvalidPointer);
    }
    let mut bytes = [0; CONTROL_READ_SIZE];
    if !read(control, &mut bytes) {
        return Err(DiscoveryError::ReadFailed);
    }
    panel_from_bytes(control, &bytes, read)
}
fn panel_from_bytes<F>(control: usize, bytes: &[u8], read: &mut F)
    -> Result<Option<Target>, DiscoveryError>
where F: FnMut(usize, &mut [u8]) -> bool {
    let length = word(bytes, STRING_LENGTH_OFFSET);
    if length != PANEL_NAME.len() - 1 {
        return Ok(None);
    }
    let data = word(bytes, STRING_DATA_OFFSET);
    // BwString's capacity sign bit marks inline storage; it is not a size bit.
    // ShieldBattery BwString::get_capacity documents this representation.
    let raw_capacity = word(bytes, STRING_CAPACITY_OFFSET);
    let inline = raw_capacity & !(usize::MAX >> 1) != 0;
    let capacity = raw_capacity & (usize::MAX >> 1);
    if data < 0x10000 || data.checked_add(PANEL_NAME.len()).is_none()
        || capacity < length || capacity > 4096
        || (inline && (data != control + STRING_INLINE_OFFSET || length >= STRING_INLINE_SIZE)) {
        return Err(DiscoveryError::InvalidName);
    }
    let mut name = [0; 8];
    if !read(data, &mut name) {
        return Err(DiscoveryError::ReadFailed);
    }
    if &name != PANEL_NAME {
        return Ok(None);
    }
    Ok(Some(Target {
        control,
        slot_address: control + CALLBACK_OFFSET,
        callback: word(bytes, CALLBACK_OFFSET),
    }))
}

/// Traverse only a bounded, acyclic list of root dialogs. The exact command-panel
/// name prevents confusing the wireframe/status dialog with the command panel.
/// Original callback candidates must be in caller-verified executable game code.
/// Existing replacements are recognized but never recorded as originals here.
pub fn discover_stat_button<F, C>(
    first_dialog: usize,
    replacement: usize,
    is_code: C,
    mut read: F,
) -> Result<Option<Target>, DiscoveryError>
where F: FnMut(usize, &mut [u8]) -> bool, C: Fn(usize) -> bool {
    let mut seen = HashSet::new();
    let mut current = first_dialog;
    let mut result = None;
    while current != 0 {
        if seen.len() >= MAX_ROOTS {
            return Err(DiscoveryError::TooManyRoots);
        }
        if !pointer_ok(current, CONTROL_READ_SIZE) {
            return Err(DiscoveryError::InvalidPointer);
        }
        if !seen.insert(current) {
            return Err(DiscoveryError::CyclicRoots);
        }
        let mut bytes = [0; CONTROL_READ_SIZE];
        if !read(current, &mut bytes) {
            return Err(DiscoveryError::ReadFailed);
        }
        let next = word(&bytes, 0);
        if let Some(target) = panel_from_bytes(current, &bytes, &mut read)? {
            if target.callback == 0 || (target.callback != replacement && !is_code(target.callback)) {
                return Err(DiscoveryError::UnknownCallback);
            }
            if result.replace(target).is_some() {
                return Err(DiscoveryError::DuplicatePanel);
            }
        }
        current = next;
    }
    Ok(result)
}

/// Recheck an object while its callback is being dispatched or immediately
/// before a UI-thread binding. Caller must separately recheck root-list identity
/// when installing and serialize its own registration operations.
pub fn recheck_target<F>(target: Target, mut read: F) -> Result<(), DiscoveryError>
where F: FnMut(usize, &mut [u8]) -> bool {
    if target.slot_address != target.control.checked_add(CALLBACK_OFFSET)
        .ok_or(DiscoveryError::InvalidPointer)? {
        return Err(DiscoveryError::InvalidPointer);
    }
    if panel_header(target.control, &mut read)? != Some(target) {
        return Err(DiscoveryError::ChangedRoot);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const BASE: usize = 0x10000;
    const ORIGINAL: usize = 0x50000;
    const REPLACEMENT: usize = 0x90000;
    struct Fixture { bytes: Vec<u8> }
    impl Fixture {
        fn new() -> Self { Self { bytes: vec![0; 0x10000] } }
        fn root(&mut self, at: usize, next: usize, name: &[u8], callback: usize) {
            let offset = at - BASE;
            let name_at = at + 0x80;
            for (field, value) in [(0, next), (0x20, name_at),
                (0x28, name.len()), (0x30, 0x40), (0x60, callback)] {
                self.bytes[offset + field..offset + field + 8]
                    .copy_from_slice(&(value as u64).to_le_bytes());
            }
            self.bytes[offset + 0x80..offset + 0x80 + name.len()].copy_from_slice(name);
            self.bytes[offset + 0x80 + name.len()] = 0;
        }
        fn read(&self, at: usize, out: &mut [u8]) -> bool {
            let Some(start) = at.checked_sub(BASE) else { return false; };
            let Some(end) = start.checked_add(out.len()) else { return false; };
            let Some(bytes) = self.bytes.get(start..end) else { return false; };
            out.copy_from_slice(bytes); true
        }
        fn find(&self, first: usize) -> Result<Option<Target>, DiscoveryError> {
            discover_stat_button(first, REPLACEMENT, |p| p == ORIGINAL,
                |at, out| self.read(at, out))
        }
    }
    #[test]
    #[cfg(target_pointer_width = "64")]
    fn upstream_repr_c_control_layout_matches_saved_x64_dispatch_contract() {
        // Field sequence copied from the primary SCR definitions linked above,
        // rather than constructing this schema from our discovery constants.
        #[repr(C)] struct Rect { left: i16, top: i16, right: i16, bottom: i16 }
        #[repr(C)] struct Surface { width: u16, height: u16, data: *mut u8 }
        #[repr(C)] struct BwString {
            data: *const u8, length: usize, capacity: usize, inline_buffer: [u8; 16],
        }
        #[repr(C)] struct Control {
            next: *mut Control, area: Rect, image: Surface, string: BwString,
            flags: u32, flags2: u32, unknown: u16, id: i16, ty: u16,
            misc_u16: u16, user_ptr: *mut std::ffi::c_void,
            event_handler: Option<unsafe extern "C" fn(*mut Control, *mut std::ffi::c_void) -> u32>,
            draw: Option<unsafe extern "C" fn(*mut Control, i32, i32, *const Rect, *const Rect)>,
            parent: *mut std::ffi::c_void,
        }
        assert_eq!(std::mem::offset_of!(Control, next), 0);
        assert_eq!(std::mem::offset_of!(Control, string), STRING_DATA_OFFSET);
        assert_eq!(std::mem::offset_of!(Control, string) + std::mem::offset_of!(BwString, length), STRING_LENGTH_OFFSET);
        assert_eq!(std::mem::offset_of!(Control, string) + std::mem::offset_of!(BwString, capacity), STRING_CAPACITY_OFFSET);
        assert_eq!(std::mem::offset_of!(Control, flags), 0x48);
        assert_eq!(std::mem::offset_of!(Control, event_handler), CALLBACK_OFFSET);
        assert_eq!(std::mem::offset_of!(Control, draw), CONTROL_READ_SIZE);
    }
    #[test]
    fn finds_exact_command_panel_among_distinct_console_roots() {
        let mut f = Fixture::new();
        f.root(BASE, BASE + 0x100, b"StatData", ORIGINAL);
        f.root(BASE + 0x100, BASE + 0x200, b"StatBtn", ORIGINAL);
        f.root(BASE + 0x200, 0, b"Minimap", ORIGINAL);
        assert_eq!(f.find(BASE).unwrap(), Some(Target {
            control: BASE + 0x100, slot_address: BASE + 0x160, callback: ORIGINAL,
        }));
        assert_eq!(f.find(0).unwrap(), None);
    }
    #[test]
    fn inline_flagged_statbtn_and_minimap_names_are_valid() {
        let mut f = Fixture::new();
        f.root(BASE, BASE + 0x100, b"Minimap", ORIGINAL);
        f.root(BASE + 0x100, 0, b"StatBtn", ORIGINAL);
        for (offset, name) in [(0, b"Minimap\0"), (0x100, b"StatBtn\0")] {
            let data = BASE + offset + 0x38;
            f.bytes[offset + 0x20..offset + 0x28].copy_from_slice(&(data as u64).to_le_bytes());
            f.bytes[offset + 0x30..offset + 0x38].copy_from_slice(&0x8000_0000_0000_000fu64.to_le_bytes());
            f.bytes[offset + 0x38..offset + 0x40].copy_from_slice(name);
        }
        let target = f.find(BASE).unwrap().unwrap();
        assert_eq!(target.control, BASE + 0x100);
        assert!(recheck_target(target, |at, out| f.read(at, out)).is_ok());
    }
    #[test]
    fn inline_flag_does_not_authorize_an_unrelated_pointer_or_small_capacity() {
        for capacity in [0x8000_0000_0000_000fu64, 0x8000_0000_0000_0006u64] {
            let mut f = Fixture::new();
            f.root(BASE, 0, b"StatBtn", ORIGINAL);
            f.bytes[0x30..0x38].copy_from_slice(&capacity.to_le_bytes());
            assert_eq!(f.find(BASE), Err(DiscoveryError::InvalidName));
        }
    }
    #[test]
    fn rejects_status_only_case_suffix_and_missing_nul() {
        for name in [b"StatData".as_slice(), b"statbtn", b"StatBtm", b"StatBtnX"] {
            let mut f = Fixture::new(); f.root(BASE, 0, name, ORIGINAL);
            assert_eq!(f.find(BASE).unwrap(), None);
        }
        let mut f = Fixture::new(); f.root(BASE, 0, b"StatBtn", ORIGINAL);
        f.bytes[0x87] = b'X';
        assert_eq!(f.find(BASE).unwrap(), None);
    }
    #[test]
    fn accepts_owned_replacement_but_rejects_unknown_or_null_original() {
        for callback in [ORIGINAL, REPLACEMENT] {
            let mut f = Fixture::new(); f.root(BASE, 0, b"StatBtn", callback);
            assert_eq!(f.find(BASE).unwrap().unwrap().callback, callback);
        }
        for callback in [0, 0x111111] {
            let mut f = Fixture::new(); f.root(BASE, 0, b"StatBtn", callback);
            assert_eq!(f.find(BASE), Err(DiscoveryError::UnknownCallback));
        }
    }
    #[test]
    fn duplicate_panel_and_cycles_are_rejected() {
        let mut f = Fixture::new();
        f.root(BASE, BASE + 0x100, b"StatBtn", ORIGINAL);
        f.root(BASE + 0x100, 0, b"StatBtn", ORIGINAL);
        assert_eq!(f.find(BASE), Err(DiscoveryError::DuplicatePanel));
        f.root(BASE + 0x100, BASE, b"Minimap", ORIGINAL);
        assert_eq!(f.find(BASE), Err(DiscoveryError::CyclicRoots));
    }
    #[test]
    fn enforces_root_count_and_pointer_bounds() {
        let mut f = Fixture::new();
        for n in 0..=MAX_ROOTS {
            let at = BASE + n * 0x100;
            f.root(at, if n == MAX_ROOTS { 0 } else { at + 0x100 }, b"Other", ORIGINAL);
        }
        assert_eq!(f.find(BASE), Err(DiscoveryError::TooManyRoots));
        for at in [1, BASE + 1, usize::MAX - 7] {
            assert_eq!(f.find(at), Err(DiscoveryError::InvalidPointer));
        }
        assert_eq!(f.find(BASE + 0x20000), Err(DiscoveryError::ReadFailed));
    }
    #[test]
    fn malformed_name_storage_and_reader_failures_are_rejected() {
        for (field, value) in [(0x20, 0), (0x30, 6), (0x30, 4097)] {
            let mut f = Fixture::new(); f.root(BASE, 0, b"StatBtn", ORIGINAL);
            f.bytes[field..field + 8].copy_from_slice(&(value as u64).to_le_bytes());
            assert_eq!(f.find(BASE), Err(DiscoveryError::InvalidName));
        }
        let mut f = Fixture::new(); f.root(BASE, 0, b"StatBtn", ORIGINAL);
        f.bytes[0x20..0x28].copy_from_slice(&((BASE + 0x20000) as u64).to_le_bytes());
        assert_eq!(f.find(BASE), Err(DiscoveryError::ReadFailed));
    }
    #[test]
    fn recheck_detects_callback_identity_and_name_changes() {
        let mut f = Fixture::new(); f.root(BASE, 0, b"StatBtn", ORIGINAL);
        let target = f.find(BASE).unwrap().unwrap();
        assert!(recheck_target(target, |at, out| f.read(at, out)).is_ok());
        f.root(BASE, 0, b"StatBtn", REPLACEMENT);
        assert_eq!(recheck_target(target, |at, out| f.read(at, out)), Err(DiscoveryError::ChangedRoot));
        f.root(BASE, 0, b"Minimap", ORIGINAL);
        assert_eq!(recheck_target(target, |at, out| f.read(at, out)), Err(DiscoveryError::ChangedRoot));
        let bad = Target { slot_address: target.slot_address + 8, ..target };
        assert_eq!(recheck_target(bad, |at, out| f.read(at, out)), Err(DiscoveryError::InvalidPointer));
    }
}
