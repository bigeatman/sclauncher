//! Strict, bounded requests and pure merging of one native diplomacy callback.
//!
//! This module has no process, window, filesystem, or game-callback access.
//! Session/generation/frame, sender, callback identity, and flush-epoch checks
//! remain the caller's responsibility before using a captured buffer delta.
//! A request never supplies a raw command or changes another player's row.

use crate::alliance::{self, Snapshot};

pub(crate) const MAX_REQUEST_BYTES: usize = 1024;
pub(crate) const MAX_CHANGES: usize = alliance::PLAYABLE_SLOTS - 1;
pub(crate) const MAX_CAPTURE_BYTES: usize = 512;
const MAGIC: &str = "SCALLYEDIT1";
const APPLY_MAGIC: &str = "SCALLYAPPLY1";
const VISION_COMMAND: u8 = 0x0d;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Edit {
    pub(crate) slot: u8,
    pub(crate) expected: u8,
    pub(crate) desired: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Edits {
    pub(crate) pid: u32,
    pub(crate) session: u64,
    pub(crate) generation: u64,
    pub(crate) frame: u32,
    pub(crate) request_id: u64,
    pub(crate) changes: Vec<Edit>,
}

fn decimal(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn normal_toggle(edit: Edit) -> bool {
    matches!((edit.expected, edit.desired), (0, 1) | (1, 0) | (2, 0))
}

/// Header: SCALLYEDIT1\tpid\tsession\tgeneration\tframe\trequestId\tcount
/// Rows: E\tslot\texpected\tdesired, strictly increasing slots in 0..=7.
/// LF and CRLF are accepted; one final line ending is optional. All other
/// whitespace, blank rows, extra fields, signs, non-ASCII, and overflow fail.
/// Zero rows mean "clear this staged request" and are otherwise well-formed.
pub(crate) fn parse(bytes: &[u8]) -> Option<Edits> {
    parse_with_magic(bytes, MAGIC)
}

/// An explicit apply request uses distinct SCALLYAPPLY1 framing. Staged
/// SCALLYEDIT1 data can never be interpreted as authorization to dispatch.
/// It has the same bounded seven-field header and sorted E rows as parse().
/// Empty requests can clear a draft, but apply_to_current() will not dispatch.
pub(crate) fn parse_apply(bytes: &[u8]) -> Option<Edits> {
    parse_with_magic(bytes, APPLY_MAGIC)
}

fn parse_with_magic(bytes: &[u8], magic: &str) -> Option<Edits> {
    if bytes.is_empty() || bytes.len() > MAX_REQUEST_BYTES || !bytes.is_ascii() {
        return None;
    }
    // Do not normalize standalone carriage returns into accepted data.
    if bytes.iter().enumerate().any(|(index, &byte)| {
        byte == b'\r' && bytes.get(index + 1) != Some(&b'\n')
    }) {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?.replace("\r\n", "\n");
    let text = text.strip_suffix('\n').unwrap_or(&text);
    let mut lines = text.split('\n');
    let header: Vec<_> = lines.next()?.split('\t').collect();
    if header.len() != 7 || header[0] != magic {
        return None;
    }
    let pid = u32::try_from(decimal(header[1])?).ok()?;
    let session = decimal(header[2])?;
    let generation = decimal(header[3])?;
    let frame = u32::try_from(decimal(header[4])?).ok()?;
    let request_id = decimal(header[5])?;
    let count = usize::try_from(decimal(header[6])?).ok()?;
    if pid == 0 || session == 0 || generation == 0 || request_id == 0 || count > MAX_CHANGES {
        return None;
    }
    let mut changes = Vec::with_capacity(count);
    let mut previous = None;
    for _ in 0..count {
        let fields: Vec<_> = lines.next()?.split('\t').collect();
        if fields.len() != 4 || fields[0] != "E" {
            return None;
        }
        let edit = Edit {
            slot: u8::try_from(decimal(fields[1])?).ok()?,
            expected: u8::try_from(decimal(fields[2])?).ok()?,
            desired: u8::try_from(decimal(fields[3])?).ok()?,
        };
        if edit.slot as usize >= alliance::PLAYABLE_SLOTS || !normal_toggle(edit)
            || previous.is_some_and(|slot| edit.slot <= slot)
        {
            return None;
        }
        previous = Some(edit.slot);
        changes.push(edit);
    }
    if lines.next().is_some() {
        return None;
    }
    Some(Edits { pid, session, generation, frame, request_id, changes })
}

/// Revalidate computer identities and expected outgoing values against the
/// immediate pre-callback snapshot. This also validates directly constructed
/// Edits, rather than relying on the file parser as the only boundary.
pub(crate) fn validate(edits: &Edits, snapshot: &Snapshot) -> bool {
    if edits.pid == 0 || edits.session == 0 || edits.generation == 0 || edits.request_id == 0
        || edits.changes.len() > MAX_CHANGES || !snapshot.has_valid_core()
    {
        return false;
    }
    let mut previous = None;
    for &edit in &edits.changes {
        if !normal_toggle(edit) || previous.is_some_and(|slot| edit.slot <= slot) {
            return false;
        }
        let Some(player) = snapshot.computer(edit.slot) else { return false; };
        if player.slot != edit.slot || player.id != u32::from(edit.slot)
            || snapshot.alliance_row[edit.slot as usize] != edit.expected
        {
            return false;
        }
        previous = Some(edit.slot);
    }
    true
}

/// Build an ordinary ALLIANCE command from the immediate current local-human
/// outgoing row, changing only explicitly requested active computer slots.
/// It does not use or wait for a native dialog append, read another row, or
/// change shared vision. All twelve other relationships are retained.
/// The caller must independently verify the explicit apply authorization,
/// PID/session/generation/frame, UI-thread ownership, sender and write policy.
/// Empty or stale requests cannot produce a command.
pub(crate) fn apply_to_current(edits: &Edits, snapshot: &Snapshot) -> Option<[u8; 5]> {
    if edits.changes.is_empty() || !validate(edits, snapshot) {
        return None;
    }
    let mut row = snapshot.alliance_row;
    for edit in &edits.changes {
        row[edit.slot as usize] = edit.desired;
    }
    alliance::encode_row(&row)
}

/// Merge this dialog's explicitly authorized computer checkbox values into a
/// single native ALLIANCE packet. The caller verifies the current owner,
/// session/dialog lifetime, request identity and pending/confirmed intent.
/// This pure helper cannot establish that authorization itself and never
/// creates a periodic command or infers intent from current relationships.
///
/// Values are ordinary checkbox intent: enemy (0) or ally (1). An enabled
/// checkbox retains native allied-victory (2); disabling it makes the target
/// an enemy. Every unmentioned relationship and upper eight payload bits stays
/// exactly as the native dialog encoded it, including unsaved human edits.
/// Current row equality is deliberately not required: a just-sent authorized
/// request may not yet have reached the simulation turn.
pub(crate) fn merge_authorized_native(
    packet: &[u8],
    snapshot: &Snapshot,
    desired: &[Option<u8>; alliance::PLAYABLE_SLOTS],
) -> Option<[u8; 5]> {
    if packet.len() != 5 || packet[0] != alliance::ALLIANCE_COMMAND
        || !snapshot.has_valid_core() || desired.iter().all(Option::is_none)
    {
        return None;
    }
    let mut result: [u8; 5] = packet.try_into().ok()?;
    let mut mask = u32::from_le_bytes(result[1..].try_into().ok()?);
    // Validate every target before modifying any packet bits. Relation 3 is
    // unsupported for an editable computer and must not be coerced into 0/1.
    for (slot, value) in desired.iter().enumerate() {
        let Some(value) = *value else { continue; };
        let player = snapshot.computer(slot as u8)?;
        if value > 1 || player.slot as usize != slot || player.id != slot as u32
            || ((mask >> (slot * 2)) & 3) == 3
        {
            return None;
        }
    }
    for (slot, value) in desired.iter().enumerate() {
        if let Some(value) = *value {
            let shift = slot * 2;
            let native = (mask >> shift) & 3;
            let merged = if value == 0 { 0 } else if native == 2 { 2 } else { 1 };
            mask = (mask & !(3u32 << shift)) | (merged << shift);
        }
    }
    result[1..].copy_from_slice(&mask.to_le_bytes());
    Some(result)
}

/// Locate one native ALLIANCE record in an intact bounded outgoing-buffer
/// append. Accept only that five-byte record, optionally one three-byte VISION
/// record in either order. The returned offset is relative to the whole `after`
/// buffer, not just its appended suffix. Opcode-looking payload bytes never
/// count as additional records. No command is created or appended here.
pub(crate) fn native_alliance_offset(before: &[u8], after: &[u8]) -> Option<usize> {
    if before.len() > MAX_CAPTURE_BYTES || after.len() > MAX_CAPTURE_BYTES
        || !after.starts_with(before)
    {
        return None;
    }
    let relative = match after.get(before.len()..)? {
        [alliance::ALLIANCE_COMMAND, _, _, _, _] => 0,
        [alliance::ALLIANCE_COMMAND, _, _, _, _, VISION_COMMAND, _, _] => 0,
        [VISION_COMMAND, _, _, alliance::ALLIANCE_COMMAND, _, _, _, _] => 3,
        _ => return None,
    };
    before.len().checked_add(relative)
}

fn native_alliance(before: &[u8], after: &[u8]) -> Option<[u8; 5]> {
    let offset = native_alliance_offset(before, after)?;
    after.get(offset..offset.checked_add(5)?)?.try_into().ok()
}
/// Accept exactly an intact occupied-buffer prefix followed by one ALLIANCE
/// record, optionally one VISION record in either order. Payload opcode bytes
/// are not scanned. Incomplete, duplicate, additional, or unrelated records
/// fail. Shared vision is left wholly in the caller's original immutable data.
/// Only requested computer bits in the captured native ALLIANCE are replaced;
/// human edits, other computers, neutral slots, allied-victory values, and the
/// unused upper eight payload bits are preserved, including unsaved UI edits.
pub(crate) fn merge_native(
    edits: &Edits,
    snapshot: &Snapshot,
    before: &[u8],
    after: &[u8],
) -> Option<[u8; 5]> {
    if !validate(edits, snapshot) {
        return None;
    }
    let mut result = native_alliance(before, after)?;
    let mut mask = u32::from_le_bytes(result[1..].try_into().unwrap());
    for edit in &edits.changes {
        let shift = u32::from(edit.slot) * 2;
        mask = (mask & !(3 << shift)) | (u32::from(edit.desired) << shift);
    }
    result[1..].copy_from_slice(&mask.to_le_bytes());
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alliance::Player;

    fn snapshot() -> Snapshot {
        let players = std::array::from_fn(|slot| Player {
            slot: slot as u8, id: slot as u32, storm_id: slot as u32,
            controller: if slot < 2 { alliance::HUMAN } else { alliance::COMPUTER },
            race: (slot % 3) as u8, team: (slot % 5) as u8,
            name: [0; alliance::PLAYER_NAME_SIZE], palette_index: None,
        });
        Snapshot { owner: 0, players, alliance_row: [1, 2, 0, 2, 1, 0, 2, 0, 1, 2, 3, 1] }
    }

    fn edits() -> Edits {
        parse(b"SCALLYEDIT1\t123\t456\t789\t0\t12\t2\nE\t2\t0\t1\nE\t3\t2\t0\n").unwrap()
    }

    fn packet(row: [u8; alliance::ALLIANCE_SLOTS], high: u8) -> [u8; 5] {
        let mut value = alliance::encode_row(&row).unwrap();
        value[4] = high;
        value
    }

    fn relation(packet: &[u8; 5], slot: usize) -> u8 {
        ((u32::from_le_bytes(packet[1..].try_into().unwrap()) >> (slot * 2)) & 3) as u8
    }

    #[test]
    fn explicit_apply_and_native_confirm_use_game_slots_for_every_local_owner() {
        for owner in 0..alliance::PLAYABLE_SLOTS as u8 {
            let target = (owner + 1) % alliance::PLAYABLE_SLOTS as u8;
            let mut current = snapshot();
            current.owner = owner;
            for player in &mut current.players {
                player.controller = if player.slot == target { alliance::COMPUTER }
                    else { alliance::HUMAN };
                player.storm_id = if player.slot == target { u32::MAX }
                    else { u32::from((player.slot + 1) % alliance::PLAYABLE_SLOTS as u8) };
            }
            current.alliance_row = std::array::from_fn(|slot| ((owner as usize + slot) % 3) as u8);
            current.alliance_row[owner as usize] = 1;
            current.alliance_row[target as usize] = 0;
            let original = current.clone();
            let mut request = edits();
            request.changes = vec![Edit { slot: target, expected: 0, desired: 1 }];
            let applied = apply_to_current(&request, &current).unwrap();
            assert_eq!(relation(&applied, target as usize), 1, "owner={owner}");
            for slot in 0..alliance::ALLIANCE_SLOTS {
                if slot != target as usize {
                    assert_eq!(relation(&applied, slot), current.alliance_row[slot]);
                }
            }

            let mut native_row = current.alliance_row;
            let other_human = (owner + 2) % alliance::PLAYABLE_SLOTS as u8;
            native_row[other_human as usize] = (native_row[other_human as usize] + 1) % 3;
            let native = packet(native_row, 0xa7);
            let mut desired = [None; alliance::PLAYABLE_SLOTS];
            desired[target as usize] = Some(1);
            let confirmed = merge_authorized_native(&native, &current, &desired).unwrap();
            assert_eq!(relation(&confirmed, target as usize), 1);
            assert_eq!(confirmed[4], native[4]);
            for slot in 0..alliance::ALLIANCE_SLOTS {
                if slot != target as usize {
                    assert_eq!(relation(&confirmed, slot), relation(&native, slot));
                }
            }
            assert_eq!(current, original);
            let mut self_edit = request.clone();
            self_edit.changes = vec![Edit { slot: owner, expected: 1, desired: 0 }];
            assert!(apply_to_current(&self_edit, &current).is_none());
        }
    }
    #[test]
    fn parses_exact_bounded_ascii_request_with_lf_crlf_and_optional_last_newline() {
        let expected = edits();
        let lf = "SCALLYEDIT1\t123\t456\t789\t0\t12\t2\nE\t2\t0\t1\nE\t3\t2\t0";
        assert_eq!(parse(lf.as_bytes()), Some(expected.clone()));
        assert_eq!(parse(format!("{lf}\n").as_bytes()), Some(expected.clone()));
        assert_eq!(parse(lf.replace('\n', "\r\n").as_bytes()), Some(expected.clone()));
        assert_eq!(parse(format!("{}\r\n", lf.replace('\n', "\r\n")).as_bytes()), Some(expected));
        assert!(parse(b"SCALLYEDIT1\t1\t1\t1\t4294967295\t18446744073709551615\t0").is_some());
    }

    #[test]
    fn accepts_zero_count_clear_and_seven_sorted_changes_only() {
        let clear = parse(b"SCALLYEDIT1\t1\t1\t1\t0\t1\t0\n").unwrap();
        assert!(clear.changes.is_empty());
        let mut text = String::from("SCALLYEDIT1\t1\t1\t1\t0\t1\t7");
        for slot in 1..=7 { text.push_str(&format!("\nE\t{slot}\t0\t1")); }
        assert_eq!(parse(text.as_bytes()).unwrap().changes.len(), 7);
        assert!(parse(text.replacen("\t7\n", "\t8\n", 1).as_bytes()).is_none());
    }

    #[test]
    fn rejects_malformed_metadata_line_counts_and_unbounded_or_non_ascii_data() {
        let valid = "SCALLYEDIT1\t123\t456\t789\t0\t12\t1\nE\t2\t0\t1";
        for bad in [
            valid.replacen("SCALLYEDIT1", "SCALLY1", 1),
            valid.replacen("\t123\t", "\t0\t", 1),
            valid.replacen("\t456\t", "\t0\t", 1),
            valid.replacen("\t789\t", "\t0\t", 1),
            valid.replacen("\t12\t", "\t0\t", 1),
            valid.replacen("\t123\t", "\t4294967296\t", 1),
            valid.replacen("\t456\t", "\t18446744073709551616\t", 1),
            valid.replacen("\t0\t12", "\t4294967296\t12", 1),
            valid.replacen("\t123\t", "\t+123\t", 1),
            valid.replacen("\t123\t", "\t 123\t", 1),
            valid.replacen("\t123\t", "\t123 \t", 1),
            valid.replacen("\t123\t", "\t-123\t", 1),
            valid.replace('\n', "\r"),
            format!("{valid}\n\n"),
            format!("{valid}\nE\t3\t1\t0"),
            valid.replacen("\t12\t1\n", "\t12\t0\n", 1),
            valid.replacen("\t12\t1\n", "\t12\t2\n", 1),
            valid.replacen("\t12\t1\n", "\t12\t1\textra\n", 1),
            format!("{valid}\0"),
            valid.replace("123", "１２３"),
        ] { assert!(parse(bad.as_bytes()).is_none(), "accepted {bad:?}"); }
        assert!(parse(&vec![b'1'; MAX_REQUEST_BYTES + 1]).is_none());
        assert!(parse(b"").is_none());
    }

    #[test]
    fn rejects_bad_slots_duplicate_unsorted_rows_and_non_toggle_values() {
        for line in ["E\t8\t0\t1", "E\t255\t0\t1", "E\t-1\t0\t1",
            "E\t2\t3\t0", "E\t2\t0\t2", "E\t2\t1\t2", "E\t2\t2\t1",
            "E\t2\t0\t0", "E\t2\t1\t1", "E\t2\t2\t2", "E\t2\t1\t3",
            "E\t2\t0\t1\textra", "C\t2\t0\t1", "E\t2\t0", " E\t2\t0\t1"] {
            let text = format!("SCALLYEDIT1\t1\t1\t1\t0\t1\t1\n{line}");
            assert!(parse(text.as_bytes()).is_none(), "accepted {line:?}");
        }
        for lines in ["E\t2\t0\t1\nE\t2\t1\t0", "E\t3\t1\t0\nE\t2\t0\t1"] {
            assert!(parse(format!("SCALLYEDIT1\t1\t1\t1\t0\t1\t2\n{lines}").as_bytes()).is_none());
        }
    }

    #[test]
    fn preserves_unsaved_human_changes_other_slots_and_unused_payload_bits() {
        let snapshot = snapshot();
        let edits = edits();
        let before = [0x09, 0x0e, 0x0d, 0xff];
        // Native dialog human1 unchecked even though the pre-callback Game row
        // is still allied-victory2. Other unchanged slots include value2/3.
        let row = [1, 0, 0, 2, 1, 0, 2, 0, 1, 2, 3, 1];
        let original = packet(row, 0xa5);
        let mut after = before.to_vec(); after.extend_from_slice(&original);
        let after_copy = after.clone();
        let merged = merge_native(&edits, &snapshot, &before, &after).unwrap();
        assert_eq!(relation(&merged, 2), 1);
        assert_eq!(relation(&merged, 3), 0);
        for slot in 0..alliance::ALLIANCE_SLOTS {
            if slot != 2 && slot != 3 { assert_eq!(relation(&merged, slot), row[slot]); }
        }
        assert_eq!(merged[4], 0xa5);
        assert_eq!(after, after_copy);
    }

    #[test]
    fn accepts_optional_vision_record_in_both_orders_without_modifying_it() {
        let snapshot = snapshot(); let edits = edits();
        let original = packet(snapshot.alliance_row, 0xd7);
        // Opcode-valued payload bytes remain data, not additional records.
        let vision = [VISION_COMMAND, alliance::ALLIANCE_COMMAND, VISION_COMMAND];
        let before = [0x01, 0x02];
        let mut first = before.to_vec(); first.extend_from_slice(&original); first.extend_from_slice(&vision);
        let mut second = before.to_vec(); second.extend_from_slice(&vision); second.extend_from_slice(&original);
        let first_copy = first.clone(); let second_copy = second.clone();
        assert_eq!(merge_native(&edits, &snapshot, &before, &first),
            merge_native(&edits, &snapshot, &before, &second));
        assert!(merge_native(&edits, &snapshot, &before, &first).is_some());
        assert_eq!(first, first_copy); assert_eq!(second, second_copy);
        assert_eq!(&first[first.len() - 3..], &vision);
        assert_eq!(&second[before.len()..before.len() + 3], &vision);
    }

    #[test]
    fn rejects_ambiguous_incomplete_duplicate_or_unrelated_native_appends() {
        let snapshot = snapshot(); let edits = edits();
        let original = packet(snapshot.alliance_row, 0);
        let before = [0x01, 0x02];
        let mut invalid = vec![vec![], original[..4].to_vec(), vec![VISION_COMMAND, 0, 0],
            vec![0x06, 0, 0, 0, 0], vec![0x0e, 0, 0, 0, 0, 0x06, 0, 0]];
        let mut duplicate = original.to_vec(); duplicate.extend_from_slice(&original); invalid.push(duplicate);
        let mut extra = original.to_vec(); extra.extend_from_slice(&[VISION_COMMAND, 0, 0, VISION_COMMAND, 0, 0]); invalid.push(extra);
        let mut trailing = original.to_vec(); trailing.push(0); invalid.push(trailing);
        for delta in invalid {
            let mut after = before.to_vec(); after.extend_from_slice(&delta);
            assert!(merge_native(&edits, &snapshot, &before, &after).is_none(), "delta={delta:?}");
        }
        let mut changed = vec![0x01, 0x03]; changed.extend_from_slice(&original);
        assert!(merge_native(&edits, &snapshot, &before, &changed).is_none());
        assert!(merge_native(&edits, &snapshot, &before, &before[..1]).is_none());
        let oversized_before = vec![0; MAX_CAPTURE_BYTES];
        let mut oversized_after = oversized_before.clone(); oversized_after.extend_from_slice(&original);
        assert!(merge_native(&edits, &snapshot, &oversized_before, &oversized_after).is_none());
    }

    #[test]
    fn rejects_stale_expected_values_and_non_computer_or_neutral_targets() {
        let snapshot = snapshot(); let edits = edits();
        let original = packet(snapshot.alliance_row, 0);
        let mut stale = snapshot.clone(); stale.alliance_row[2] = 1;
        assert!(!validate(&edits, &stale));
        assert!(merge_native(&edits, &stale, &[], &original).is_none());
        for controller in [0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 255] {
            let mut value = snapshot.clone(); value.players[2].controller = controller;
            assert!(!validate(&edits, &value), "controller={controller}");
        }
        let mut identity = snapshot.clone(); identity.players[2].id = 7;
        assert!(!validate(&edits, &identity));
        let mut identity = snapshot.clone(); identity.players[2].slot = 7;
        assert!(!validate(&edits, &identity));
        let mut owner = snapshot.clone(); owner.owner = 8;
        assert!(!validate(&edits, &owner));
        let mut owner = snapshot.clone(); owner.players[0].controller = alliance::COMPUTER;
        assert!(!validate(&edits, &owner));
    }

    #[test]
    fn unknown_race_and_team_do_not_block_computer_edits_or_change_other_payload_bits() {
        let mut snapshot = snapshot();
        for slot in [0, 1, 2, 3, 7] {
            snapshot.players[slot].race = 255;
            snapshot.players[slot].team = 255;
        }
        let edits = edits();
        assert!(validate(&edits, &snapshot));
        let original = packet([1, 0, 0, 2, 1, 0, 2, 0, 1, 2, 3, 1], 0xa7);
        let vision = [VISION_COMMAND, 0x5a, 0xa5];
        let mut after = original.to_vec();
        after.extend_from_slice(&vision);
        let untouched = after.clone();
        let merged = merge_native(&edits, &snapshot, &[], &after).unwrap();
        assert_eq!(relation(&merged, 2), 1);
        assert_eq!(relation(&merged, 3), 0);
        for slot in 0..alliance::ALLIANCE_SLOTS {
            if slot != 2 && slot != 3 {
                assert_eq!(relation(&merged, slot), relation(&original, slot));
            }
        }
        assert_eq!(merged[4], original[4]);
        assert_eq!(after, untouched);
        assert_eq!(&after[5..], &vision);
    }

    #[test]
    fn core_identity_and_owner_stay_required_for_directly_constructed_snapshots() {
        let snapshot = snapshot();
        let edits = edits();
        for slot in [0, 1, 2, 3, 7] {
            let mut value = snapshot.clone();
            value.players[slot].race = 255;
            value.players[slot].team = 255;
            value.players[slot].id = ((slot + 1) % 8) as u32;
            assert!(!validate(&edits, &value), "invalid id slot={slot}");

            let mut value = snapshot.clone();
            value.players[slot].slot = ((slot + 1) % 8) as u8;
            assert!(!validate(&edits, &value), "invalid slot identity={slot}");
        }
        for controller in [0, 1, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 255] {
            let mut value = snapshot.clone();
            value.players[0].controller = controller;
            assert!(!validate(&edits, &value), "invalid local controller={controller}");
        }
        let mut value = snapshot.clone();
        value.alliance_row[11] = 4;
        assert!(!validate(&edits, &value));
    }

    #[test]
    fn directly_constructed_requests_cannot_bypass_protocol_or_target_validation() {
        let snapshot = snapshot(); let original = edits();
        let mut malformed = original.clone(); malformed.changes.reverse();
        assert!(!validate(&malformed, &snapshot));
        malformed = original.clone(); malformed.changes[1] = malformed.changes[0];
        assert!(!validate(&malformed, &snapshot));
        for (slot, expected, desired) in [(0, 1, 0), (1, 2, 0), (8, 0, 1), (255, 0, 1),
            (2, 0, 2), (2, 0, 0), (2, 3, 0)] {
            malformed = original.clone(); malformed.changes = vec![Edit { slot, expected, desired }];
            assert!(!validate(&malformed, &snapshot));
        }
        malformed = original.clone(); malformed.changes = vec![original.changes[0]; MAX_CHANGES + 1];
        assert!(!validate(&malformed, &snapshot));
        malformed = original.clone(); malformed.session = 0;
        assert!(!validate(&malformed, &snapshot));
    }


    fn apply_request(lines: &[&str]) -> Edits {
        let mut text = format!("SCALLYAPPLY1\t123\t456\t789\t50\t12\t{}", lines.len());
        for line in lines { text.push('\n'); text.push_str(line); }
        parse_apply(text.as_bytes()).unwrap()
    }

    #[test]
    fn apply_protocol_accepts_only_explicit_magic_with_complete_ascii_framing() {
        let staging = b"SCALLYEDIT1\t123\t456\t789\t50\t12\t1\nE\t2\t0\t1";
        let apply = "SCALLYAPPLY1\t123\t456\t789\t50\t12\t1\nE\t2\t0\t1";
        assert!(parse_apply(staging).is_none());
        assert!(parse(apply.as_bytes()).is_none());
        let expected = parse_apply(apply.as_bytes()).unwrap();
        assert_eq!(parse_apply(format!("{apply}\n").as_bytes()), Some(expected.clone()));
        assert_eq!(parse_apply(apply.replace('\n', "\r\n").as_bytes()), Some(expected.clone()));
        assert_eq!(parse_apply(format!("{}\r\n", apply.replace('\n', "\r\n")).as_bytes()), Some(expected));
        for malformed in [
            apply.replace("SCALLYAPPLY1", "SCALLYAPPLY"),
            apply.replace("SCALLYAPPLY1", "SCALLYEDIT1"),
            apply.replacen("\t123\t", "\t0\t", 1),
            apply.replacen("\t456\t", "\t0\t", 1),
            apply.replacen("\t789\t", "\t0\t", 1),
            apply.replacen("\t12\t1", "\t0\t1", 1),
            apply.replacen("\t123\t", "\t4294967296\t", 1),
            apply.replacen("\t50\t", "\t4294967296\t", 1),
            apply.replacen("\t456\t", "\t18446744073709551616\t", 1),
            apply.replacen("\t123\t", "\t+123\t", 1),
            apply.replacen("\t123\t", "\t 123\t", 1),
            apply.replacen("\t123\t", "\t-123\t", 1),
            apply.replacen("\t1\n", "\t2\n", 1),
            apply.replacen("\t1\n", "\t0\n", 1),
            apply.replacen("\t1\n", "\t1\textra\n", 1),
            apply.replace("E\t2", "E\t8"),
            apply.replace("E\t2\t0\t1", "E\t2\t3\t0"),
            apply.replace("E\t2\t0\t1", "E\t2\t0\t2"),
            apply.replace("E\t2\t0\t1", "E\t2\t1\t1"),
            apply.replace("E\t2\t0\t1", "C\t2\t0\t1"),
            apply.replace('\n', "\r"),
            apply.replace("123", "１２３"),
            format!("{apply}\n\n"),
            format!("{apply}\0"),
            format!("{apply}\nE\t3\t2\t0"),
            "SCALLYAPPLY1\t1\t1\t1\t0\t1\t2\nE\t2\t0\t1\nE\t2\t1\t0".into(),
            "SCALLYAPPLY1\t1\t1\t1\t0\t1\t2\nE\t3\t0\t1\nE\t2\t1\t0".into(),
        ] { assert!(parse_apply(malformed.as_bytes()).is_none(), "accepted {malformed:?}"); }
        assert!(parse_apply(&vec![b'1'; MAX_REQUEST_BYTES + 1]).is_none());
        assert!(parse_apply(b"").is_none());
    }

    #[test]
    fn explicit_apply_with_one_computer_and_no_other_humans_needs_no_native_append() {
        let mut current = snapshot();
        for player in current.players.iter_mut().skip(1) { player.controller = 0; }
        current.players[5].controller = alliance::COMPUTER;
        current.players[5].race = 255;
        current.players[5].team = 255;
        assert_eq!(current.other_human_count(), 0);
        assert_eq!(current.active_computer_count(), 1);
        let untouched = current.clone();
        let request = apply_request(&["E\t5\t0\t1"]);
        let result = apply_to_current(&request, &current).unwrap();
        for slot in 0..alliance::ALLIANCE_SLOTS {
            assert_eq!(relation(&result, slot),
                if slot == 5 { 1 } else { current.alliance_row[slot] });
        }
        assert_eq!(result[0], alliance::ALLIANCE_COMMAND);
        assert_eq!(current, untouched);
    }

    #[test]
    fn explicit_apply_can_change_all_seven_computers_without_touching_neutral_relationships() {
        let mut current = snapshot();
        for player in current.players.iter_mut().skip(1) { player.controller = alliance::COMPUTER; }
        current.alliance_row = [2, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 1];
        let lines: Vec<_> = (1..=7).map(|slot| format!("E\t{slot}\t0\t1")).collect();
        let lines: Vec<_> = lines.iter().map(String::as_str).collect();
        let request = apply_request(&lines);
        let result = apply_to_current(&request, &current).unwrap();
        assert_eq!(current.other_human_count(), 0);
        assert_eq!(request.changes.len(), 7);
        for slot in 0..alliance::ALLIANCE_SLOTS {
            assert_eq!(relation(&result, slot),
                if (1..=7).contains(&slot) { 1 } else { current.alliance_row[slot] });
        }
        let mut too_many = request.clone();
        too_many.changes.push(Edit { slot: 7, expected: 0, desired: 1 });
        assert!(apply_to_current(&too_many, &current).is_none());
        let oversized = "SCALLYAPPLY1\t1\t1\t1\t0\t1\t8";
        assert!(parse_apply(oversized.as_bytes()).is_none());
    }

    #[test]
    fn explicit_apply_preserves_latest_other_human_computer_and_neutral_values() {
        let mut current = snapshot();
        current.alliance_row = [2, 0, 0, 2, 3, 0, 2, 1, 3, 2, 1, 0];
        // These fields may be unknown for the local human, another human or AI.
        for player in &mut current.players { player.race = 255; player.team = 255; }
        let request = apply_request(&["E\t2\t0\t1", "E\t3\t2\t0"]);
        let untouched = current.clone();
        let result = apply_to_current(&request, &current).unwrap();
        for slot in 0..alliance::ALLIANCE_SLOTS {
            let expected = match slot { 2 => 1, 3 => 0, _ => current.alliance_row[slot] };
            assert_eq!(relation(&result, slot), expected);
        }
        assert_eq!(current, untouched);
    }

    #[test]
    fn explicit_apply_rejects_stale_expected_core_identity_and_ineligible_computers() {
        let current = snapshot();
        let request = apply_request(&["E\t2\t0\t1", "E\t3\t2\t0"]);
        let mut stale = current.clone();
        stale.alliance_row[3] = 1;
        assert!(apply_to_current(&request, &stale).is_none());
        for controller in [0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 255] {
            let mut invalid = current.clone();
            invalid.players[2].controller = controller;
            assert!(apply_to_current(&request, &invalid).is_none(), "controller={controller}");
        }
        let mut invalid = current.clone();
        invalid.players[2].id = 7;
        assert!(apply_to_current(&request, &invalid).is_none());
        let mut invalid = current.clone();
        invalid.players[0].controller = alliance::COMPUTER;
        assert!(apply_to_current(&request, &invalid).is_none());
        let mut invalid = current.clone();
        invalid.owner = 8;
        assert!(apply_to_current(&request, &invalid).is_none());
        let mut invalid = current.clone();
        invalid.alliance_row[11] = 4;
        assert!(apply_to_current(&request, &invalid).is_none());
        let mut invalid_request = request.clone();
        invalid_request.changes[0] = Edit { slot: 1, expected: 2, desired: 0 };
        assert!(apply_to_current(&invalid_request, &current).is_none());
    }

    #[test]
    fn explicit_apply_empty_clear_is_parseable_but_never_dispatches_a_command() {
        let clear = parse_apply(b"SCALLYAPPLY1\t1\t1\t1\t0\t1\t0\n").unwrap();
        assert!(clear.changes.is_empty());
        assert!(apply_to_current(&clear, &snapshot()).is_none());
    }

    #[test]
    fn native_confirm_preserves_authorized_computers_and_unsaved_human_edits() {
        let current = snapshot();
        let mut desired = [None; alliance::PLAYABLE_SLOTS];
        desired[2] = Some(1); desired[3] = Some(0);
        // The current row still has AI2=0 and AI3=2 while the authorized
        // immediate requests await a turn. Human1=0 is an unsaved native edit.
        let original = packet([2, 0, 0, 2, 2, 3, 1, 0, 3, 2, 1, 0], 0xa7);
        let untouched = current.clone();
        let result = merge_authorized_native(&original, &current, &desired).unwrap();
        assert_eq!(relation(&result, 2), 1); assert_eq!(relation(&result, 3), 0);
        for slot in [0, 1, 4, 5, 6, 7, 8, 9, 10, 11] {
            assert_eq!(relation(&result, slot), relation(&original, slot));
        }
        assert_eq!(result[4], 0xa7); assert_eq!(current, untouched);
        assert_eq!(original, packet([2, 0, 0, 2, 2, 3, 1, 0, 3, 2, 1, 0], 0xa7));
    }

    #[test]
    fn native_confirm_keeps_native_allied_victory_for_enabled_computer_only() {
        let current = snapshot();
        for native in 0..=3 {
            for checked in 0..=1 {
                let mut desired = [None; alliance::PLAYABLE_SLOTS]; desired[2] = Some(checked);
                let mut row = [2, 2, 0, 2, 3, 1, 0, 2, 3, 2, 1, 0]; row[2] = native;
                let original = packet(row, 0xff);
                if native == 3 {
                    assert!(merge_authorized_native(&original, &current, &desired).is_none());
                    continue;
                }
                let result = merge_authorized_native(&original, &current, &desired).unwrap();
                let expected = if checked == 0 { 0 } else if native == 2 { 2 } else { 1 };
                assert_eq!(relation(&result, 2), expected);
                for slot in 0..alliance::ALLIANCE_SLOTS {
                    if slot != 2 { assert_eq!(relation(&result, slot), row[slot]); }
                }
                assert_eq!(result[4], 0xff);
            }
        }
    }

    #[test]
    fn native_confirm_rejects_unsupported_target_relation_without_coercing_or_touching_others() {
        let current = snapshot();
        let original = packet([2, 0, 3, 2, 1, 3, 0, 2, 3, 2, 1, 0], 0xa5);
        let original_copy = original;
        for checked in [0, 1] {
            let mut desired = [None; alliance::PLAYABLE_SLOTS];
            desired[2] = Some(checked);
            assert!(merge_authorized_native(&original, &current, &desired).is_none());
            // One other valid target cannot make a partially authorized packet.
            desired[3] = Some(0);
            assert!(merge_authorized_native(&original, &current, &desired).is_none());
        }
        let mut other_only = [None; alliance::PLAYABLE_SLOTS]; other_only[3] = Some(0);
        let result = merge_authorized_native(&original, &current, &other_only).unwrap();
        for slot in 0..alliance::ALLIANCE_SLOTS {
            assert_eq!(relation(&result, slot), if slot == 3 { 0 } else { relation(&original, slot) });
        }
        assert_eq!(result[4], 0xa5); assert_eq!(original, original_copy);
    }
    #[test]
    fn native_confirm_can_merge_all_seven_computers_and_unknown_race_team() {
        let mut current = snapshot();
        for player in current.players.iter_mut().skip(1) {
            player.controller = alliance::COMPUTER; player.race = 255; player.team = 255;
        }
        current.players[0].race = 255; current.players[0].team = 255;
        let mut desired = [None; alliance::PLAYABLE_SLOTS];
        for (slot, value) in desired.iter_mut().enumerate().skip(1) { *value = Some((slot % 2) as u8); }
        let original = packet([2, 2, 2, 0, 1, 1, 0, 2, 1, 2, 3, 1], 0xfe);
        let result = merge_authorized_native(&original, &current, &desired).unwrap();
        assert_eq!(current.other_human_count(), 0); assert_eq!(current.active_computer_count(), 7);
        for slot in 1..=7 {
            let expected = if slot % 2 == 0 { 0 } else if relation(&original, slot) == 2 { 2 } else { 1 };
            assert_eq!(relation(&result, slot), expected);
        }
        for slot in [0, 8, 9, 10, 11] { assert_eq!(relation(&result, slot), relation(&original, slot)); }
        assert_eq!(result[4], 0xfe);
    }

    #[test]
    fn native_confirm_requires_explicit_checkbox_intent_and_exact_alliance_packet() {
        let current = snapshot(); let original = packet(current.alliance_row, 0x73);
        let mut desired = [None; alliance::PLAYABLE_SLOTS];
        assert!(merge_authorized_native(&original, &current, &desired).is_none());
        for invalid_value in [2, 3, 255] {
            desired[2] = Some(invalid_value);
            assert!(merge_authorized_native(&original, &current, &desired).is_none());
        }
        desired[2] = Some(0);
        for invalid in [
            Vec::new(), original[..4].to_vec(), [original.as_slice(), &[0u8][..]].concat(),
            [original.as_slice(), &[VISION_COMMAND, 1, 0][..]].concat(),
            [&[VISION_COMMAND, 1, 0][..], original.as_slice()].concat(),
            [original.as_slice(), original.as_slice()].concat(),
            vec![VISION_COMMAND, 0, 0, 0, 0], vec![0x0f, 0, 0, 0, 0],
        ] { assert!(merge_authorized_native(&invalid, &current, &desired).is_none()); }
        assert_eq!(merge_authorized_native(&original, &current, &desired), Some(original));
    }

    #[test]
    fn native_confirm_rejects_own_human_inactive_and_inconsistent_computer_slots() {
        let current = snapshot(); let original = packet(current.alliance_row, 0x11);
        let mut desired = [None; alliance::PLAYABLE_SLOTS];
        for slot in [0, 1] {
            desired[slot] = Some(0);
            assert!(merge_authorized_native(&original, &current, &desired).is_none()); desired[slot] = None;
        }
        desired[2] = Some(1);
        for controller in [0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 255] {
            let mut invalid = current.clone(); invalid.players[2].controller = controller;
            assert!(merge_authorized_native(&original, &invalid, &desired).is_none());
        }
        for (slot, id) in [(7, 2), (2, 7)] {
            let mut invalid = current.clone(); invalid.players[2].slot = slot; invalid.players[2].id = id;
            assert!(merge_authorized_native(&original, &invalid, &desired).is_none());
        }
        let mut invalid = current.clone(); invalid.owner = 8;
        assert!(merge_authorized_native(&original, &invalid, &desired).is_none());
        let mut invalid = current.clone(); invalid.players[0].controller = alliance::COMPUTER;
        assert!(merge_authorized_native(&original, &invalid, &desired).is_none());
        let mut invalid = current.clone(); invalid.players[0].id = 1;
        assert!(merge_authorized_native(&original, &invalid, &desired).is_none());
        let mut invalid = current.clone(); invalid.alliance_row[11] = 4;
        assert!(merge_authorized_native(&original, &invalid, &desired).is_none());
    }

    #[test]
    fn native_alliance_offsets_include_prefix_and_keep_vision_bytes_in_both_orders() {
        let before = [0x0e, 0x0d, 0xff, 0, 0x13];
        // All opcode-looking payload bytes remain payload, including VISION.
        let alliance = [alliance::ALLIANCE_COMMAND, 0x0d, 0x0e, 0x0d, 0x0e];
        let vision = [VISION_COMMAND, 0x0e, 0x0d];
        for (append, relative) in [
            (alliance.to_vec(), 0),
            ([alliance.as_slice(), vision.as_slice()].concat(), 0),
            ([vision.as_slice(), alliance.as_slice()].concat(), 3),
        ] {
            let after = [before.as_slice(), append.as_slice()].concat();
            let offset = native_alliance_offset(&before, &after).unwrap();
            assert_eq!(offset, before.len() + relative);
            assert_eq!(&after[offset..offset + 5], &alliance);
            assert_eq!(native_alliance(&before, &after), Some(alliance));
        }
        let prefix = vec![0xab; MAX_CAPTURE_BYTES - 5];
        let after = [prefix.as_slice(), alliance.as_slice()].concat();
        assert_eq!(native_alliance_offset(&prefix, &after), Some(MAX_CAPTURE_BYTES - 5));
    }

    #[test]
    fn native_alliance_offsets_reject_changed_prefix_extra_duplicate_or_truncated_records() {
        let before = [0xaa, 0xbb]; let alliance = [alliance::ALLIANCE_COMMAND, 0, 0x0d, 0x0e, 0];
        let vision = [VISION_COMMAND, 1, 0];
        for append in [
            Vec::new(), alliance[..4].to_vec(), vision.to_vec(),
            [alliance.as_slice(), &[0][..]].concat(),
            [alliance.as_slice(), alliance.as_slice()].concat(),
            [vision.as_slice(), vision.as_slice(), alliance.as_slice()].concat(),
            [alliance.as_slice(), vision.as_slice(), vision.as_slice()].concat(),
            vec![0x0f, 0, 0, 0, 0], vec![0x0e, 0, 0, 0, 0, 0x0e, 0, 0],
        ] {
            let after = [before.as_slice(), append.as_slice()].concat();
            assert!(native_alliance_offset(&before, &after).is_none());
        }
        let changed = [0xaa, 0xbc, 0x0e, 0, 0, 0, 0];
        assert!(native_alliance_offset(&before, &changed).is_none());
        let oversized = vec![0; MAX_CAPTURE_BYTES + 1];
        assert!(native_alliance_offset(&oversized, &oversized).is_none());
        let prefix = vec![0; MAX_CAPTURE_BYTES - 4];
        let after = [prefix.as_slice(), alliance.as_slice()].concat();
        assert!(native_alliance_offset(&prefix, &after).is_none());
    }
    #[test]
    fn zero_change_request_returns_exact_original_native_alliance_without_a_new_edit() {
        let snapshot = snapshot();
        let clear = parse(b"SCALLYEDIT1\t1\t1\t1\t0\t1\t0").unwrap();
        let original = packet([2, 0, 1, 2, 0, 3, 1, 0, 1, 2, 3, 1], 0xee);
        assert_eq!(merge_native(&clear, &snapshot, &[], &original), Some(original));
        assert!(merge_native(&clear, &snapshot, &[], &[VISION_COMMAND, 0, 0]).is_none());
    }
}
