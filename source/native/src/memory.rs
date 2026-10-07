use scarf::{
    ArithOpType, BinaryFile, BinarySection, MemAccessSize, Operand, OperandType, VirtualAddress64,
};
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use winapi::ctypes::c_void;
use winapi::shared::minwindef::HMODULE;
use winapi::um::libloaderapi::GetModuleFileNameW;
use winapi::um::memoryapi::ReadProcessMemory;
use winapi::um::processthreadsapi::GetCurrentProcess;
pub(crate) enum Value {
    Constant(usize),
    Memory(Box<Value>, usize, usize),
    Arithmetic(ArithOpType, Box<Value>, Box<Value>),
}

impl Value {
    pub(crate) fn from_operand(
        op: Operand<'_>,
        preferred: u64,
        image_end: u64,
        actual: usize,
        depth: u8,
    ) -> Result<Self, String> {
        if depth > 32 {
            return Err("Operand nesting exceeds limit".into());
        }
        let recurse =
            |operand| Self::from_operand(operand, preferred, image_end, actual, depth + 1);
        match *op.ty() {
            OperandType::Constant(x) => {
                Ok(Self::Constant(if (preferred..image_end).contains(&x) {
                    actual.wrapping_add((x - preferred) as usize)
                } else {
                    x as usize
                }))
            }
            OperandType::Memory(ref memory) => {
                let (base, offset) = memory.address();
                let size = match memory.size {
                    MemAccessSize::Mem8 => 1,
                    MemAccessSize::Mem16 => 2,
                    MemAccessSize::Mem32 => 4,
                    MemAccessSize::Mem64 => 8,
                };
                // Scarf stores absolute addresses in memory offsets too.
                // Pointers read from process memory are already runtime addresses.
                let offset = if (preferred..image_end).contains(&offset) {
                    actual.wrapping_add((offset - preferred) as usize)
                } else {
                    offset as usize
                };
                Ok(Self::Memory(Box::new(recurse(base)?), offset, size))
            }
            OperandType::Arithmetic(ref arithmetic) => {
                match arithmetic.ty {
                    ArithOpType::Add
                    | ArithOpType::Sub
                    | ArithOpType::Mul
                    | ArithOpType::Div
                    | ArithOpType::Modulo
                    | ArithOpType::And
                    | ArithOpType::Or
                    | ArithOpType::Xor
                    | ArithOpType::Lsh
                    | ArithOpType::Rsh
                    | ArithOpType::Equal
                    | ArithOpType::GreaterThan => (),
                    _ => return Err("Unsupported operand arithmetic".into()),
                }
                Ok(Self::Arithmetic(
                    arithmetic.ty,
                    Box::new(recurse(arithmetic.left)?),
                    Box::new(recurse(arithmetic.right)?),
                ))
            }
            _ => Err("Unsupported runtime operand".into()),
        }
    }

    pub(crate) fn read(&self) -> Option<usize> {
        match self {
            Self::Constant(x) => Some(*x),
            Self::Memory(base, offset, size) => {
                read_integer(base.read()?.wrapping_add(*offset), *size)
            }
            Self::Arithmetic(op, left, right) => {
                let (left, right) = (left.read()?, right.read()?);
                Some(match op {
                    ArithOpType::Add => left.wrapping_add(right),
                    ArithOpType::Sub => left.wrapping_sub(right),
                    ArithOpType::Mul => left.wrapping_mul(right),
                    ArithOpType::Div => left.checked_div(right)?,
                    ArithOpType::Modulo => left.checked_rem(right)?,
                    ArithOpType::And => left & right,
                    ArithOpType::Or => left | right,
                    ArithOpType::Xor => left ^ right,
                    ArithOpType::Lsh => left.wrapping_shl(right as u32),
                    ArithOpType::Rsh => left.wrapping_shr(right as u32),
                    ArithOpType::Equal => usize::from(left == right),
                    ArithOpType::GreaterThan => usize::from(left > right),
                    _ => return None,
                })
            }
        }
    }
}

pub(crate) fn read_memory(address: usize, out: &mut [u8]) -> bool {
    if address < 0x10000 || address.checked_add(out.len()).is_none() {
        return false;
    }
    let mut received = 0;
    unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            out.as_mut_ptr().cast(),
            out.len(),
            &mut received,
        ) != 0
            && received == out.len()
    }
}

pub(crate) fn read_integer(address: usize, size: usize) -> Option<usize> {
    let mut bytes = [0u8; 8];
    if size > bytes.len() || !read_memory(address, &mut bytes[..size]) {
        return None;
    }
    Some(u64::from_le_bytes(bytes) as usize)
}

pub(crate) fn module_path(module: HMODULE) -> Option<PathBuf> {
    let mut buffer = [0u16; 32768];
    let len =
        unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if len == 0 || len >= buffer.len() {
        return None;
    }
    Some(PathBuf::from(std::ffi::OsString::from_wide(&buffer[..len])))
}

pub(crate) fn snapshot_image(
    base: usize,
    mut read: impl FnMut(usize, &mut [u8]) -> bool,
) -> Result<BinaryFile<VirtualAddress64>, String> {
    const MAX_IMAGE: usize = 512 * 1024 * 1024;
    const MAX_HEADERS: usize = 1024 * 1024;
    let at = |offset: usize| base.checked_add(offset).ok_or("Image address overflow");
    let mut dos = [0u8; 0x40];
    if base < 0x10000 || !read(base, &mut dos) || &dos[..2] != b"MZ" {
        return Err("Loaded executable DOS header unavailable".into());
    }
    let pe_offset = u32::from_le_bytes(dos[0x3c..0x40].try_into().unwrap()) as usize;
    if !(0x40..MAX_HEADERS - 0x18).contains(&pe_offset) {
        return Err("Invalid PE offset".into());
    }
    let mut file_header = [0u8; 0x18];
    if !read(at(pe_offset)?, &mut file_header)
        || &file_header[..4] != b"PE\0\0"
        || u16::from_le_bytes(file_header[4..6].try_into().unwrap()) != 0x8664
    {
        return Err("Loaded executable is not a supported x64 PE image".into());
    }
    let section_count = u16::from_le_bytes(file_header[6..8].try_into().unwrap()) as usize;
    let optional_size = u16::from_le_bytes(file_header[20..22].try_into().unwrap()) as usize;
    if section_count == 0 || section_count > 96 || !(0xf0..=0x1000).contains(&optional_size) {
        return Err("Invalid PE section table".into());
    }
    let section_offset = pe_offset + 0x18 + optional_size;
    let table_end = section_offset + section_count * 40;
    if table_end > MAX_HEADERS {
        return Err("PE section table exceeds limit".into());
    }
    let mut header = vec![0u8; table_end];
    if !read(base, &mut header) {
        return Err("Loaded executable headers unreadable".into());
    }
    let optional = pe_offset + 0x18;
    let u32_at =
        |offset: usize| u32::from_le_bytes(header[offset..offset + 4].try_into().unwrap()) as usize;
    let image_size = u32_at(optional + 56);
    let header_size = u32_at(optional + 60);
    if u16::from_le_bytes(header[optional..optional + 2].try_into().unwrap()) != 0x20b
        || image_size < table_end
        || image_size > MAX_IMAGE
        || header_size < table_end
        || header_size > MAX_HEADERS
        || header_size > image_size
    {
        return Err("Invalid loaded executable dimensions".into());
    }
    at(image_size)?;
    let names = [
        *b".text\0\0\0",
        *b".rdata\0\0",
        *b".data\0\0\0",
        *b".pdata\0\0",
        *b".reloc\0\0",
    ];
    let mut sections = Vec::new();
    let mut total = header_size;
    for name in names {
        let mut found = None;
        for index in 0..section_count {
            let offset = section_offset + index * 40;
            if header[offset..offset + 8] != name {
                continue;
            }
            if found.is_some() {
                return Err("Duplicate analysis section".into());
            }
            let virtual_size = u32_at(offset + 8);
            let rva = u32_at(offset + 12);
            let physical_size = u32_at(offset + 16);
            let span = virtual_size.max(physical_size);
            if rva < header_size
                || physical_size == 0
                || rva.checked_add(span).is_none_or(|end| end > image_size)
            {
                return Err("Analysis section is outside loaded image".into());
            }
            total = total
                .checked_add(span)
                .filter(|&x| x <= MAX_IMAGE)
                .ok_or("Image snapshot exceeds limit")?;
            // The loaded image may contain initialized globals past the on-disk
            // raw data, including outgoing buffers in the virtual tail.
            let mut data = vec![0u8; span];
            let address = at(rva)?;
            if !read(address, &mut data) {
                return Err(format!(
                    "Loaded analysis section {} unreadable",
                    String::from_utf8_lossy(&name)
                ));
            }
            found = Some(BinarySection {
                name,
                virtual_address: VirtualAddress64(address as u64),
                virtual_size: virtual_size as u32,
                data,
            });
        }
        sections.push(found.ok_or_else(|| {
            format!(
                "Loaded analysis section {} missing",
                String::from_utf8_lossy(&name)
            )
        })?);
    }
    let mut full_header = vec![0u8; header_size];
    if !read(base, &mut full_header) {
        return Err("Loaded executable header snapshot unavailable".into());
    }
    if full_header[..header.len()] != header {
        return Err("Executable headers changed during snapshot".into());
    }
    sections.insert(
        0,
        BinarySection {
            name: *b"(header)",
            virtual_address: VirtualAddress64(base as u64),
            virtual_size: header_size as u32,
            data: full_header,
        },
    );
    Ok(scarf::raw_bin(VirtualAddress64(base as u64), sections))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded_pe() -> Vec<u8> {
        let mut image = vec![0u8; 0x6000];
        image[..2].copy_from_slice(b"MZ");
        image[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        image[0x80..0x84].copy_from_slice(b"PE\0\0");
        image[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
        image[0x86..0x88].copy_from_slice(&5u16.to_le_bytes());
        image[0x94..0x96].copy_from_slice(&0xf0u16.to_le_bytes());
        image[0x98..0x9a].copy_from_slice(&0x20bu16.to_le_bytes());
        image[0xd0..0xd4].copy_from_slice(&0x6000u32.to_le_bytes());
        image[0xd4..0xd8].copy_from_slice(&0x400u32.to_le_bytes());
        let names = [*b".text\0\0\0", *b".rdata\0\0", *b".data\0\0\0", *b".pdata\0\0", *b".reloc\0\0"];
        for (index, name) in names.iter().enumerate() {
            let at = 0x188 + index * 40;
            image[at..at+8].copy_from_slice(name);
            let virtual_size = if index == 2 {0x500u32} else {0x200u32};
            image[at+8..at+12].copy_from_slice(&virtual_size.to_le_bytes());
            image[at+12..at+16].copy_from_slice(&((index as u32 + 1) * 0x1000).to_le_bytes());
            image[at+16..at+20].copy_from_slice(&0x200u32.to_le_bytes());
        }
        image[0x3240] = 0x77;
        image
    }
    fn snapshot(image: &[u8]) -> Result<BinaryFile<VirtualAddress64>, String> {
        snapshot_image(0x10000, |address, out| {
            let Some(offset) = address.checked_sub(0x10000) else {return false;};
            let Some(end) = offset.checked_add(out.len()) else {return false;};
            let Some(bytes) = image.get(offset..end) else {return false;};
            out.copy_from_slice(bytes); true
        })
    }
    #[test]
    fn loaded_snapshot_preserves_globals_in_virtual_data_tail() {
        let image = snapshot(&loaded_pe()).unwrap();
        let data = image.sections().find(|s| s.name == *b".data\0\0\0").unwrap();
        assert_eq!(data.data.len(), 0x500);
        assert_eq!(data.data[0x240], 0x77);
    }
    #[test]
    fn oversized_virtual_section_is_rejected_before_reading() {
        let mut image = loaded_pe();
        image[0x1d8+8..0x1d8+12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(snapshot(&image).is_err());
    }
}