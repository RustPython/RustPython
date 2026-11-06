//! Immutable, bounded instruction metadata for virtual monitoring.

use super::MonitoringLayerSite;
use crate::builtins::PyCode;
use rustpython_compiler_core::bytecode::{Instruction, oparg};
use std::collections::HashMap;

#[derive(Clone, Copy)]
pub(crate) struct MonitoringInstruction {
    pub(crate) base: Instruction,
    pub(crate) raw_arg: u8,
    // Sequential prefix decoding is for structural targets only. Dispatch must
    // retain its own argument state when control enters inside a prefix chain.
    pub(crate) full_arg: Option<u32>,
    pub(crate) cache_entries: usize,
    pub(crate) line: Option<i32>,
    pub(crate) line_eligible: bool,
    pub(crate) instruction_eligible: bool,
    pub(crate) jump_target: Option<usize>,
    pub(crate) for_iter_target: Option<usize>,
}

pub(crate) struct MonitoringMetadata {
    instructions: HashMap<usize, MonitoringInstruction>,
    pub(crate) first_traceable: Option<usize>,
    pub(crate) prefix_len: usize,
    pub(crate) line_info_valid: bool,
}

impl MonitoringMetadata {
    pub(crate) fn build(code: &PyCode) -> Self {
        let source = code.instructions.base_instruction_prefix();
        let mut instructions = HashMap::new();
        let mut starts = Vec::new();
        let mut index = 0usize;
        let mut prefix_arg = Some(0u32);
        let mut first_traceable = None;
        while index < source.len() {
            let base = source[index].op.deoptimize();
            let raw_arg = u8::from(source[index].arg);
            let cache_entries = base.cache_entries();
            let Some(end) = index.checked_add(1 + cache_entries) else {
                break;
            };
            if end > source.len() {
                break;
            }
            let full_arg = prefix_arg
                .and_then(|prefix| prefix.checked_mul(256)?.checked_add(u32::from(raw_arg)));
            prefix_arg = if matches!(base, Instruction::ExtendedArg) {
                full_arg
            } else {
                Some(0)
            };
            if first_traceable.is_none()
                && matches!(base, Instruction::Resume { .. })
                && u32::from(raw_arg) & oparg::ResumeContext::LOCATION_MASK
                    != oparg::ResumeLocation::AtGenExprStart.as_u32()
            {
                first_traceable = Some(index);
            }
            starts.push(index);
            instructions.insert(
                index,
                MonitoringInstruction {
                    base,
                    raw_arg,
                    full_arg,
                    cache_entries,
                    line: None,
                    line_eligible: false,
                    instruction_eligible: first_traceable.is_some()
                        && !matches!(base, Instruction::Resume { .. } | Instruction::EndFor),
                    jump_target: None,
                    for_iter_target: None,
                },
            );
            index = end;
        }
        let prefix_len = index;
        let first_line = code
            .first_line_number
            .map_or(Some(0), |line| i32::try_from(line.get()).ok());
        let lines = first_line.and_then(|first_line| {
            read_lines(code.co_linetable.as_bytes(), first_line, prefix_len)
        });
        let handlers =
            exception_targets(code.co_exceptiontable.as_bytes(), code.instructions.len());
        let line_info_valid = lines.is_some()
            && handlers.as_ref().is_some_and(|handlers| {
                handlers.iter().all(|&(start, end, target)| {
                    (start >= prefix_len || instructions.contains_key(&start))
                        && (end >= prefix_len || instructions.contains_key(&end))
                        && (target >= prefix_len || instructions.contains_key(&target))
                })
            });
        let mut current_line = None;
        for &index in &starts {
            let operation = instructions.get_mut(&index).unwrap();
            operation.line = lines.as_ref().and_then(|lines| lines[index]);
            if first_traceable.is_some_and(|first| index >= first)
                && !matches!(
                    operation.base,
                    Instruction::EndAsyncFor
                        | Instruction::EndFor
                        | Instruction::EndSend
                        | Instruction::Resume { .. }
                        | Instruction::PopIter
                )
            {
                operation.line_eligible =
                    line_info_valid && operation.line.is_some() && operation.line != current_line;
                // NO_LOCATION advances current_line too. Excluded operations
                // above leave it unchanged, matching initialize_lines.
                current_line = operation.line;
            }
        }
        let mut line_targets = Vec::new();
        for &index in &starts {
            let operation = instructions[&index];
            let Some(arg) = operation.full_arg.and_then(|arg| usize::try_from(arg).ok()) else {
                continue;
            };
            let end = index + 1 + operation.cache_entries;
            let target = match operation.base {
                Instruction::PopJumpIfFalse { .. }
                | Instruction::PopJumpIfTrue { .. }
                | Instruction::PopJumpIfNone { .. }
                | Instruction::PopJumpIfNotNone { .. }
                | Instruction::JumpForward { .. }
                | Instruction::ForIter { .. }
                | Instruction::Send { .. } => end.checked_add(arg),
                Instruction::JumpBackward { .. } | Instruction::JumpBackwardNoInterrupt { .. } => {
                    end.checked_sub(arg)
                }
                _ => None,
            };
            let target = target.filter(|target| instructions.contains_key(target));
            instructions.get_mut(&index).unwrap().jump_target = target;
            if let Some(target) = target {
                if matches!(operation.base, Instruction::ForIter { .. }) {
                    let continuation = if matches!(instructions[&target].base, Instruction::EndFor)
                    {
                        target.checked_add(1)
                    } else {
                        Some(target)
                    };
                    instructions.get_mut(&index).unwrap().for_iter_target =
                        continuation.filter(|target| instructions.contains_key(target));
                }
                let line_target = if matches!(
                    operation.base,
                    Instruction::ForIter { .. } | Instruction::Send { .. }
                ) {
                    target.checked_add(1)
                } else {
                    Some(target)
                };
                if let Some(target) = line_target.filter(|target| instructions.contains_key(target))
                    && first_traceable.is_some_and(|first| index >= first)
                {
                    line_targets.push(target);
                }
            }
        }
        if line_info_valid {
            for target in line_targets {
                let operation = instructions.get_mut(&target).unwrap();
                if operation.line.is_some() && !matches!(operation.base, Instruction::PopIter) {
                    operation.line_eligible = true;
                }
            }
            for (_, _, target) in handlers.unwrap() {
                if let Some(operation) = instructions.get_mut(&target)
                    && operation.line.is_some()
                    && !matches!(operation.base, Instruction::EndAsyncFor)
                {
                    operation.line_eligible = true;
                }
            }
        }
        Self {
            instructions,
            first_traceable,
            prefix_len,
            line_info_valid,
        }
    }

    pub(crate) fn get(&self, index: usize) -> Option<&MonitoringInstruction> {
        self.instructions.get(&index)
    }

    pub(crate) fn layer_sites(&self) -> HashMap<usize, MonitoringLayerSite> {
        self.instructions
            .iter()
            .filter_map(|(&index, operation)| {
                self.first_traceable
                    .filter(|first| index >= *first)
                    .map(|_| {
                        (
                            index,
                            MonitoringLayerSite::new(
                                operation.base,
                                operation.raw_arg,
                                operation.line_eligible,
                                operation.instruction_eligible,
                            ),
                        )
                    })
            })
            .collect()
    }
}

// Location-table varints use little-endian six-bit groups. Reject truncation
// and overflow internally rather than inventing a default source line.
fn location_varint(data: &[u8], pos: &mut usize) -> Option<u32> {
    let mut value = 0u32;
    let mut shift = 0u32;
    loop {
        let byte = *data.get(*pos)?;
        *pos += 1;
        let part = u32::from(byte & 0x3f);
        if shift >= 32 || part > (u32::MAX >> shift) {
            return None;
        }
        value |= part << shift;
        if byte & 0x40 == 0 {
            return Some(value);
        }
        shift += 6;
    }
}

fn signed_location_varint(data: &[u8], pos: &mut usize) -> Option<i32> {
    let value = location_varint(data, pos)?;
    let magnitude = i32::try_from(value >> 1).ok()?;
    Some(if value & 1 == 0 {
        magnitude
    } else {
        -magnitude
    })
}

fn read_lines(data: &[u8], first_line: i32, length: usize) -> Option<Vec<Option<i32>>> {
    let mut lines = Vec::with_capacity(length);
    let mut pos = 0usize;
    let mut line = first_line;
    while lines.len() < length {
        let header = *data.get(pos)?;
        pos += 1;
        if header & 0x80 == 0 {
            return None;
        }
        let kind = (header >> 3) & 0x0f;
        let count = usize::from(header & 7) + 1;
        let delta = match kind {
            0..=9 => {
                data.get(pos)?;
                pos += 1;
                0
            }
            10..=12 => {
                data.get(pos..pos.checked_add(2)?)?;
                pos += 2;
                i32::from(kind - 10)
            }
            13 => signed_location_varint(data, &mut pos)?,
            14 => {
                let delta = signed_location_varint(data, &mut pos)?;
                for _ in 0..3 {
                    location_varint(data, &mut pos)?;
                }
                delta
            }
            15 => 0,
            _ => return None,
        };
        line = line.checked_add(delta)?;
        let value = if kind == 15 || line < 0 {
            None
        } else {
            Some(line)
        };
        lines.extend(core::iter::repeat_n(value, count.min(length - lines.len())));
    }
    Some(lines)
}

// Exception-table varints use big-endian six-bit groups, unlike locations.
fn exception_varint(data: &[u8], pos: &mut usize) -> Option<usize> {
    let mut value = 0usize;
    loop {
        let byte = *data.get(*pos)?;
        *pos += 1;
        value = value
            .checked_mul(64)?
            .checked_add(usize::from(byte & 0x3f))?;
        if byte & 0x40 == 0 {
            return Some(value);
        }
    }
}

fn exception_targets(data: &[u8], code_length: usize) -> Option<Vec<(usize, usize, usize)>> {
    let mut targets = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        let start = exception_varint(data, &mut pos)?;
        let size = exception_varint(data, &mut pos)?;
        let target = exception_varint(data, &mut pos)?;
        exception_varint(data, &mut pos)?;
        if start >= code_length || start.checked_add(size)? > code_length || target >= code_length {
            return None;
        }
        targets.push((start, start.checked_add(size)?, target));
    }
    Some(targets)
}
