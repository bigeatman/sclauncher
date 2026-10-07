//! Pure observation of one synchronous UI callback's outgoing-buffer delta.
//!
//! The caller supplies complete snapshots of the occupied buffer before and
//! after calling the original callback, on the same game thread and frame. No
//! process access, memory write or scan for embedded opcode-like bytes occurs.
//!
//! This is a prototype: equal bytes do not prove that no command was sent. A
//! flush followed by recreation of the same prefix (ABA) cannot be detected
//! without an independently verified monotonic flush epoch. `NoObservedAppend`
//! deliberately makes no stronger claim. The caller must reject callbacks if
//! thread/frame/session identity or any other causal guard changes, and suppress
//! observation around its own synthetic appends.

use crate::batch;
use std::collections::HashSet;

/// Hard upper bound for caller-owned snapshot slices. The caller may use 480.
pub const MAX_SNAPSHOT_BYTES: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Observation {
    /// Same occupied-buffer bytes; command absence is not established.
    NoObservedAppend,
    /// Exactly one supported complete command was appended to an intact prefix.
    Command(Vec<u8>),
    /// An observed append cannot safely keep the existing virtual selection.
    Clear(ClearReason),
    /// Snapshots cannot establish a single causal append; stop the mode.
    Ambiguous(AmbiguityReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClearReason {
    ManualSelection,
    UnsupportedOrIncompleteRecord,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AmbiguityReason {
    InvalidSnapshotLimit,
    SnapshotExceedsLimit,
    LengthDecreased,
    PrefixChanged,
}

/// Classify only bytes appended after an unchanged occupied-buffer prefix.
///
/// A supported opcode must occupy the entire delta. Multiple commands, trailing
/// bytes, incomplete records and bad queue flags are cleared, never split or
/// searched. A selection/hotkey record at the delta's first byte clears mode;
/// bytes later in an otherwise valid control payload have no such meaning.
/// Snapshot read failure must be handled by the caller before invoking this.
pub fn observe_append(before: &[u8], after: &[u8], limit: usize) -> Observation {
    if limit == 0 || limit > MAX_SNAPSHOT_BYTES {
        return Observation::Ambiguous(AmbiguityReason::InvalidSnapshotLimit);
    }
    if before.len() > limit || after.len() > limit {
        return Observation::Ambiguous(AmbiguityReason::SnapshotExceedsLimit);
    }
    if after.len() < before.len() {
        return Observation::Ambiguous(AmbiguityReason::LengthDecreased);
    }
    if !after.starts_with(before) {
        return Observation::Ambiguous(AmbiguityReason::PrefixChanged);
    }
    let delta = &after[before.len()..];
    if delta.is_empty() {
        return Observation::NoObservedAppend;
    }
    if matches!(delta[0], 0x09..=0x0b | 0x13 | 0x63..=0x65) {
        return Observation::Clear(ClearReason::ManualSelection);
    }
    if batch::validated_command(delta).is_err() {
        return Observation::Clear(ClearReason::UnsupportedOrIncompleteRecord);
    }
    Observation::Command(delta.to_vec())
}

/// A Larva production UI may synchronize its unchanged original selection
/// immediately before one Morph command. Accept only that exact two-record
/// shape and only when the caller supplied the validated pre-callback IDs.
/// Arbitrary selections, additional commands and payload opcode searches remain
/// rejected by the ordinary observer.
pub fn observe_control_append(before: &[u8], after: &[u8], limit: usize,
    larva_original: Option<&[u32]>) -> Observation
{
    let ordinary = observe_append(before, after, limit);
    if ordinary != Observation::Clear(ClearReason::ManualSelection) { return ordinary; }
    let Some(ids) = larva_original else { return ordinary; };
    if ids.is_empty() || ids.len() > batch::SELECTION_LIMIT || unique_nonzero(ids).is_err() {
        return ordinary;
    }
    // observe_append already proved limit, intact prefix and non-empty delta.
    let delta = &after[before.len()..];
    let selection_length = 2 + ids.len() * 4;
    if delta.len() != selection_length + 3 || delta[0] != 0x63
        || delta[1] as usize != ids.len() { return ordinary; }
    if delta[2..selection_length].chunks_exact(4).zip(ids)
        .any(|(wire, id)| u32::from_le_bytes(wire.try_into().unwrap()) != *id) {
        return ordinary;
    }
    let command = &delta[selection_length..];
    if command[0] != 0x23 || batch::validated_command(command).is_err() { return ordinary; }
    Observation::Command(command.to_vec())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CopyIntent {
    /// Every matching unit already received the original control.
    NoCopies,
    /// IDs still needing the command, in caller-supplied matching order.
    Copies {
        ids: Vec<u32>,
        /// Preserve the original selection, in its original order.
        restore_ids: Vec<u32>,
        command: Vec<u8>,
    },
}

/// Exclude every unit that received the original synchronous callback's command.
///
/// Both input sets are bounded and must contain unique nonzero IDs. The original
/// selection must be 1..=12 IDs, and every original ID must belong to matching.
/// This conservative prototype accepts no mixed-type original selection. The
/// caller remains responsible for ownership, type, liveness, ID generations,
/// command target/semantics and snapshotting the selection before the callback.
///
/// `Copies.ids` may be split into <=12-unit chunks. Each chunk must be planned
/// with `batch::plan_one`, using the complete `restore_ids` and revalidated
/// against the current world immediately before an atomic buffer commit. The
/// normal original command must never be suppressed or replayed for restore IDs.
pub fn build_copy_intent(
    matching_ids: &[u32],
    original_selection_ids: &[u32],
    command: &[u8],
) -> Result<CopyIntent, &'static str> {
    batch::validated_command(command)?;
    if matching_ids.is_empty() || matching_ids.len() > batch::MAX_UNIT_IDS {
        return Err("matching unit count must be between 1 and 8192");
    }
    if original_selection_ids.is_empty() || original_selection_ids.len() > batch::SELECTION_LIMIT {
        return Err("original selection must contain 1 to 12 units");
    }
    let matching = unique_nonzero(matching_ids)?;
    let original = unique_nonzero(original_selection_ids)?;
    if !original.is_subset(&matching) {
        return Err("original selected unit is not in matching units");
    }
    let ids: Vec<u32> = matching_ids
        .iter()
        .copied()
        .filter(|id| !original.contains(id))
        .collect();
    if ids.is_empty() {
        return Ok(CopyIntent::NoCopies);
    }
    Ok(CopyIntent::Copies {
        ids,
        restore_ids: original_selection_ids.to_vec(),
        command: command.to_vec(),
    })
}

fn unique_nonzero(ids: &[u32]) -> Result<HashSet<u32>, &'static str> {
    let mut seen = HashSet::with_capacity(ids.len());
    for &id in ids {
        if id == 0 {
            return Err("unit ID must be nonzero");
        }
        if !seen.insert(id) {
            return Err("duplicate unit ID");
        }
    }
    Ok(seen)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(op: u8) -> Vec<u8> {
        let len = match op {
            0x60 => 12,
            0x61 => 13,
            _ => 2,
        };
        let mut bytes = vec![0; len];
        bytes[0] = op;
        bytes
    }

    #[test]
    fn supported_single_controls_pass_through_unchanged() {
        for op in [0x60, 0x61, 0x1a, 0x2b] {
            for prefix in [vec![], vec![0x63, 1, 8, 0, 0, 0]] {
                let cmd = order(op);
                let mut after = prefix.clone();
                after.extend_from_slice(&cmd);
                assert_eq!(
                    observe_append(&prefix, &after, 480),
                    Observation::Command(cmd)
                );
            }
        }
    }

    #[test]
    fn equal_snapshots_only_report_no_observed_append() {
        assert_eq!(observe_append(&[], &[], 480), Observation::NoObservedAppend);
        let bytes = order(0x60);
        assert_eq!(
            observe_append(&bytes, &bytes, 480),
            Observation::NoObservedAppend
        );
    }

    #[test]
    fn decreased_length_or_changed_prefix_is_ambiguous() {
        assert_eq!(
            observe_append(&[1, 2], &[1], 480),
            Observation::Ambiguous(AmbiguityReason::LengthDecreased)
        );
        assert_eq!(
            observe_append(&[1, 2], &[1, 3], 480),
            Observation::Ambiguous(AmbiguityReason::PrefixChanged)
        );
        let cmd = order(0x60);
        assert_eq!(
            observe_append(&[7], &cmd, 480),
            Observation::Ambiguous(AmbiguityReason::PrefixChanged)
        );
    }

    #[test]
    fn respects_conservative_snapshot_limits() {
        for limit in [0, 513, usize::MAX] {
            assert_eq!(
                observe_append(&[], &[], limit),
                Observation::Ambiguous(AmbiguityReason::InvalidSnapshotLimit)
            );
        }
        for (before, after) in [(vec![0; 481], vec![0; 481]), (vec![0; 480], vec![0; 481])] {
            assert_eq!(
                observe_append(&before, &after, 480),
                Observation::Ambiguous(AmbiguityReason::SnapshotExceedsLimit)
            );
        }
        let mut before = vec![0; 478];
        let mut after = before.clone();
        after.extend_from_slice(&order(0x1a));
        assert!(matches!(
            observe_append(&before, &after, 480),
            Observation::Command(_)
        ));
        before.resize(512, 0);
        assert_eq!(
            observe_append(&before, &before, 512),
            Observation::NoObservedAppend
        );
    }

    #[test]
    fn manual_selection_and_hotkey_at_first_record_clear_mode() {
        for op in [0x09, 0x0a, 0x0b, 0x13, 0x63, 0x64, 0x65] {
            assert_eq!(
                observe_append(&[0x07], &[0x07, op], 480),
                Observation::Clear(ClearReason::ManualSelection)
            );
        }
    }

    #[test]
    fn payload_selection_like_bytes_are_never_searched() {
        let mut cmd = order(0x60);
        cmd[1..8].copy_from_slice(&[0x09, 0x0a, 0x0b, 0x13, 0x63, 0x64, 0x65]);
        assert_eq!(observe_append(&[], &cmd, 480), Observation::Command(cmd));
    }

    #[test]
    fn unrecognized_incomplete_invalid_or_multiple_records_clear_mode() {
        for op in [0x60, 0x61, 0x1a, 0x2b] {
            let cmd = order(op);
            let mut concatenated = cmd.clone();
            concatenated.extend_from_slice(&cmd);
            let mut trailing = cmd.clone();
            trailing.push(0x00);
            let mut bad_queue = cmd.clone();
            *bad_queue.last_mut().unwrap() = 2;
            for bytes in [
                cmd[..cmd.len() - 1].to_vec(),
                concatenated,
                trailing,
                bad_queue,
            ] {
                assert_eq!(
                    observe_append(&[], &bytes, 480),
                    Observation::Clear(ClearReason::UnsupportedOrIncompleteRecord)
                );
            }
        }
        for op in 0..=255 {
            if [0x1b,0x1c].contains(&op) {
                assert_eq!(observe_append(&[],&[op],480),Observation::Command(vec![op]));
                assert_eq!(observe_append(&[],&[op,0],480),Observation::Clear(ClearReason::UnsupportedOrIncompleteRecord));
                continue;
            }
            if [0x09, 0x0a, 0x0b, 0x13, 0x63, 0x64, 0x65].contains(&op) {
                continue;
            }
            assert_eq!(
                observe_append(&[], &[op], 480),
                Observation::Clear(ClearReason::UnsupportedOrIncompleteRecord)
            );
        }
    }

    #[test]
    fn copies_exclude_every_originally_selected_unit_and_preserve_orders() {
        let cmd = order(0x60);
        assert_eq!(
            build_copy_intent(&[8, 4, 9, 3, 5], &[9, 8], &cmd).unwrap(),
            CopyIntent::Copies {
                ids: vec![4, 3, 5],
                restore_ids: vec![9, 8],
                command: cmd
            }
        );
    }

    #[test]
    fn no_copies_when_all_matching_units_already_received_original_order() {
        assert_eq!(
            build_copy_intent(&[8, 4], &[4, 8], &order(0x60)).unwrap(),
            CopyIntent::NoCopies
        );
    }

    #[test]
    fn invalid_selection_or_matching_ids_are_rejected() {
        let cmd = order(0x60);
        for matching in [vec![], vec![0], vec![1, 1], vec![2], vec![1, 0]] {
            assert!(build_copy_intent(&matching, &[1], &cmd).is_err());
        }
        for original in [vec![], vec![0], vec![1, 1], vec![2], vec![1, 0]] {
            assert!(build_copy_intent(&[1], &original, &cmd).is_err());
        }
        let too_many_matching: Vec<u32> = (1..=8193).collect();
        let too_many_original: Vec<u32> = (1..=13).collect();
        assert!(build_copy_intent(&too_many_matching, &[1], &cmd).is_err());
        assert!(build_copy_intent(&too_many_original, &too_many_original, &cmd).is_err());
        assert!(build_copy_intent(&[1], &[1], &[0x05]).is_err());
    }

    #[test]
    fn fifteen_matches_create_fourteen_copies_then_restore_each_batch() {
        let ids: Vec<u32> = (1..=15).collect();
        let cmd = order(0x61);
        let CopyIntent::Copies {
            ids: copies,
            restore_ids,
            command,
        } = build_copy_intent(&ids, &[1], &cmd).unwrap()
        else {
            panic!("expected copies")
        };
        assert_eq!(copies, (2..=15).collect::<Vec<_>>());
        let plans: Vec<_> = copies
            .chunks(12)
            .map(|chunk| batch::plan_one(chunk, &restore_ids, &command, 480).unwrap())
            .collect();
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0][0][1], 12);
        assert_eq!(plans[1][0][1], 2);
        for plan in plans {
            assert_eq!(plan[2], [0x63, 1, 1, 0, 0, 0]);
            assert_eq!(plan[1], cmd);
        }
    }

    #[test]
    fn maximum_matching_count_is_bounded_and_generation_bits_are_preserved() {
        let ids: Vec<u32> = (1..=8192).map(|n| n | 0xab00_0000).collect();
        let CopyIntent::Copies {
            ids: copies,
            restore_ids,
            ..
        } = build_copy_intent(&ids, &[ids[99]], &order(0x2b)).unwrap()
        else {
            panic!("expected copies")
        };
        assert_eq!(copies.len(), 8191);
        assert!(!copies.contains(&ids[99]));
        assert_eq!(restore_ids, [ids[99]]);
        assert_eq!(copies[0], ids[0]);
        assert_eq!(copies.last(), ids.last());
    }
}
