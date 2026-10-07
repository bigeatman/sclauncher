//! Exact, bounded rewriting of an already verified five-byte data packet.
//!
//! The caller owns the current UI-thread/sender-lock lifetime lease and checks
//! packet boundaries, buffer pointer, capacity and length. This helper changes
//! no protection, executes no engine callback and never opens another process.

const PACKET_LEN: usize = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Region {
    base: usize,
    size: usize,
    state: u32,
    protect: u32,
    kind: u32,
}

fn region_allows_packet(address: usize, region: Region) -> bool {
    // Exact protection equality excludes execute, guard, write-copy and other
    // modifiers. MEM_IMAGE is accepted only for such ordinary writable data.
    if address < 0x10000 || region.state != 0x1000 || region.protect != 0x04
        || !matches!(region.kind, 0x20000 | 0x40000 | 0x1000000)
    {
        return false;
    }
    let Some(end) = address.checked_add(PACKET_LEN) else { return false; };
    let Some(region_end) = region.base.checked_add(region.size) else { return false; };
    address >= region.base && end <= region_end
}

#[cfg(windows)]
mod platform {
    use super::{PACKET_LEN, Region, region_allows_packet};
    use std::ffi::c_void;
    use std::mem::{MaybeUninit, size_of};

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
        fn GetCurrentProcess() -> *mut c_void;
        fn VirtualQuery(address: *const c_void, information: *mut MemoryBasicInformation,
            length: usize) -> usize;
        fn ReadProcessMemory(process: *mut c_void, address: *const c_void,
            buffer: *mut c_void, size: usize, read: *mut usize) -> i32;
        fn WriteProcessMemory(process: *mut c_void, address: *mut c_void,
            buffer: *const c_void, size: usize, written: *mut usize) -> i32;
    }

    fn query(address: usize) -> Option<Region> {
        let mut information = MaybeUninit::<MemoryBasicInformation>::zeroed();
        if unsafe { VirtualQuery(address as *const c_void, information.as_mut_ptr(),
            size_of::<MemoryBasicInformation>()) } != size_of::<MemoryBasicInformation>()
        {
            return None;
        }
        let information = unsafe { information.assume_init() };
        Some(Region {
            base: information.base_address as usize,
            size: information.region_size,
            state: information.state,
            protect: information.protect,
            kind: information.kind,
        })
    }

    fn read_packet(process: *mut c_void, address: usize) -> Option<[u8; PACKET_LEN]> {
        let mut bytes = [0; PACKET_LEN];
        let mut read = 0;
        (unsafe { ReadProcessMemory(process, address as *const c_void,
            bytes.as_mut_ptr().cast(), PACKET_LEN, &mut read) } != 0
            && read == PACKET_LEN).then_some(bytes)
    }

    pub(super) unsafe fn rewrite(address: usize, expected: &[u8; PACKET_LEN],
        replacement: &[u8; PACKET_LEN]) -> bool
    {
        if address < 0x10000 || address.checked_add(PACKET_LEN).is_none() { return false; }
        let Some(region) = query(address).filter(|value| region_allows_packet(address, *value))
            else { return false; };
        let process = unsafe { GetCurrentProcess() };
        if read_packet(process, address).as_ref() != Some(expected)
            || query(address) != Some(region)
        {
            return false;
        }
        let mut written = 0;
        if unsafe { WriteProcessMemory(process, address as *mut c_void,
            replacement.as_ptr().cast(), PACKET_LEN, &mut written) } == 0
            || written != PACKET_LEN
        {
            return false;
        }
        read_packet(process, address).as_ref() == Some(replacement)
    }
}

/// Returns true only after the exact replacement was read back. False after a
/// write attempt does not prove that no bytes were written; the caller must
/// retain its lock and inspect the same verified packet before proceeding.
///
/// # Safety
/// The caller must have exclusive mutation access to this live writable data
/// range. Its sender lock/UI-thread lease must exclude concurrent buffer reuse,
/// free, resize and any native sender operation throughout this call.
pub(crate) unsafe fn rewrite_exact_5(address: usize, expected: &[u8; PACKET_LEN],
    replacement: &[u8; PACKET_LEN]) -> bool
{
    #[cfg(windows)]
    { unsafe { platform::rewrite(address, expected, replacement) } }
    #[cfg(not(windows))]
    { let _ = (address, expected, replacement); false }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn writable_region() -> Region {
        Region { base: 0x10000, size: 0x1000, state: 0x1000,
            protect: 0x04, kind: 0x20000 }
    }

    #[test]
    fn ordinary_writable_private_mapped_and_image_data_are_supported() {
        let mut region = writable_region();
        for kind in [0x20000, 0x40000, 0x1000000] {
            region.kind = kind;
            assert!(region_allows_packet(0x10003, region));
        }
    }

    #[test]
    fn executable_guard_writecopy_and_uncommitted_regions_are_refused() {
        let valid = writable_region();
        for protect in [0, 0x01, 0x02, 0x08, 0x10, 0x20, 0x40, 0x80, 0x104, 0x204, 0x404] {
            assert!(!region_allows_packet(0x10000, Region { protect, ..valid }));
        }
        for state in [0, 0x2000, 0x10000] {
            assert!(!region_allows_packet(0x10000, Region { state, ..valid }));
        }
        for kind in [0, 0x1000, 0x60000] {
            assert!(!region_allows_packet(0x10000, Region { kind, ..valid }));
        }
    }

    #[test]
    fn the_whole_five_byte_packet_must_fit_one_region() {
        let region = writable_region();
        assert!(region_allows_packet(0x10ffb, region));
        assert!(!region_allows_packet(0x10ffc, region));
        assert!(!region_allows_packet(0x11000, region));
        assert!(!region_allows_packet(0x10000, Region { size: 4, ..region }));
        assert!(!region_allows_packet(0x10000, Region { base: 0x10001, ..region }));
    }

    #[test]
    fn low_addresses_and_arithmetic_overflow_are_refused() {
        let region = writable_region();
        for address in [0, 1, 0xffff, usize::MAX, usize::MAX - 4] {
            assert!(!region_allows_packet(address, region));
        }
        assert!(!region_allows_packet(usize::MAX - 8,
            Region { base: usize::MAX - 8, size: 16, ..region }));
    }

    #[cfg(windows)]
    #[test]
    fn owned_vector_rewrite_preserves_every_byte_outside_the_packet() {
        let mut data = vec![0xa5; 32];
        let expected = [0x0e, 0x01, 0x00, 0x00, 0x00];
        let replacement = [0x0e, 0x05, 0x00, 0x00, 0x00];
        data[7..12].copy_from_slice(&expected);
        let before = data.clone();
        let address = unsafe { data.as_mut_ptr().add(7) } as usize;
        assert!(unsafe { rewrite_exact_5(address, &expected, &replacement) });
        assert_eq!(&data[..7], &before[..7]);
        assert_eq!(&data[7..12], &replacement);
        assert_eq!(&data[12..], &before[12..]);
    }

    #[cfg(windows)]
    #[test]
    fn stale_expected_packet_is_refused_without_mutating_owned_bytes() {
        let mut data = vec![0xa5; 32];
        let expected = [0x0e, 0x01, 0x00, 0x00, 0x00];
        let replacement = [0x0e, 0x05, 0x00, 0x00, 0x00];
        data[7..12].copy_from_slice(&expected);
        data[8] = 0x02;
        let before = data.clone();
        let address = unsafe { data.as_mut_ptr().add(7) } as usize;
        assert!(!unsafe { rewrite_exact_5(address, &expected, &replacement) });
        assert_eq!(data, before);
    }

    #[cfg(windows)]
    #[test]
    fn invalid_target_addresses_are_refused_before_any_packet_read() {
        let packet = [0x0e, 0, 0, 0, 0];
        for address in [0, 1, 0xffff, usize::MAX - 4, usize::MAX] {
            assert!(!unsafe { rewrite_exact_5(address, &packet, &packet) });
        }
    }
}
