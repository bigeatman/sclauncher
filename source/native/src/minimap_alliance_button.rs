//! Pure discovery of the native Minimap alliance/chat button pair.
//!
//! Primary evidence:
//! https://github.com/ShieldBattery/ShieldBattery/blob/master/game/src/bw_scr/dialog_hook.rs
//! minimap_event_handler identifies direct Minimap children 3 and 4 as the
//! alliance/chat pair, without assigning either numeric ID to a particular icon.
//! https://github.com/neivv/aise/blob/da4fed681dc09bda7a21c0051f60cdad98cc7226/bw_dat/src/bw/structs.rs
//! SCR Control/Dialog layout, the same pinned header used by alliance_dialog.
//! https://github.com/neivv/aise/blob/da4fed681dc09bda7a21c0051f60cdad98cc7226/bw_dat/src/dialog.rs
//! show: flags & 2 == 0 -> ext 0xd; hide: flags & 2 != 0 -> ext 0xe,
//! followed by ext 0x6 only if the hide handler returns nonzero.
//!
//! The user's game image places alliance on the left and chat on the right.
//! Identity needs BOTH IDs, verified original native handlers, and a bounded,
//! nonoverlapping common top band. No numeric ID or screen coordinate alone
//! identifies alliance. No child strings, player names or executable bytes
//! are read. No callback, flags, or game state are changed by this module.
//!
//! An intent is not a lifetime lease. The caller must recheck the current first
//! root and this entire discovery on the game UI thread immediately before
//! invoking an ordinary show/hide event. Original click handlers stay untouched.

use crate::alliance_dialog::Rect;
use std::collections::{HashMap, HashSet};

const HEADER: usize = 0x78;
const FIRST_CHILD: usize = 0x90;
const PARENT: usize = 0x70;
const CALLBACK: usize = 0x60;
const MAX_ROOTS: usize = 128;
const MAX_CHILDREN: usize = 256;
const NAME: &[u8] = b"Minimap";
pub const EXT_SHOW: usize = 0xd;
pub const EXT_HIDE: usize = 0xe;
pub const EXT_HIDE_FOLLOWUP: usize = 0x6;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Control {
    pub control: usize,
    /// The observed callback. Children always retain verified native handlers;
    /// the root may carry a caller-verified owned wrapper. Never forward the
    /// root wrapper as if it were the saved native original.
    pub callback: usize,
    pub id: i16,
    pub ty: u16,
    /// Minimap: absolute logical rect. Children: relative to Minimap.
    pub area: Rect,
    pub visible: bool,
    pub enabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Discovery {
    pub minimap: Control,
    pub alliance: Control,
    /// Identity witness only. No generated intent ever targets this control.
    pub chat: Control,
    // Recheck detects unrelated children being replaced/reparented/reordered.
    // These header witnesses are never exported as diagnostics.
    roots: Vec<Observed>,
    children: Vec<Observed>,
    first_child: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShowHideIntent {
    pub control: usize,
    pub callback: usize,
    pub id: i16,
    pub ext_type: usize,
}

impl Discovery {
    /// Preserve enabled state; do not make a disabled native control appear usable.
    pub fn intent(&self, desired_visible: bool) -> Option<ShowHideIntent> {
        let button = self.alliance;
        if button.visible == desired_visible || (desired_visible && !button.enabled) {
            return None;
        }
        Some(ShowHideIntent {
            control: button.control, callback: button.callback, id: button.id,
            ext_type: if desired_visible { EXT_SHOW } else { EXT_HIDE },
        })
    }
}

impl ShowHideIntent {
    /// Matches bw_dat::Control::hide without invoking the second event here.
    /// The caller must recheck lifetime before a followup invocation too.
    pub fn followup_ext_type(self, native_result: u32) -> Option<usize> {
        (self.ext_type == EXT_HIDE && native_result != 0).then_some(EXT_HIDE_FOLLOWUP)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscoveryError {
    InvalidPointer, ReadFailed, InvalidString, InvalidRoot, CyclicRoots,
    TooManyRoots, DuplicateMinimap, CyclicChildren, TooManyChildren,
    WrongParent, DuplicateChildId, MissingPair, InvalidPair, UnknownCallback, Changed,
}

impl std::fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reason = match self {
            Self::InvalidPointer => "invalid control pointer",
            Self::ReadFailed => "UI metadata read failed",
            Self::InvalidString => "invalid root string storage",
            Self::InvalidRoot => "invalid root type or area",
            Self::CyclicRoots => "cyclic root list",
            Self::TooManyRoots => "root limit exceeded",
            Self::DuplicateMinimap => "multiple Minimap roots",
            Self::CyclicChildren => "cyclic child list",
            Self::TooManyChildren => "child limit exceeded",
            Self::WrongParent => "child belongs to another dialog",
            Self::DuplicateChildId => "duplicate child identifier",
            Self::MissingPair => "native alliance/chat pair missing",
            Self::InvalidPair => "unrecognized alliance/chat pair geometry",
            Self::UnknownCallback => "unverified native event handler",
            Self::Changed => "Minimap UI changed during discovery",
        };
        write!(f, "Minimap alliance discovery: {reason}")
    }
}

fn pointer(p: usize, n: usize) -> bool {
    p >= 0x10000 && p % 8 == 0 && p.checked_add(n).is_some()
}
fn word(bytes: &[u8], at: usize) -> usize {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize
}
fn i16_at(bytes: &[u8], at: usize) -> i16 {
    i16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
}
fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
}
fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}
fn valid(area: Rect) -> bool { area.width() > 0 && area.height() > 0 }
fn inside(area: Rect, root: Rect) -> bool {
    valid(area) && area.left >= 0 && area.top >= 0
        && i32::from(area.right) < root.width() && i32::from(area.bottom) < root.height()
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Observed {
    address: usize,
    header: [u8; HEADER],
    // Only exact-length root names are read, never child label strings.
    string: Option<Vec<u8>>,
}

impl Observed {
    fn next(&self) -> usize { word(&self.header, 0) }
    fn ty(&self) -> u16 { u16_at(&self.header, 0x54) }
    fn id(&self) -> i16 { i16_at(&self.header, 0x52) }
    fn visible(&self) -> bool { u32_at(&self.header, 0x48) & 2 != 0 }
    fn enabled(&self) -> bool { u32_at(&self.header, 0x4c) & 1 == 0 }
    fn area(&self) -> Rect {
        Rect { left: i16_at(&self.header, 8), top: i16_at(&self.header, 10),
            right: i16_at(&self.header, 12), bottom: i16_at(&self.header, 14) }
    }
    fn control(&self) -> Control {
        Control { control: self.address, callback: word(&self.header, CALLBACK),
            id: self.id(), ty: self.ty(), area: self.area(),
            visible: self.visible(), enabled: self.enabled() }
    }
}

fn read_header<F>(address: usize, read: &mut F) -> Result<Observed, DiscoveryError>
where F: FnMut(usize, &mut [u8]) -> bool {
    if !pointer(address, HEADER) { return Err(DiscoveryError::InvalidPointer); }
    let mut header = [0; HEADER];
    if !read(address, &mut header) { return Err(DiscoveryError::ReadFailed); }
    Ok(Observed { address, header, string: None })
}

fn read_name<F>(root: &Observed, read: &mut F) -> Result<Vec<u8>, DiscoveryError>
where F: FnMut(usize, &mut [u8]) -> bool {
    let length = word(&root.header, 0x28);
    // Only seven bytes of a candidate root name can be inspected.
    if length != NAME.len() { return Err(DiscoveryError::InvalidString); }
    let data = word(&root.header, 0x20);
    let raw_capacity = word(&root.header, 0x30);
    let inline_flag = !(usize::MAX >> 1);
    let inline = raw_capacity & inline_flag != 0;
    let capacity = raw_capacity & !inline_flag;
    if capacity < length || capacity > 8192 || data < 0x10000
        || data.checked_add(length + 1).is_none()
        || (inline && Some(data) != root.address.checked_add(0x38)) {
        return Err(DiscoveryError::InvalidString);
    }
    let mut bytes = vec![0; length + 1];
    if !read(data, &mut bytes) { return Err(DiscoveryError::ReadFailed); }
    if bytes[length] != 0 || bytes[..length].contains(&0)
        || std::str::from_utf8(&bytes[..length]).is_err() {
        return Err(DiscoveryError::InvalidString);
    }
    bytes.pop();
    Ok(bytes)
}

fn recheck_observed<F>(observed: &Observed, read: &mut F) -> Result<(), DiscoveryError>
where F: FnMut(usize, &mut [u8]) -> bool {
    let again = read_header(observed.address, read)?;
    if again.header != observed.header { return Err(DiscoveryError::Changed); }
    if let Some(name) = &observed.string {
        if read_name(&again, read)? != *name { return Err(DiscoveryError::Changed); }
    }
    Ok(())
}

fn native_callback<C>(control: Control, is_code: &C) -> Result<(), DiscoveryError>
where C: Fn(usize) -> bool {
    if control.callback == 0 || !is_code(control.callback) {
        return Err(DiscoveryError::UnknownCallback);
    }
    Ok(())
}

fn pair<C>(root: Control, children: &HashMap<i16, Observed>, is_code: &C)
    -> Result<(Control, Control), DiscoveryError>
where C: Fn(usize) -> bool {
    let a = children.get(&3).ok_or(DiscoveryError::MissingPair)?.control();
    let b = children.get(&4).ok_or(DiscoveryError::MissingPair)?.control();
    native_callback(a, is_code)?;
    native_callback(b, is_code)?;
    let (left, right) = if a.area.left < b.area.left { (a, b) } else { (b, a) };
    // Conservative shape bounds, not target coordinates. Unknown layouts fail closed.
    if !inside(left.area, root.area) || !inside(right.area, root.area)
        || left.ty == 0 || left.ty != right.ty
        || left.area.top != right.area.top || left.area.bottom != right.area.bottom
        || left.area.right >= right.area.left || left.area.top > 64
        || left.area.bottom >= 64
        || !(8..=64).contains(&left.area.height())
        || !(8..=128).contains(&left.area.width())
        || !(8..=128).contains(&right.area.width())
        || left.area.width() < left.area.height() || right.area.width() < right.area.height() {
        return Err(DiscoveryError::InvalidPair);
    }
    Ok((left, right))
}

/// Return the first, visible, enabled native Minimap only. A preceding modal
/// prevents an intent, even if a Minimap remains behind it in the root list.
/// Absence/hidden roots return None; malformed or ambiguous metadata is an error.
pub fn discover<F, C>(first_dialog: usize, is_code: C, read: F)
    -> Result<Option<Discovery>, DiscoveryError>
where F: FnMut(usize, &mut [u8]) -> bool, C: Fn(usize) -> bool {
    discover_with_root_provider(first_dialog, is_code, |_|false, read)
}

/// Accept an already-installed root wrapper only through the caller's exact
/// provider predicate. The caller must verify its immutable saved original is
/// executable native game code. The alliance/chat children still require native
/// callbacks; root-provider recognition never applies to child handlers.
pub fn discover_with_root_provider<F, C, O>(first_dialog: usize, is_code: C,
    is_owned_root_provider: O, mut read: F) -> Result<Option<Discovery>, DiscoveryError>
where F: FnMut(usize, &mut [u8]) -> bool, C: Fn(usize) -> bool, O: Fn(usize) -> bool {
    let mut roots = Vec::new();
    let mut root_seen = HashSet::new();
    let mut current = first_dialog;
    let mut minimap = None;
    while current != 0 {
        if roots.len() >= MAX_ROOTS { return Err(DiscoveryError::TooManyRoots); }
        if !root_seen.insert(current) { return Err(DiscoveryError::CyclicRoots); }
        let mut root = read_header(current, &mut read)?;
        if root.ty() != 0 || !valid(root.area()) { return Err(DiscoveryError::InvalidRoot); }
        if word(&root.header, 0x28) == NAME.len() {
            let name = read_name(&root, &mut read)?;
            if name == NAME && minimap.replace(roots.len()).is_some() {
                return Err(DiscoveryError::DuplicateMinimap);
            }
            root.string = Some(name);
        }
        current = root.next();
        roots.push(root);
    }
    let Some(index) = minimap else {
        for root in &roots { recheck_observed(root, &mut read)?; }
        return Ok(None);
    };
    let root = &roots[index];
    if index != 0 || !root.visible() || !root.enabled() {
        for root in &roots { recheck_observed(root, &mut read)?; }
        return Ok(None);
    }
    let root_control = root.control();
    let area = root_control.area;
    if area.left < 0 || area.top < 0 || area.right > 4095 || area.bottom > 479
        || !(64..=512).contains(&area.width()) || !(64..=480).contains(&area.height()) {
        return Err(DiscoveryError::InvalidRoot);
    }
    if root_control.callback == 0
        || (!is_code(root_control.callback) && !is_owned_root_provider(root_control.callback)) {
        return Err(DiscoveryError::UnknownCallback);
    }
    let child_slot = root.address.checked_add(FIRST_CHILD).ok_or(DiscoveryError::InvalidPointer)?;
    let mut pointer_bytes = [0; 8];
    if !read(child_slot, &mut pointer_bytes) { return Err(DiscoveryError::ReadFailed); }
    let first_child = word(&pointer_bytes, 0);
    current = first_child;
    let mut children = Vec::new();
    let mut children_by_id = HashMap::new();
    let mut child_seen = HashSet::new();
    while current != 0 {
        if children.len() >= MAX_CHILDREN { return Err(DiscoveryError::TooManyChildren); }
        if root_seen.contains(&current) || !child_seen.insert(current) {
            return Err(DiscoveryError::CyclicChildren);
        }
        let child = read_header(current, &mut read)?;
        if word(&child.header, PARENT) != root.address { return Err(DiscoveryError::WrongParent); }
        current = child.next();
        if children_by_id.insert(child.id(), child.clone()).is_some() {
            return Err(DiscoveryError::DuplicateChildId);
        }
        children.push(child);
    }
    let (alliance, chat) = pair(root_control, &children_by_id, &is_code)?;
    for child in &children { recheck_observed(child, &mut read)?; }
    let mut again = [0; 8];
    if !read(child_slot, &mut again) { return Err(DiscoveryError::ReadFailed); }
    if again != pointer_bytes { return Err(DiscoveryError::Changed); }
    for root in &roots { recheck_observed(root, &mut read)?; }
    Ok(Some(Discovery { minimap: root_control, alliance, chat, roots, children, first_child }))
}

/// Re-discover the complete bounded graph immediately before UI-thread use.
/// Caller must also read first_dialog again on either side of this check.
pub fn recheck<F, C>(first_dialog: usize, expected: &Discovery, is_code: C, read: F)
    -> Result<(), DiscoveryError>
where F: FnMut(usize, &mut [u8]) -> bool, C: Fn(usize) -> bool {
    recheck_with_root_provider(first_dialog, expected, is_code, |_|false, read)
}

/// Recheck with the same verified root-provider policy used for discovery.
/// The exact observed callback and every header witness must remain unchanged.
pub fn recheck_with_root_provider<F, C, O>(first_dialog: usize, expected: &Discovery,
    is_code: C, is_owned_root_provider: O, read: F) -> Result<(), DiscoveryError>
where F: FnMut(usize, &mut [u8]) -> bool, C: Fn(usize) -> bool, O: Fn(usize) -> bool {
    if discover_with_root_provider(first_dialog, is_code, is_owned_root_provider, read)?
        .as_ref() != Some(expected) {
        return Err(DiscoveryError::Changed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const BASE: usize = 0x10000;
    const ROOT: usize = BASE;
    const LEFT: usize = BASE + 0x400;
    const RIGHT: usize = BASE + 0x600;
    const OTHER: usize = BASE + 0x800;
    const ORIGINAL: usize = 0x20000;
    struct Fixture { bytes: Vec<u8> }
    fn rect(left:i16, top:i16, right:i16, bottom:i16) -> Rect { Rect { left,top,right,bottom } }
    impl Fixture {
        fn new() -> Self {
            let mut f = Self { bytes: vec![0; 0x10000] };
            f.control(ROOT, 0, 0, 0, rect(0,315,137,479), true);
            f.name(ROOT, b"Minimap");
            f.word(ROOT, FIRST_CHILD, LEFT);
            f.control(LEFT, RIGHT, 3, 3, rect(5,2,65,23), false);
            f.control(RIGHT, 0, 4, 3, rect(72,2,132,23), true);
            f.word(LEFT, PARENT, ROOT); f.word(RIGHT, PARENT, ROOT);
            f
        }
        fn word(&mut self, p:usize, at:usize, value:usize) {
            self.bytes[p-BASE+at..p-BASE+at+8].copy_from_slice(&(value as u64).to_le_bytes());
        }
        fn short(&mut self, p:usize, at:usize, value:i16) {
            self.bytes[p-BASE+at..p-BASE+at+2].copy_from_slice(&value.to_le_bytes());
        }
        fn area(&mut self, p:usize, area:Rect) {
            for (at,value) in [(8,area.left),(10,area.top),(12,area.right),(14,area.bottom)] {
                self.short(p,at,value);
            }
        }
        fn control(&mut self,p:usize,next:usize,id:i16,ty:u16,area:Rect,visible:bool) {
            self.word(p,0,next); self.area(p,area); self.short(p,0x52,id);
            self.short(p,0x54,ty as i16); self.bytes[p-BASE+0x48]=if visible {2}else{0};
            self.word(p,CALLBACK,ORIGINAL + usize::from(ty)*0x10);
        }
        fn name(&mut self,p:usize,name:&[u8]) {
            self.word(p,0x20,p+0xa0); self.word(p,0x28,name.len()); self.word(p,0x30,0x100);
            self.bytes[p-BASE+0xa0..p-BASE+0xa0+name.len()].copy_from_slice(name);
        }
        fn read(&self, at:usize, out:&mut[u8]) -> bool {
            let Some(start)=at.checked_sub(BASE) else {return false;};
            let Some(end)=start.checked_add(out.len()) else {return false;};
            let Some(bytes)=self.bytes.get(start..end) else {return false;};
            out.copy_from_slice(bytes); true
        }
        fn discover(&self) -> Result<Option<Discovery>,DiscoveryError> {
            discover(ROOT, |p|(ORIGINAL..ORIGINAL+0x1000).contains(&p), |at,out|self.read(at,out))
        }
        fn found(&self) -> Discovery { self.discover().unwrap().unwrap() }
    }

    #[test] fn owned_root_wrapper_requires_explicit_verified_provider_policy() {
        const WRAPPER:usize=0x30000;
        let mut f=Fixture::new(); f.word(ROOT,CALLBACK,WRAPPER);
        let native=|p|(ORIGINAL..ORIGINAL+0x1000).contains(&p);
        assert_eq!(f.discover(),Err(DiscoveryError::UnknownCallback));
        assert_eq!(discover_with_root_provider(ROOT,native,|_|false,|at,out|f.read(at,out)),
            Err(DiscoveryError::UnknownCallback));
        assert_eq!(discover_with_root_provider(ROOT,native,|p|p==WRAPPER+8,|at,out|f.read(at,out)),
            Err(DiscoveryError::UnknownCallback));
        let before=f.bytes.clone();
        let found=discover_with_root_provider(ROOT,native,|p|p==WRAPPER,|at,out|f.read(at,out))
            .unwrap().unwrap();
        assert_eq!(found.minimap.callback,WRAPPER);
        assert_eq!(found.alliance.callback,ORIGINAL+0x30);
        assert_eq!(found.intent(true).unwrap().callback,ORIGINAL+0x30);
        assert_eq!(f.bytes,before);
        assert_eq!(recheck_with_root_provider(ROOT,&found,native,|p|p==WRAPPER,|at,out|f.read(at,out)),Ok(()));
        assert_eq!(recheck(ROOT,&found,native,|at,out|f.read(at,out)),Err(DiscoveryError::UnknownCallback));
        f.word(ROOT,CALLBACK,0);
        assert_eq!(discover_with_root_provider(ROOT,native,|_|true,|at,out|f.read(at,out)),
            Err(DiscoveryError::UnknownCallback));
    }
    #[test] fn root_wrapper_policy_never_accepts_wrapped_alliance_or_chat_children() {
        const WRAPPER:usize=0x30000;
        let native=|p|(ORIGINAL..ORIGINAL+0x1000).contains(&p);
        for child in [LEFT,RIGHT] {
            let mut f=Fixture::new(); f.word(ROOT,CALLBACK,WRAPPER); f.word(child,CALLBACK,WRAPPER);
            assert_eq!(discover_with_root_provider(ROOT,native,|_|true,|at,out|f.read(at,out)),
                Err(DiscoveryError::UnknownCallback));
        }
    }
    #[test] fn owned_root_provider_recheck_rejects_changed_callback_and_topology() {
        const WRAPPER:usize=0x30000;
        let native=|p|(ORIGINAL..ORIGINAL+0x1000).contains(&p);
        for changed in [ORIGINAL,WRAPPER+8] {
            let mut f=Fixture::new(); f.word(ROOT,CALLBACK,WRAPPER);
            let found=discover_with_root_provider(ROOT,native,|p|p==WRAPPER,|at,out|f.read(at,out))
                .unwrap().unwrap();
            f.word(ROOT,CALLBACK,changed);
            // Even a newly verified provider cannot replace the exact witness.
            assert_eq!(recheck_with_root_provider(ROOT,&found,native,|p|p==WRAPPER||p==WRAPPER+8,
                |at,out|f.read(at,out)),Err(DiscoveryError::Changed));
        }
        let mut f=Fixture::new(); f.word(ROOT,CALLBACK,WRAPPER);
        let found=discover_with_root_provider(ROOT,native,|p|p==WRAPPER,|at,out|f.read(at,out))
            .unwrap().unwrap();
        f.word(RIGHT,PARENT,OTHER);
        assert_eq!(recheck_with_root_provider(ROOT,&found,native,|p|p==WRAPPER,|at,out|f.read(at,out)),
            Err(DiscoveryError::WrongParent));
        assert_eq!(recheck_with_root_provider(0,&found,native,|p|p==WRAPPER,|at,out|f.read(at,out)),
            Err(DiscoveryError::Changed));
    }
    #[test] fn spatial_left_not_numeric_id_defines_alliance() {
        let mut f=Fixture::new();
        assert_eq!(f.found().alliance.control,LEFT);
        f.short(LEFT,0x52,4); f.short(RIGHT,0x52,3);
        let d=f.found(); assert_eq!(d.alliance.id,4); assert_eq!(d.chat.id,3);
        assert_eq!(d.alliance.control,LEFT);
    }
    #[test] fn show_and_hide_intents_touch_only_alliance() {
        let mut f=Fixture::new(); let d=f.found(); let before=f.bytes.clone();
        let show=d.intent(true).unwrap();
        assert_eq!(show.control,LEFT); assert_eq!(show.ext_type,EXT_SHOW);
        assert_ne!(show.control,d.chat.control); assert_eq!(show.followup_ext_type(1),None);
        assert_eq!(d.intent(false),None); assert_eq!(f.bytes,before);
        f.bytes[LEFT-BASE+0x48]=2; let d=f.found(); let hide=d.intent(false).unwrap();
        assert_eq!(hide.ext_type,EXT_HIDE); assert_eq!(hide.control,LEFT);
        assert_eq!(hide.followup_ext_type(0),None);
        assert_eq!(hide.followup_ext_type(1),Some(EXT_HIDE_FOLLOWUP));
        assert!(d.chat.visible); assert_eq!(d.chat.callback,ORIGINAL+0x30);
        assert_eq!(d.intent(true),None);
    }
    #[test] fn disabled_alliance_is_not_made_usable() {
        let mut f=Fixture::new(); f.bytes[LEFT-BASE+0x4c]=1;
        assert_eq!(f.found().intent(true),None);
    }
    #[test] fn exact_root_name_and_inline_storage_are_required() {
        let mut f=Fixture::new(); f.name(ROOT,b"MinimaX"); assert_eq!(f.discover().unwrap(),None);
        let mut f=Fixture::new(); f.word(ROOT,0x20,ROOT+0x38);
        f.word(ROOT,0x30,(1usize<<63)|15);
        f.bytes[0x38..0x3f].copy_from_slice(b"Minimap"); assert!(f.discover().unwrap().is_some());
        f.word(ROOT,0x20,ROOT+0x40); assert_eq!(f.discover(),Err(DiscoveryError::InvalidString));
    }
    #[test] fn child_strings_are_never_read() {
        let mut f=Fixture::new(); f.word(LEFT,0x20,1); f.word(LEFT,0x28,usize::MAX);
        f.word(RIGHT,0x20,2); f.word(RIGHT,0x28,usize::MAX);
        assert!(f.discover().unwrap().is_some());
    }
    #[test] fn hidden_disabled_or_covered_minimap_yields_no_intent() {
        let mut f=Fixture::new(); f.bytes[0x48]=0; assert_eq!(f.discover().unwrap(),None);
        let mut f=Fixture::new(); f.bytes[0x4c]=1; assert_eq!(f.discover().unwrap(),None);
        let mut f=Fixture::new(); f.control(OTHER,ROOT,0,0,rect(0,0,100,100),true);
        f.name(OTHER,b"Menu");
        assert_eq!(discover(OTHER,|_|true,|at,out|f.read(at,out)).unwrap(),None);
    }
    #[test] fn duplicate_minimap_roots_and_root_cycles_are_rejected() {
        let mut f=Fixture::new(); f.control(OTHER,0,0,0,rect(0,315,137,479),true);
        f.name(OTHER,b"Minimap"); f.word(ROOT,0,OTHER);
        assert_eq!(f.discover(),Err(DiscoveryError::DuplicateMinimap));
        let mut f=Fixture::new(); f.word(ROOT,0,ROOT);
        assert_eq!(f.discover(),Err(DiscoveryError::CyclicRoots));
    }
    #[test] fn child_cycles_and_root_child_alias_are_rejected() {
        let mut f=Fixture::new(); f.word(RIGHT,0,LEFT);
        assert_eq!(f.discover(),Err(DiscoveryError::CyclicChildren));
        let mut f=Fixture::new(); f.word(ROOT,FIRST_CHILD,ROOT);
        assert_eq!(f.discover(),Err(DiscoveryError::CyclicChildren));
    }
    #[test] fn parent_mismatch_and_duplicate_child_ids_are_rejected() {
        let mut f=Fixture::new(); f.word(RIGHT,PARENT,OTHER);
        assert_eq!(f.discover(),Err(DiscoveryError::WrongParent));
        let mut f=Fixture::new(); f.short(RIGHT,0x52,3);
        assert_eq!(f.discover(),Err(DiscoveryError::DuplicateChildId));
    }
    #[test] fn missing_pair_and_unverified_native_callbacks_are_rejected() {
        let mut f=Fixture::new(); f.short(RIGHT,0x52,5);
        assert_eq!(f.discover(),Err(DiscoveryError::MissingPair));
        for p in [ROOT,LEFT,RIGHT] {
            let mut f=Fixture::new(); f.word(p,CALLBACK,0);
            assert_eq!(f.discover(),Err(DiscoveryError::UnknownCallback));
            f.word(p,CALLBACK,0x30000);
            assert_eq!(f.discover(),Err(DiscoveryError::UnknownCallback));
        }
    }
    #[test] fn misaligned_overlapping_outside_or_unknown_type_pairs_are_rejected() {
        for (p,r) in [
            (RIGHT,rect(72,3,132,23)), (RIGHT,rect(60,2,120,23)),
            (LEFT,rect(-1,2,65,23)), (RIGHT,rect(72,2,140,23)),
            (LEFT,rect(5,64,65,85)), (LEFT,rect(5,2,65,65)),
            (LEFT,rect(5,2,10,23)),
        ] {
            let mut f=Fixture::new(); f.area(p,r);
            assert_eq!(f.discover(),Err(DiscoveryError::InvalidPair));
        }
        let mut f=Fixture::new(); f.short(LEFT,0x54,0);
        assert_eq!(f.discover(),Err(DiscoveryError::InvalidPair));
        let mut f=Fixture::new(); f.short(RIGHT,0x54,9);
        assert_eq!(f.discover(),Err(DiscoveryError::InvalidPair));
    }
    #[test] fn root_bounds_and_wrong_type_are_rejected() {
        let mut f=Fixture::new(); f.area(ROOT,rect(0,315,5000,479));
        assert_eq!(f.discover(),Err(DiscoveryError::InvalidRoot));
        let mut f=Fixture::new(); f.short(ROOT,0x54,3);
        assert_eq!(f.discover(),Err(DiscoveryError::InvalidRoot));
    }
    #[test] fn recheck_detects_callback_visibility_and_root_list_changes() {
        let mut f=Fixture::new(); let d=f.found();
        assert_eq!(recheck(ROOT,&d,|_|true,|at,out|f.read(at,out)),Ok(()));
        f.bytes[LEFT-BASE+0x48]=2;
        assert_eq!(recheck(ROOT,&d,|_|true,|at,out|f.read(at,out)),Err(DiscoveryError::Changed));
        let mut f=Fixture::new(); let d=f.found(); f.word(RIGHT,CALLBACK,ORIGINAL+0x40);
        assert_eq!(recheck(ROOT,&d,|_|true,|at,out|f.read(at,out)),Err(DiscoveryError::Changed));
        assert_eq!(recheck(0,&d,|_|true,|at,out|f.read(at,out)),Err(DiscoveryError::Changed));
    }
    #[test] fn traversal_rechecks_earlier_children_after_later_reads() {
        let mut f=Fixture::new(); let mut seen_right=false;
        let result=discover(ROOT,|_|true,|at,out| {
            if at==RIGHT && !seen_right {
                seen_right=true; f.bytes[LEFT-BASE+0x48]=2;
            }
            f.read(at,out)
        });
        assert_eq!(result,Err(DiscoveryError::Changed));
    }
    #[test] fn invalid_pointers_and_unreadable_headers_fail_closed() {
        let f=Fixture::new();
        assert_eq!(discover(1,|_|true,|at,out|f.read(at,out)),Err(DiscoveryError::InvalidPointer));
        assert_eq!(discover(ROOT,|_|true,|_,_|false),Err(DiscoveryError::ReadFailed));
    }
    #[test] fn second_game_rediscovers_recreated_minimap_and_refuses_previous_control_objects() {
        let mut f=Fixture::new();let old=f.found();
        assert_eq!(discover(0,|_|true,|at,out|f.read(at,out)).unwrap(),None);
        let root=BASE+0x1000;let left=root+0x400;let right=root+0x600;
        f.control(root,0,0,0,rect(0,315,137,479),true);f.name(root,b"Minimap");f.word(root,FIRST_CHILD,left);
        f.control(left,right,4,3,rect(5,2,65,23),false);f.control(right,0,3,3,rect(72,2,132,23),true);
        f.word(left,PARENT,root);f.word(right,PARENT,root);
        assert_eq!(recheck(root,&old,|_|true,|at,out|f.read(at,out)),Err(DiscoveryError::Changed));
        let fresh=discover(root,|p|(ORIGINAL..ORIGINAL+0x1000).contains(&p),|at,out|f.read(at,out)).unwrap().unwrap();
        let intent=fresh.intent(true).unwrap();assert_eq!(intent.control,left);assert_eq!(intent.ext_type,EXT_SHOW);
        assert_ne!(intent.control,old.alliance.control);assert_ne!(intent.control,fresh.chat.control);
        assert_eq!(recheck(root,&fresh,|_|true,|at,out|f.read(at,out)),Ok(()));
        f.bytes[left-BASE+0x48]=2;
        let shown=discover(root,|_|true,|at,out|f.read(at,out)).unwrap().unwrap();assert_eq!(shown.intent(true),None);
    }
}
