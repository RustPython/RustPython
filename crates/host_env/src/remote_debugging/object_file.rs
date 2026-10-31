use super::{ReadBudget, invalid_data};
use object::{BinaryFormat, Object, ObjectSection, ObjectSegment};
use std::{fs::OpenOptions, io, io::Read, path::Path};

const MAX_OBJECT_BYTES: u64 = 256 * 1024 * 1024;

pub(super) fn read(path: &Path, budget: &mut ReadBudget) -> io::Result<Vec<u8>> {
    budget.consume(0)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A mapped pathname can be replaced with a FIFO while being inspected.
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_OBJECT_BYTES {
        return Err(invalid_data(
            "runtime candidate is not a bounded regular file",
        ));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(metadata.len() as usize)
        .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
    let result = file.take(MAX_OBJECT_BYTES + 1).read_to_end(&mut bytes);
    // A failed read may already have appended bytes. Account for them before
    // propagating the error, as well as before rejecting a grown file.
    budget.consume(bytes.len())?;
    result?;
    if bytes.len() as u64 > MAX_OBJECT_BYTES {
        return Err(invalid_data("runtime candidate grew beyond its size bound"));
    }
    Ok(bytes)
}

pub(super) fn section_address(
    bytes: &[u8],
    mapping_base: u64,
    format: BinaryFormat,
) -> io::Result<Option<u64>> {
    let file = object::File::parse(bytes).map_err(|_| invalid_data("invalid object file"))?;
    if file.format() != format {
        return Ok(None);
    }
    let name = match format {
        BinaryFormat::Elf => b".PyRuntime".as_slice(),
        BinaryFormat::MachO => b"PyRuntime".as_slice(),
        BinaryFormat::Pe => b"PyRuntim".as_slice(),
        _ => return Ok(None),
    };
    let section = if format == BinaryFormat::MachO {
        file.sections().find(|section| {
            section.name_bytes().ok() == Some(name)
                && section.segment_name_bytes().ok() == Some(Some(b"__DATA".as_slice()))
        })
    } else {
        file.section_by_name_bytes(name)
    };
    let Some(section) = section else {
        return Ok(None);
    };
    let image_base = match format {
        BinaryFormat::Elf => {
            // The ELF reader's segment iterator contains only PT_LOAD entries.
            let first = file
                .segments()
                .next()
                .ok_or_else(|| invalid_data("ELF image has no load segment"))?;
            let alignment = first.align();
            if alignment == 0 || !alignment.is_power_of_two() {
                return Err(invalid_data("invalid ELF load alignment"));
            }
            first.address() - first.address() % alignment
        }
        BinaryFormat::MachO => file
            .segments()
            .find(|segment| segment.name_bytes().ok() == Some(Some(b"__TEXT".as_slice())))
            .ok_or_else(|| invalid_data("Mach-O image has no text segment"))?
            .address(),
        BinaryFormat::Pe => file.relative_address_base(),
        _ => unreachable!(),
    };
    let relative = section
        .address()
        .checked_sub(image_base)
        .ok_or_else(|| invalid_data("runtime section precedes its image base"))?;
    let address = mapping_base
        .checked_add(relative)
        .ok_or_else(|| invalid_data("runtime section address overflow"))?;
    Ok((address != 0).then_some(address))
}
