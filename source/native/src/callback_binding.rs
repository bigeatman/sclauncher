//! Binds already writable callback table entries without modifying code or memory protection.
//! The caller must pin this DLL and keep the backing table mapped while any binding exists.
//! Resolved originals and replacements must have matching callback ABIs. This module never
//! executes callbacks, frees code, opens another process, or changes page protections.

use std::mem::{align_of, size_of};
use std::sync::atomic::{AtomicUsize, Ordering};

// samase_scarf src/dialog.rs ResetUiEventHandlersAnalyzer::try_finish.
pub const KEY_DOWN_INDEX: usize = 0x0;
pub const LEFT_DOWN_INDEX: usize = 0x4;
pub const RIGHT_DOWN_INDEX: usize = 0x7;
pub const PERIODIC_INDEX: usize = 0xd;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub address: usize,
    pub original: usize,
    pub replacement: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotState {
    Original,
    Installed,
    Other(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingError {
    InvalidAddress(usize),
    InvalidValues(usize),
    DuplicateAddress(usize),
    QueryFailed {
        address: usize,
        win32: u32,
    },
    UnsafeRegion {
        address: usize,
        state: u32,
        protect: u32,
        kind: u32,
    },
    OutsideRegion(usize),
    Mismatch {
        address: usize,
        expected: usize,
        actual: usize,
    },
}

impl std::fmt::Display for BindingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidAddress(a) => write!(f, "Invalid callback slot address: 0x{a:x}"),
            Self::InvalidValues(a) => write!(f, "Invalid callback values for slot: 0x{a:x}"),
            Self::DuplicateAddress(a) => write!(f, "Duplicate callback slot: 0x{a:x}"),
            Self::QueryFailed { address, win32 } => write!(
                f,
                "Callback slot query failed: slot=0x{address:x}; win32={win32}"
            ),
            Self::UnsafeRegion {
                address,
                state,
                protect,
                kind,
            } => write!(
                f,
                "Callback slot is not ordinary writable memory: slot=0x{address:x}; state=0x{state:x}; protect=0x{protect:x}; type=0x{kind:x}"
            ),
            Self::OutsideRegion(a) => write!(f, "Callback slot crosses its memory region: 0x{a:x}"),
            Self::Mismatch {
                address,
                expected,
                actual,
            } => write!(
                f,
                "Callback slot changed: slot=0x{address:x}; expected=0x{expected:x}; actual=0x{actual:x}"
            ),
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct RestoreReport {
    pub restored: usize,
    pub already_original: usize,
    pub errors: Vec<BindingError>,
}

#[derive(Clone, Copy)]
struct Region {
    base: usize,
    size: usize,
    state: u32,
    protect: u32,
    kind: u32,
}

fn validate_region(address: usize, region: Region) -> Result<(), BindingError> {
    // Exact equality also rejects PAGE_GUARD, PAGE_NOACCESS and write-copy mappings.
    if region.state != 0x1000
        || !matches!(region.protect, 0x04 | 0x40)
        || !matches!(region.kind, 0x20000 | 0x40000 | 0x1000000)
    {
        return Err(BindingError::UnsafeRegion {
            address,
            state: region.state,
            protect: region.protect,
            kind: region.kind,
        });
    }
    let end = address
        .checked_add(size_of::<usize>())
        .ok_or(BindingError::OutsideRegion(address))?;
    let region_end = region
        .base
        .checked_add(region.size)
        .ok_or(BindingError::OutsideRegion(address))?;
    if address < region.base || end > region_end {
        return Err(BindingError::OutsideRegion(address));
    }
    Ok(())
}

#[cfg(windows)]
fn query_region(address: usize) -> Result<Region, BindingError> {
    use std::ffi::c_void;
    #[repr(C)]
    struct MemoryBasicInformation {
        base_address: *mut c_void,
        allocation_base: *mut c_void,
        allocation_protect: u32,
        region_size: usize,
        state: u32,
        protect: u32,
        kind: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn VirtualQuery(
            address: *const c_void,
            info: *mut MemoryBasicInformation,
            length: usize,
        ) -> usize;
        fn GetLastError() -> u32;
    }
    let mut info = std::mem::MaybeUninit::<MemoryBasicInformation>::zeroed();
    let result = unsafe {
        VirtualQuery(
            address as *const c_void,
            info.as_mut_ptr(),
            size_of::<MemoryBasicInformation>(),
        )
    };
    if result != size_of::<MemoryBasicInformation>() {
        return Err(BindingError::QueryFailed {
            address,
            win32: unsafe { GetLastError() },
        });
    }
    let info = unsafe { info.assume_init() };
    Ok(Region {
        base: info.base_address as usize,
        size: info.region_size,
        state: info.state,
        protect: info.protect,
        kind: info.kind,
    })
}

#[cfg(not(windows))]
fn query_region(address: usize) -> Result<Region, BindingError> {
    // Production binding is Windows-only; unsupported platforms cannot write anything.
    Err(BindingError::QueryFailed { address, win32: 50 })
}

pub fn validate(slot: Slot) -> Result<(), BindingError> {
    if size_of::<usize>() != 8 || slot.address == 0 || slot.address % align_of::<AtomicUsize>() != 0
    {
        return Err(BindingError::InvalidAddress(slot.address));
    }
    if slot.original == 0 || slot.replacement == 0 || slot.original == slot.replacement {
        return Err(BindingError::InvalidValues(slot.address));
    }
    validate_region(slot.address, query_region(slot.address)?)
}

fn validate_all(slots: &[Slot]) -> Result<(), BindingError> {
    for (index, &slot) in slots.iter().enumerate() {
        validate(slot)?;
        if slots[..index]
            .iter()
            .any(|previous| previous.address == slot.address)
        {
            return Err(BindingError::DuplicateAddress(slot.address));
        }
    }
    Ok(())
}

/// # Safety
/// `slot.address` must remain mapped and refer to an aligned pointer-sized table
/// entry. All concurrent access must preserve aligned pointer atomicity. Foreign
/// code may reset a slot to its original callback; arbitrary values are rejected.
unsafe fn atomic_slot(slot: Slot) -> &'static AtomicUsize {
    unsafe { &*(slot.address as *const AtomicUsize) }
}

/// Installs all slots only when each still has its exact resolved original value.
/// Failure restores only entries installed by this call that still contain ours.
/// Executable memory and callback lifetimes are never modified.
///
/// # Safety
/// See `atomic_slot`. The DLL containing replacements must remain pinned even if
/// this function fails: another thread could already be executing a replacement.
pub unsafe fn install(slots: &[Slot]) -> Result<(), BindingError> {
    unsafe { bind_all(slots, false) }.map(|_| ())
}

/// Leaves installed slots intact and rebinds slots reset to their exact originals.
/// Never overwrites another callback provider. Newly changed entries are rolled
/// back on error; entries already installed before this call remain intact.
///
/// # Safety
/// Same table and callback lifetime requirements as `install`.
pub unsafe fn maintain(slots: &[Slot]) -> Result<(), BindingError> {
    unsafe { maintain_report(slots) }.map(|_| ())
}

/// Returns every slot this call actually changed from original to replacement.
/// This reports resets racing an earlier inspection, rather than inferring a
/// rebind from a separate pre-check. It does not promise continuous ownership.
///
/// # Safety
/// Same backing table and callback lifetime requirements as `install`. Callers
/// must serialize registration operations across their own threads.
pub unsafe fn maintain_report(slots: &[Slot]) -> Result<Vec<Slot>, BindingError> {
    unsafe { bind_all(slots, true) }
}

unsafe fn bind_all(slots: &[Slot], allow_installed: bool) -> Result<Vec<Slot>, BindingError> {
    validate_all(slots)?;
    let mut changed = Vec::with_capacity(slots.len());
    for &slot in slots {
        let entry = unsafe { atomic_slot(slot) };
        match entry.compare_exchange(
            slot.original,
            slot.replacement,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => changed.push(slot),
            Err(actual) if allow_installed && actual == slot.replacement => (),
            Err(actual) => {
                for &previous in changed.iter().rev() {
                    let entry = unsafe { atomic_slot(previous) };
                    let _ = entry.compare_exchange(
                        previous.replacement,
                        previous.original,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    );
                }
                return Err(BindingError::Mismatch {
                    address: slot.address,
                    expected: slot.original,
                    actual,
                });
            }
        }
    }
    Ok(changed)
}

/// # Safety
/// Same backing table lifetime and atomicity requirements as `install`.
pub unsafe fn check(slot: Slot) -> Result<SlotState, BindingError> {
    validate(slot)?;
    let value = unsafe { atomic_slot(slot) }.load(Ordering::SeqCst);
    Ok(if value == slot.original {
        SlotState::Original
    } else if value == slot.replacement {
        SlotState::Installed
    } else {
        SlotState::Other(value)
    })
}

/// Tries every slot; only replaces our exact callback pointer with its original.
/// An original value is already restored, and every other value is left intact.
/// Replacements must remain pinned after restoration because a callback may have
/// been fetched before the pointer was restored.
///
/// # Safety
/// Same backing table lifetime and atomicity requirements as `install`.
pub unsafe fn restore(slots: &[Slot]) -> RestoreReport {
    let mut report = RestoreReport::default();
    for &slot in slots {
        if let Err(error) = validate(slot) {
            report.errors.push(error);
            continue;
        }
        let entry = unsafe { atomic_slot(slot) };
        match entry.compare_exchange(
            slot.replacement,
            slot.original,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => report.restored += 1,
            Err(actual) if actual == slot.original => report.already_original += 1,
            Err(actual) => report.errors.push(BindingError::Mismatch {
                address: slot.address,
                expected: slot.replacement,
                actual,
            }),
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    const ORIGINAL: usize = 0x1000;
    const OURS: usize = 0x2000;
    const ALIEN: usize = 0x3000;
    fn slot(entry: &AtomicUsize) -> Slot {
        Slot {
            address: entry as *const AtomicUsize as usize,
            original: ORIGINAL,
            replacement: OURS,
        }
    }
    fn region() -> Region {
        Region {
            base: 0x1000,
            size: 0x1000,
            state: 0x1000,
            protect: 4,
            kind: 0x40000,
        }
    }
    #[test]
    fn callback_indices_match_resolver() {
        assert_eq!(
            [
                KEY_DOWN_INDEX,
                LEFT_DOWN_INDEX,
                RIGHT_DOWN_INDEX,
                PERIODIC_INDEX
            ],
            [0, 4, 7, 13]
        );
    }
    #[test]
    fn region_requires_ordinary_writable_committed_memory() {
        assert!(validate_region(0x1100, region()).is_ok());
        for protect in [0, 1, 2, 0x20, 0x80, 0x104, 0x204, 0x404] {
            let mut r = region();
            r.protect = protect;
            assert!(matches!(
                validate_region(0x1100, r),
                Err(BindingError::UnsafeRegion { .. })
            ));
        }
        for state in [0, 0x2000, 0x10000] {
            let mut r = region();
            r.state = state;
            assert!(matches!(
                validate_region(0x1100, r),
                Err(BindingError::UnsafeRegion { .. })
            ));
        }
    }
    #[test]
    fn region_bounds_checked_without_overflow() {
        assert!(validate_region(0x1ff8, region()).is_ok());
        for address in [0xff8, 0x1ffc, usize::MAX - 3] {
            assert!(matches!(
                validate_region(address, region()),
                Err(BindingError::OutsideRegion(_))
            ));
        }
        let mut r = region();
        r.base = usize::MAX - 7;
        r.size = 16;
        assert!(matches!(
            validate_region(usize::MAX - 7, r),
            Err(BindingError::OutsideRegion(_))
        ));
    }
    #[test]
    fn invalid_slot_never_dereferenced() {
        for address in [0, 1, 7, usize::MAX] {
            let s = Slot {
                address,
                original: ORIGINAL,
                replacement: OURS,
            };
            assert!(matches!(
                unsafe { install(&[s]) },
                Err(BindingError::InvalidAddress(_))
            ));
        }
        let entry = AtomicUsize::new(ORIGINAL);
        let mut s = slot(&entry);
        s.replacement = s.original;
        assert!(matches!(
            unsafe { install(&[s]) },
            Err(BindingError::InvalidValues(_))
        ));
        assert_eq!(entry.load(Ordering::SeqCst), ORIGINAL);
    }
    #[cfg(windows)]
    #[test]
    fn installation_and_restore_preserve_original_values() {
        let entries = [AtomicUsize::new(ORIGINAL), AtomicUsize::new(ORIGINAL)];
        let slots = [slot(&entries[0]), slot(&entries[1])];
        unsafe {
            install(&slots).unwrap();
        }
        for &s in &slots {
            assert_eq!(unsafe { check(s).unwrap() }, SlotState::Installed);
        }
        assert!(matches!(
            unsafe { install(&slots) },
            Err(BindingError::Mismatch { actual: OURS, .. })
        ));
        assert_eq!(
            unsafe { restore(&slots) },
            RestoreReport {
                restored: 2,
                already_original: 0,
                errors: vec![]
            }
        );
        assert_eq!(unsafe { restore(&slots) }.already_original, 2);
    }
    #[cfg(windows)]
    #[test]
    fn installation_rolls_back_and_never_overwrites_alien_values() {
        let entries = [AtomicUsize::new(ORIGINAL), AtomicUsize::new(ALIEN)];
        let slots = [slot(&entries[0]), slot(&entries[1])];
        assert!(matches!(
            unsafe { install(&slots) },
            Err(BindingError::Mismatch { actual: ALIEN, .. })
        ));
        assert_eq!(entries[0].load(Ordering::SeqCst), ORIGINAL);
        assert_eq!(entries[1].load(Ordering::SeqCst), ALIEN);
    }
    #[cfg(windows)]
    #[test]
    fn validation_is_complete_before_any_install() {
        let entry = AtomicUsize::new(ORIGINAL);
        let good = slot(&entry);
        let bad = Slot { address: 1, ..good };
        assert!(unsafe { install(&[good, bad]) }.is_err());
        assert_eq!(entry.load(Ordering::SeqCst), ORIGINAL);
        assert!(matches!(
            unsafe { install(&[good, good]) },
            Err(BindingError::DuplicateAddress(_))
        ));
        assert_eq!(entry.load(Ordering::SeqCst), ORIGINAL);
    }
    #[cfg(windows)]
    #[test]
    fn maintenance_rebinds_engine_reset_but_keeps_prior_bindings_on_error() {
        let entries = [
            AtomicUsize::new(OURS),
            AtomicUsize::new(ORIGINAL),
            AtomicUsize::new(ALIEN),
        ];
        let slots = [slot(&entries[0]), slot(&entries[1]), slot(&entries[2])];
        assert!(unsafe { maintain(&slots) }.is_err());
        assert_eq!(entries[0].load(Ordering::SeqCst), OURS);
        assert_eq!(entries[1].load(Ordering::SeqCst), ORIGINAL);
        assert_eq!(entries[2].load(Ordering::SeqCst), ALIEN);
        unsafe {
            maintain(&slots[..2]).unwrap();
        }
        entries[0].store(ORIGINAL, Ordering::SeqCst);
        unsafe {
            maintain(&slots[..2]).unwrap();
        }
        assert_eq!(entries[0].load(Ordering::SeqCst), OURS);
    }
    #[cfg(windows)]
    #[test]
    fn restore_continues_and_does_not_clobber_changed_callback() {
        let entries = [
            AtomicUsize::new(ALIEN),
            AtomicUsize::new(OURS),
            AtomicUsize::new(ORIGINAL),
        ];
        let slots = entries.iter().map(slot).collect::<Vec<_>>();
        let report = unsafe { restore(&slots) };
        assert_eq!(report.restored, 1);
        assert_eq!(report.already_original, 1);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(entries[0].load(Ordering::SeqCst), ALIEN);
        assert_eq!(entries[1].load(Ordering::SeqCst), ORIGINAL);
    }
    #[cfg(windows)]
    #[test]
    fn concurrent_engine_resets_and_binding_do_not_clobber_other_provider() {
        let entry = Arc::new(AtomicUsize::new(ORIGINAL));
        let barrier = Arc::new(Barrier::new(2));
        let engine_entry = entry.clone();
        let engine_barrier = barrier.clone();
        let engine = std::thread::spawn(move || {
            for _ in 0..1000 {
                engine_entry.store(ORIGINAL, Ordering::SeqCst);
                engine_barrier.wait();
                engine_barrier.wait();
                engine_entry.store(ALIEN, Ordering::SeqCst);
                engine_barrier.wait();
                engine_barrier.wait();
                assert_eq!(engine_entry.load(Ordering::SeqCst), ALIEN);
                engine_barrier.wait();
            }
        });
        let s = slot(&entry);
        for _ in 0..1000 {
            barrier.wait();
            unsafe {
                maintain(&[s]).unwrap();
            }
            barrier.wait();
            barrier.wait();
            assert!(matches!(
                unsafe { maintain(&[s]) },
                Err(BindingError::Mismatch { actual: ALIEN, .. })
            ));
            assert_eq!(unsafe { restore(&[s]) }.errors.len(), 1);
            barrier.wait();
            barrier.wait();
        }
        engine.join().unwrap();
        assert_eq!(entry.load(Ordering::SeqCst), ALIEN);
    }
    #[cfg(windows)]
    #[test]
    fn install_races_with_original_reset_use_only_atomic_cas() {
        let entry = Arc::new(AtomicUsize::new(ORIGINAL));
        let barrier = Arc::new(Barrier::new(2));
        let reset_entry = entry.clone();
        let reset_barrier = barrier.clone();
        let engine = std::thread::spawn(move || {
            reset_barrier.wait();
            for _ in 0..5000 {
                reset_entry.store(ORIGINAL, Ordering::SeqCst);
            }
        });
        barrier.wait();
        let s = slot(&entry);
        for _ in 0..5000 {
            unsafe {
                maintain(&[s]).unwrap();
            }
        }
        engine.join().unwrap();
        unsafe {
            maintain(&[s]).unwrap();
        }
        assert_eq!(entry.load(Ordering::SeqCst), OURS);
        assert_eq!(unsafe { restore(&[s]) }.restored, 1);
    }
    #[cfg(windows)]
    #[test]
    fn maintenance_report_uses_actual_cas_changes_even_after_old_inspection() {
        let entries = [AtomicUsize::new(OURS), AtomicUsize::new(OURS)];
        let slots = [slot(&entries[0]), slot(&entries[1])];
        assert!(unsafe { maintain_report(&slots).unwrap() }.is_empty());
        // The old read said installed, then the engine reset one slot. The
        // actual successful CAS must report this observation gap.
        assert_eq!(unsafe { check(slots[1]).unwrap() }, SlotState::Installed);
        entries[1].store(ORIGINAL, Ordering::SeqCst);
        assert_eq!(unsafe { maintain_report(&slots).unwrap() }, vec![slots[1]]);
        assert_eq!(entries[1].load(Ordering::SeqCst), OURS);
        assert!(unsafe { maintain_report(&slots).unwrap() }.is_empty());
        assert_eq!(unsafe { restore(&slots) }.restored, 2);
        entries[1].store(ALIEN, Ordering::SeqCst);
        assert!(matches!(
            unsafe { maintain_report(&slots) },
            Err(BindingError::Mismatch { actual: ALIEN, .. })
        ));
        assert_eq!(entries[0].load(Ordering::SeqCst), ORIGINAL);
        assert_eq!(entries[1].load(Ordering::SeqCst), ALIEN);
    }
}
