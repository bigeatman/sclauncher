use crate::memory::{Value, module_path, read_integer, read_memory, snapshot_image};
use samase_scarf::Analysis;
use scarf::{BinaryFile, ExecutionStateX86_64, Operand, OperandContext, VirtualAddress64};
use std::cell::Cell;
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use winapi::ctypes::c_void;
use winapi::shared::minwindef::{DWORD, LPVOID};
use winapi::um::libloaderapi::{GetModuleHandleExW, GetModuleHandleW};
use winapi::um::processthreadsapi::{GetCurrentProcessId, GetCurrentThreadId};
use winapi::um::synchapi::{CreateEventW, WaitForSingleObject};
use winapi::um::sysinfoapi::GetTickCount64;
use winapi::um::winbase::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW};
use winapi::um::winuser::{GetAsyncKeyState, GetForegroundWindow, GetWindowThreadProcessId};

const UNIT_SIZE: usize = 0x1e8;
// Pinned samase_scarf x64 StructLayouts::unit_flags(): 0x140.
const UNIT_FLAGS_OFFSET: usize = 0x140;
const UNIT_READ_SIZE: usize = UNIT_FLAGS_OFFSET + 4;
const MAX_UNITS: usize = 8192;
// Leave space below the classic 0x200-byte outgoing command buffer limit.
const OUTGOING_BUDGET: usize = 0x1e0;
const MAX_PENDING: usize = 2048;
type SendCommand = unsafe extern "C" fn(*const u8, usize);
// Saved caller/target analysis supports this void single-event-pointer ABI.
// Global callbacks cover map clicks; a separate command-panel callback observes
// GUI-consumed stop/attack inputs. Other callback paths remain outside this prototype.
type UiCallback = unsafe extern "C" fn(*const c_void);
// Native SCR Control callback, separately verified from global event callbacks.
type PanelCallback = unsafe extern "C" fn(*const c_void, *const c_void) -> u32;
static PANEL_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static SEND_LOCK: Mutex<()> = Mutex::new(());
static EVENT_THREAD: AtomicU32 = AtomicU32::new(0);
// Installed providers outlive a match; ownership of input and queued actions
// does not. Read fresh state while holding the lifecycle mutex.
static GAME_SESSION: Mutex<crate::game_session::Tracker> = Mutex::new(crate::game_session::Tracker::new());
static GAME_SESSION_GENERATION: AtomicU64 = AtomicU64::new(0);
static SESSION_BOUNDARY_EPOCH: AtomicU64 = AtomicU64::new(0);
static SESSION_HISTORY: Mutex<(crate::game_session_diagnostics::History,bool)> = Mutex::new((crate::game_session_diagnostics::History::new(),false));
thread_local! { static IN_ORIGINAL: Cell<bool> = const { Cell::new(false) }; }
static ORIGINAL_SEND: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CALLBACKS: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];
static BINDINGS: OnceLock<[crate::callback_binding::Slot; 4]> = OnceLock::new();
static REGISTRATION_LOCK: Mutex<()> = Mutex::new(());
static BINDING_EPOCH: AtomicU64 = AtomicU64::new(0);
thread_local! {
    static CALLBACK_DEPTH: Cell<u32> = const { Cell::new(0) };
    static CALLBACK_NESTED: Cell<bool> = const { Cell::new(false) };
}
// INSTALLED records the first complete publication. Readiness may subsequently
// return to WAITING when the engine clears its table between games.
static INSTALLED: AtomicBool = AtomicBool::new(false);
static CALLBACKS_READY: AtomicBool = AtomicBool::new(false);
#[derive(Clone, Copy)]
#[repr(u32)]
enum InitPhase {
    Analyzing = 0,
    WaitingForCallbacks = 1,
    Ready = 2,
    Failed = 3,
}
static INIT_PHASE: AtomicU32 = AtomicU32::new(InitPhase::Analyzing as u32);
static INIT_MESSAGE: Mutex<String> = Mutex::new(String::new());
static INIT_DIAGNOSTIC: Mutex<String> = Mutex::new(String::new());
static FAULT: AtomicBool = AtomicBool::new(false);
static STOP_EVENT: AtomicUsize = AtomicUsize::new(0);
static CLIENT_SEEN_TICK: AtomicU64 = AtomicU64::new(0);
static RUNTIME: OnceLock<Runtime> = OnceLock::new();
#[path = "runtime_alliance.rs"]
mod runtime_alliance;
static UI_METADATA_LAST: Mutex<(String, u32, u32, u64)> = Mutex::new((String::new(), 0, 0, 0));
static STATE: Mutex<State> = Mutex::new(State {
    active: false,
    count: 0,
    kind: 0,
    owner: 0,
    shift: 13,
    frame: 0,
    sent: 0,
    last_key: false,
    last_selection: Vec::new(),
    pending: VecDeque::new(),
    message: "Connect and enter a game",
    events: VecDeque::new(),
    last_scan: 0,
    visual_frame: None,
    visual_tick: None,
    panel_diagnostic: None,
    control_traces: 0,
    session_boundary: 0,
});
struct Runtime {
    alliance: Option<runtime_alliance::Config>,
    unit_vectors: Vec<Value>,
    units: Value,
    first: Value,
    selection: Value,
    local: Value,
    replay: Value,
    main_state: Value,
    // The color-command handler's lobby branch can reflect host authority.
    // Retain it only as optional diagnostics; it does not prove a started match.
    lobby_color_gate: Option<Value>,
    continuing: Value,
    paused: Value,
    frame: Value,
    outgoing: Value,
    outgoing_buffer: Value,
    outgoing_capacity: Value,
    visual: Option<crate::visuals::Config>,
    gui: Option<GuiRuntime>,
    metadata: Option<UnitMetadataSource>,
}
struct UnitMetadataSource {
    table: Value,
    stride: usize,
}
struct GuiRuntime {
    first_dialog: Value,
    code_start: usize,
    code_end: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UnitLayout {
    base: usize,
    count: Option<usize>,
    shift: Option<u8>,
}
struct Resolved {
    runtime: Runtime,
    send: usize,
    table: usize,
    callbacks: [usize; 4],
    send_bytes: [u8; 16],
}
struct Unit {
    pointer: usize,
    index: u32,
    minor: u8,
    kind: u16,
    owner: u8,
    next: usize,
}
struct Pending {
    ids: Vec<u32>,
    restore: Vec<u32>,
    command: Vec<u8>,
    created: Instant,
}
struct State {
    active: bool,
    count: usize,
    kind: u16,
    owner: u8,
    shift: u8,
    frame: u32,
    sent: u64,
    last_key: bool,
    last_selection: Vec<u32>,
    pending: VecDeque<Pending>,
    message: &'static str,
    events: VecDeque<String>,
    last_scan: u32,
    visual_frame: Option<crate::visuals::Frame>,
    visual_tick: Option<Instant>,
    panel_diagnostic: Option<String>,
    control_traces: u8,
    session_boundary: u64,
}
impl State {
    fn note(&mut self, text: &'static str) {
        self.message = text;
        if self.events.len() >= 256 {
            self.events.pop_front();
        }
        self.events.push_back(format!(
            "{}\t{}\t{}\t{}\t{}",
            self.frame, self.count, self.kind, self.sent, text
        ));
    }
    fn stop(&mut self, reason: &'static str) {
        let changed = self.active || !self.pending.is_empty();
        self.active = false;
        self.count = 0;
        self.pending.clear();
        self.visual_frame = None;
        self.visual_tick = None;
        if changed {
            self.note(reason);
        } else {
            self.message = reason;
        }
    }
}
impl Unit {
    fn uid(&self, shift: u8) -> Option<u32> {
        if shift == 0 {
            let short = self.uid(11)?;
            return (Some(short) == self.uid(13)).then_some(short);
        }
        if !matches!(shift, 11 | 13) || self.index + 1 >= (1 << shift) {
            None
        } else {
            Some((self.index + 1) | ((self.minor as u32) << shift))
        }
    }
}
#[derive(Debug)]
struct SelectionReadError {
    message: &'static str,
    detail: String,
}
fn selection_error(message: &'static str, detail: String) -> SelectionReadError {
    SelectionReadError { message, detail }
}
fn read_unit_checked(pointer: usize, base: usize) -> Result<Unit, SelectionReadError> {
    let offset = pointer.checked_sub(base).ok_or_else(|| selection_error(
        "Selected unit pointer is outside unit array",
        format!("pointer=0x{pointer:x}; base=0x{base:x}; before_base=1"),
    ))?;
    if offset % UNIT_SIZE != 0 || offset / UNIT_SIZE >= MAX_UNITS {
        return Err(selection_error("Selected unit pointer is outside unit array",
            format!("pointer=0x{pointer:x}; base=0x{base:x}; offset=0x{offset:x}; stride=0x{UNIT_SIZE:x}")));
    }
    let mut data = [0u8; UNIT_READ_SIZE];
    if !read_memory(pointer, &mut data) {
        return Err(selection_error("Selected unit memory read failed",
            format!("pointer=0x{pointer:x}; requested=0x{UNIT_READ_SIZE:x}")));
    }
    let q = |x| u64::from_le_bytes(data[x..x + 8].try_into().unwrap()) as usize;
    let d = |x| u32::from_le_bytes(data[x..x + 4].try_into().unwrap());
    let flags = d(UNIT_FLAGS_OFFSET);
    let sprite = q(0x18);
    let hp = d(0x10);
    let order = data[0x69];
    let kind = u16::from_le_bytes(data[0x8c..0x8e].try_into().unwrap());
    let owner = data[0x68];
    let detail = || format!("pointer=0x{pointer:x}; base=0x{base:x}; index={}; sprite=0x{sprite:x}; hp={hp}; order={order}; owner={owner}; kind={kind}; flags@0x{UNIT_FLAGS_OFFSET:x}=0x{flags:x}", offset / UNIT_SIZE);
    let reason = if sprite == 0 {
        Some("Selected unit sprite is unavailable")
    } else if hp == 0 || order == 0 {
        Some("Selected unit is dead or has no live order")
    } else if flags & 1 == 0 {
        Some("Selected unit is not completed")
    } else if flags & 0x40 != 0 {
        Some("Selected unit is inside transport")
    } else if kind >= 228 || owner >= 8 {
        Some("Selected unit type or owner is invalid")
    } else {
        None
    };
    if let Some(message) = reason { return Err(selection_error(message, detail())); }
    Ok(Unit { pointer, index: (offset / UNIT_SIZE) as u32, minor: data[0xe9], kind, owner, next: q(8) })
}
fn read_unit(pointer: usize, base: usize) -> Option<Unit> {
    read_unit_checked(pointer, base).ok()
}
// Passive HUD validation deliberately differs from command validation: an
// ordinary selection can contain unfinished/morphing or enemy/neutral units.
// This reader never admits anything into the existing control path.
fn read_unit_for_display(pointer: usize, base: usize, limit: usize) -> Option<(u16, u8)> {
    let offset = pointer.checked_sub(base)?;
    if offset % UNIT_SIZE != 0 || offset / UNIT_SIZE >= limit.min(MAX_UNITS) {
        return None;
    }
    let mut data = [0u8; UNIT_READ_SIZE];
    if !read_memory(pointer, &mut data) { return None; }
    let hp = i32::from_le_bytes(data[0x10..0x14].try_into().ok()?);
    let sprite = u64::from_le_bytes(data[0x18..0x20].try_into().ok()?);
    let kind = u16::from_le_bytes(data[0x8c..0x8e].try_into().ok()?);
    let owner = data[0x68];
    if hp <= 0 || sprite == 0 || data[0x69] == 0 || kind >= 228 || owner >= 12 {
        return None;
    }
    Some((kind, data[0xe9]))
}
impl Runtime {
    fn command_metadata(&self, kind: u16) -> Option<crate::building_commands::Metadata> {
        self.metadata.as_ref().and_then(|source| {
            crate::building_commands::read_metadata(source.table.read()?,source.stride,kind,read_integer)
        }).or_else(|| crate::building_commands::standard_production_metadata(kind))
    }
    fn morph_restore_unit(&self, pointer: usize, owner: u8, shift: u8) -> Option<u32> {
        let layout=self.unit_layout()?;
        let offset=pointer.checked_sub(layout.base)?;
        if offset % UNIT_SIZE != 0 || offset / UNIT_SIZE >= layout.count.unwrap_or(MAX_UNITS)
            || layout.shift.is_some_and(|actual|shift!=actual && shift!=0) {return None;}
        let mut data=[0u8;UNIT_READ_SIZE];
        if !read_memory(pointer,&mut data) {return None;}
        let hp=i32::from_le_bytes(data[0x10..0x14].try_into().ok()?);
        let sprite=u64::from_le_bytes(data[0x18..0x20].try_into().ok()?);
        let kind=u16::from_le_bytes(data[0x8c..0x8e].try_into().ok()?);
        let flags=u32::from_le_bytes(data[0x140..0x144].try_into().ok()?);
        if data[0x68] != owner || hp <= 0 || sprite == 0 || data[0x69] == 0 || flags & 0x40 != 0 {return None;}
        if kind != 35 && kind != 36 {return None;}
        if kind == 35 && flags & 1 == 0 {return None;}
        let unit=Unit{pointer,index:(offset/UNIT_SIZE) as u32,minor:data[0xe9],kind,owner,next:0};
        unit.uid(shift)
    }
    fn morph_selection_ids(&self, owner: u8, shift: u8) -> Option<Vec<u32>> {
        let address=self.selection.read()?;
        let mut ids=Vec::new();let mut seen=HashSet::new();
        for index in 0..12 {
            let pointer=read_integer(address.checked_add(index*8)?,8)?;
            if pointer == 0 {continue;}
            let id=self.morph_restore_unit(pointer,owner,shift)?;
            if !seen.insert(id) {return None;}
            // The first selected unit may be an Egg. Later slots must still
            // be multi-selectable Larvae; never silently drop later Eggs.
            if !ids.is_empty() && read_integer(pointer.checked_add(0x8c)?,2)? == 36 {
                return None;
            }
            ids.push(id);
        }
        if ids.is_empty() {None} else {Some(ids)}
    }
    fn valid_morph_restore(&self, id: u32, owner: u8, shift: u8) -> bool {
        if !matches!(shift,0 | 11 | 13) {return false;}
        let mask_shift=if shift==0 {11} else {shift};
        let index=id & ((1 << mask_shift)-1);
        if index==0 {return false;}
        let Some(layout)=self.unit_layout() else {return false;};
        let Some(pointer)=layout.base.checked_add((index as usize-1)*UNIT_SIZE) else {return false;};
        self.morph_restore_unit(pointer,owner,shift)==Some(id)
    }
    // SC:R BwVector is { data, length, capacity }, each pointer-sized. Candidate
    // metadata comes from samase_scarf limits(), whose auxiliary arrays are not
    // ordered. Equality with the independently resolved units pointer identifies
    // the primary unit vector. The UnitArray ID rule uses length > 1700.
    fn unit_layout(&self) -> Option<UnitLayout> {
        let base = self.units.read()?;
        if base < 0x10000 {
            return None;
        }
        let mut count = None;
        for candidate in &self.unit_vectors {
            let Some(address) = candidate.read() else {
                continue;
            };
            let mut data = [0u8; 24];
            if !read_memory(address, &mut data) {
                continue;
            }
            let word =
                |offset| u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap()) as usize;
            if word(0) != base {
                continue;
            }
            let length = word(8);
            let capacity = word(16);
            if length == 0
                || length >= MAX_UNITS
                || capacity < length
                || capacity > MAX_UNITS
                || base.checked_add(capacity.checked_mul(UNIT_SIZE)?).is_none()
            {
                return None;
            }
            if count.is_some_and(|previous| previous != length) {
                return None;
            }
            count = Some(length);
        }
        Some(UnitLayout {
            base,
            count,
            shift: count.map(|n| if n > 1700 { 13 } else { 11 }),
        })
    }
    fn selection_shift(&self, reference: &Unit, last: &[u32]) -> Option<u8> {
        let layout = self.unit_layout()?;
        if let Some(count) = layout.count {
            if reference.index as usize >= count {
                return None;
            }
        }
        match layout.shift {
            Some(shift) => {
                let id = reference.uid(shift)?;
                (last.len() == 1 && last[0] == id).then_some(shift)
            }
            None => infer_shift(reference, last),
        }
    }
    fn context(&self) -> Option<(bool, u32, u8)> {
        let frame = u32::try_from(self.frame.read()?).ok()?;
        let owner = u8::try_from(self.local.read()?).ok()?;
        // Pinned samase_scarf GameInit proves scmain_state switch case 3 calls
        // game_loop, while case 4 calls run_menus. This gate is independent of
        // lobby host authority and excludes stale loop/frame values in menus.
        let active = self.replay.read()? == 0
            && self.main_state.read()? == 3
            && self.continuing.read()? == 1
            && owner < 8;
        Some((active, frame, owner))
    }
    fn display_selection(&self) -> Option<DisplaySelection> {
        let layout = self.unit_layout()?;
        let address = self.selection.read()?;
        // Capture the bounded UI array twice. A selection changed between reads
        // is discarded rather than publishing a partial or stale count.
        let mut pointers = [0usize; 12];
        for (index, pointer) in pointers.iter_mut().enumerate() {
            *pointer = read_integer(address.checked_add(index.checked_mul(8)?)?, 8)?;
        }
        let mut seen = HashSet::new();
        let mut units = Vec::new();
        for &pointer in &pointers {
            if pointer == 0 { continue; }
            if !seen.insert(pointer) { return None; }
            let (kind, generation) = read_unit_for_display(pointer, layout.base,
                layout.count.unwrap_or(MAX_UNITS))?;
            units.push((pointer, kind, generation));
        }
        for (index, &pointer) in pointers.iter().enumerate() {
            if read_integer(address.checked_add(index.checked_mul(8)?)?, 8)? != pointer {
                return None;
            }
        }
        // Detect a recycled slot while the UI array retained the same address.
        for &(pointer, kind, generation) in &units {
            if read_unit_for_display(pointer, layout.base, layout.count.unwrap_or(MAX_UNITS))?
                != (kind, generation) { return None; }
        }
        let first = units.first().map(|unit| unit.1);
        let kind = first.filter(|kind| units.iter().all(|unit| unit.1 == *kind))
            .unwrap_or(NO_DISPLAY_UNIT);
        Some(DisplaySelection { in_game: true, count: units.len(), kind, control_active: false })
    }
    fn selected_checked(&self) -> Result<Vec<Unit>, SelectionReadError> {
        let layout = self.unit_layout().ok_or_else(|| selection_error(
            "Unit array layout unavailable", "unit_layout=unavailable".into(),
        ))?;
        let base = layout.base;
        let address = self.selection.read().ok_or_else(|| selection_error(
            "Selection array address is unavailable", format!("base=0x{base:x}"),
        ))?;
        let mut result = Vec::new();
        let mut seen = HashSet::new();
        for i in 0..12 {
            let slot_address = address.checked_add(i * 8).ok_or_else(|| selection_error(
                "Selection slot memory read failed", format!("array=0x{address:x}; slot={i}; overflow=1"),
            ))?;
            let pointer = read_integer(slot_address, 8).ok_or_else(|| selection_error(
                "Selection slot memory read failed", format!("array=0x{address:x}; slot={i}; address=0x{slot_address:x}"),
            ))?;
            if pointer == 0 { continue; }
            if !seen.insert(pointer) {
                return Err(selection_error("Selected unit pointers are duplicated",
                    format!("array=0x{address:x}; slot={i}; pointer=0x{pointer:x}")));
            }
            let offset = pointer.checked_sub(base).ok_or_else(|| selection_error(
                "Selected unit pointer is outside unit array",
                format!("array=0x{address:x}; slot={i}; pointer=0x{pointer:x}; base=0x{base:x}; before_base=1"),
            ))?;
            if layout.count.is_some_and(|count| offset / UNIT_SIZE >= count) {
                return Err(selection_error("Selected unit pointer is outside unit array",
                    format!("array=0x{address:x}; slot={i}; pointer=0x{pointer:x}; base=0x{base:x}; length={:?}", layout.count)));
            }
            match read_unit_checked(pointer, base) {
                Ok(unit) => result.push(unit),
                Err(mut error) => {
                    error.detail = format!("array=0x{address:x}; slot={i}; {}", error.detail);
                    return Err(error);
                }
            }
        }
        Ok(result)
    }
    fn selected(&self) -> Option<Vec<Unit>> {
        self.selected_checked().ok()
    }
    fn matching(&self, owner: u8, kind: u16, shift: u8) -> Option<Vec<u32>> {
        let layout = self.unit_layout()?;
        let base = layout.base;
        if layout
            .shift
            .is_some_and(|actual| shift != actual && shift != 0)
        {
            return None;
        }
        let mut pointer = self.first.read()?;
        let mut visited = HashSet::new();
        let mut ids = Vec::new();
        while pointer != 0 {
            let offset = pointer.checked_sub(base)?;
            if offset % UNIT_SIZE != 0
                || offset / UNIT_SIZE >= layout.count.unwrap_or(MAX_UNITS)
                || !visited.insert(pointer)
            {
                return None;
            }
            let next = read_integer(pointer.checked_add(8)?, 8)?;
            if let Some(unit) = read_unit(pointer, base) {
                if unit.owner == owner && unit.kind == kind {
                    ids.push(unit.uid(shift)?);
                }
            }
            if visited.len() > MAX_UNITS {
                return None;
            }
            pointer = next;
        }
        Some(ids)
    }
    fn valid_id(&self, id: u32, owner: u8, kind: u16, shift: u8) -> bool {
        if !matches!(shift, 0 | 11 | 13) {
            return false;
        }
        let mask_shift = if shift == 0 { 11 } else { shift };
        let index = id & ((1 << mask_shift) - 1);
        if index == 0 || index as usize > MAX_UNITS {
            return false;
        }
        let Some(layout) = self.unit_layout() else {
            return false;
        };
        if layout.count.is_some_and(|count| index as usize > count)
            || layout
                .shift
                .is_some_and(|actual| shift != actual && shift != 0)
        {
            return false;
        }
        let Some(pointer) = layout.base.checked_add((index as usize - 1) * UNIT_SIZE) else {
            return false;
        };
        read_unit(pointer, layout.base)
            .is_some_and(|u| u.owner == owner && u.kind == kind && u.uid(shift) == Some(id))
    }
}
fn infer_shift(reference: &Unit, last: &[u32]) -> Option<u8> {
    if last.len() != 1 {
        return None;
    }
    let short = reference.uid(11) == Some(last[0]);
    let long = reference.uid(13) == Some(last[0]);
    match (short, long) {
        (true, false) => Some(11),
        (false, true) => Some(13),
        (true, true) => Some(0),
        _ => None,
    }
}
fn selection_record(ids: &[u32]) -> Option<Vec<u8>> {
    if ids.is_empty() || ids.len() > 12 || ids.contains(&0) {
        return None;
    }
    let mut result = vec![0x63, ids.len() as u8];
    for id in ids {
        result.extend(id.to_le_bytes());
    }
    Some(result)
}
fn supported(command: &[u8]) -> bool {
    crate::batch::validated_command(command).is_ok()
}
fn capture_selection(command: &[u8]) -> Option<Vec<u32>> {
    if command.first() != Some(&0x63)
        || command.len() < 2
        || command[1] == 0
        || command[1] > 12
        || command.len() != 2 + command[1] as usize * 4
    {
        return None;
    }
    let ids: Vec<u32> = command[2..]
        .chunks_exact(4)
        .map(|x| u32::from_le_bytes(x.try_into().unwrap()))
        .collect();
    let mut seen = HashSet::new();
    if ids.iter().any(|&id| id == 0 || !seen.insert(id)) {
        return None;
    }
    Some(ids)
}
fn selection_ids_for_mode(selected: &[Unit], shift: u8) -> Option<Vec<u32>> {
    if selected.is_empty() || selected.len() > 12 {
        return None;
    }
    let mut seen = HashSet::new();
    selected.iter().map(|unit| {
        let id = unit.uid(shift)?;
        (id != 0 && seen.insert(id)).then_some(id)
    }).collect()
}
fn infer_group_shift(selected: &[Unit], observed: &[u32]) -> Option<u8> {
    if observed.is_empty() || observed.len() != selected.len() {
        return None;
    }
    let short = selection_ids_for_mode(selected, 11).as_deref() == Some(observed);
    let long = selection_ids_for_mode(selected, 13).as_deref() == Some(observed);
    match (short, long) {
        (true, false) => Some(11),
        (false, true) => Some(13),
        (true, true) => Some(0),
        _ => None,
    }
}
fn activate_same_type(runtime: &Runtime, state: &mut State, owner: u8, frame: u32)
    -> Result<(), &'static str>
{
    let selected = match runtime.selected_checked() {
        Ok(selected) => selected,
        Err(error) => {
            if state.events.len() >= 256 { state.events.pop_front(); }
            state.events.push_back(format!("{}\t{}\t{}\t{}\tSelection diagnostic; reason={}; {}",
                frame, state.count, state.kind, state.sent, error.message, error.detail));
            return Err(error.message);
        }
    };
    if selected.is_empty() {
        return Err("Select owned same-type units first");
    }
    if selected.iter().any(|unit| unit.owner != owner) {
        return Err("Selected units are not all owned by local player");
    }
    let kind = selected[0].kind;
    if selected.iter().any(|unit| unit.kind != kind) {
        return Err("Selected units have different types");
    }
    if runtime.command_metadata(kind).is_some_and(|meta|meta.building) && selected.len()!=1 {
        return Err("Select one completed owned building first");
    }
    let layout = runtime.unit_layout().ok_or("Unit array layout unavailable")?;
    let shift = layout.shift.or_else(|| infer_group_shift(&selected, &state.last_selection))
        .ok_or("Selection ID encoding unavailable; click selected units again")?;
    let original_ids = selection_ids_for_mode(&selected, shift)
        .ok_or("Selected unit IDs are invalid or duplicated")?;
    let matching = runtime.matching(owner, kind, shift)
        .ok_or("Matching unit list unavailable")?;
    if matching.is_empty() {
        return Err("No controllable matching units");
    }
    let matching_set: HashSet<u32> = matching.iter().copied().collect();
    if matching_set.len() != matching.len()
        || original_ids.iter().any(|id| !matching_set.contains(id)
            || !runtime.valid_id(*id, owner, kind, shift))
    {
        return Err("Selected units are missing from matching unit list");
    }
    // Only virtual control state changes here. No selection command is sent,
    // and the native UI array and original selection order stay untouched.
    state.last_selection = original_ids;
    state.control_traces = 0;
    state.active = true;
    state.owner = owner;
    state.kind = kind;
    state.shift = shift;
    state.count = matching.len();
    state.last_scan = frame;
    state.frame = frame;
    state.note("Matching owned units selected");
    Ok(())
}
fn selected_ids(runtime: &Runtime, owner: u8, kind: u16, shift: u8) -> Option<Vec<u32>> {
    let selected = runtime.selected()?;
    if selected.is_empty() || selected.iter().any(|u| u.owner != owner || u.kind != kind) {
        return None;
    }
    selected.iter().map(|unit| unit.uid(shift)).collect()
}
fn morph_copies_pending(state: &State) -> bool {
    state.kind==35 && state.pending.iter().any(|job|job.command.first()==Some(&0x23))
}
fn active_selection_ids(runtime: &Runtime, state: &State) -> Option<Vec<u32>> {
    if morph_copies_pending(state) {runtime.morph_selection_ids(state.owner,state.shift)}
    else {selected_ids(runtime,state.owner,state.kind,state.shift)}
}
fn is_larva_morph(command: &[u8], kind: u16) -> bool {
    kind == 35 && matches!(crate::building_commands::classify(command),
        Some(crate::building_commands::Action::Morph { .. }))
}
fn observed_selection_matches(runtime: &Runtime, state: &State, original: &[u32],
    command: &[u8]) -> bool
{
    selected_ids(runtime, state.owner, state.kind, state.shift).as_deref() == Some(original)
        || (is_larva_morph(command, state.kind)
            && runtime.morph_selection_ids(state.owner, state.shift).as_deref() == Some(original))
}
// Numeric, bounded diagnostics in the module's own action log. No code bytes,
// addresses, raw packets, or additional process access are recorded.
fn trace_control(runtime: &Runtime, state: &mut State, stage: &str,
    delta_length: Option<usize>, opcode: Option<u8>)
{
    if state.control_traces >= 24 || !(state.kind == 35 || state.kind >= 106) { return; }
    state.control_traces += 1;
    let display = runtime.display_selection();
    let row = format!("{}\t{}\t{}\t{}\tControl trace; stage={stage}; native_count={}; native_type={}; pending={}; delta_len={}; opcode={}",
        state.frame, state.count, state.kind, state.sent,
        display.map_or(0, |s| s.count), display.map_or(NO_DISPLAY_UNIT, |s| s.kind),
        state.pending.len(), delta_length.map_or(-1, |n| n as i64), opcode.map_or(-1, i64::from));
    if state.events.len() >= 256 { state.events.pop_front(); }
    state.events.push_back(row);
}
fn queue_control(data: &[u8], original_ids: &[u32], state: &mut State, runtime: &Runtime) -> bool {
    queue_observed_control(data, original_ids, state, runtime, false)
}
fn queue_observed_control(data: &[u8], original_ids: &[u32], state: &mut State,
    runtime: &Runtime, morph_after_original: bool) -> bool
{
    let Some((live, frame, owner)) = runtime.context() else {
        state.stop("Game state unavailable");
        return false;
    };
    if !live || owner != state.owner || frame < state.frame {
        state.stop("Game changed; selection cleared");
        return false;
    }
    state.frame = frame;
    let morph_transition = morph_after_original && is_larva_morph(data, state.kind);
    if morph_transition && original_ids != state.last_selection.as_slice()
        || if morph_transition { !observed_selection_matches(runtime, state, original_ids, data) }
            else { selected_ids(runtime, state.owner, state.kind, state.shift).as_deref() != Some(original_ids) }
    {
        trace_control(runtime, state, "queue-selection-rejected", Some(data.len()), data.first().copied());
        state.stop("Selection changed; group cleared");
        return false;
    }
    let Some(mut ids) = runtime.matching(state.owner, state.kind, state.shift) else {
        state.stop("Unit list validation failed");
        return false;
    };
    if morph_transition {
        // The capture proves these exact original Larvae received the command.
        // A generation-identical Egg no longer appears in the Larva list, but
        // it must still be excluded from copies and retained for exact restore.
        for &id in original_ids {
            if !runtime.valid_morph_restore(id, state.owner, state.shift) {
                state.stop("Original morph identity changed; group cleared"); return false;
            }
            if !ids.contains(&id) { ids.push(id); }
        }
    }
    let Some(action)=crate::building_commands::classify(data) else {
        state.stop("Unsupported control; group cleared");return false;
    };
    let metadata=runtime.command_metadata(state.kind);
    let old_control=matches!(action,crate::building_commands::Action::RightClick{..}|crate::building_commands::Action::Attack{..}|crate::building_commands::Action::Stop{..});
    let fallback=crate::building_commands::Metadata{building:false,produces:false,race_bits:0};
    if metadata.is_none() && (!old_control || state.kind>=106) {
        state.stop("Unit command metadata unavailable; group cleared");return false;
    }
    let metadata=metadata.unwrap_or(fallback);
    if !crate::building_commands::allowed(action,state.kind,metadata) {
        state.stop("Unsupported production or rally for selected type; group cleared");return false;
    }
    if metadata.building && original_ids.len()!=1 {
        state.stop("Building control requires one original building; group cleared");return false;
    }
    let matching_count=ids.len();
    let intent=match crate::event_capture::build_copy_intent(&ids,original_ids,data) {
        Ok(intent)=>intent,
        Err(_)=>{state.stop("Original selection or control validation failed");return false;}
    };
    let crate::event_capture::CopyIntent::Copies{ids,restore_ids,command}=intent else {
        state.count=ids.len();
        if morph_transition && state.pending.is_empty()
            && selected_ids(runtime,state.owner,state.kind,state.shift).is_none() {
            state.stop("Original larvae already received production; group cleared");
        } else { state.note("Original selection already received control"); }
        return true;
    };
    let batch_limit=crate::building_commands::dispatch_batch_limit(metadata);
    let retained=state.pending.iter().filter(|job|crate::building_commands::retains_pending(action,&job.command)).count();
    if retained+ids.len().div_ceil(batch_limit)>MAX_PENDING {
        state.stop("Too many queued commands; group cleared");return false;
    }
    // Production is additive. Rally/stop replaces stale control jobs, but keeps
    // already requested production for every remaining building or larva.
    state.pending.retain(|job|crate::building_commands::retains_pending(action,&job.command));
    state.count=matching_count;
    for chunk in ids.chunks(batch_limit) {
        state.pending.push_back(Pending{ids:chunk.to_vec(),restore:restore_ids.clone(),command:command.clone(),created:Instant::now()});
    }
    state.note("Observed control queued for other matching owned units");
    true
}
unsafe fn forward_original(data: *const u8, length: usize) {
    runtime_alliance::note_owned_append();
    let original: SendCommand =
        unsafe { std::mem::transmute(ORIGINAL_SEND.load(Ordering::Acquire)) };
    let previous = IN_ORIGINAL.with(|flag| flag.replace(true));
    unsafe {
        original(data, length);
    }
    IN_ORIGINAL.with(|flag| flag.set(previous));
}
fn client_present(now: u64, last: u64) -> bool {
    last != 0 && now >= last && now - last < 3000
}
fn foreground_is_game() -> bool {
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(GetForegroundWindow(), &mut pid);
    }
    pid == unsafe { GetCurrentProcessId() }
}
fn validate_pending_reference(runtime: &Runtime, state: &State, job: &Pending)
    -> Result<(), &'static str>
{
    let morph = is_larva_morph(&job.command, state.kind);
    let selection = if morph { runtime.morph_selection_ids(state.owner,state.shift) }
        else { selected_ids(runtime,state.owner,state.kind,state.shift) };
    if selection.as_deref() != Some(job.restore.as_slice()) {
        return Err("Selection changed before copying; group cleared");
    }
    if job.restore.iter().any(|&id| if morph {
        !runtime.valid_morph_restore(id,state.owner,state.shift)
    } else { !runtime.valid_id(id,state.owner,state.kind,state.shift) }) {
        return Err("Reference unit removed; group cleared");
    }
    Ok(())
}
fn pump() {
    if !INSTALLED.load(Ordering::Acquire)
        || !CALLBACKS_READY.load(Ordering::Acquire)
        || FAULT.load(Ordering::Acquire)
        || !session_allows_control()
    {
        return;
    }
    let _sender = SEND_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    if !INSTALLED.load(Ordering::Acquire)
        || !CALLBACKS_READY.load(Ordering::Acquire)
        || FAULT.load(Ordering::Acquire)
        || !session_allows_control()
    {
        return;
    }
    let Some(runtime) = RUNTIME.get() else {
        return;
    };
    runtime_alliance::maintain_button(runtime);
    runtime_alliance::bind(runtime);
    runtime_alliance::apply_requested(runtime);
    let Ok(mut state) = STATE.lock() else {
        FAULT.store(true, Ordering::Release);
        return;
    };
    reset_state_for_session(&mut state,SESSION_BOUNDARY_EPOCH.load(Ordering::Acquire),unsafe{GetAsyncKeyState(0xc0)}<0);
    // Heap control slots are discovered/bound only while the game's UI thread
    // is dispatching a periodic callback. Worker registration never touches them.
    let panel_error = ensure_panel_binding(runtime).err();
    let down = unsafe { GetAsyncKeyState(0xc0) } < 0;
    let rising = down && !state.last_key;
    state.last_key = down;
    let stop = STOP_EVENT.load(Ordering::Acquire);
    if stop != 0 && unsafe { WaitForSingleObject(stop as *mut c_void, 0) } == 0 {
        state.stop("Stopped by test app");
        runtime_alliance::cancel_pending_on_stop();
        return;
    }
    if !client_present(
        unsafe { GetTickCount64() },
        CLIENT_SEEN_TICK.load(Ordering::Acquire),
    ) {
        state.stop("Test app inactive; group cleared");
        return;
    }
    let Some((live, frame, owner)) = runtime.context() else {
        state.stop("Waiting for game state");
        return;
    };
    if !live || frame < state.frame {
        state.stop("Waiting for a live game");
        state.last_selection.clear();
        state.frame = frame;
        return;
    }
    state.frame = frame;
    if !foreground_is_game() {
        if state.active {
            state.stop("Game lost focus; group cleared");
        }
        return;
    }
    if unsafe { GetAsyncKeyState(0x1b) } < 0 {
        state.stop("Escape pressed; group cleared");
        return;
    }
    if rising {
        if state.active {
            state.stop("Backtick toggled off");
            return;
        }
        if let Err(reason) = activate_same_type(runtime, &mut state, owner, frame) {
            state.note(reason);
            return;
        }
    }
    if !state.active {
        return;
    }
    if owner != state.owner {
        state.stop("Local player changed; group cleared");
        return;
    }
    if active_selection_ids(runtime,&state).as_deref()
        != Some(state.last_selection.as_slice())
    {
        trace_control(runtime, &mut state, "pump-selection-rejected", None, None);
        state.stop("Manual selection changed; group cleared");
        return;
    }
    if frame.wrapping_sub(state.last_scan) >= 12 {
        state.last_scan = frame;
        match runtime.matching(state.owner, state.kind, state.shift) {
            Some(ids) if !ids.is_empty() => state.count = ids.len(),
            _ => {
                state.stop("Matching units unavailable");
                return;
            }
        }
    }
    if panel_error != state.panel_diagnostic {
        let detail = match panel_error.as_ref() {
            Some(error) => format!("Command panel capture unavailable; {error}"),
            None => "Command panel capture bound".into(),
        };
        let row = format!("{}\t{}\t{}\t{}\t{}",state.frame,state.count,state.kind,state.sent,detail);
        if state.events.len() >= 256 { state.events.pop_front(); }
        state.events.push_back(row);
        state.panel_diagnostic = panel_error;
    }
    update_visual_frame(runtime, &mut state);
    if runtime.paused.read() != Some(0) {
        return;
    }
    for _ in 0..8 {
        let Some(job) = state.pending.front() else {
            break;
        };
        if let Err(reason) = validate_pending_reference(runtime, &state, job) {
            trace_control(runtime, &mut state, "pump-reference-rejected", None, None);
            state.stop(reason); break;
        }
        if job.created.elapsed() > Duration::from_secs(5) {
            state.stop("Queued command expired; group cleared"); break;
        }
        let valid: Vec<u32> = job
            .ids
            .iter()
            .copied()
            .filter(|&id| runtime.valid_id(id, state.owner, state.kind, state.shift))
            .collect();
        if valid.is_empty() {
            state.pending.pop_front();
            continue;
        }
        let Some(capacity) = runtime
            .outgoing_capacity
            .read()
            .filter(|&n| (64..=512).contains(&n))
        else {
            state.stop("Outgoing capacity unavailable");
            break;
        };
        let budget = OUTGOING_BUDGET.min(capacity);
        let Ok(records) = crate::batch::plan_one(&valid, &job.restore, &job.command, budget) else {
            state.stop("Invalid batch");
            break;
        };
        let bytes: Vec<u8> = records.into_iter().flatten().collect();
        let Some(before) = runtime.outgoing.read() else {
            state.stop("Outgoing buffer state unavailable");
            break;
        };
        if before.checked_add(bytes.len()).is_none_or(|n| n > budget) {
            break;
        }
        if runtime.outgoing_capacity.read() != Some(capacity) {
            state.stop("Outgoing capacity changed; group cleared");
            break;
        }
        // The original sender appends this complete select/control/restore bundle in
        // one call. It never changes the client's UI selection or simulation memory.
        unsafe {
            forward_original(bytes.as_ptr(), bytes.len());
        }
        if runtime.outgoing.read() != Some(before + bytes.len()) {
            state.stop("Outgoing sender rejected batch; no more copies");
            break;
        }
        state.pending.pop_front();
        state.sent = state.sent.saturating_add(1);
        state.note("Batch appended to game outgoing buffer");
    }
}
fn update_visual_frame(runtime: &Runtime, state: &mut State) {
    if state.visual_tick.is_some_and(|t| t.elapsed() < Duration::from_millis(30)) { return; }
    state.visual_tick = Some(Instant::now());
    state.visual_frame = (|| {
        let config = runtime.visual.as_ref()?;
        let view = config.view()?;
        let appearance = config.appearance(state.kind)?;
        let ids = runtime.matching(state.owner, state.kind, state.shift)?;
        let layout = runtime.unit_layout()?;
        let mask_shift = if state.shift == 0 {11} else {state.shift};
        let mut markers = Vec::new();
        for id in ids {
            // The original native selection already has rings and health bars.
            if state.last_selection.contains(&id) { continue; }
            let index = id & ((1 << mask_shift) - 1);
            if index == 0 { return None; }
            let pointer = layout.base.checked_add((index as usize - 1) * UNIT_SIZE)?;
            let unit = read_unit(pointer, layout.base)?;
            if unit.owner != state.owner || unit.kind != state.kind || unit.uid(state.shift) != Some(id) { return None; }
            if let Some(marker) = crate::visuals::read_marker(pointer,id,state.owner,view,appearance) { markers.push(marker); }
        }
        if config.view()? != view || runtime.context()? != (true,state.frame,state.owner) { return None; }
        Some(crate::visuals::Frame {frame:state.frame,view,markers})
    })();
}
#[derive(Debug)]
struct BufferSnapshot {
    bytes: Vec<u8>,
    buffer: usize,
    capacity: usize,
}
fn buffer_snapshot(runtime: &Runtime) -> Option<BufferSnapshot> {
    let capacity = runtime.outgoing_capacity.read()?;
    let buffer = runtime.outgoing_buffer.read()?;
    let length = runtime.outgoing.read()?;
    if !(64..=512).contains(&capacity)
        || length > OUTGOING_BUDGET
        || length > capacity
        || buffer < 0x10000
        || buffer.checked_add(capacity).is_none()
    {
        return None;
    }
    let mut bytes = vec![0; length];
    if (!bytes.is_empty() && !read_memory(buffer, &mut bytes))
        || runtime.outgoing.read() != Some(length)
        || runtime.outgoing_capacity.read() != Some(capacity)
        || runtime.outgoing_buffer.read() != Some(buffer)
    {
        return None;
    }
    Some(BufferSnapshot {
        bytes,
        buffer,
        capacity,
    })
}
struct Capture {
    buffer: BufferSnapshot,
    ids: Vec<u32>,
    context: (bool, u32, u8),
    session: [usize; 3],
    owner: u8,
    kind: u16,
    shift: u8,
    epoch: u64,
}
fn session_identity(runtime: &Runtime) -> Option<[usize; 3]> {
    Some([
        runtime.replay.read()?,
        runtime.main_state.read()?,
        runtime.continuing.read()?,
    ])
}
fn coherent_session_context(runtime:&Runtime)->Option<crate::game_session::Context> {
    let identity=session_identity(runtime)?;
    let before=runtime.context()?;
    let after=runtime.context()?;
    if identity!=session_identity(runtime)? || before.0!=after.0 || before.2!=after.2
        ||after.1<before.1 {return None;}
    Some(crate::game_session::Context{active:after.0,frame:after.1,owner:after.2,identity})
}
fn sync_session_change(tracker:&crate::game_session::Tracker,change:crate::game_session::Change) {
    if change.invalidated {
        // Pending unit captures expire even when an original is still running.
        BINDING_EPOCH.fetch_add(1,Ordering::AcqRel);
        SESSION_BOUNDARY_EPOCH.fetch_add(1,Ordering::AcqRel);
    }
    GAME_SESSION_GENERATION.store(change.generation,Ordering::Release);
    EVENT_THREAD.store(tracker.ui_thread(),Ordering::Release);
}
fn observe_game_session() {
    let Some(runtime)=RUNTIME.get()else{return;};
    let mut tracker=GAME_SESSION.lock().unwrap_or_else(|p|p.into_inner());
    // A context read before taking this mutex could look like a frame rollback
    // if a callback had already observed a newer frame.
    let change=tracker.observe(coherent_session_context(runtime));
    sync_session_change(&tracker,change);
}
struct SessionCallback(crate::game_session::Decision);
impl SessionCallback {
    fn enter()->Self {
        let mut tracker=GAME_SESSION.lock().unwrap_or_else(|p|p.into_inner());
        let context=RUNTIME.get().and_then(coherent_session_context);
        let entry=tracker.enter(context,unsafe{GetCurrentThreadId()});
        sync_session_change(&tracker,entry.change);
        Self(entry.decision)
    }
    fn active(&self)->bool {self.0==crate::game_session::Decision::Active}
    fn wrong_thread(&self)->bool {self.0==crate::game_session::Decision::WrongThread}
}
impl Drop for SessionCallback {
    fn drop(&mut self) {
        let mut tracker=GAME_SESSION.lock().unwrap_or_else(|p|p.into_inner());
        let change=tracker.leave();sync_session_change(&tracker,change);
    }
}
fn session_allows_control()->bool {
    let Some(runtime)=RUNTIME.get()else{return false;};
    let mut tracker=GAME_SESSION.lock().unwrap_or_else(|p|p.into_inner());
    let context=coherent_session_context(runtime);
    let change=tracker.observe(context);sync_session_change(&tracker,change);
    context.is_some_and(|c|c.active)&&tracker.control_allowed(unsafe{GetCurrentThreadId()})
}
fn session_is_current(session:u64)->bool {
    session!=0 && session==GAME_SESSION_GENERATION.load(Ordering::Acquire)
}
fn reset_state_for_session(state:&mut State,epoch:u64,key_down:bool) {
    if state.session_boundary==epoch{return;}
    state.stop("Game session changed; select again and press backtick");
    state.last_selection.clear();state.last_key=key_down;state.frame=0;state.last_scan=0;
    state.panel_diagnostic=None;state.control_traces=0;state.session_boundary=epoch;
}
fn start_capture() -> Option<Capture> {
    if !INSTALLED.load(Ordering::Acquire)
        || !CALLBACKS_READY.load(Ordering::Acquire)
        || FAULT.load(Ordering::Acquire)
        || IN_ORIGINAL.with(Cell::get)
        || !session_allows_control()
    {
        return None;
    }
    let runtime = RUNTIME.get()?;
    let mut state = STATE.lock().unwrap_or_else(|p| p.into_inner());
    reset_state_for_session(&mut state,SESSION_BOUNDARY_EPOCH.load(Ordering::Acquire),unsafe{GetAsyncKeyState(0xc0)}<0);
    if !state.active {return None;}
    if morph_copies_pending(&state)
        && selected_ids(runtime,state.owner,state.kind,state.shift).as_deref()!=Some(state.last_selection.as_slice())
        && runtime.morph_selection_ids(state.owner,state.shift).as_deref()==Some(state.last_selection.as_slice()) {
        return None;
    }
    let capture = (|| {
        if !foreground_is_game()
            || !client_present(
                unsafe { GetTickCount64() },
                CLIENT_SEEN_TICK.load(Ordering::Acquire),
            )
        {
            return None;
        }
        let context = runtime.context()?;
        if !context.0
            || context.2 != state.owner
            || context.1 < state.frame
            || runtime.paused.read()? != 0
        {
            return None;
        }
        let ids = selected_ids(runtime, state.owner, state.kind, state.shift)?;
        if ids != state.last_selection {
            return None;
        }
        let buffer = buffer_snapshot(runtime)?;
        // Conservative overflow exclusion for a single candidate UI order. This
        // does not establish a flush epoch or exclude equal-prefix ABA: prototype.
        if buffer.bytes.len().checked_add(64)? > buffer.capacity {
            return None;
        }
        Some(Capture {
            buffer,
            ids,
            context,
            session: session_identity(runtime)?,
            owner: state.owner,
            kind: state.kind,
            shift: state.shift,
            epoch: BINDING_EPOCH.load(Ordering::Acquire),
        })
    })();
    if capture.is_none() {
        trace_control(runtime, &mut state, "capture-rejected", None, None);
        state.stop("Callback snapshot guard failed; group cleared");
    }
    capture
}
fn finish_capture(capture: Capture) {
    if !CALLBACKS_READY.load(Ordering::Acquire)
        || FAULT.load(Ordering::Acquire)
        || IN_ORIGINAL.with(Cell::get)
    { return; }
    let Some(runtime) = RUNTIME.get() else { return; };
    let _sender = SEND_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut state = STATE.lock().unwrap_or_else(|p| p.into_inner());
    complete_captured_control(runtime, &mut state, capture, CALLBACK_NESTED.with(Cell::get),
        BINDING_EPOCH.load(Ordering::Acquire));
}
// Extracted so the actual callback-completion lifecycle can be exercised with
// owned fixture buffers, including post-original Larva -> Egg transitions.
fn complete_captured_control(runtime: &Runtime, state: &mut State, capture: Capture,
    nested: bool, epoch: u64)
{
    if !state.active { return; }
    let after = buffer_snapshot(runtime);
    if nested || capture.epoch != epoch
        || runtime.context() != Some(capture.context)
        || session_identity(runtime) != Some(capture.session)
        || runtime.paused.read() != Some(0)
        || (state.owner, state.kind, state.shift) != (capture.owner, capture.kind, capture.shift)
        || state.last_selection != capture.ids
        || after.as_ref().is_none_or(|a| {
            a.buffer != capture.buffer.buffer || a.capacity != capture.buffer.capacity
        })
    {
        trace_control(runtime, state, "finish-context-rejected", None, None);
        state.stop("Callback context changed; group cleared"); return;
    }
    let after = after.unwrap();
    let appended = after.bytes.get(capture.buffer.bytes.len()..);
    let observation = crate::event_capture::observe_control_append(
        &capture.buffer.bytes, &after.bytes, OUTGOING_BUDGET,
        (capture.kind == 35).then_some(capture.ids.as_slice()));
    if !matches!(observation, crate::event_capture::Observation::NoObservedAppend) {
        trace_control(runtime, state, "finish-output", appended.map(|s| s.len()),
            appended.and_then(|s| s.first().copied()));
    }
    match observation {
        crate::event_capture::Observation::Command(command) if callback_control_supported(&command) => {
            // The Morph exception is available only after an exact causal
            // command append was observed. Ordinary clicks keep strict guards.
            if !observed_selection_matches(runtime, state, &capture.ids, &command) {
                trace_control(runtime, state, "finish-selection-rejected", Some(command.len()),
                    command.first().copied());
                state.stop("Selection changed during control; group cleared"); return;
            }
            queue_observed_control(&command, &capture.ids, state, runtime,
                is_larva_morph(&command, capture.kind));
        }
        crate::event_capture::Observation::NoObservedAppend => {
            if selected_ids(runtime, capture.owner, capture.kind, capture.shift).as_deref()
                != Some(capture.ids.as_slice()) {
                trace_control(runtime, state, "finish-no-command-selection-changed", Some(0), None);
                state.stop("Selection changed without a control; group cleared");
            }
        }
        crate::event_capture::Observation::Command(_)
        | crate::event_capture::Observation::Clear(_)
        | crate::event_capture::Observation::Ambiguous(_) => {
            state.stop("Ambiguous or unsupported callback output; group cleared");
        }
    }
}
fn callback_control_supported(command: &[u8]) -> bool {
    crate::building_commands::classify(command).is_some()
}
fn ensure_panel_binding(runtime: &Runtime) -> Result<(),String> {
    if !session_allows_control(){return Err("Waiting for current game UI thread".into());}
    let gui = runtime.gui.as_ref().ok_or("Command panel root unresolved")?;
    let first = gui.first_dialog.read().ok_or("Command panel list unavailable")?;
    let replacement = panel_callback as *const () as usize;
    let find = || crate::gui_capture::discover_stat_button(first,replacement,
        |p| p >= gui.code_start && p.checked_add(16).is_some_and(|end| end <= gui.code_end),read_memory);
    let target = find().map_err(|e|e.to_string())?.ok_or("StatBtn command panel unavailable")?;
    if target.callback == replacement {
        return (PANEL_ORIGINAL.load(Ordering::Acquire) != 0).then_some(()).ok_or("Command panel original missing".into());
    }
    // Every installed wrapper forwards the same verified original. A different
    // callback provider is never overwritten, even when a new root is allocated.
    let original = PANEL_ORIGINAL.load(Ordering::Acquire);
    if original != 0 && original != target.callback { return Err("Command panel callback changed".into()); }
    if gui.first_dialog.read() != Some(first) || find().map_err(|e|e.to_string())? != Some(target) {
        return Err("Command panel roots changed before binding".into());
    }
    crate::gui_capture::recheck_target(target,read_memory).map_err(|e|e.to_string())?;
    PANEL_ORIGINAL.compare_exchange(0,target.callback,Ordering::AcqRel,Ordering::Acquire)
        .or_else(|value| if value == target.callback {Ok(value)} else {Err(value)})
        .map_err(|_|"Command panel original changed")?;
    let slot = crate::callback_binding::Slot{address:target.slot_address,original:target.callback,replacement};
    // During this UI-thread callback, the engine is not destroying its roots.
    // Exact CAS plus ordinary writable-region validation; no code/protection write.
    unsafe { crate::callback_binding::install(&[slot]) }.map_err(|e|e.to_string())
}
// Extended button activation is a native control event too. Init, paint, hover,
// show and hide notifications must not start snapshots or revoke a group.
// SCR ControlEvent has ty at +0x18 and a pointer-sized ext_type at +0;
// pinned samase_scarf dialog analysis identifies ext_type 2 as activation.
fn observe_panel_call<C>(kind: Option<usize>, extended: Option<usize>, outer: bool,
    begin: impl FnOnce() -> Option<C>, original: impl FnOnce() -> u32,
    finish: impl FnOnce(Option<C>)) -> u32
{
    let admitted = outer && (matches!(kind, Some(0 | 2 | 4 | 5 | 7 | 8 | 0xf))
        || (kind == Some(0xe) && extended == Some(2)));
    let capture = if admitted { begin() } else { None };
    let result = original();
    finish(capture);
    result
}
unsafe extern "C" fn panel_callback(control: *const c_void, event: *const c_void) -> u32 {
    let session=SessionCallback::enter();
    let address = PANEL_ORIGINAL.load(Ordering::Acquire);
    if address == 0 { FAULT.store(true,Ordering::Release); return 0; }
    let original: PanelCallback = unsafe {std::mem::transmute(address)};
    let previous_depth = CALLBACK_DEPTH.with(|depth| {let old=depth.get();depth.set(old.saturating_add(1));old});
    let _depth = CallbackDepth(previous_depth);
    if previous_depth != 0 || IN_ORIGINAL.with(Cell::get) {
        let kind = (event as usize).checked_add(0x18).and_then(|p|read_integer(p,2));
        // Internal extended widget notifications are synchronous descendants of
        // the outer native input. They forward only; the outer snapshot covers
        // the complete tree. A recursively dispatched new input invalidates it.
        if kind != Some(0xe) { CALLBACK_NESTED.with(|flag|flag.set(true)); }
        return observe_panel_call(kind, None, false, || None::<Capture>,
            || unsafe {original(control,event)}, |_| {});
    }
    CALLBACK_NESTED.with(|flag|flag.set(false));
    if !session.active() {
        if session.wrong_thread(){FAULT.store(true,Ordering::Release);}
        return unsafe {original(control,event)};
    }
    let refreshed = std::panic::catch_unwind(refresh_bindings).map(|_|true)
        .unwrap_or_else(|_|{FAULT.store(true,Ordering::Release);false});
    let kind = (event as usize).checked_add(0x18).and_then(|p|read_integer(p,2));
    let extended = if kind == Some(0xe) { read_integer(event as usize,8) } else { None };
    observe_panel_call(kind, extended, refreshed, || std::panic::catch_unwind(|| {
        let target = crate::gui_capture::Target{control:control as usize,
            slot_address:(control as usize).checked_add(crate::gui_capture::CALLBACK_OFFSET).unwrap_or(0),
            callback:panel_callback as *const () as usize};
        if crate::gui_capture::recheck_target(target,read_memory).is_err() { return None; }
        let capture = start_capture();
        if capture.is_some() && kind == Some(0xe) {
            if let Some(runtime) = RUNTIME.get() {
                let mut state = STATE.lock().unwrap_or_else(|p|p.into_inner());
                trace_control(runtime, &mut state, "panel-activation-captured", None, None);
            }
        }
        capture
    }).unwrap_or_else(|_|{FAULT.store(true,Ordering::Release);None}),
    // Preserve both native arguments and consumed return; invoke exactly once.
    || unsafe {original(control,event)}, |before| {
        if std::panic::catch_unwind(|| {
            if let Some(capture)=before {
                finish_capture(capture);
                refresh_bindings();
                pump();
            } else {refresh_bindings();}
        }).is_err() {FAULT.store(true,Ordering::Release);}
    })
}
fn set_init_phase(phase: InitPhase, message: String) {
    let mut current = INIT_MESSAGE.lock().unwrap_or_else(|p| p.into_inner());
    if *current != message {
        *current = message;
    }
    INIT_PHASE.store(phase as u32, Ordering::Release);
}
fn initialization_message() -> String {
    let current = INIT_MESSAGE.lock().unwrap_or_else(|p| p.into_inner());
    if current.is_empty() {
        "Analyzing loaded game image".into()
    } else {
        current.clone()
    }
}
fn set_init_diagnostic(message: String) {
    let mut current = INIT_DIAGNOSTIC.lock().unwrap_or_else(|p| p.into_inner());
    if *current != message {
        *current = message;
    }
}
struct RegistrationPoll {
    pending: bool,
    context: (bool, u32, u8),
    detail: String,
}
fn can_preserve_ui_reset(runtime: &Runtime, state: &State, context: (bool,u32,u8)) -> bool {
    state.active && context.0 && context.2 == state.owner && context.1 >= state.frame
        && active_selection_ids(runtime,state).as_deref() == Some(state.last_selection.as_slice())
}
fn poll_registration() -> Result<RegistrationPoll, String> {
    let slots = BINDINGS.get().ok_or("Callback bindings unavailable")?;
    // Constructor, worker and callback operations share the same lock order.
    // No game callback is invoked while any of these locks is held.
    let _sender = SEND_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let _registration = REGISTRATION_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    observe_game_session();
    {
        let mut state=STATE.lock().unwrap_or_else(|p|p.into_inner());
        reset_state_for_session(&mut state,SESSION_BOUNDARY_EPOCH.load(Ordering::Acquire),unsafe{GetAsyncKeyState(0xc0)}<0);
    }
    let mut observed = [crate::callback_startup::ObservedSlot {
        slot: slots[0],
        actual: 0,
    }; 4];
    for (index, &slot) in slots.iter().enumerate() {
        let actual =
            match unsafe { crate::callback_binding::check(slot) }.map_err(|e| e.to_string())? {
                crate::callback_binding::SlotState::Original => slot.original,
                crate::callback_binding::SlotState::Installed => slot.replacement,
                crate::callback_binding::SlotState::Other(value) => value,
            };
        observed[index] = crate::callback_startup::ObservedSlot { slot, actual };
    }
    let runtime = RUNTIME.get().ok_or("Callback runtime unavailable")?;
    let context = runtime.context().unwrap_or((false, 0, 255));
    let base = unsafe { GetModuleHandleW(null()) } as usize;
    let table_rva = slots[0]
        .address
        .checked_sub(base)
        .ok_or("Callback table below image base")?;
    let values = observed
        .iter()
        .map(|item| format!("0x{:x}", item.actual))
        .collect::<Vec<_>>()
        .join(",");
    let detail = format!(
        "table_rva=0x{table_rva:x}; frame={}; live={}; owner={}; main_state={}; continuing={}; replay={}; lobby_color_gate={}; slots[0,4,7,13]=[{values}]",
        context.1,
        u8::from(context.0),
        context.2,
        runtime.main_state.read().map_or(-1, |n| n as i64),
        runtime.continuing.read().map_or(-1, |n| n as i64),
        runtime.replay.read().map_or(-1, |n| n as i64),
        runtime.lobby_color_gate.as_ref().and_then(Value::read).map_or(-1, |n| n as i64)
    );
    let outcome = crate::callback_startup::attempt(&observed, || unsafe {
        crate::callback_binding::maintain_report(slots)
    });
    match outcome {
        Ok(crate::callback_startup::Attempt::Ready { changed }) => {
            let was_ready = CALLBACKS_READY.load(Ordering::Acquire);
            if !changed.is_empty() || !was_ready {
                let mut state = STATE.lock().unwrap_or_else(|p| p.into_inner());
                let preserve = was_ready && can_preserve_ui_reset(runtime,&state,context);
                if !preserve {
                    BINDING_EPOCH.fetch_add(1, Ordering::AcqRel);
                    state.stop("Callback table ready again; selection cleared");
                    state.last_key = unsafe {GetAsyncKeyState(0xc0)} < 0;
                }
            }
            CALLBACKS_READY.store(true, Ordering::Release);
            set_init_phase(
                InitPhase::Ready,
                "Game session connection ready; production button fix 20261007; live validation pending"
                    .into(),
            );
            set_init_diagnostic(format!("READY; changed={}; {detail}", changed.len()));
            Ok(RegistrationPoll {
                pending: false,
                context,
                detail,
            })
        }
        Ok(waiting) => {
            if CALLBACKS_READY.swap(false, Ordering::AcqRel) {
                BINDING_EPOCH.fetch_add(1, Ordering::AcqRel);
            }
            let reason = match waiting {
                crate::callback_startup::Attempt::Waiting {
                    initialized,
                    pending_mask,
                } => format!("initialized={initialized}/4; pending_mask=0x{pending_mask:x}"),
                crate::callback_startup::Attempt::WaitingRace { address } => {
                    format!("zero_cas_race=0x{address:x}")
                }
                _ => unreachable!(),
            };
            let message = format!("Waiting for in-game UI callbacks... {reason}; {detail}");
            set_init_phase(InitPhase::WaitingForCallbacks, message.clone());
            set_init_diagnostic(format!("WAITING; {reason}; {detail}"));
            let mut state = STATE.lock().unwrap_or_else(|p| p.into_inner());
            state.stop("Waiting for in-game UI callbacks; selection cleared");
            state.last_key = unsafe { GetAsyncKeyState(0xc0) } < 0;
            Ok(RegistrationPoll {
                pending: true,
                context,
                detail,
            })
        }
        Err(error) => {
            CALLBACKS_READY.store(false, Ordering::Release);
            FAULT.store(true, Ordering::Release);
            BINDING_EPOCH.fetch_add(1, Ordering::AcqRel);
            STATE
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .stop("Callback table changed; copying disabled");
            let message = format!("{error}; {detail}");
            set_init_phase(InitPhase::Failed, message.clone());
            set_init_diagnostic(format!("FAILED; {message}"));
            Err(message)
        }
    }
}
fn registration_failed(message: String) {
    let _sender = SEND_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    CALLBACKS_READY.store(false, Ordering::Release);
    FAULT.store(true, Ordering::Release);
    BINDING_EPOCH.fetch_add(1, Ordering::AcqRel);
    STATE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .stop("Callback connection failed; copying disabled");
    set_init_phase(InitPhase::Failed, message.clone());
    set_init_diagnostic(format!("FAILED; {message}"));
}
fn refresh_bindings() {
    // Partial constructor publications only forward originals. The constructor
    // alone retries startup; after first installation the worker keeps polling.
    if !INSTALLED.load(Ordering::Acquire) || FAULT.load(Ordering::Acquire) {
        return;
    }
    if let Err(message) = poll_registration() {
        registration_failed(message);
    }
}
struct CallbackDepth(u32);
impl Drop for CallbackDepth {
    fn drop(&mut self) {
        CALLBACK_DEPTH.with(|depth| depth.set(self.0));
    }
}
unsafe fn callback_dispatch(index: usize, event: *const c_void) {
    let session=SessionCallback::enter();
    runtime_alliance::note_callback(None);
    let address = ORIGINAL_CALLBACKS[index].load(Ordering::Acquire);
    if address == 0 {
        FAULT.store(true, Ordering::Release);
        return;
    }
    let original: UiCallback = unsafe { std::mem::transmute(address) };
    let previous_depth = CALLBACK_DEPTH.with(|depth| {
        let old = depth.get();
        depth.set(old.saturating_add(1));
        old
    });
    let _depth = CallbackDepth(previous_depth);
    if previous_depth != 0 || IN_ORIGINAL.with(Cell::get) {
        CALLBACK_NESTED.with(|flag| flag.set(true));
        unsafe {
            original(event);
        }
        return;
    }
    CALLBACK_NESTED.with(|flag| flag.set(false));
    if !session.active() {
        if session.wrong_thread(){FAULT.store(true,Ordering::Release);}
        unsafe {original(event);}
        // Forward menu/transition inputs normally; the worker continues to
        // reinstall only verified originals when the next table is ready.
        return;
    }
    let before = std::panic::catch_unwind(|| {
        refresh_bindings();
        if index != 3 { start_capture() } else { None }
    })
    .unwrap_or_else(|_| {
        FAULT.store(true, Ordering::Release);
        None
    });
    let alliance_output=std::panic::catch_unwind(||runtime_alliance::begin_output(None)).ok().flatten();
    // The normal original callback is always invoked exactly once. No private
    // mutex is held here; nested callbacks are forwarded and invalidate capture.
    unsafe {
        original(event);
    }
    if std::panic::catch_unwind(|| {
        if let Some(capture)=alliance_output {runtime_alliance::finish_output(capture);}
        runtime_alliance::end_output_callback();
        if let Some(capture) = before {
            finish_capture(capture);
        }
        refresh_bindings();
        let immediate_morph = index != 3 && STATE.lock().unwrap_or_else(|p| p.into_inner())
            .pending.iter().any(|job| job.command.first() == Some(&0x23));
        if (index == 3 || immediate_morph) && !IN_ORIGINAL.with(Cell::get) {
            pump();
        }
    })
    .is_err()
    {
        FAULT.store(true, Ordering::Release);
    }
}
unsafe extern "C" fn key_callback(event: *const c_void) {
    unsafe {
        callback_dispatch(0, event);
    }
}
unsafe extern "C" fn left_callback(event: *const c_void) {
    unsafe {
        callback_dispatch(1, event);
    }
}
unsafe extern "C" fn right_callback(event: *const c_void) {
    unsafe {
        callback_dispatch(2, event);
    }
}
unsafe extern "C" fn periodic_callback(event: *const c_void) {
    unsafe {
        callback_dispatch(3, event);
    }
}
fn analyze(binary: &BinaryFile<VirtualAddress64>, base: usize) -> Result<Resolved, String> {
    let ctx = OperandContext::new();
    let mut a = Analysis::<ExecutionStateX86_64<'_>>::new(binary, &ctx);
    let end = binary
        .sections()
        .map(|s| s.virtual_address.0 + s.virtual_size as u64)
        .max()
        .ok_or("Empty image")?;
    let value = |op: Option<Operand<'_>>, name: &str| {
        Value::from_operand(
            op.ok_or_else(|| format!("{name} unresolved"))?,
            base as u64,
            end,
            base,
            0,
        )
    };
    // The fallback proves an append body and supplies both buffer and length.
    // Dynamic capacity is separately resolved from its overflow comparison.
    let sender = crate::resolve_sender::resolve(binary, &ctx, &mut a)?;
    if a.send_command()
        .is_some_and(|known| known != sender.address)
    {
        return Err("Command sender analyses disagree".into());
    }
    let capacity = crate::resolve_sender::resolve_capacity(binary, &ctx, sender)?;
    let send = sender.address.0 as usize;
    let table = value(a.global_event_handlers(), "global_event_handlers")?
        .read()
        .ok_or("Global callback table unavailable")?;
    let callbacks = [
        a.ui_default_key_down_handler()
            .ok_or("Default key handler unresolved")?
            .0 as usize,
        a.ui_default_left_down_handler()
            .ok_or("Default left handler unresolved")?
            .0 as usize,
        a.ui_default_right_down_handler()
            .ok_or("Default right handler unresolved")?
            .0 as usize,
        a.ui_default_periodic_handler()
            .ok_or("Default periodic handler unresolved")?
            .0 as usize,
    ];
    let mut unit_vectors = Vec::new();
    let mut seen_vectors = Vec::new();
    for arrays in &a.limits().arrays {
        for &(op, _, _) in arrays {
            if !seen_vectors.contains(&op) {
                seen_vectors.push(op);
                if let Ok(candidate) = value(Some(op), "unit vector") {
                    unit_vectors.push(candidate);
                }
            }
        }
    }
    let metadata = a.dat(samase_scarf::DatType::Units).and_then(|dat| {
        Some(UnitMetadataSource {table:value(Some(dat.address), "unit command metadata").ok()?,stride:dat.entry_size as usize})
    });
    // Visual discovery is optional: inability to project graphics must not
    // disable the already working command path.
    let visual = (|| -> Result<crate::visuals::Config,String> {
        let dat = a.dat(samase_scarf::DatType::Units).ok_or("Units data unavailable")?;
        Ok(crate::visuals::Config {
            screen_x: value(a.screen_x(), "screen_x")?,
            screen_y: value(a.screen_y(), "screen_y")?,
            zoom: value(a.zoom(), "zoom")?,
            width: value(a.game_screen_width_bwpx(), "game_screen_width")?,
            height: value(a.game_screen_height_bwpx(), "game_screen_height")?,
            units_dat: value(Some(dat.address), "units_dat")?,
            dat_stride: dat.entry_size as usize,
        })
    })().ok();
    let gui = value(a.first_dialog(), "first_dialog").ok().and_then(|first_dialog| {
        let code = binary.sections().find(|s|s.name == *b".text\0\0\0")?;
        Some(GuiRuntime{first_dialog,code_start:code.virtual_address.0 as usize,
            code_end:(code.virtual_address.0 as usize).checked_add(code.data.len())?})
    });
    let runtime = Runtime {
        alliance: (|| -> Result<runtime_alliance::Config,String> { Ok(runtime_alliance::Config {
            game:value(a.game(),"alliance game")?, players:value(a.players(),"alliance players")?,
            // Unresolved write policy must not erase a readable player roster.
            policy:(|| -> Result<runtime_alliance::Policy,String> {Ok(runtime_alliance::Policy {
                game_data:value(a.game_data(),"alliance game data")?,
                matcher_count:value(a.matchmaker_session_count(),"alliance matcher count")?,
                matcher_string:value(a.matchmaker_string(),"alliance matcher string")?,
            })})().ok(),
        }) })().ok(),
        unit_vectors,
        visual,
        gui,
        metadata,
        units: value(a.units(), "units")?,
        first: value(a.first_active_unit(), "first_active_unit")?,
        // client_selection() returns the array address via mem_sub_const_op;
        // its outer expression may be a constant, dynamic base load or sum.
        // Evaluate that address as returned. Do not remove any memory load.
        selection: value(a.client_selection(), "client_selection")?,
        local: value(a.local_player_id(), "local_player_id")?,
        replay: value(a.is_replay(), "is_replay")?,
        main_state: value(a.scmain_state(), "scmain_state")?,
        lobby_color_gate: value(a.in_lobby_or_game(), "lobby color command gate").ok(),
        continuing: value(a.continue_game_loop(), "continue_game_loop")?,
        paused: value(a.is_paused(), "is_paused")?,
        frame: value(a.game_frame_count(), "frame")?,
        outgoing: value(Some(sender.length), "outgoing_command_length")?,
        outgoing_buffer: value(Some(sender.buffer), "outgoing_command_buffer")?,
        outgoing_capacity: value(Some(capacity), "outgoing_command_capacity")?,
    };
    let code = binary
        .sections()
        .find(|s| s.name == *b".text\0\0\0")
        .ok_or("Code missing")?;
    let bytes = |address: usize| -> Result<[u8; 16], String> {
        if address < code.virtual_address.0 as usize
            || address
                .checked_add(16)
                .is_none_or(|n| n > code.virtual_address.0 as usize + code.data.len())
        {
            return Err("Callback or sender outside code".into());
        }
        binary
            .slice_from_address(VirtualAddress64(address as u64), 16)
            .map_err(|_| "Unreadable callback or sender".into())
            .map(|x| x.try_into().unwrap())
    };
    for address in callbacks {
        bytes(address)?;
    }
    Ok(Resolved {
        runtime,
        send,
        table,
        callbacks,
        send_bytes: bytes(send)?,
    })
}
fn initialize() -> Result<(), String> {
    let path = module_path(null_mut()).ok_or("No executable path")?;
    if !path
        .file_name()
        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("StarCraft.exe"))
    {
        return Err("Expected StarCraft.exe".into());
    }
    let base = unsafe { GetModuleHandleW(null()) } as usize;
    let image = snapshot_image(base, read_memory)?;
    let resolved = analyze(&image, base)?;
    let mut now = [0; 16];
    if !read_memory(resolved.send, &mut now) || now != resolved.send_bytes {
        return Err("Game sender changed during analysis".into());
    }
    let replacements = [
        key_callback as *const () as usize,
        left_callback as *const () as usize,
        right_callback as *const () as usize,
        periodic_callback as *const () as usize,
    ];
    let indices = [
        crate::callback_binding::KEY_DOWN_INDEX,
        crate::callback_binding::LEFT_DOWN_INDEX,
        crate::callback_binding::RIGHT_DOWN_INDEX,
        crate::callback_binding::PERIODIC_INDEX,
    ];
    let mut slots = [crate::callback_binding::Slot {
        address: 0,
        original: 0,
        replacement: 0,
    }; 4];
    for index in 0..4 {
        slots[index] = crate::callback_binding::Slot {
            address: resolved
                .table
                .checked_add(indices[index] * 8)
                .ok_or("Callback slot overflow")?,
            original: resolved.callbacks[index],
            replacement: replacements[index],
        };
        crate::callback_binding::validate(slots[index]).map_err(|e| e.to_string())?;
    }
    // A callback can run as soon as its pointer is published. Every original
    // and all immutable runtime state must be ready before the first slot CAS.
    ORIGINAL_SEND.store(resolved.send, Ordering::Release);
    for (slot, &callback) in ORIGINAL_CALLBACKS.iter().zip(&resolved.callbacks) {
        slot.store(callback, Ordering::Release);
    }
    RUNTIME
        .set(resolved.runtime)
        .map_err(|_| "Already initialized")?;
    BINDINGS
        .set(slots)
        .map_err(|_| "Callback bindings already initialized")?;
    let mut wait = crate::callback_startup::WaitPolicy::default();
    loop {
        // Immutable analysis and OnceLocks above are set once. This loop only
        // observes all four initialized entries and performs exact original CAS.
        let observation = poll_registration()?;
        if !observation.pending {
            // READY is justified only by the complete successful CAS operation.
            INSTALLED.store(true, Ordering::Release);
            STATE.lock().unwrap_or_else(|p| p.into_inner()).note(
                "Unit control ready; production fix 20261004; live validation pending",
            );
            return Ok(());
        }
        if wait.expired(
            unsafe { GetTickCount64() },
            true,
            observation.context.0,
            observation.context.1,
            observation.context.2,
        ) {
            let message = format!(
                "Game UI callbacks stayed empty for 10 seconds; {}",
                observation.detail
            );
            registration_failed(message.clone());
            return Err(message);
        }
        // Menu/lobby waiting has no deadline and never writes null table slots.
        std::thread::sleep(Duration::from_millis(100));
    }
}
const NO_DISPLAY_UNIT: u16 = u16::MAX;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DisplaySelection {
    // Internal provenance: only the validated virtual-group branch sets this.
    control_active: bool,
    in_game: bool,
    count: usize,
    kind: u16,
}
impl DisplaySelection {
    fn empty(in_game: bool) -> Self { Self { control_active: false, in_game, count: 0, kind: NO_DISPLAY_UNIT } }
}
fn hud_selection(runtime: Option<&Runtime>, state: &State, ready: bool) -> DisplaySelection {
    if !ready { return DisplaySelection::empty(false); }
    let Some(runtime) = runtime else { return DisplaySelection::empty(false); };
    let Some((in_game, frame, owner)) = runtime.context() else {
        return DisplaySelection::empty(false);
    };
    if !in_game { return DisplaySelection::empty(false); }
    let display = if state.active && state.count > 0 && state.count <= MAX_UNITS
        && state.kind < 228 && state.owner == owner && frame >= state.frame
        && (!morph_copies_pending(state)
            || selected_ids(runtime, state.owner, state.kind, state.shift).as_deref()
                == Some(state.last_selection.as_slice()))
        && active_selection_ids(runtime, state).as_deref() == Some(state.last_selection.as_slice())
    {
        DisplaySelection { control_active: true, in_game: true, count: state.count, kind: state.kind }
    } else {
        runtime.display_selection().unwrap_or_else(|| DisplaySelection::empty(true))
    };
    // Frame progression is allowed; a match/owner change invalidates the read.
    match runtime.context() {
        Some((true, after_frame, after_owner)) if after_owner == owner && after_frame >= frame => display,
        _ => DisplaySelection::empty(false),
    }
}
fn encode_status(pid: u32, state: &State, status: &str, error: Option<&str>,
    display: DisplaySelection) -> String
{
    // Publish active controls only when the same HUD read confirmed virtual
    // group provenance. Ordinary selection with coincidentally equal count and
    // type must remain inactive; control State itself is not changed here.
    let active = state.active && status == "READY" && display.control_active
        && display.in_game && display.count > 0 && display.count <= MAX_UNITS
        && display.count == state.count && display.kind == state.kind && display.kind < 228;
    let message = error.unwrap_or(state.message).chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ').take(400).collect::<String>();
    // SCMULTI2 retains the seven legacy values and appends passive HUD values.
    // NO_DISPLAY_UNIT with count0 is empty; with a positive count it is mixed.
    format!("SCMULTI2\t{pid}\t{}\t{}\t{}\t{}\t{status}\t{message}\t{}\t{}\t{}\n",
        u8::from(active), if active { state.count } else { 0 }, state.kind, state.sent,
        u8::from(display.in_game), display.count, display.kind)
}
fn publish(path: &Path, pid: u32, status: &str, error: Option<&str>) -> std::io::Result<()> {
    let state = STATE.lock().unwrap_or_else(|p| p.into_inner());
    let display = hud_selection(RUNTIME.get(), &state, status == "READY");
    let active = state.active && status == "READY" && display.control_active;
    let text = encode_status(pid, &state, status, error, display);
    let visual = if active && state.visual_tick.is_some_and(|t| t.elapsed() < Duration::from_millis(300)) {
        state.visual_frame.as_ref().map(|f| f.encode(pid))
    } else { None };
    let events: Vec<String> = state.events.iter().cloned().collect();
    drop(state);
    atomic_write_text(path,&text)?;
    let visual_path = path.with_file_name(format!("mc-{pid}-visuals.tsv"));
    if let Some(visual) = visual {
        atomic_write_text(&visual_path, &visual)?;
    } else {
        // A fresh empty marker frame clears graphics immediately on deactivation.
        atomic_write_text(&visual_path, &format!("SCVIS1\t{pid}\t0\t1\t640\t383\t0\n"))?;
    }
    if !events.is_empty() {
        use std::io::Write as _;
        let log = path.with_file_name(format!("mc-{pid}-actions.tsv"));
        let mut file = fs::OpenOptions::new().append(true).create(true).open(log)?;
        for line in &events {
            writeln!(file, "{line}")?;
        }
        let mut state = STATE.lock().unwrap_or_else(|p|p.into_inner());
        if state.events.iter().take(events.len()).eq(events.iter()) { state.events.drain(..events.len()); }
    }
    Ok(())
}
// winapi 0.3.9 incorrectly omits this API's BOOL return value. Declare the
// documented ABI locally so failure cannot be mistaken for successful replace.
#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "ReplaceFileW"]
    fn replace_file_w(replaced: *const u16, replacement: *const u16, backup: *const u16,
        flags: u32, exclude: *mut c_void, reserved: *mut c_void) -> i32;
}
fn atomic_write_text(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp,text)?;
    let a:Vec<u16> = tmp.as_os_str().encode_wide().chain(Some(0)).collect();
    let b:Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let replaced = unsafe { replace_file_w(b.as_ptr(),a.as_ptr(),null(),0,null_mut(),null_mut()) };
    if replaced == 0 {
        let error = std::io::Error::last_os_error();
        if !matches!(error.raw_os_error(),Some(2 | 3)) {return Err(error);}
        if unsafe {MoveFileExW(a.as_ptr(),b.as_ptr(),MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)} == 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}
// Bounded diagnostic UI metadata for troubleshooting the alliance dialog.
// Do not export executable bytes or any raw pointer value. This reads the
// existing first_dialog operand; there is no extra image/code collection.
fn write_alliance_ui_metadata(folder: &Path, pid: u32) -> std::io::Result<()> {
    if !client_present(unsafe {GetTickCount64()}, CLIENT_SEEN_TICK.load(Ordering::Acquire)) { return Ok(()); }
    let tick = unsafe {GetTickCount64()};
    let mut previous = UI_METADATA_LAST.lock().unwrap_or_else(|p| p.into_inner());
    if tick.saturating_sub(previous.3) < 300 { return Ok(()); }
    previous.3 = tick;
    let Some(gui) = RUNTIME.get().and_then(|r| r.gui.as_ref()) else { return Ok(()); };
    let Some(first) = gui.first_dialog.read() else { return Ok(()); };
    let metadata = crate::alliance_ui_probe::inspect(first, read_memory).unwrap_or_else(||crate::alliance_ui_probe::Metadata {
        key:"UI_METADATA_UNAVAILABLE_OR_CHANGED".into(),text:"UI_METADATA_UNAVAILABLE_OR_CHANGED\n".into(),candidate:false,
    });
    if gui.first_dialog.read() != Some(first) { return Ok(()); }
    if previous.0 == metadata.key || (metadata.candidate && previous.2 >= 16)
        || (!metadata.candidate && previous.1 >= 16) { return Ok(()); }
    let path = folder.join(format!("mc-{pid}-alliance-ui.txt"));
    let record = format!("SCALLIANCEUI1\t{pid}\t{tick}\n{}END\n",metadata.text);
    let existing = fs::metadata(&path).map(|m|m.len()).unwrap_or(0);
    if existing.saturating_add(record.len() as u64) > 128 * 1024 { return Ok(()); }
    use std::io::Write as _;
    let mut file = fs::OpenOptions::new().append(true).create(true).open(path)?;
    write!(file, "{record}")?;
    previous.0 = metadata.key;
    if metadata.candidate {previous.2 += 1;} else {previous.1 += 1;}
    Ok(())
}
fn write_changed_init_diagnostic(folder: &Path, pid: u32, last_record: &mut String) {
    let record = INIT_DIAGNOSTIC
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    if !record.is_empty()
        && record != *last_record
        && fs::write(folder.join(format!("mc-{pid}-callback-init.txt")), &record).is_ok()
    {
        *last_record = record;
    }
}
fn write_game_session_diagnostic(folder:&Path,pid:u32)->std::io::Result<()> {
    // No extra memory discovery, identities, addresses or command contents.
    let context=RUNTIME.get().and_then(|r|r.context());
    let record=crate::game_session_diagnostics::Record {
        session:GAME_SESSION_GENERATION.load(Ordering::Acquire),
        boundary_epoch:SESSION_BOUNDARY_EPOCH.load(Ordering::Acquire),
        frame:context.map(|c|c.1).unwrap_or(0),owner:context.map(|c|c.2).unwrap_or(255),
        active:context.is_some_and(|c|c.0),ready:CALLBACKS_READY.load(Ordering::Acquire),
        thread_bound:EVENT_THREAD.load(Ordering::Acquire)!=0,fault:FAULT.load(Ordering::Acquire),
    };
    let mut history=SESSION_HISTORY.lock().unwrap_or_else(|p|p.into_inner());
    if history.0.observe(record){history.1=true;}
    if history.1 {
        let text=history.0.render(pid);debug_assert!(text.len()<crate::game_session_diagnostics::MAX_RENDER_BYTES);
        atomic_write_text(&folder.join(format!("mc-{pid}-game-sessions.tsv")),&text)?;history.1=false;
    }
    Ok(())
}
pub unsafe extern "system" fn worker(parameter: LPVOID) -> DWORD {
    let mut pinned = null_mut();
    if unsafe { GetModuleHandleExW(0x1 | 0x4, worker as *const () as *const u16, &mut pinned) } == 0
    {
        return 1;
    }
    let pid = unsafe { GetCurrentProcessId() };
    if module_path(parameter.cast()).is_none() {
        return 2;
    }
    let Some(data_root) = std::env::var_os("LOCALAPPDATA") else {
        return 3;
    };
    // Status and logs are never written beside the DLL or into the game folder.
    let folder = std::path::PathBuf::from(data_root).join("SCMultiTest");
    if fs::create_dir_all(&folder).is_err() {
        return 6;
    }
    let path = folder.join(format!("mc-{pid}.tsv"));
    let name: Vec<u16> = format!("Local\\SCMultiTest.Stop.{pid}")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let event = unsafe { CreateEventW(null_mut(), 0, 0, name.as_ptr()) };
    if event.is_null() {
        let _ = publish(&path, pid, "FAILED", Some("Stop event unavailable"));
        return 4;
    }
    STOP_EVENT.store(event as usize, Ordering::Release);
    let heartbeat_name: Vec<u16> = format!("Local\\SCMultiTest.Heartbeat.{pid}")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let heartbeat = unsafe { CreateEventW(null_mut(), 0, 0, heartbeat_name.as_ptr()) };
    if heartbeat.is_null() {
        let _ = publish(
            &path,
            pid,
            "FAILED",
            Some("Client heartbeat event unavailable"),
        );
        return 5;
    }
    set_init_phase(InitPhase::Analyzing, "Analyzing loaded game image".into());
    let _ = publish(&path, pid, "WAITING", Some("Analyzing loaded game image"));
    let init = std::thread::spawn(|| {
        std::panic::catch_unwind(initialize).unwrap_or_else(|_| Err("Analysis panicked".into()))
    });
    let mut last_publish = Instant::now();
    let mut last_diagnostic = String::new();
    while !init.is_finished() {
        if last_publish.elapsed() >= Duration::from_millis(50) {
            if let Err(error)=write_game_session_diagnostic(&folder,pid) {
                let _=fs::write(folder.join(format!("mc-{pid}-game-sessions-error.txt")),error.to_string());
            }
            let message = initialization_message();
            let _ = publish(&path, pid, "WAITING", Some(&message));
            last_publish = Instant::now();
        }
        write_changed_init_diagnostic(&folder, pid, &mut last_diagnostic);
        // Consume GUI heartbeat during both analysis and indefinite menu wait.
        let until_publish = Duration::from_millis(50).saturating_sub(last_publish.elapsed());
        let wait_ms = until_publish.as_millis().min(100).max(1) as u32;
        if unsafe { WaitForSingleObject(heartbeat, wait_ms) } == 0 {
            CLIENT_SEEN_TICK.store(unsafe { GetTickCount64() }, Ordering::Release);
        }
    }
    let failure = init
        .join()
        .unwrap_or_else(|_| Err("Initialization thread failed".into()))
        .err();
    if let Some(ref error) = failure {
        let _ = fs::write(folder.join(format!("mc-{pid}-init-error.txt")), error);
    }
    let mut last_error = failure.clone().unwrap_or_default();
    let mut wait = crate::callback_startup::WaitPolicy::default();
    last_publish = Instant::now()
        .checked_sub(Duration::from_millis(50))
        .unwrap_or_else(Instant::now);
    loop {
        if failure.is_none() && !FAULT.load(Ordering::Acquire) {
            match poll_registration() {
                Ok(observation) => {
                    if wait.expired_after_install(
                        unsafe { GetTickCount64() },
                        observation.pending,
                        observation.context.0,
                        observation.context.1,
                        observation.context.2,
                        INSTALLED.load(Ordering::Acquire),
                    ) && !CALLBACKS_READY.load(Ordering::Acquire)
                    {
                        registration_failed(format!(
                            "Game UI callbacks stayed empty for 10 seconds; {}",
                            observation.detail
                        ));
                    }
                }
                Err(message) => registration_failed(message),
            }
        }
        let message = initialization_message();
        let (status, error) = if let Some(ref error) = failure {
            ("FAILED", Some(error.as_str()))
        } else if FAULT.load(Ordering::Acquire) {
            let reason = if INIT_PHASE.load(Ordering::Acquire) == InitPhase::Failed as u32 {
                message.as_str()
            } else {
                "Callback failed; command cloning disabled"
            };
            ("FAILED", Some(reason))
        } else if !CALLBACKS_READY.load(Ordering::Acquire) {
            ("WAITING", Some(message.as_str()))
        } else {
            ("READY", None)
        };
        if status == "FAILED" {
            if let Some(error) = error {
                if error != last_error {
                    if fs::write(folder.join(format!("mc-{pid}-init-error.txt")), error).is_ok() {
                        last_error = error.into();
                    }
                }
            }
        }
        if last_publish.elapsed() >= Duration::from_millis(50) {
            if let Err(error) = runtime_alliance::publish(&folder,pid,status == "READY") {
                let _ = fs::write(folder.join(format!("mc-{pid}-alliance-error.txt")),error.to_string());
            }
            if let Err(error)=write_game_session_diagnostic(&folder,pid) {
                let _=fs::write(folder.join(format!("mc-{pid}-game-sessions-error.txt")),error.to_string());
            }
            if let Err(error) = publish(&path, pid, status, error) {
                let _ = fs::write(
                    folder.join(format!("mc-{pid}.io-error.txt")),
                    error.to_string(),
                );
            }
            if status == "READY" {
                if let Err(error) = write_alliance_ui_metadata(&folder, pid) {
                    let _ = fs::write(folder.join(format!("mc-{pid}-alliance-ui-error.txt")),error.to_string());
                }
            }
            last_publish = Instant::now();
        }
        // This reuses the four-slot observation already taken above. Diagnostics
        // do not query the game again and are written only when records change.
        write_changed_init_diagnostic(&folder, pid, &mut last_diagnostic);
        let until_publish = Duration::from_millis(50).saturating_sub(last_publish.elapsed());
        let wait_ms = until_publish.as_millis().min(100).max(1) as u32;
        if unsafe { WaitForSingleObject(heartbeat, wait_ms) } == 0 {
            CLIENT_SEEN_TICK.store(unsafe { GetTickCount64() }, Ordering::Release);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn infers_id_mode_without_guessing() {
        let u = Unit {
            pointer: 0,
            index: 4,
            minor: 1,
            kind: 37,
            owner: 7,
            next: 0,
        };
        assert_eq!(infer_shift(&u, &[5 | (1 << 13)]), Some(13));
        assert_eq!(infer_shift(&u, &[5 | (1 << 11)]), Some(11));
        let zero = Unit { minor: 0, ..u };
        assert_eq!(infer_shift(&zero, &[5]), Some(0));
        assert_eq!(zero.uid(0), Some(5));
        assert_eq!(u.uid(0), None);
    }
    #[test]
    fn captures_only_bounded_modern_selection() {
        assert!(capture_selection(&[0x63, 13]).is_none());
        assert_eq!(capture_selection(&[0x63, 1, 7, 0, 0, 0]), Some(vec![7]));
        assert!(capture_selection(&[0x63, 1, 0, 0, 0, 0]).is_none());
    }
    #[test]
    fn bundle_restores_selection_and_keeps_limits() {
        let ids: Vec<_> = (1..=15).collect();
        let command = vec![0x60, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        for chunk in ids.chunks(12) {
            let mut bytes = selection_record(chunk).unwrap();
            bytes.extend(&command);
            bytes.extend(selection_record(&[1]).unwrap());
            assert!(bytes.len() < OUTGOING_BUDGET);
            assert!(bytes[1] <= 12);
            assert_eq!(&bytes[bytes.len() - 6..], &[0x63, 1, 1, 0, 0, 0]);
        }
    }
    #[test]
    fn rejects_non_control_and_invalid_queue_flag() {
        assert!(!supported(&[0x5c; 82]));
        assert!(supported(&[0x1a, 0]));
        assert!(!supported(&[0x1a, 2]));
    }
    #[test]
    #[ignore = "Read-only disk image: requires SC_MULTI_EXE"]
    fn disk_image_analysis() {
        let path = std::env::var_os("SC_MULTI_EXE").expect("SC_MULTI_EXE is required");
        let image = scarf::parse_x86_64(&path).expect("Parse on-disk x64 executable");
        let base = image.base().0 as usize;
        let resolved =
            analyze(&image, base).expect("Resolve disk image command sender and game operands");
        println!(
            "DISK ONLY: send RVA {:x}, callback table RVA {:x}; prototype operands resolved; live behavior unverified; no game callbacks installed",
            resolved.send - base,
            resolved.table - base
        );
    }
    #[test]
    #[ignore = "Read-only: requires SC_MULTI_PID and SC_MULTI_BASE"]
    fn running_image_analysis() {
        use winapi::um::handleapi::CloseHandle;
        use winapi::um::memoryapi::ReadProcessMemory;
        use winapi::um::processthreadsapi::OpenProcess;
        let pid = std::env::var("SC_MULTI_PID").unwrap().parse().unwrap();
        let base = std::env::var("SC_MULTI_BASE").unwrap().parse().unwrap();
        let process = unsafe { OpenProcess(0x10, 0, pid) };
        assert!(!process.is_null());
        let read = |address: usize, out: &mut [u8]| {
            let mut got = 0;
            unsafe {
                ReadProcessMemory(
                    process,
                    address as *const c_void,
                    out.as_mut_ptr().cast(),
                    out.len(),
                    &mut got,
                ) != 0
                    && got == out.len()
            }
        };
        let image = snapshot_image(base, read).unwrap();
        let resolved = analyze(&image, base).unwrap();
        unsafe {
            CloseHandle(process);
        }
        println!(
            "READ ONLY: send RVA {:x}, callback table RVA {:x}; prototype operands resolved; no game callbacks installed",
            resolved.send - base,
            resolved.table - base
        );
    }
}
#[cfg(test)]
#[path = "runtime_tests.rs"]
mod runtime_tests;
