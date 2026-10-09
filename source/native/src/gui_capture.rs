//! Discovery of StarCraft Remastered console roots and direct command-panel buttons.
//!
//! This module reads bounded root lists and direct command-panel button children
//! through caller-supplied readers. It never changes protection, patches code,
//! invokes callbacks, or writes a game object. Discovery is not a lifetime
//! reservation: callers must recheck current membership on the game's UI thread
//! immediately before an exact callback-slot compare-and-exchange. Never cache
//! heap controls for later worker-thread writes or teardown restores.
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
const MINIMAP_NAME: &[u8; 8] = b"Minimap\0";

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
    DuplicateMinimap,
    InvalidMinimapRoot,
    UnknownCallback,
    ChangedRoot,
    InvalidPanelRoot,
    TooManyChildren,
    CyclicChildren,
    WrongParent,
    ChangedChild,
}
impl std::fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Console callback discovery: {}", match self {
            Self::InvalidPointer => "invalid or unaligned root pointer",
            Self::ReadFailed => "root or name read failed",
            Self::CyclicRoots => "cyclic root list",
            Self::TooManyRoots => "root count exceeded limit",
            Self::InvalidName => "invalid command-panel name storage",
            Self::DuplicatePanel => "multiple StatBtn roots",
            Self::DuplicateMinimap => "multiple Minimap roots",
            Self::InvalidMinimapRoot => "Minimap is not a dialog root",
            Self::UnknownCallback => "callback is outside verified game code",
            Self::ChangedRoot => "root identity or callback changed",
            Self::InvalidPanelRoot => "StatBtn is not a dialog root",
            Self::TooManyChildren => "command-panel child count exceeded limit",
            Self::CyclicChildren => "cyclic command-panel child list",
            Self::WrongParent => "command-panel child has an incorrect parent",
            Self::ChangedChild => "command-panel child membership or callback changed",
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
    named_from_bytes(control, bytes, PANEL_NAME, read)
}
fn named_from_bytes<F>(control: usize, bytes: &[u8], expected_name: &[u8; 8], read: &mut F)
    -> Result<Option<Target>, DiscoveryError>
where F: FnMut(usize, &mut [u8]) -> bool {
    let length = word(bytes, STRING_LENGTH_OFFSET);
    if length != expected_name.len() - 1 {
        return Ok(None);
    }
    let data = word(bytes, STRING_DATA_OFFSET);
    // BwString's capacity sign bit marks inline storage; it is not a size bit.
    // ShieldBattery BwString::get_capacity documents this representation.
    let raw_capacity = word(bytes, STRING_CAPACITY_OFFSET);
    let inline = raw_capacity & !(usize::MAX >> 1) != 0;
    let capacity = raw_capacity & (usize::MAX >> 1);
    if data < 0x10000 || data.checked_add(expected_name.len()).is_none()
        || capacity < length || capacity > 4096
        || (inline && (data != control + STRING_INLINE_OFFSET || length >= STRING_INLINE_SIZE)) {
        return Err(DiscoveryError::InvalidName);
    }
    let mut name = [0; 8];
    if !read(data, &mut name) {
        return Err(DiscoveryError::ReadFailed);
    }
    if &name != expected_name {
        return Ok(None);
    }
    Ok(Some(Target {
        control,
        slot_address: control + CALLBACK_OFFSET,
        callback: word(bytes, CALLBACK_OFFSET),
    }))
}

#[derive(Debug, Eq, PartialEq)]
struct NamedObservation {
    target: Option<Target>,
    roots: Vec<(usize, [u8; CONTROL_READ_SIZE])>,
}

fn discover_named_root<F, C>(
    first_dialog: usize,
    replacement: usize,
    expected_name: &[u8; 8],
    duplicate_error: DiscoveryError,
    is_code: &C,
    read: &mut F,
) -> Result<NamedObservation, DiscoveryError>
where F: FnMut(usize, &mut [u8]) -> bool, C: Fn(usize) -> bool {
    let mut seen = HashSet::new();
    let mut current = first_dialog;
    let mut result = None;
    let mut roots = Vec::new();
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
        if let Some(target) = named_from_bytes(current, &bytes, expected_name, read)? {
            if target.callback == 0 || (target.callback != replacement && !is_code(target.callback)) {
                return Err(DiscoveryError::UnknownCallback);
            }
            if result.replace(target).is_some() {
                return Err(duplicate_error);
            }
        }
        roots.push((current, bytes));
        current = next;
    }
    Ok(NamedObservation { target: result, roots })
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
    Ok(discover_named_root(first_dialog, replacement, PANEL_NAME,
        DiscoveryError::DuplicatePanel, &is_code, &mut read)?.target)
}

/// Discover the exact Minimap dialog root, independently of StatBtn. Minimap
/// target clicks dispatch through this root's own two-argument u32 callback.
/// Both passes must agree on bounded root topology, headers, exact name, and
/// provider; discovery never follows children, invokes a callback, or writes.
/// Caller must still recheck current membership immediately before a UI-thread
/// callback-slot CAS, because two stable reads do not reserve object lifetime.
pub fn discover_minimap<F, C>(
    first_dialog: usize,
    replacement: usize,
    is_code: C,
    mut read: F,
) -> Result<Option<Target>, DiscoveryError>
where F: FnMut(usize, &mut [u8]) -> bool, C: Fn(usize) -> bool {
    let first = discover_named_root(first_dialog, replacement, MINIMAP_NAME,
        DiscoveryError::DuplicateMinimap, &is_code, &mut read)?;
    if let Some(target) = first.target {
        let header = first.roots.iter().find(|(address, _)| *address == target.control)
            .ok_or(DiscoveryError::ChangedRoot)?;
        if control_type(&header.1) != 0 { return Err(DiscoveryError::InvalidMinimapRoot); }
    }
    let second = discover_named_root(first_dialog, replacement, MINIMAP_NAME,
        DiscoveryError::DuplicateMinimap, &is_code, &mut read)?;
    if first != second { return Err(DiscoveryError::ChangedRoot); }
    Ok(first.target)
}

/// Re-resolve exact current Minimap list membership and provider. A readable old
/// heap control or matching name outside the current root list is insufficient.
pub fn recheck_minimap<F, C>(
    first_dialog: usize,
    expected: Target,
    replacement: usize,
    is_code: C,
    read: F,
) -> Result<(), DiscoveryError>
where F: FnMut(usize, &mut [u8]) -> bool, C: Fn(usize) -> bool {
    if expected.slot_address != expected.control.checked_add(CALLBACK_OFFSET)
        .ok_or(DiscoveryError::InvalidPointer)?
        || !pointer_ok(expected.control, CONTROL_READ_SIZE) {
        return Err(DiscoveryError::InvalidPointer);
    }
    if discover_minimap(first_dialog, replacement, is_code, read)? == Some(expected) { Ok(()) }
    else { Err(DiscoveryError::ChangedRoot) }
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


/// The command-panel root and its verified direct button children. These are
/// observations for an immediate UI-thread recheck, not cached heap ownership.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Panel {
    pub root: Target,
    pub buttons: Vec<Target>,
}

const CHILD_READ_SIZE: usize = 0x78;
const FIRST_CHILD_OFFSET: usize = 0x90;
const PARENT_OFFSET: usize = 0x70;
const CONTROL_TYPE_OFFSET: usize = 0x54;
const MAX_PANEL_CHILDREN: usize = 64;

fn control_type(bytes: &[u8]) -> u16 {
    u16::from_le_bytes(bytes[CONTROL_TYPE_OFFSET..CONTROL_TYPE_OFFSET + 2].try_into().unwrap())
}

/// Discover only direct default-button/button controls belonging to the exact
/// StatBtn root. SCR Control/ Dialog layouts and per-child dispatch are defined
/// by the pinned bw_dat sources linked above. ShieldBattery's control_type_name
/// identifies types 1 and 2 as buttons. IDs and labels are not guessed or read.
///
/// The traversal is bounded and acyclic, rejects incorrect parents, and checks
/// every eligible provider against verified code or caller-owned replacements.
/// Unsupported control types are ignored after their parent was validated.
/// A second read checks the observed topology before returning. Callers must
/// still recheck on the UI thread immediately before an exact callback-slot CAS.
pub fn discover_stat_buttons<F, C>(
    first_dialog: usize,
    root_replacement: usize,
    child_replacements: &[usize],
    is_code: C,
    mut read: F,
) -> Result<Option<Panel>, DiscoveryError>
where F: FnMut(usize, &mut [u8]) -> bool, C: Fn(usize) -> bool {
    let Some(root) = discover_stat_button(first_dialog, root_replacement, &is_code, &mut read)? else {
        return Ok(None);
    };
    if !pointer_ok(root.control, CHILD_READ_SIZE) { return Err(DiscoveryError::InvalidPointer); }
    let mut root_header = [0; CHILD_READ_SIZE];
    if !read(root.control, &mut root_header) { return Err(DiscoveryError::ReadFailed); }
    if control_type(&root_header) != 0 { return Err(DiscoveryError::InvalidPanelRoot); }
    if panel_from_bytes(root.control, &root_header, &mut read)? != Some(root) {
        return Err(DiscoveryError::ChangedRoot);
    }
    let child_slot = root.control.checked_add(FIRST_CHILD_OFFSET)
        .ok_or(DiscoveryError::InvalidPointer)?;
    if !pointer_ok(child_slot, 8) { return Err(DiscoveryError::InvalidPointer); }
    let mut first_child = [0; 8];
    if !read(child_slot, &mut first_child) { return Err(DiscoveryError::ReadFailed); }
    let mut current = word(&first_child, 0);
    let mut seen = HashSet::new();
    let mut observed = Vec::new();
    let mut buttons = Vec::new();
    while current != 0 {
        if seen.len() >= MAX_PANEL_CHILDREN { return Err(DiscoveryError::TooManyChildren); }
        if !pointer_ok(current, CHILD_READ_SIZE) { return Err(DiscoveryError::InvalidPointer); }
        if current == root.control || !seen.insert(current) {
            return Err(DiscoveryError::CyclicChildren);
        }
        let mut header = [0; CHILD_READ_SIZE];
        if !read(current, &mut header) { return Err(DiscoveryError::ReadFailed); }
        if word(&header, PARENT_OFFSET) != root.control {
            return Err(DiscoveryError::WrongParent);
        }
        if matches!(control_type(&header), 1 | 2) {
            let callback = word(&header, CALLBACK_OFFSET);
            if callback == 0 || (!child_replacements.contains(&callback) && !is_code(callback)) {
                return Err(DiscoveryError::UnknownCallback);
            }
            buttons.push(Target { control: current,
                slot_address: current + CALLBACK_OFFSET, callback });
        }
        observed.push((current, header));
        current = word(&header, 0);
    }
    for (address, header) in observed {
        let mut again = [0; CHILD_READ_SIZE];
        if !read(address, &mut again) { return Err(DiscoveryError::ReadFailed); }
        if again != header { return Err(DiscoveryError::ChangedChild); }
    }
    let mut again_root = [0; CHILD_READ_SIZE];
    let mut again_first = [0; 8];
    if !read(root.control, &mut again_root) || !read(child_slot, &mut again_first) {
        return Err(DiscoveryError::ReadFailed);
    }
    if again_root != root_header || again_first != first_child
        || discover_stat_button(first_dialog, root_replacement, &is_code, &mut read)? != Some(root) {
        return Err(DiscoveryError::ChangedRoot);
    }
    Ok(Some(Panel { root, buttons }))
}

/// Re-resolve the current root list and exact child membership/provider. Never
/// treat a still-readable old heap address as proof that it remains a StatBtn
/// child, and never substitute the root's callback for a child's own provider.
pub fn recheck_stat_child<F, C>(
    first_dialog: usize,
    expected: Target,
    root_replacement: usize,
    child_replacements: &[usize],
    is_code: C,
    read: F,
) -> Result<(), DiscoveryError>
where F: FnMut(usize, &mut [u8]) -> bool, C: Fn(usize) -> bool {
    if expected.slot_address != expected.control.checked_add(CALLBACK_OFFSET)
        .ok_or(DiscoveryError::InvalidPointer)? {
        return Err(DiscoveryError::InvalidPointer);
    }
    if !pointer_ok(expected.control, CHILD_READ_SIZE) { return Err(DiscoveryError::InvalidPointer); }
    let panel = discover_stat_buttons(first_dialog, root_replacement, child_replacements, is_code, read)?;
    if panel.as_ref().is_some_and(|panel| panel.buttons.contains(&expected)) { Ok(()) }
    else { Err(DiscoveryError::ChangedChild) }
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

    const CHILD_ORIGINAL: usize = 0x60000;
    const CHILD_REPLACEMENT: usize = 0xa0000;
    const CHILD_REPLACEMENT_TWO: usize = 0xb0000;
    impl Fixture {
        fn set_word(&mut self, at: usize, field: usize, value: usize) {
            let start = at - BASE + field;
            self.bytes[start..start + 8].copy_from_slice(&(value as u64).to_le_bytes());
        }
        fn child(&mut self, at: usize, next: usize, parent: usize, ty: u16, callback: usize) {
            self.set_word(at, 0, next);
            self.set_word(at, CALLBACK_OFFSET, callback);
            self.set_word(at, PARENT_OFFSET, parent);
            let start = at - BASE + CONTROL_TYPE_OFFSET;
            self.bytes[start..start + 2].copy_from_slice(&ty.to_le_bytes());
        }
        fn panel(&self, first: usize) -> Result<Option<Panel>, DiscoveryError> {
            discover_stat_buttons(first, REPLACEMENT, &[CHILD_REPLACEMENT, CHILD_REPLACEMENT_TWO],
                |p| matches!(p, ORIGINAL | CHILD_ORIGINAL), |at, out| self.read(at, out))
        }
        fn check_child(&self, first: usize, target: Target) -> Result<(), DiscoveryError> {
            recheck_stat_child(first, target, REPLACEMENT, &[CHILD_REPLACEMENT, CHILD_REPLACEMENT_TWO],
                |p| matches!(p, ORIGINAL | CHILD_ORIGINAL), |at, out| self.read(at, out))
        }
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn upstream_scr_dialog_layout_places_first_child_and_parent_in_verified_fields() {
        #[repr(C)] struct Rect { left: i16, top: i16, right: i16, bottom: i16 }
        #[repr(C)] struct Surface { width: u16, height: u16, data: *mut u8 }
        #[repr(C)] struct BwString { data: *const u8, length: usize, capacity: usize, inline: [u8; 16] }
        #[repr(C)] struct Control {
            next: *mut Control, area: Rect, image: Surface, string: BwString,
            flags: u32, flags2: u32, unknown: u16, id: i16, ty: u16, misc_u16: u16,
            user_ptr: *mut std::ffi::c_void,
            event: Option<unsafe extern "C" fn(*mut Control, *mut std::ffi::c_void) -> u32>,
            draw: Option<unsafe extern "C" fn(*mut Control, i32, i32, *const Rect, *const Rect)>,
            parent: *mut Dialog,
        }
        #[repr(C)] struct Dialog {
            control: Control, surface: Surface, highlighted: *mut Control,
            first_child: *mut Control, active: *mut Control,
        }
        assert_eq!(std::mem::size_of::<Control>(), CHILD_READ_SIZE);
        assert_eq!(std::mem::offset_of!(Control, event), CALLBACK_OFFSET);
        assert_eq!(std::mem::offset_of!(Control, parent), PARENT_OFFSET);
        assert_eq!(std::mem::offset_of!(Control, ty), CONTROL_TYPE_OFFSET);
        assert_eq!(std::mem::offset_of!(Dialog, first_child), FIRST_CHILD_OFFSET);
    }

    #[test]
    fn command_children_can_bypass_root_and_keep_separate_verified_providers() {
        let mut f = Fixture::new();
        f.root(BASE, BASE + 0x100, b"StatData", ORIGINAL);
        f.root(BASE + 0x100, BASE + 0x200, b"StatBtn", REPLACEMENT);
        f.root(BASE + 0x200, 0, b"Minimap", ORIGINAL);
        f.set_word(BASE + 0x100, FIRST_CHILD_OFFSET, BASE + 0x400);
        f.child(BASE + 0x400, BASE + 0x500, BASE + 0x100, 1, CHILD_ORIGINAL);
        f.child(BASE + 0x500, BASE + 0x600, BASE + 0x100, 2, ORIGINAL);
        f.child(BASE + 0x600, 0, BASE + 0x100, 9, 0); // A label is not a button.
        // Eligible child labels need not be strings; no child name is followed.
        f.set_word(BASE + 0x400, STRING_DATA_OFFSET, 1);
        f.set_word(BASE + 0x400, STRING_LENGTH_OFFSET, usize::MAX);
        let panel = f.panel(BASE).unwrap().unwrap();
        assert_eq!(panel.root.control, BASE + 0x100);
        assert_eq!(panel.root.callback, REPLACEMENT);
        assert_eq!(panel.buttons, [
            Target { control: BASE + 0x400, slot_address: BASE + 0x460, callback: CHILD_ORIGINAL },
            Target { control: BASE + 0x500, slot_address: BASE + 0x560, callback: ORIGINAL },
        ]);
        for target in panel.buttons { assert!(f.check_child(BASE, target).is_ok()); }
        assert_eq!(f.panel(0), Ok(None));
    }

    #[test]
    fn known_child_wrappers_are_accepted_but_null_or_unverified_providers_are_rejected() {
        for callback in [ORIGINAL, CHILD_ORIGINAL, CHILD_REPLACEMENT, CHILD_REPLACEMENT_TWO] {
            let mut f = Fixture::new(); f.root(BASE, 0, b"StatBtn", ORIGINAL);
            f.set_word(BASE, FIRST_CHILD_OFFSET, BASE + 0x400);
            f.child(BASE + 0x400, 0, BASE, 2, callback);
            assert_eq!(f.panel(BASE).unwrap().unwrap().buttons[0].callback, callback);
        }
        for callback in [0, 0x123456, REPLACEMENT] {
            let mut f = Fixture::new(); f.root(BASE, 0, b"StatBtn", ORIGINAL);
            f.set_word(BASE, FIRST_CHILD_OFFSET, BASE + 0x400);
            f.child(BASE + 0x400, 0, BASE, 2, callback);
            assert_eq!(f.panel(BASE), Err(DiscoveryError::UnknownCallback));
        }
    }

    #[test]
    fn child_cycles_bad_parent_and_non_dialog_statbtn_root_are_rejected() {
        let mut f = Fixture::new(); f.root(BASE, 0, b"StatBtn", ORIGINAL);
        f.set_word(BASE, FIRST_CHILD_OFFSET, BASE + 0x400);
        f.child(BASE + 0x400, BASE + 0x500, BASE, 2, CHILD_ORIGINAL);
        f.child(BASE + 0x500, BASE + 0x400, BASE, 2, CHILD_ORIGINAL);
        assert_eq!(f.panel(BASE), Err(DiscoveryError::CyclicChildren));
        f.set_word(BASE + 0x500, 0, 0);
        f.set_word(BASE + 0x500, PARENT_OFFSET, BASE + 0x200);
        assert_eq!(f.panel(BASE), Err(DiscoveryError::WrongParent));
        f.child(BASE + 0x500, 0, BASE + 0x200, 9, 0);
        assert_eq!(f.panel(BASE), Err(DiscoveryError::WrongParent)); // Even ignored controls need correct parents.
        f.child(BASE + 0x500, 0, BASE, 9, 0);
        let ty = CONTROL_TYPE_OFFSET;
        f.bytes[ty..ty + 2].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(f.panel(BASE), Err(DiscoveryError::InvalidPanelRoot));
        f.bytes[ty..ty + 2].fill(0);
        f.set_word(BASE, FIRST_CHILD_OFFSET, BASE);
        assert_eq!(f.panel(BASE), Err(DiscoveryError::CyclicChildren));
    }

    #[test]
    fn child_traversal_has_exact_count_pointer_and_read_bounds() {
        let mut f = Fixture::new(); f.root(BASE, 0, b"StatBtn", ORIGINAL);
        f.set_word(BASE, FIRST_CHILD_OFFSET, BASE + 0x400);
        for index in 0..MAX_PANEL_CHILDREN {
            let child = BASE + 0x400 + index * 0x100;
            let next = if index + 1 == MAX_PANEL_CHILDREN { 0 } else { child + 0x100 };
            f.child(child, next, BASE, 2, CHILD_ORIGINAL);
        }
        assert_eq!(f.panel(BASE).unwrap().unwrap().buttons.len(), MAX_PANEL_CHILDREN);
        f.set_word(BASE + 0x400 + (MAX_PANEL_CHILDREN - 1) * 0x100, 0,
            BASE + 0x400 + MAX_PANEL_CHILDREN * 0x100);
        assert_eq!(f.panel(BASE), Err(DiscoveryError::TooManyChildren));
        for pointer in [1, BASE + 1, usize::MAX - 7] {
            f.set_word(BASE, FIRST_CHILD_OFFSET, pointer);
            assert_eq!(f.panel(BASE), Err(DiscoveryError::InvalidPointer));
        }
        f.set_word(BASE, FIRST_CHILD_OFFSET, BASE + f.bytes.len());
        assert_eq!(f.panel(BASE), Err(DiscoveryError::ReadFailed));
    }

    #[test]
    fn child_recheck_requires_exact_current_membership_callback_type_and_slot() {
        let mut f = Fixture::new(); f.root(BASE, 0, b"StatBtn", ORIGINAL);
        f.set_word(BASE, FIRST_CHILD_OFFSET, BASE + 0x400);
        f.child(BASE + 0x400, 0, BASE, 2, CHILD_ORIGINAL);
        let target = f.panel(BASE).unwrap().unwrap().buttons[0];
        assert!(f.check_child(BASE, target).is_ok());
        assert_eq!(f.check_child(BASE, Target { slot_address: target.slot_address + 8, ..target }),
            Err(DiscoveryError::InvalidPointer));
        f.set_word(BASE + 0x400, CALLBACK_OFFSET, ORIGINAL);
        assert_eq!(f.check_child(BASE, target), Err(DiscoveryError::ChangedChild));
        f.child(BASE + 0x400, 0, BASE, 9, CHILD_ORIGINAL);
        assert_eq!(f.check_child(BASE, target), Err(DiscoveryError::ChangedChild));
        f.child(BASE + 0x400, 0, BASE, 2, CHILD_ORIGINAL);
        f.set_word(BASE, FIRST_CHILD_OFFSET, 0);
        assert_eq!(f.check_child(BASE, target), Err(DiscoveryError::ChangedChild));
        assert_eq!(f.check_child(0, target), Err(DiscoveryError::ChangedChild));
    }

    #[test]
    fn next_match_recreated_statbtn_uses_new_direct_children_without_reusing_old_membership() {
        let mut f = Fixture::new(); f.root(BASE, 0, b"StatBtn", ORIGINAL);
        f.set_word(BASE, FIRST_CHILD_OFFSET, BASE + 0x400);
        f.child(BASE + 0x400, 0, BASE, 2, CHILD_ORIGINAL);
        let old = f.panel(BASE).unwrap().unwrap().buttons[0];
        f.root(BASE + 0x100, 0, b"StatBtn", REPLACEMENT);
        f.set_word(BASE + 0x100, FIRST_CHILD_OFFSET, BASE + 0x800);
        f.child(BASE + 0x800, 0, BASE + 0x100, 1, CHILD_REPLACEMENT);
        let new = f.panel(BASE + 0x100).unwrap().unwrap().buttons[0];
        assert_ne!(new.control, old.control);
        assert_eq!(f.check_child(BASE + 0x100, old), Err(DiscoveryError::ChangedChild));
        assert!(f.check_child(BASE + 0x100, new).is_ok());
        // The old object remains readable; readability alone never permits use.
        let mut bytes = [0; CHILD_READ_SIZE];
        assert!(f.read(old.control, &mut bytes));
    }

    #[test]
    fn child_topology_or_provider_changing_during_discovery_is_not_returned_as_stable() {
        use std::cell::RefCell;
        for change_parent in [false, true] {
            let mut f = Fixture::new(); f.root(BASE, 0, b"StatBtn", ORIGINAL);
            f.set_word(BASE, FIRST_CHILD_OFFSET, BASE + 0x400);
            f.child(BASE + 0x400, BASE + 0x500, BASE, 2, CHILD_ORIGINAL);
            f.child(BASE + 0x500, 0, BASE, 2, CHILD_ORIGINAL);
            let f = RefCell::new(f);
            let result = discover_stat_buttons(BASE, REPLACEMENT, &[CHILD_REPLACEMENT],
                |p| matches!(p, ORIGINAL | CHILD_ORIGINAL), |at, out| {
                    let success = f.borrow().read(at, out);
                    if at == BASE + 0x500 && out.len() == CHILD_READ_SIZE {
                        f.borrow_mut().set_word(BASE + 0x400,
                            if change_parent { PARENT_OFFSET } else { CALLBACK_OFFSET },
                            if change_parent { BASE + 0x100 } else { ORIGINAL });
                    }
                    success
                });
            assert_eq!(result, Err(DiscoveryError::ChangedChild));
        }
    }

    impl Fixture {
        fn minimap(&self, first: usize) -> Result<Option<Target>, DiscoveryError> {
            discover_minimap(first, REPLACEMENT, |p| matches!(p, ORIGINAL | CHILD_ORIGINAL),
                |at, out| self.read(at, out))
        }
        fn check_minimap(&self, first: usize, expected: Target) -> Result<(), DiscoveryError> {
            recheck_minimap(first, expected, REPLACEMENT,
                |p| matches!(p, ORIGINAL | CHILD_ORIGINAL), |at, out| self.read(at, out))
        }
    }

    #[test]
    fn minimap_callback_is_independent_of_status_and_command_panel_roots() {
        let mut f = Fixture::new();
        f.root(BASE, BASE + 0x100, b"StatData", 0);
        f.root(BASE + 0x100, BASE + 0x200, b"StatBtn", ORIGINAL);
        f.root(BASE + 0x200, 0, b"Minimap", CHILD_ORIGINAL);
        let target = f.minimap(BASE).unwrap().unwrap();
        assert_eq!(target, Target {
            control: BASE + 0x200, slot_address: BASE + 0x260, callback: CHILD_ORIGINAL,
        });
        assert!(f.check_minimap(BASE, target).is_ok());
        assert_eq!(f.find(BASE).unwrap().unwrap().control, BASE + 0x100);
        assert_eq!(f.minimap(0), Ok(None));
    }

    #[test]
    fn minimap_requires_exact_name_and_accepts_only_valid_inline_storage() {
        for name in [b"StatBtn".as_slice(), b"MinimaP", b"minimap", b"MinimapX", b"MiniMap"] {
            let mut f = Fixture::new(); f.root(BASE, 0, name, ORIGINAL);
            assert_eq!(f.minimap(BASE), Ok(None));
        }
        let mut f = Fixture::new(); f.root(BASE, 0, b"Minimap", ORIGINAL);
        f.bytes[0x87] = b'X';
        assert_eq!(f.minimap(BASE), Ok(None));
        f.set_word(BASE, STRING_DATA_OFFSET, BASE + STRING_INLINE_OFFSET);
        f.set_word(BASE, STRING_CAPACITY_OFFSET, 0x8000_0000_0000_000f);
        f.bytes[STRING_INLINE_OFFSET..STRING_INLINE_OFFSET + 8].copy_from_slice(b"Minimap\0");
        let target = f.minimap(BASE).unwrap().unwrap();
        assert!(f.check_minimap(BASE, target).is_ok());
        f.set_word(BASE, STRING_DATA_OFFSET, BASE + 0x80);
        assert_eq!(f.minimap(BASE), Err(DiscoveryError::InvalidName));
        f.set_word(BASE, STRING_DATA_OFFSET, BASE + STRING_INLINE_OFFSET);
        f.set_word(BASE, STRING_CAPACITY_OFFSET, 0x8000_0000_0000_0006);
        assert_eq!(f.minimap(BASE), Err(DiscoveryError::InvalidName));
    }

    #[test]
    fn minimap_accepts_its_owned_replacement_and_rejects_null_or_foreign_callbacks() {
        for callback in [ORIGINAL, CHILD_ORIGINAL, REPLACEMENT] {
            let mut f = Fixture::new(); f.root(BASE, 0, b"Minimap", callback);
            assert_eq!(f.minimap(BASE).unwrap().unwrap().callback, callback);
        }
        for callback in [0, CHILD_REPLACEMENT, 0x111111] {
            let mut f = Fixture::new(); f.root(BASE, 0, b"Minimap", callback);
            assert_eq!(f.minimap(BASE), Err(DiscoveryError::UnknownCallback));
        }
    }

    #[test]
    fn minimap_discovery_requires_unique_acyclic_bounded_root_membership() {
        let mut f = Fixture::new();
        f.root(BASE, BASE + 0x100, b"Minimap", ORIGINAL);
        f.root(BASE + 0x100, 0, b"Minimap", CHILD_ORIGINAL);
        assert_eq!(f.minimap(BASE), Err(DiscoveryError::DuplicateMinimap));
        f.root(BASE + 0x100, BASE, b"StatBtn", ORIGINAL);
        assert_eq!(f.minimap(BASE), Err(DiscoveryError::CyclicRoots));
        let mut f = Fixture::new();
        for index in 0..MAX_ROOTS {
            let root = BASE + index * 0x100;
            let last = index + 1 == MAX_ROOTS;
            f.root(root, if last { 0 } else { root + 0x100 },
                if last { b"Minimap" } else { b"Other" }, ORIGINAL);
        }
        assert_eq!(f.minimap(BASE).unwrap().unwrap().control,
            BASE + (MAX_ROOTS - 1) * 0x100);
        f.set_word(BASE + (MAX_ROOTS - 1) * 0x100, 0, BASE + MAX_ROOTS * 0x100);
        assert_eq!(f.minimap(BASE), Err(DiscoveryError::TooManyRoots));
        for pointer in [1, BASE + 1, usize::MAX - 7] {
            assert_eq!(f.minimap(pointer), Err(DiscoveryError::InvalidPointer));
        }
        assert_eq!(f.minimap(BASE + f.bytes.len()), Err(DiscoveryError::ReadFailed));
    }

    #[test]
    fn minimap_named_button_is_not_accepted_as_a_dialog_root() {
        let mut f = Fixture::new(); f.root(BASE, 0, b"Minimap", ORIGINAL);
        let start = CONTROL_TYPE_OFFSET;
        f.bytes[start..start + 2].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(f.minimap(BASE), Err(DiscoveryError::InvalidMinimapRoot));
    }

    #[test]
    fn minimap_recheck_requires_current_membership_exact_slot_name_and_provider() {
        let mut f = Fixture::new(); f.root(BASE, 0, b"Minimap", ORIGINAL);
        let target = f.minimap(BASE).unwrap().unwrap();
        assert!(f.check_minimap(BASE, target).is_ok());
        assert_eq!(f.check_minimap(BASE, Target { slot_address: target.slot_address + 8, ..target }),
            Err(DiscoveryError::InvalidPointer));
        assert_eq!(f.check_minimap(0, target), Err(DiscoveryError::ChangedRoot));
        f.root(BASE, 0, b"Minimap", CHILD_ORIGINAL);
        assert_eq!(f.check_minimap(BASE, target), Err(DiscoveryError::ChangedRoot));
        f.root(BASE, 0, b"StatBtn", ORIGINAL);
        assert_eq!(f.check_minimap(BASE, target), Err(DiscoveryError::ChangedRoot));
        f.root(BASE, 0, b"Minimap", ORIGINAL);
        f.set_word(BASE, STRING_DATA_OFFSET, BASE + f.bytes.len());
        assert_eq!(f.minimap(BASE), Err(DiscoveryError::ReadFailed));
    }

    #[test]
    fn recreated_minimap_root_does_not_authorize_a_still_readable_old_slot() {
        let mut f = Fixture::new(); f.root(BASE, 0, b"Minimap", ORIGINAL);
        let old = f.minimap(BASE).unwrap().unwrap();
        f.root(BASE + 0x100, 0, b"Minimap", REPLACEMENT);
        let new = f.minimap(BASE + 0x100).unwrap().unwrap();
        assert_ne!(old.control, new.control);
        assert!(f.check_minimap(BASE + 0x100, new).is_ok());
        assert_eq!(f.check_minimap(BASE + 0x100, old), Err(DiscoveryError::ChangedRoot));
        let mut bytes = [0; CONTROL_READ_SIZE];
        assert!(f.read(old.control, &mut bytes));
    }

    #[test]
    fn minimap_topology_name_or_provider_changing_between_passes_is_rejected() {
        use std::cell::RefCell;
        for mutation in 0..3 {
            let mut f = Fixture::new();
            f.root(BASE, BASE + 0x100, b"Minimap", ORIGINAL);
            f.root(BASE + 0x100, 0, b"StatBtn", ORIGINAL);
            let f = RefCell::new(f);
            let mut changed = false;
            let result = discover_minimap(BASE, REPLACEMENT,
                |p| matches!(p, ORIGINAL | CHILD_ORIGINAL), |at, out| {
                    let success = f.borrow().read(at, out);
                    if !changed && at == BASE + 0x100 && out.len() == CONTROL_READ_SIZE {
                        changed = true;
                        match mutation {
                            0 => f.borrow_mut().set_word(BASE, CALLBACK_OFFSET, CHILD_ORIGINAL),
                            1 => f.borrow_mut().set_word(BASE, 0, 0),
                            _ => f.borrow_mut().bytes[0x80] = b'X',
                        }
                    }
                    success
                });
            assert!(changed);
            assert_eq!(result, Err(DiscoveryError::ChangedRoot));
        }
    }

}
