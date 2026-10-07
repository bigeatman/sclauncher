//! Read-only discovery of the verified SCR alliance dialog and its spare rows.
//!
//! The control IDs/types and relative geometry were observed in the user's
//! mc-28968-alliance-ui.txt (2026-10-06), not inferred from a screenshot.
//! The pointer/string/control layout is the pinned bw_dat SCR layout used by
//! gui_capture.rs. No callbacks are invoked and no game memory is written here.
//!
//! A discovery is NOT a lifetime reservation. Bind only on the game's UI thread,
//! recheck against the current root list immediately before an exact callback
//! compare-and-exchange, and never restore a cached heap control from a worker.
//! Confirm/Cancel IDs identify controls; their event/notification semantics are
//! intentionally not guessed by this module.

use std::collections::{HashMap, HashSet};

pub const CALLBACK_OFFSET: usize = 0x60;
pub const CONFIRM_ID: i16 = -2;
pub const CANCEL_ID: i16 = -3;
pub const ALLIED_VICTORY_ID: i16 = 25;
const MAX_ROOTS: usize = 128;
const MAX_CHILDREN: usize = 256;
const HEADER: usize = 0x78;
const FIRST_CHILD: usize = 0x90;
const PARENT: usize = 0x70;
const NAME: &[u8] = b"AllyFltr";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rect {
    pub left: i16,
    pub top: i16,
    pub right: i16,
    pub bottom: i16,
}

impl Rect {
    /// SCR rect endpoints are inclusive. Keep the arithmetic wider than i16.
    pub fn width(self) -> i32 { i32::from(self.right) - i32::from(self.left) + 1 }
    pub fn height(self) -> i32 { i32::from(self.bottom) - i32::from(self.top) + 1 }
    fn valid(self) -> bool { self.width() > 0 && self.height() > 0 }
    fn inside(self, root: Rect) -> bool {
        self.valid() && self.left >= 0 && self.top >= 0
            && i32::from(self.right) < root.width()
            && i32::from(self.bottom) < root.height()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Target {
    pub control: usize,
    pub slot_address: usize,
    /// May be the caller's own replacement; never forward to it as an original.
    pub callback: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Button {
    pub control: usize,
    pub id: i16,
    /// Relative to Frame.area, in SCR logical UI coordinates.
    pub area: Rect,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RowArea {
    /// Native visual row index, NOT a game player ID.
    pub native_row: u8,
    /// All rectangles are relative to Frame.area.
    pub label: Rect,
    pub alliance: Rect,
    pub vision: Rect,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    /// Absolute SCR logical UI coordinates; callers must apply the actual
    /// game UI-to-client transform before positioning a desktop overlay.
    pub area: Rect,
    pub visible_human_rows: u8,
    pub confirm: Button,
    pub cancel: Button,
    pub allied_victory: Rect,
    /// Hidden native visual rows after the human prefix and strictly above
    /// Allied Victory. These are layout slots; actual computer IDs come from
    /// the validated player state, never from this native visual index.
    pub available_rows: Vec<RowArea>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Discovery {
    pub target: Target,
    pub frame: Frame,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscoveryError {
    InvalidPointer,
    ReadFailed,
    InvalidString,
    InvalidRoot,
    CyclicRoots,
    TooManyRoots,
    DuplicateDialog,
    CyclicChildren,
    TooManyChildren,
    WrongParent,
    DuplicateChildId,
    MissingControl,
    WrongControlType,
    InvalidLayout,
    UnknownCallback,
    Changed,
}

impl std::fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reason = match self {
            Self::InvalidPointer => "invalid control pointer",
            Self::ReadFailed => "UI metadata read failed",
            Self::InvalidString => "invalid UI string storage",
            Self::InvalidRoot => "invalid root type or area",
            Self::CyclicRoots => "cyclic root list",
            Self::TooManyRoots => "root limit exceeded",
            Self::DuplicateDialog => "multiple alliance roots",
            Self::CyclicChildren => "cyclic child list",
            Self::TooManyChildren => "child limit exceeded",
            Self::WrongParent => "child belongs to another dialog",
            Self::DuplicateChildId => "duplicate child identifier",
            Self::MissingControl => "required alliance control missing",
            Self::WrongControlType => "unexpected alliance control type",
            Self::InvalidLayout => "unrecognized alliance row layout",
            Self::UnknownCallback => "unverified alliance callback",
            Self::Changed => "alliance UI changed during discovery",
        };
        // Never include game addresses or player names in diagnostic errors.
        write!(f, "Alliance dialog discovery: {reason}")
    }
}

fn pointer(p: usize, n: usize) -> bool {
    p >= 0x10000 && p % 8 == 0 && p.checked_add(n).is_some()
}
fn word(bytes: &[u8], at: usize) -> usize {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize
}
fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
}
fn i16_at(bytes: &[u8], at: usize) -> i16 {
    i16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
}
fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}
fn rect(bytes: &[u8]) -> Rect {
    Rect { left: i16_at(bytes, 8), top: i16_at(bytes, 10),
        right: i16_at(bytes, 12), bottom: i16_at(bytes, 14) }
}

#[derive(Clone)]
struct Observed {
    address: usize,
    header: [u8; HEADER],
    /// Name only when needed. These strings never enter the returned discovery.
    string: Option<Vec<u8>>,
}

impl Observed {
    fn id(&self) -> i16 { i16_at(&self.header, 0x52) }
    fn ty(&self) -> u16 { u16_at(&self.header, 0x54) }
    fn visible(&self) -> bool { u32_at(&self.header, 0x48) & 2 != 0 }
    fn enabled(&self) -> bool { u32_at(&self.header, 0x4c) & 1 == 0 }
    fn area(&self) -> Rect { rect(&self.header) }
    fn next(&self) -> usize { word(&self.header, 0) }
}

fn read_header<F>(address: usize, read: &mut F) -> Result<Observed, DiscoveryError>
where F: FnMut(usize, &mut [u8]) -> bool {
    if !pointer(address, HEADER) { return Err(DiscoveryError::InvalidPointer); }
    let mut header = [0; HEADER];
    if !read(address, &mut header) { return Err(DiscoveryError::ReadFailed); }
    Ok(Observed { address, header, string: None })
}

fn read_string<F>(control: &Observed, read: &mut F) -> Result<Vec<u8>, DiscoveryError>
where F: FnMut(usize, &mut [u8]) -> bool {
    let bytes = &control.header;
    let length = word(bytes, 0x28);
    if length == 0 { return Ok(Vec::new()); }
    if length > 192 { return Err(DiscoveryError::InvalidString); }
    let data = word(bytes, 0x20);
    let raw_cap = word(bytes, 0x30);
    let inline_flag = !(usize::MAX >> 1);
    let inline = raw_cap & inline_flag != 0;
    let capacity = raw_cap & !inline_flag;
    if capacity < length || capacity > 8192 || data < 0x10000
        || data.checked_add(length + 1).is_none()
        || (inline && (Some(data) != control.address.checked_add(0x38) || length >= 16)) {
        return Err(DiscoveryError::InvalidString);
    }
    let mut result = vec![0; length + 1];
    if !read(data, &mut result) { return Err(DiscoveryError::ReadFailed); }
    if result[length] != 0 || result[..length].contains(&0)
        || std::str::from_utf8(&result[..length]).is_err() {
        return Err(DiscoveryError::InvalidString);
    }
    result.pop();
    Ok(result)
}

fn recheck_observed<F>(observed: &Observed, read: &mut F) -> Result<(), DiscoveryError>
where F: FnMut(usize, &mut [u8]) -> bool {
    let again = read_header(observed.address, read)?;
    if again.header != observed.header { return Err(DiscoveryError::Changed); }
    if let Some(string) = &observed.string {
        if read_string(&again, read)? != *string { return Err(DiscoveryError::Changed); }
    }
    Ok(())
}

fn child<'a>(children: &'a HashMap<i16, Observed>, id: i16, ty: u16)
    -> Result<&'a Observed, DiscoveryError> {
    let result = children.get(&id).ok_or(DiscoveryError::MissingControl)?;
    if result.ty() != ty { return Err(DiscoveryError::WrongControlType); }
    Ok(result)
}

fn layout(root: &Observed, children: &HashMap<i16, Observed>)
    -> Result<Frame, DiscoveryError> {
    let area = root.area();
    let confirm = child(children, CONFIRM_ID, 1)?;
    let cancel = child(children, CANCEL_ID, 2)?;
    let victory = child(children, ALLIED_VICTORY_ID, 4)?;
    if !confirm.visible() || !cancel.visible() || !victory.visible()
        || !confirm.area().inside(area) || !cancel.area().inside(area)
        || !victory.area().inside(area) || victory.area().bottom >= confirm.area().top
        || victory.area().bottom >= cancel.area().top
        || confirm.area().right >= cancel.area().left {
        return Err(DiscoveryError::InvalidLayout);
    }
    let mut visible_human_rows = 0u8;
    let mut hidden_seen = false;
    let mut available_rows = Vec::new();
    let mut previous: Option<RowArea> = None;
    let mut pitch = None;
    for row in 0..9i16 {
        let label = child(children, -10 - row, 9)?;
        let alliance = child(children, 1 + row, 4)?;
        let vision = child(children, 13 + row, 4)?;
        let row_area = RowArea { native_row: row as u8,
            label: label.area(), alliance: alliance.area(), vision: vision.area() };
        if !row_area.label.inside(area) || !row_area.alliance.inside(area)
            || !row_area.vision.inside(area)
            || label.visible() != alliance.visible() || label.visible() != vision.visible()
            || row_area.label.right >= row_area.alliance.left
            || row_area.alliance.right >= row_area.vision.left
            || row_area.alliance.top != row_area.vision.top
            || row_area.alliance.bottom != row_area.vision.bottom
            || row_area.label.top > row_area.alliance.top
            || row_area.label.bottom < row_area.alliance.bottom {
            return Err(DiscoveryError::InvalidLayout);
        }
        if let Some(prev) = previous {
            let step = i32::from(row_area.label.top) - i32::from(prev.label.top);
            if step <= prev.label.height() || pitch.is_some_and(|p| p != step)
                || row_area.label.left != prev.label.left || row_area.label.right != prev.label.right
                || row_area.label.height() != prev.label.height()
                || row_area.alliance.left != prev.alliance.left
                || row_area.alliance.right != prev.alliance.right
                || row_area.alliance.height() != prev.alliance.height()
                || i32::from(row_area.alliance.top) - i32::from(prev.alliance.top) != step
                || row_area.vision.left != prev.vision.left || row_area.vision.right != prev.vision.right
                || i32::from(row_area.vision.top) - i32::from(prev.vision.top) != step {
                return Err(DiscoveryError::InvalidLayout);
            }
            pitch = Some(step);
        }
        previous = Some(row_area);
        if label.visible() {
            if hidden_seen || row_area.label.bottom >= victory.area().top {
                return Err(DiscoveryError::InvalidLayout);
            }
            visible_human_rows += 1;
        } else {
            hidden_seen = true;
            if row_area.label.bottom < victory.area().top {
                available_rows.push(row_area);
            }
        }
    }
    Ok(Frame {
        area, visible_human_rows,
        confirm: Button { control: confirm.address, id: CONFIRM_ID,
            area: confirm.area(), enabled: confirm.enabled() },
        cancel: Button { control: cancel.address, id: CANCEL_ID,
            area: cancel.area(), enabled: cancel.enabled() },
        allied_victory: victory.area(), available_rows,
    })
}

/// Read a bounded, acyclic root/child graph. An exact visible AllyFltr root is
/// accepted only with the target-build control IDs/types and a consistent layout.
/// Unknown callback addresses are rejected by the caller's verified-code predicate.
pub fn discover<F, C>(first_dialog: usize, replacement: usize, is_code: C, mut read: F)
    -> Result<Option<Discovery>, DiscoveryError>
where F: FnMut(usize, &mut [u8]) -> bool, C: Fn(usize) -> bool {
    let mut roots = Vec::new();
    let mut seen = HashSet::new();
    let mut current = first_dialog;
    let mut alliance_root = None;
    while current != 0 {
        if roots.len() >= MAX_ROOTS { return Err(DiscoveryError::TooManyRoots); }
        if !seen.insert(current) { return Err(DiscoveryError::CyclicRoots); }
        let mut root = read_header(current, &mut read)?;
        if root.ty() != 0 || !root.area().valid() { return Err(DiscoveryError::InvalidRoot); }
        // Do not read names/children of unrelated controls unnecessarily.
        if word(&root.header, 0x28) == NAME.len() {
            let string = read_string(&root, &mut read)?;
            if string == NAME {
                if alliance_root.replace(roots.len()).is_some() {
                    return Err(DiscoveryError::DuplicateDialog);
                }
            }
            root.string = Some(string);
        }
        current = root.next();
        roots.push(root);
    }
    let Some(index) = alliance_root else {
        for root in &roots { recheck_observed(root, &mut read)?; }
        return Ok(None);
    };
    let root = &roots[index];
    // The observed native modal is prepended to the root list. Never display
    // an interactive overlay over a later alliance root while another root is
    // in front (for example a nested menu/confirmation dialog).
    if index != 0 || !root.visible() {
        for root in &roots { recheck_observed(root, &mut read)?; }
        return Ok(None);
    }
    let callback = word(&root.header, CALLBACK_OFFSET);
    if callback == 0 || (callback != replacement && !is_code(callback)) {
        return Err(DiscoveryError::UnknownCallback);
    }
    let mut pointer_bytes = [0; 8];
    let child_slot = root.address.checked_add(FIRST_CHILD).ok_or(DiscoveryError::InvalidPointer)?;
    if !read(child_slot, &mut pointer_bytes) { return Err(DiscoveryError::ReadFailed); }
    let mut current = word(&pointer_bytes, 0);
    let mut child_seen = HashSet::new();
    let mut children = HashMap::new();
    while current != 0 {
        if child_seen.len() >= MAX_CHILDREN { return Err(DiscoveryError::TooManyChildren); }
        if seen.contains(&current) || !child_seen.insert(current) {
            return Err(DiscoveryError::CyclicChildren);
        }
        let control = read_header(current, &mut read)?;
        if word(&control.header, PARENT) != root.address { return Err(DiscoveryError::WrongParent); }
        // Child labels can contain human player names. Their bytes are not
        // needed to identify the verified IDs/types/layout, so do not read them.
        current = control.next();
        if children.insert(control.id(), control).is_some() {
            return Err(DiscoveryError::DuplicateChildId);
        }
    }
    let frame = layout(root, &children)?;
    // Recheck the complete graph after traversal; a child checked only directly
    // after its initial read could have changed while later siblings were read.
    for child in children.values() { recheck_observed(child, &mut read)?; }
    let mut pointer_again = [0; 8];
    if !read(child_slot, &mut pointer_again) { return Err(DiscoveryError::ReadFailed); }
    if pointer_again != pointer_bytes { return Err(DiscoveryError::Changed); }
    for root in &roots { recheck_observed(root, &mut read)?; }
    Ok(Some(Discovery {
        target: Target { control: root.address,
            slot_address: root.address + CALLBACK_OFFSET, callback }, frame,
    }))
}

/// Re-discover from the current root list before UI-thread callback CAS. Caller
/// must separately reread the first_dialog operand immediately around this call.
pub fn recheck<F, C>(first_dialog: usize, expected: &Discovery, replacement: usize,
    is_code: C, read: F) -> Result<(), DiscoveryError>
where F: FnMut(usize, &mut [u8]) -> bool, C: Fn(usize) -> bool {
    if discover(first_dialog, replacement, is_code, read)?.as_ref() != Some(expected) {
        return Err(DiscoveryError::Changed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const BASE: usize = 0x10000;
    const ROOT: usize = BASE + 0x400;
    const ORIGINAL: usize = BASE + 0xf000;
    const REPLACEMENT: usize = BASE + 0xf100;
    struct Fixture { bytes: Vec<u8>, ids: HashMap<i16, usize> }
    impl Fixture {
        fn new(humans: usize) -> Self {
            let mut f = Self { bytes: vec![0; 0x10000], ids: HashMap::new() };
            f.control(BASE, 0, "StatBtn", 0, 0, true,
                Rect { left: 708, top: 354, right: 851, bottom: 479 }, 0);
            f.control(ROOT, BASE, "AllyFltr", 0, 0, true,
                Rect { left: 276, top: 21, right: 571, bottom: 356 }, 0);
            f.set_word(ROOT, CALLBACK_OFFSET, ORIGINAL);
            // Exact IDs/types/rectangles from the supplied target-build UI log.
            // Human display names are replaced with fixture text, never copied.
            let mut rows = vec![
                (-2, 1, true, Rect { left: 11, top: 299, right: 130, bottom: 326 }),
                (-3, 2, true, Rect { left: 165, top: 299, right: 284, bottom: 326 }),
            ];
            for i in 0..9i16 {
                rows.push((1 + i, 4, (i as usize) < humans,
                    Rect { left: 227, top: 48 + i * 26, right: 242, bottom: 64 + i * 26 }));
                rows.push((13 + i, 4, (i as usize) < humans,
                    Rect { left: 269, top: 48 + i * 26, right: 284, bottom: 64 + i * 26 }));
                rows.push((-10 - i, 9, (i as usize) < humans,
                    Rect { left: 11, top: 46 + i * 26, right: 196, bottom: 65 + i * 26 }));
            }
            rows.push((25, 4, true, Rect { left: 11, top: 260, right: 260, bottom: 279 }));
            for (i, (id, ty, visible, area)) in rows.iter().copied().enumerate() {
                let at = BASE + 0x800 + i * 0x200;
                let next = if i + 1 == rows.len() { 0 } else { at + 0x200 };
                let name = if id == -2 { "확인" } else if id == -3 { "취소" }
                    else if id == 25 { "동맹 승리" } else { " " };
                f.control(at, next, name, id, ty, visible, area, ROOT);
                f.ids.insert(id, at);
            }
            f.set_word(ROOT, FIRST_CHILD, BASE + 0x800);
            f
        }
        fn set_word(&mut self, at: usize, field: usize, value: usize) {
            let start = at - BASE + field;
            self.bytes[start..start + 8].copy_from_slice(&(value as u64).to_le_bytes());
        }
        fn set_u16(&mut self, at: usize, field: usize, value: u16) {
            let start = at - BASE + field;
            self.bytes[start..start + 2].copy_from_slice(&value.to_le_bytes());
        }
        fn control(&mut self, at: usize, next: usize, name: &str, id: i16, ty: u16,
            visible: bool, area: Rect, parent: usize) {
            self.set_word(at, 0, next);
            self.set_word(at, 0x20, at + 0xa0);
            self.set_word(at, 0x28, name.len());
            self.set_word(at, 0x30, 0x100);
            self.set_word(at, PARENT, parent);
            for (field, value) in [(8, area.left), (10, area.top), (12, area.right), (14, area.bottom)] {
                self.set_u16(at, field, value as u16);
            }
            self.set_u16(at, 0x52, id as u16); self.set_u16(at, 0x54, ty);
            self.bytes[at - BASE + 0x48] = if visible { 2 } else { 0 };
            let start = at - BASE + 0xa0;
            self.bytes[start..start + name.len()].copy_from_slice(name.as_bytes());
        }
        fn read(&self, at: usize, out: &mut [u8]) -> bool {
            let Some(start) = at.checked_sub(BASE) else { return false; };
            let Some(end) = start.checked_add(out.len()) else { return false; };
            let Some(src) = self.bytes.get(start..end) else { return false; };
            out.copy_from_slice(src); true
        }
        fn find(&self) -> Result<Option<Discovery>, DiscoveryError> {
            discover(ROOT, REPLACEMENT, |p| p == ORIGINAL, |at, out| self.read(at, out))
        }
    }

    #[test]
    fn saved_target_build_layout_yields_only_spare_rows_above_victory() {
        let f = Fixture::new(3); let d = f.find().unwrap().unwrap();
        assert_eq!(d.target, Target { control: ROOT, slot_address: ROOT + 0x60, callback: ORIGINAL });
        assert_eq!(d.frame.visible_human_rows, 3);
        assert_eq!(d.frame.available_rows.len(), 5);
        assert_eq!(d.frame.available_rows[0].label,
            Rect { left: 11, top: 124, right: 196, bottom: 143 });
        assert_eq!(d.frame.available_rows.last().unwrap().native_row, 7);
        assert!(d.frame.available_rows.iter().all(|r| r.label.bottom < d.frame.allied_victory.top));
        assert_eq!(d.frame.confirm.id, -2); assert_eq!(d.frame.cancel.id, -3);
        assert!(recheck(ROOT, &d, REPLACEMENT, |p| p == ORIGINAL, |at, out| f.read(at, out)).is_ok());
    }
    #[test]
    fn visible_prefix_length_changes_available_layout_without_using_player_ids() {
        for humans in 0..=8 {
            let f = Fixture::new(humans); let d = f.find().unwrap().unwrap();
            assert_eq!(usize::from(d.frame.visible_human_rows), humans);
            assert_eq!(d.frame.available_rows.len(), 8 - humans);
        }
        assert_eq!(Fixture::new(9).find(), Err(DiscoveryError::InvalidLayout));
    }
    #[test]
    fn hidden_candidate_never_follows_children() {
        let mut f = Fixture::new(3); f.bytes[ROOT - BASE + 0x48] = 0;
        f.set_word(ROOT, FIRST_CHILD, 1);
        assert_eq!(f.find(), Ok(None));
    }
    #[test]
    fn exact_name_and_nul_are_required_before_child_traversal() {
        let mut f = Fixture::new(3); f.set_word(ROOT, FIRST_CHILD, 1);
        f.bytes[ROOT - BASE + 0xa0..ROOT - BASE + 0xa8].copy_from_slice(b"AllyFlts");
        assert_eq!(f.find(), Ok(None));
        f.bytes[ROOT - BASE + 0xa0..ROOT - BASE + 0xa8].copy_from_slice(b"AllyFltr");
        f.bytes[ROOT - BASE + 0xa8] = b'x';
        assert_eq!(f.find(), Err(DiscoveryError::InvalidString));
        f.bytes[ROOT - BASE + 0xa8] = 0;
        f.bytes[ROOT - BASE + 0xa4] = 0;
        assert_eq!(f.find(), Err(DiscoveryError::InvalidString));
    }
    #[test]
    fn incorrect_types_parents_duplicate_ids_and_missing_controls_reject() {
        let mut f = Fixture::new(3); let at = f.ids[&-2];
        f.set_u16(at, 0x54, 2); assert_eq!(f.find(), Err(DiscoveryError::WrongControlType));
        let mut f = Fixture::new(3); let at = f.ids[&-2];
        f.set_word(at, PARENT, BASE); assert_eq!(f.find(), Err(DiscoveryError::WrongParent));
        let mut f = Fixture::new(3); let at = f.ids[&-3];
        f.set_u16(at, 0x52, CONFIRM_ID as u16);
        assert_eq!(f.find(), Err(DiscoveryError::DuplicateChildId));
        let mut f = Fixture::new(3); let at = f.ids[&-2];
        f.set_u16(at, 0x52, 100); assert_eq!(f.find(), Err(DiscoveryError::MissingControl));
    }
    #[test]
    fn gaps_mismatched_visibility_and_overlapping_controls_reject() {
        let mut f = Fixture::new(3); let at = f.ids[&-11];
        f.bytes[at - BASE + 0x48] = 0; assert_eq!(f.find(), Err(DiscoveryError::InvalidLayout));
        let mut f = Fixture::new(3);
        for id in [-11, 2, 14] { let at = f.ids[&id]; f.bytes[at - BASE + 0x48] = 0; }
        assert_eq!(f.find(), Err(DiscoveryError::InvalidLayout));
        let mut f = Fixture::new(3); let at = f.ids[&-10];
        f.set_u16(at, 12, 230); assert_eq!(f.find(), Err(DiscoveryError::InvalidLayout));
        let mut f = Fixture::new(3); let at = f.ids[&25];
        f.set_u16(at, 10, 110); assert_eq!(f.find(), Err(DiscoveryError::InvalidLayout));
    }
    #[test]
    fn original_or_owned_replacement_callbacks_only() {
        let mut f = Fixture::new(3); f.set_word(ROOT, CALLBACK_OFFSET, REPLACEMENT);
        assert_eq!(f.find().unwrap().unwrap().target.callback, REPLACEMENT);
        for callback in [0, BASE + 0x1110] {
            let mut f = Fixture::new(3); f.set_word(ROOT, CALLBACK_OFFSET, callback);
            assert_eq!(f.find(), Err(DiscoveryError::UnknownCallback));
        }
    }
    #[test]
    fn root_and_child_cycles_are_bounded_and_unrelated_children_are_skipped() {
        let mut f = Fixture::new(3); f.set_word(BASE, FIRST_CHILD, 1);
        assert!(f.find().is_ok());
        f.set_word(BASE, 0, ROOT); assert_eq!(f.find(), Err(DiscoveryError::CyclicRoots));
        let mut f = Fixture::new(3); let at = f.ids[&25];
        f.set_word(at, 0, BASE + 0x800); assert_eq!(f.find(), Err(DiscoveryError::CyclicChildren));
    }
    #[test]
    fn stale_child_header_or_string_never_produces_a_discovery() {
        let f = Fixture::new(3); let at = f.ids[&-2]; let mut reads = 0;
        assert_eq!(discover(ROOT, REPLACEMENT, |p| p == ORIGINAL, |p, out| {
            let ok = f.read(p, out);
            if p == at { reads += 1; if reads == 2 { out[0x52] ^= 1; } }
            ok
        }), Err(DiscoveryError::Changed));
        let mut reads = 0;
        assert_eq!(discover(ROOT, REPLACEMENT, |p| p == ORIGINAL, |p, out| {
            let ok = f.read(p, out);
            if p == ROOT + 0xa0 { reads += 1; if reads == 2 { out[0] = b'x'; } }
            ok
        }), Err(DiscoveryError::Changed));
    }
    #[test]
    fn root_list_callback_or_geometry_change_fails_immediate_recheck() {
        let mut f = Fixture::new(3); let d = f.find().unwrap().unwrap();
        f.set_word(ROOT, CALLBACK_OFFSET, REPLACEMENT);
        assert_eq!(recheck(ROOT, &d, REPLACEMENT, |p| p == ORIGINAL, |p, out| f.read(p, out)),
            Err(DiscoveryError::Changed));
        let f = Fixture::new(3);
        assert_eq!(recheck(0, &d, REPLACEMENT, |p| p == ORIGINAL, |p, out| f.read(p, out)),
            Err(DiscoveryError::Changed));
    }
    #[test]
    fn invalid_pointers_names_and_root_type_reject() {
        let f = Fixture::new(3);
        for first in [1, BASE + 1, usize::MAX - 7] {
            assert_eq!(discover(first, REPLACEMENT, |p| p == ORIGINAL, |p, out| f.read(p, out)),
                Err(DiscoveryError::InvalidPointer));
        }
        let mut f = Fixture::new(3); f.set_u16(ROOT, 0x54, 1);
        assert_eq!(f.find(), Err(DiscoveryError::InvalidRoot));
        let mut f = Fixture::new(3); f.set_word(ROOT, 0x30, 7);
        assert_eq!(f.find(), Err(DiscoveryError::InvalidString));
    }
    #[test]
    fn proper_inline_root_name_matches_and_invalid_inline_pointer_rejects() {
        let mut f = Fixture::new(3); f.set_word(ROOT, 0x20, ROOT + 0x38);
        f.set_word(ROOT, 0x30, (!(usize::MAX >> 1)) | 15);
        f.bytes[ROOT - BASE + 0x38..ROOT - BASE + 0x40].copy_from_slice(NAME);
        assert!(f.find().unwrap().is_some());
        f.set_word(ROOT, 0x20, ROOT + 0xa0);
        assert_eq!(f.find(), Err(DiscoveryError::InvalidString));
    }
    #[test]
    fn nested_modal_or_later_alliance_root_never_yields_an_overlay_frame() {
        let mut f = Fixture::new(3);
        f.set_word(BASE, 0, ROOT); f.set_word(ROOT, 0, 0);
        f.set_word(ROOT, FIRST_CHILD, 1);
        assert_eq!(discover(BASE, REPLACEMENT, |p| p == ORIGINAL, |p, out| f.read(p, out)), Ok(None));
    }
    #[test]
    fn child_player_label_storage_is_never_followed_or_exported() {
        let mut f = Fixture::new(3);
        for at in f.ids.values().copied().collect::<Vec<_>>() {
            f.set_word(at, 0x20, 1); f.set_word(at, 0x28, 10_000);
        }
        let d = f.find().unwrap().unwrap();
        assert_eq!(d.frame.visible_human_rows, 3);
    }
}
