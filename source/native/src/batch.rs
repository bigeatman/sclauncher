//! Pure, bounded command planner. No process or game access.
//!
//! Selection wire layout is documented in ShieldBattery game/src/bw/commands.rs:
//! https://github.com/ShieldBattery/ShieldBattery/blob/master/game/src/bw/commands.rs
//! That source's length table also records 0x60 as 12 bytes and 0x61 as 13 bytes.
//! Stop/hold layouts: BWAPI/bwapi/BWAPI/Source/BW/OrderTypes.h.
//! Only exact-size, caller-validated controllable commands are accepted.

use std::collections::HashSet;

pub const SELECTION_LIMIT: usize = 12;
pub const MAX_UNIT_IDS: usize = 8192;
/// A conservative planning limit, not a claim about the game's transport capacity.
pub const DEFAULT_BUDGET: usize = 1024;

/// Plan select/order pairs, then restore the original selection.
///
/// `ids` must contain current, controllable, same-type unit IDs already validated
/// by the caller. `restore[0]` is the reference unit and must occur in `ids`.
/// The other restore entries need not have the same type. Both slices must have
/// unique nonzero IDs; `ids` has 1..=8192 entries and `restore` has 1..=12.
///
/// The command must be one complete 0x60 (12-byte), 0x61 (13-byte), 0x1a
/// (2-byte stop) or 0x2b (2-byte hold) command, with a 0/1 queue flag.
/// Carrier/Reaver stop variants 0x1b/0x1c are exactly one byte, without a queue flag.
/// Unit train/morph 0x1f/0x23 are 3-byte records with a u16 unit kind, no queue flag.
/// Its fields, legality and suitability for every selected unit are the caller's
/// responsibility. Payload bytes are copied unchanged. In particular, the
/// planner cannot verify ownership, type, liveness, ID reuse/generation, current
/// match identity, command targets or the available outgoing buffer capacity.
///
/// Every selection contains at most 12 IDs. The byte budget includes every
/// selection, every repeated order, and the final restore. Failure returns no
/// partial plan and has no side effects. A successful plan does not reserve any
/// game buffer space: the caller must recheck capacity and commit it atomically.
pub fn plan(
    ids: &[u32],
    restore: &[u32],
    command: &[u8],
    budget: usize,
) -> Result<Vec<Vec<u8>>, &'static str> {
    if ids.is_empty() || ids.len() > MAX_UNIT_IDS {
        return Err("unit count must be between 1 and 8192");
    }
    if restore.is_empty() || restore.len() > SELECTION_LIMIT {
        return Err("restore selection must contain 1 to 12 units");
    }
    validated_command(command)?;
    validate_ids(ids)?;
    validate_ids(restore)?;
    if !ids.contains(&restore[0]) {
        return Err("reference unit is not in the planned selection");
    }

    // Inputs are bounded above before these calculations. Checked arithmetic
    // also keeps this invariant explicit should those bounds change later.
    let batches = ids.len().div_ceil(SELECTION_LIMIT);
    let total = ids
        .len()
        .checked_mul(4)
        .and_then(|n| {
            batches
                .checked_mul(2 + command.len())
                .and_then(|v| n.checked_add(v))
        })
        .and_then(|n| {
            restore
                .len()
                .checked_mul(4)
                .and_then(|v| v.checked_add(2))
                .and_then(|v| n.checked_add(v))
        })
        .ok_or("plan size overflow")?;
    if total > budget {
        return Err("complete plan exceeds byte budget");
    }

    let mut result = Vec::with_capacity(batches * 2 + 1);
    for batch in ids.chunks(SELECTION_LIMIT) {
        result.push(selection(batch));
        result.push(command.to_vec());
    }
    result.push(selection(restore));
    debug_assert_eq!(result.iter().map(Vec::len).sum::<usize>(), total);
    Ok(result)
}

/// Validate only the supported opcode, exact byte length and queue flag.
/// Unit/target/order semantics must already be validated by the caller.
pub fn validated_command(command: &[u8]) -> Result<(), &'static str> {
    if matches!(command.first(),Some(0x1f | 0x23)) {
        if command.len()!=3 {return Err("production must contain exactly one complete record");}
        if u16::from_le_bytes([command[1],command[2]])>=106 {return Err("production type is not a unit");}
        return Ok(());
    }
    let expected_len=match command.first() {
        Some(0x60)=>12,Some(0x61)=>13,Some(0x1a | 0x2b)=>2,Some(0x1b | 0x1c)=>1,
        _=>return Err("unsupported command"),
    };
    if command.len()!=expected_len {return Err("command must contain exactly one complete order");}
    if expected_len>1 && command[expected_len-1]>1 {return Err("queue flag must be zero or one");}
    Ok(())
}

/// One independently committable select/order/restore batch.
/// No reference membership check is made because later batches need not contain
/// the reference unit. Both selections must contain 1..=12 unique nonzero IDs.
/// The caller must commit all three records together or send none of them.
pub fn plan_one(
    ids: &[u32],
    restore: &[u32],
    command: &[u8],
    budget: usize,
) -> Result<Vec<Vec<u8>>, &'static str> {
    if ids.is_empty() || ids.len() > SELECTION_LIMIT {
        return Err("batch selection must contain 1 to 12 units");
    }
    if restore.is_empty() || restore.len() > SELECTION_LIMIT {
        return Err("restore selection must contain 1 to 12 units");
    }
    validated_command(command)?;
    validate_ids(ids)?;
    validate_ids(restore)?;
    let total = 4 + (ids.len() + restore.len()) * 4 + command.len();
    if total > budget {
        return Err("complete plan exceeds byte budget");
    }
    Ok(vec![selection(ids), command.to_vec(), selection(restore)])
}

fn validate_ids(ids: &[u32]) -> Result<(), &'static str> {
    let mut seen = HashSet::with_capacity(ids.len());
    for &id in ids {
        if id == 0 {
            return Err("unit ID must be nonzero");
        }
        if !seen.insert(id) {
            return Err("duplicate unit ID");
        }
    }
    Ok(())
}

fn selection(ids: &[u32]) -> Vec<u8> {
    debug_assert!(!ids.is_empty() && ids.len() <= SELECTION_LIMIT);
    let mut result = Vec::with_capacity(2 + ids.len() * 4);
    result.push(0x63);
    result.push(ids.len() as u8);
    for id in ids {
        result.extend_from_slice(&id.to_le_bytes());
    }
    result
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
        let mut bytes: Vec<u8> = (0..len).collect();
        bytes[0] = op;
        bytes[len as usize - 1] = 0;
        bytes
    }
    fn decode_selection(bytes: &[u8]) -> Vec<u32> {
        assert_eq!(bytes[0], 0x63);
        assert!((1..=12).contains(&bytes[1]));
        assert_eq!(bytes.len(), 2 + bytes[1] as usize * 4);
        bytes[2..]
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .collect()
    }

    #[test]
    fn fifteen_units_produce_twelve_then_three_and_restore() {
        let ids: Vec<u32> = (1..=15).collect();
        let cmd = order(0x60);
        let output = plan(&ids, &[5], &cmd, DEFAULT_BUDGET).unwrap();
        assert_eq!(output.len(), 5);
        assert_eq!(decode_selection(&output[0]), ids[..12]);
        assert_eq!(decode_selection(&output[2]), ids[12..]);
        assert_eq!(output[1], cmd);
        assert_eq!(output[3], cmd);
        assert_eq!(decode_selection(&output[4]), vec![5]);
    }
    #[test]
    fn all_supported_command_bytes_pass_through_unchanged() {
        for op in [0x60, 0x61, 0x1a, 0x2b] {
            let cmd = order(op);
            let output = plan(&[0x1234_abcd], &[0x1234_abcd], &cmd, 128).unwrap();
            assert_eq!(output[0], [0x63, 1, 0xcd, 0xab, 0x34, 0x12]);
            assert_eq!(output[1], cmd);
        }
    }
    #[test]
    fn exact_budget_includes_final_restore() {
        let ids: Vec<u32> = (1..=15).collect();
        let cmd = order(0x60);
        // 50 + 12 + 14 + 12 + 6 = 94 bytes.
        assert!(plan(&ids, &[1], &cmd, 94).is_ok());
        assert_eq!(
            plan(&ids, &[1], &cmd, 93).unwrap_err(),
            "complete plan exceeds byte budget"
        );
        assert!(plan(&ids, &[1], &cmd, 0).is_err());
    }
    #[test]
    fn restores_every_original_id_in_original_order() {
        let ids: Vec<u32> = (1..=24).collect();
        let restore = [7, 99, 3, 2];
        let output = plan(&ids, &restore, &order(0x61), 1024).unwrap();
        assert_eq!(decode_selection(output.last().unwrap()), restore);
    }
    #[test]
    fn rejects_zero_duplicate_and_missing_reference_ids() {
        let cmd = order(0x60);
        for ids in [vec![0], vec![1, 1], vec![1, 0], vec![2, 3]] {
            assert!(plan(&ids, &[1], &cmd, 1024).is_err());
        }
        for restore in [vec![0], vec![1, 1], vec![1, 0], vec![]] {
            assert!(plan(&[1, 2], &restore, &cmd, 1024).is_err());
        }
    }
    #[test]
    fn rejects_empty_and_excessive_counts() {
        let cmd = order(0x60);
        assert!(plan(&[], &[1], &cmd, usize::MAX).is_err());
        let too_many: Vec<u32> = (1..=MAX_UNIT_IDS as u32 + 1).collect();
        assert!(plan(&too_many, &[1], &cmd, usize::MAX).is_err());
        let restore: Vec<u32> = (1..=13).collect();
        assert!(plan(&restore, &restore, &cmd, usize::MAX).is_err());
    }
    #[test]
    fn maximum_count_stays_bounded_and_never_exceeds_twelve() {
        let ids: Vec<u32> = (1..=MAX_UNIT_IDS as u32).collect();
        assert!(plan(&ids, &[1], &order(0x60), DEFAULT_BUDGET).is_err());
        let output = plan(&ids, &[1], &order(0x60), usize::MAX).unwrap();
        let flattened: Vec<u32> = output[..output.len() - 1]
            .iter()
            .step_by(2)
            .flat_map(|bytes| decode_selection(bytes))
            .collect();
        assert_eq!(flattened, ids);
    }
    #[test]
    fn rejects_unrecognized_or_concatenated_commands() {
        for op in 0..=u8::MAX {
            if ![0x60, 0x61, 0x1a, 0x1b, 0x1c, 0x2b].contains(&op) {
                assert!(plan(&[1], &[1], &[op], 1024).is_err());
            }
        }
        for op in [0x1b,0x1c] {
            let records=plan(&[1],&[1],&[op],1024).unwrap();
            assert_eq!(records[1],vec![op]);
            assert!(plan(&[1],&[1],&[op,0],1024).is_err());
            assert!(plan(&[1],&[1],&[op,op],1024).is_err());
        }
        assert!(plan(&[1], &[1], &[], 1024).is_err());
        for op in [0x60, 0x61, 0x1a, 0x2b] {
            let mut cmd = order(op);
            cmd.pop();
            assert!(plan(&[1], &[1], &cmd, 1024).is_err());
            let mut cmd = order(op);
            cmd.push(0x05);
            assert!(plan(&[1], &[1], &cmd, 1024).is_err());
            let mut cmd = order(op);
            cmd.extend_from_slice(&order(op));
            assert!(plan(&[1], &[1], &cmd, 1024).is_err());
        }
    }
    #[test]
    fn selection_boundaries_have_no_empty_extra_batch() {
        for count in [1, 12, 13, 24, 25, 255, 256] {
            let ids: Vec<u32> = (1..=count).collect();
            let output = plan(&ids, &[1], &order(0x60), usize::MAX).unwrap();
            assert_eq!(output.len(), (count as usize).div_ceil(12) * 2 + 1);
        }
    }
    #[test]
    fn player_slots_do_not_change_planning_or_share_state() {
        let cmd = order(0x60);
        for slot in 0..8u32 {
            // Slots are deliberately absent from the wire selection/planner;
            // the caller determines ownership before supplying IDs.
            let ids = [0x1000 + slot * 2, 0x1001 + slot * 2];
            let output = plan(&ids, &ids[..1], &cmd, 1024).unwrap();
            assert_eq!(decode_selection(&output[0]), ids);
            assert_eq!(output[1], cmd);
        }
    }
    #[test]
    fn caller_supplied_generation_bits_are_preserved_without_guessing() {
        let ids = [0x0100_0001, 0x0200_0001, u32::MAX];
        let output = plan(&ids, &ids[..1], &order(0x61), 1024).unwrap();
        assert_eq!(decode_selection(&output[0]), ids);
        // Matching an index to a current generation is a caller-side check;
        // this module has no unit table or time-dependent world state.
    }
    #[test]
    fn single_batch_can_restore_reference_from_a_different_batch() {
        let output = plan_one(&[13, 14, 15], &[1], &order(0x60), 32).unwrap();
        assert_eq!(output.len(), 3);
        assert_eq!(decode_selection(&output[0]), [13, 14, 15]);
        assert_eq!(decode_selection(&output[2]), [1]);
        assert!(plan_one(&[13, 14, 15], &[1], &order(0x60), 31).is_err());
    }
    #[test]
    fn single_batch_checks_both_selections_and_command() {
        let too_many: Vec<u32> = (1..=13).collect();
        let cmd = order(0x60);
        for ids in [vec![], vec![0], vec![1, 1], too_many.clone()] {
            assert!(plan_one(&ids, &[1], &cmd, 1024).is_err());
            assert!(plan_one(&[1], &ids, &cmd, 1024).is_err());
        }
        assert!(plan_one(&[1], &[1], &[0x05], 1024).is_err());
    }
    #[test]
    fn queue_flag_is_checked_for_all_supported_controls() {
        for op in [0x60, 0x61, 0x1a, 0x2b] {
            for flag in 0..=255 {
                let mut cmd = order(op);
                *cmd.last_mut().unwrap() = flag;
                assert_eq!(validated_command(&cmd).is_ok(), flag <= 1);
                assert_eq!(plan(&[1], &[1], &cmd, 1024).is_ok(), flag <= 1);
                assert_eq!(plan_one(&[1], &[2], &cmd, 1024).is_ok(), flag <= 1);
            }
        }
    }
}
