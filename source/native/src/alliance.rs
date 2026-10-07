//! Read-only diplomacy data and the ordinary local-player alliance command.
//!
//! The caller supplies the bounded reader and verified game/players bases. This
//! module never dereferences a process pointer, changes a player's controller,
//! writes game memory, or selects a command's sender.
//!
//! Layout evidence:
//! - neivv/aise, da4fed681dc09bda7a21c0051f60cdad98cc7226,
//!   bw_dat/src/bw/structs.rs: Game and Player are repr(C); Player is 0x24 bytes
//!   on both x86 and x64. Game has no pointer-width-dependent fields before
//!   alliances. Player itself does not contain a color field.
//! - samase_scarf, 4ecc2f8466292c125f824fbeb9d3d3fb39236bf5,
//!   src/commands.rs, command_user: command 0x0e writes one row of twelve bytes
//!   at game + 0xe544 + command_user * 12.
//! - ShieldBattery game/src/bw/commands.rs: ALLIANCE = 0x0e, length = 5.
//! - OpenBW actions.h: the four payload bytes encode twelve two-bit entries;
//!   game_types.h: computer_game = 1, occupied = 2, rescue = 3/4,
//!   lobby computer = 5, neutral = 7, defeated computer = 11.

pub(crate) const PLAYABLE_SLOTS: usize = 8;
pub(crate) const ALLIANCE_SLOTS: usize = 12;
pub(crate) const PLAYER_SIZE: usize = 0x24;
pub(crate) const PLAYER_NAME_OFFSET: usize = 11;
pub(crate) const PLAYER_NAME_SIZE: usize = 25;
pub(crate) const ALLIANCES_OFFSET: usize = 0xe544;
pub(crate) const VISIONS_OFFSET: usize = 0xfc;
pub(crate) const ALLIANCE_COMMAND: u8 = 0x0e;
pub(crate) const COMPUTER: u8 = 1;
pub(crate) const HUMAN: u8 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Player {
    pub(crate) slot: u8,
    pub(crate) id: u32,
    pub(crate) storm_id: u32,
    pub(crate) controller: u8,
    pub(crate) race: u8,
    pub(crate) team: u8,
    pub(crate) name: [u8; PLAYER_NAME_SIZE],
    /// Palette index, not an RGB value or a player-number color assumption.
    /// Resolve through the verified current palette to draw a color swatch.
    pub(crate) palette_index: Option<u8>,
}

impl Player {
    pub(crate) fn is_computer(&self) -> bool {
        self.slot < PLAYABLE_SLOTS as u8 && self.id == u32::from(self.slot)
            && self.controller == COMPUTER
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Snapshot {
    pub(crate) owner: u8,
    pub(crate) players: [Player; PLAYABLE_SLOTS],
    /// Only the local player's outgoing row. No AI's incoming row is read or
    /// encoded by this interface.
    pub(crate) alliance_row: [u8; ALLIANCE_SLOTS],
}

impl Snapshot {
    /// Only controller, playable-slot identity, local-human ownership and the
    /// outgoing relationship values determine diplomacy validity. Race and
    /// force/team are retained as opaque roster data, including unknown values.
    pub(crate) fn has_valid_core(&self) -> bool {
        let Some(owner) = self.players.get(self.owner as usize) else { return false; };
        owner.controller == HUMAN && owner.slot == self.owner
            && owner.id == u32::from(self.owner)
            && self.alliance_row.iter().all(|&relation| relation <= 3)
            && self.players.iter().enumerate().all(|(slot, player)| {
                !matches!(player.controller, COMPUTER | HUMAN)
                    || (player.slot as usize == slot && player.id == slot as u32)
            })
    }

    pub(crate) fn computer(&self, slot: u8) -> Option<&Player> {
        if slot == self.owner {
            return None;
        }
        self.players.get(slot as usize).filter(|p| p.slot == slot && p.is_computer())
    }

    pub(crate) fn computer_mask(&self) -> u8 {
        self.players.iter().fold(0, |mask, player| {
            if self.computer(player.slot).is_some() {
                mask | (1 << player.slot)
            } else {
                mask
            }
        })
    }

    pub(crate) fn human_count(&self) -> usize {
        self.players.iter().filter(|p| p.controller == HUMAN).count()
    }

    /// Participating humans other than the local player; departed, observers,
    /// rescue and lobby controllers do not make the alliance button useful.
    pub(crate) fn other_human_count(&self) -> usize {
        self.players.iter().enumerate().filter(|(slot, player)| {
            *slot != self.owner as usize && player.controller == HUMAN
                && player.slot as usize == *slot && player.id == *slot as u32
        }).count()
    }

    /// Counts all active computer slots P1..P8, independently of whether their
    /// current relationship can be represented by an editable checkbox.
    pub(crate) fn active_computer_count(&self) -> usize {
        (0..PLAYABLE_SLOTS as u8).filter(|&slot| self.computer(slot).is_some()).count()
    }

    pub(crate) fn has_alliance_targets(&self) -> bool {
        self.has_valid_core() && (self.other_human_count() != 0 || self.active_computer_count() != 0)
    }

    pub(crate) fn checked(&self, slot: u8) -> Option<bool> {
        self.computer(slot)?;
        Some(self.alliance_row[slot as usize] != 0)
    }

    pub(crate) fn command(&self) -> Option<[u8; 5]> {
        encode_row(&self.alliance_row)
    }
}

fn read_at<const N: usize>(
    base: usize,
    offset: usize,
    read: &mut impl FnMut(usize, &mut [u8]) -> bool,
) -> Option<[u8; N]> {
    let address = base.checked_add(offset)?;
    if base < 0x10000 || address.checked_add(N).is_none() {
        return None;
    }
    let mut bytes = [0; N];
    read(address, &mut bytes).then_some(bytes)
}

/// Performs exactly two small, fixed-size reads (288 and 12 bytes).
/// `owner` must come from the runtime's validated local-human game context.
/// A missing read or inconsistent active-player layout produces no UI data.
pub(crate) fn read_snapshot(
    game_base: usize,
    players_base: usize,
    owner: u8,
    mut read: impl FnMut(usize, &mut [u8]) -> bool,
) -> Option<Snapshot> {
    if owner as usize >= PLAYABLE_SLOTS {
        return None;
    }
    let roster = read_at::<{ PLAYER_SIZE * PLAYABLE_SLOTS }>(players_base, 0, &mut read)?;
    let row_offset = ALLIANCES_OFFSET.checked_add(owner as usize * ALLIANCE_SLOTS)?;
    let alliance_row = read_at::<ALLIANCE_SLOTS>(game_base, row_offset, &mut read)?;
    if alliance_row.iter().any(|&relation| relation > 3) {
        return None;
    }
    let players = std::array::from_fn(|slot| {
        let bytes = &roster[slot * PLAYER_SIZE..(slot + 1) * PLAYER_SIZE];
        Player {
            slot: slot as u8,
            id: u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            storm_id: u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            controller: bytes[8],
            race: bytes[9],
            team: bytes[10],
            name: bytes[PLAYER_NAME_OFFSET..PLAYER_NAME_OFFSET + PLAYER_NAME_SIZE]
                .try_into().unwrap(),
            // Current palette/RGB mode is not resolved by this data reader.
            // An unknown swatch is preferable to assuming slot-number colors.
            palette_index: None,
        }
    });
    // Inactive slots can retain arbitrary old names and identity fields.
    // Unknown race/team values do not invalidate another player's diplomacy.
    let snapshot = Snapshot { owner, players, alliance_row };
    snapshot.has_valid_core().then_some(snapshot)
}

/// Encodes all twelve existing outgoing relationships. It deliberately does
/// not touch shared vision or infer a mutual alliance.
pub(crate) fn encode_row(row: &[u8; ALLIANCE_SLOTS]) -> Option<[u8; 5]> {
    let mut mask = 0u32;
    for (slot, &relation) in row.iter().enumerate() {
        if relation > 3 {
            return None;
        }
        mask |= u32::from(relation) << (slot * 2);
    }
    let mut packet = [ALLIANCE_COMMAND, 0, 0, 0, 0];
    packet[1..].copy_from_slice(&mask.to_le_bytes());
    Some(packet)
}

/// Merges one computer checkbox into a current/captured full alliance command.
/// Passing the native dialog's own confirmation packet preserves changes made
/// to its human checkboxes even before network turns update the Game row.
/// Every other relationship and the unused upper payload bits are preserved.
/// Enabling an enemy creates the ordinary alliance value 1; an already allied
/// value remains unchanged, including an existing allied-victory relationship.
pub(crate) fn merge_toggle(
    packet: &[u8; 5],
    snapshot: &Snapshot,
    target: u8,
    checked: bool,
) -> Option<[u8; 5]> {
    if packet[0] != ALLIANCE_COMMAND || !snapshot.has_valid_core() {
        return None;
    }
    snapshot.computer(target)?;
    let mut mask = u32::from_le_bytes(packet[1..].try_into().unwrap());
    let shift = u32::from(target) * 2;
    let previous = (mask >> shift) & 3;
    let desired = if checked { previous.max(1) } else { 0 };
    mask = (mask & !(3 << shift)) | (desired << shift);
    let mut result = *packet;
    result[1..].copy_from_slice(&mask.to_le_bytes());
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAME: usize = 0x100000;
    const PLAYERS: usize = 0x200000;

    fn fixture() -> (Vec<u8>, Vec<u8>) {
        let mut game = vec![0; ALLIANCES_OFFSET + ALLIANCE_SLOTS * ALLIANCE_SLOTS];
        let mut roster = vec![0; PLAYABLE_SLOTS * PLAYER_SIZE];
        for slot in 0..PLAYABLE_SLOTS {
            let start = slot * PLAYER_SIZE;
            roster[start..start + 4].copy_from_slice(&(slot as u32).to_le_bytes());
            roster[start + 4..start + 8].copy_from_slice(&(0x10 + slot as u32).to_le_bytes());
            roster[start + 8] = if slot < 2 { HUMAN } else { COMPUTER };
            roster[start + 9] = (slot % 3) as u8;
            roster[start + 10] = (slot % 5) as u8;
            roster[start + PLAYER_NAME_OFFSET..start + PLAYER_NAME_OFFSET + 4]
                .copy_from_slice(b"Test");
        }
        let row = &mut game[ALLIANCES_OFFSET..ALLIANCES_OFFSET + ALLIANCE_SLOTS];
        row.copy_from_slice(&[1, 2, 0, 1, 2, 3, 0, 0, 1, 2, 3, 1]);
        (game, roster)
    }

    fn snapshot(game: &[u8], players: &[u8]) -> Option<Snapshot> {
        read_snapshot(GAME, PLAYERS, 0, |address, out| {
            let source = if let Some(offset) = address.checked_sub(PLAYERS) {
                players.get(offset..offset.saturating_add(out.len()))
            } else if let Some(offset) = address.checked_sub(GAME) {
                game.get(offset..offset.saturating_add(out.len()))
            } else {
                None
            };
            if let Some(source) = source {
                out.copy_from_slice(source);
                true
            } else { false }
        })
    }

    fn relation(packet: &[u8; 5], slot: usize) -> u8 {
        ((u32::from_le_bytes(packet[1..].try_into().unwrap()) >> (slot * 2)) & 3) as u8
    }

    #[test]
    fn upstream_player_repr_c_fields_match_reader_offsets_and_stride() {
        // Declaration order from the pinned primary bw_dat source, independent
        // of the offsets used by the reader above.
        #[repr(C)]
        struct UpstreamPlayer {
            id: u32, storm_id: u32, player_type: u8, race: u8, team: u8,
            name: [u8; 25],
        }
        assert_eq!(std::mem::size_of::<UpstreamPlayer>(), PLAYER_SIZE);
        assert_eq!(std::mem::offset_of!(UpstreamPlayer, id), 0);
        assert_eq!(std::mem::offset_of!(UpstreamPlayer, storm_id), 4);
        assert_eq!(std::mem::offset_of!(UpstreamPlayer, player_type), 8);
        assert_eq!(std::mem::offset_of!(UpstreamPlayer, race), 9);
        assert_eq!(std::mem::offset_of!(UpstreamPlayer, team), 10);
        assert_eq!(std::mem::offset_of!(UpstreamPlayer, name), PLAYER_NAME_OFFSET);
    }

    #[test]
    fn reads_only_fixed_roster_own_row_and_does_not_guess_colors() {
        let (game, roster) = fixture();
        let mut calls = Vec::new();
        let value = read_snapshot(GAME, PLAYERS, 0, |address, out| {
            calls.push((address, out.len()));
            let source = if address == PLAYERS { &roster[..] }
                else { &game[address - GAME..address - GAME + out.len()] };
            out.copy_from_slice(source);
            true
        }).unwrap();
        assert_eq!(calls, vec![(PLAYERS, 288), (GAME + ALLIANCES_OFFSET, 12)]);
        assert_eq!(value.owner, 0);
        assert_eq!(value.human_count(), 2);
        assert_eq!(value.players[7].palette_index, None);
        assert_eq!(value.players[7].name[..4], *b"Test");
        assert_eq!(value.computer_mask(), 0xfc);
        assert_eq!(value.checked(2), Some(false));
        assert_eq!(value.checked(3), Some(true));
        assert_eq!(value.checked(0), None);
        assert_eq!(value.checked(1), None);
    }

    #[test]
    fn rejects_rescue_neutral_lobby_defeated_observer_and_inactive_controllers() {
        let (game, mut roster) = fixture();
        for controller in [0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 255] {
            roster[2 * PLAYER_SIZE + 8] = controller;
            let value = snapshot(&game, &roster).unwrap();
            assert!(value.computer(2).is_none(), "controller={controller}");
            assert!(merge_toggle(&value.command().unwrap(), &value, 2, true).is_none());
        }
    }

    #[test]
    fn rejects_unreadable_or_inconsistent_active_player_memory() {
        let (game, roster) = fixture();
        assert!(read_snapshot(GAME, PLAYERS, 0, |_, _| false).is_none());
        assert!(snapshot(&game[..ALLIANCES_OFFSET + 11], &roster).is_none());
        assert!(snapshot(&game, &roster[..287]).is_none());
        let mut changed = roster.clone();
        changed[2 * PLAYER_SIZE] = 7;
        assert!(snapshot(&game, &changed).is_none());
        changed = roster.clone();
        changed[8] = COMPUTER;
        assert!(snapshot(&game, &changed).is_none());
        changed = roster.clone();
        changed[2 * PLAYER_SIZE + 8] = 0;
        changed[2 * PLAYER_SIZE] = 255;
        changed[2 * PLAYER_SIZE + 9] = 255;
        changed[2 * PLAYER_SIZE + 10] = 255;
        assert!(snapshot(&game, &changed).is_some());
    }

    #[test]
    fn unknown_race_and_team_of_computer_other_human_or_owner_do_not_hide_diplomacy() {
        let (game, roster) = fixture();
        for slot in [0, 1, 2, 7] {
            for (race, team) in [(3, 5), (255, 255)] {
                let mut changed = roster.clone();
                changed[slot * PLAYER_SIZE + 9] = race;
                changed[slot * PLAYER_SIZE + 10] = team;
                let value = snapshot(&game, &changed).unwrap();
                assert!(value.has_valid_core());
                assert_eq!(value.players[slot].race, race);
                assert_eq!(value.players[slot].team, team);
                assert_eq!(value.other_human_count(), 1);
                assert_eq!(value.active_computer_count(), 6);
                assert!(value.has_alliance_targets());
                assert!(merge_toggle(&value.command().unwrap(), &value, 2, true).is_some());
            }
        }
    }

    #[test]
    fn alliance_presence_excludes_self_and_keeps_computer_only_or_human_only_dialogs() {
        let (mut game, mut roster) = fixture();
        for slot in 1..PLAYABLE_SLOTS { roster[slot * PLAYER_SIZE + 8] = 0; }
        let alone = snapshot(&game, &roster).unwrap();
        assert_eq!(alone.other_human_count(), 0);
        assert_eq!(alone.active_computer_count(), 0);
        assert!(!alone.has_alliance_targets());

        // A reserved relationship can disable editing this row but must not
        // erase the existence of an active computer from the button policy.
        game[ALLIANCES_OFFSET + 7] = 3;
        roster[7 * PLAYER_SIZE + 8] = COMPUTER;
        roster[7 * PLAYER_SIZE + 9] = 255;
        roster[7 * PLAYER_SIZE + 10] = 255;
        let computer_only = snapshot(&game, &roster).unwrap();
        assert_eq!(computer_only.other_human_count(), 0);
        assert_eq!(computer_only.active_computer_count(), 1);
        assert!(computer_only.has_alliance_targets());

        roster[7 * PLAYER_SIZE + 8] = HUMAN;
        let human_only = snapshot(&game, &roster).unwrap();
        assert_eq!(human_only.other_human_count(), 1);
        assert_eq!(human_only.active_computer_count(), 0);
        assert!(human_only.has_alliance_targets());

        for controller in [0, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 255] {
            roster[7 * PLAYER_SIZE + 8] = controller;
            let value = snapshot(&game, &roster).unwrap();
            assert_eq!(value.other_human_count(), 0, "controller={controller}");
            assert_eq!(value.active_computer_count(), 0, "controller={controller}");
            assert!(!value.has_alliance_targets(), "controller={controller}");
        }
    }

    #[test]
    fn malformed_active_identity_cannot_create_alliance_presence_or_a_command() {
        let (game, roster) = fixture();
        for slot in [0, 1, 2, 7] {
            let mut changed = roster.clone();
            changed[slot * PLAYER_SIZE..slot * PLAYER_SIZE + 4]
                .copy_from_slice(&((slot as u32 + 1) % 8).to_le_bytes());
            changed[slot * PLAYER_SIZE + 9] = 255;
            changed[slot * PLAYER_SIZE + 10] = 255;
            assert!(snapshot(&game, &changed).is_none(), "invalid identity slot={slot}");

            let mut value = snapshot(&game, &roster).unwrap();
            value.players[slot].slot = ((slot + 1) % 8) as u8;
            assert!(!value.has_valid_core());
            assert!(!value.has_alliance_targets());
            assert!(merge_toggle(&value.command().unwrap(), &value, 2, true).is_none());
        }
    }

    #[test]
    fn never_reads_observer_neutral_rows_or_invalid_owner_bases() {
        for owner in [8, 11, 12, 255] {
            let mut calls = 0;
            assert!(read_snapshot(GAME, PLAYERS, owner, |_, _| { calls += 1; true }).is_none());
            assert_eq!(calls, 0);
        }
        let mut calls = 0;
        assert!(read_snapshot(GAME, 0, 0, |_, _| { calls += 1; true }).is_none());
        assert_eq!(calls, 0);
        assert!(read_snapshot(GAME, usize::MAX - 16, 0, |_, _| panic!("overflow read")).is_none());
    }

    #[test]
    fn rejects_invalid_relationship_bytes_in_snapshot_and_encoding() {
        let (mut game, roster) = fixture();
        game[ALLIANCES_OFFSET + 4] = 4;
        assert!(snapshot(&game, &roster).is_none());
        let mut row = [0; ALLIANCE_SLOTS];
        row[11] = 255;
        assert!(encode_row(&row).is_none());
    }

    #[test]
    fn encodes_every_relation_including_unmodified_neutral_slots() {
        let row = [1, 2, 0, 1, 2, 3, 0, 0, 1, 2, 3, 1];
        let packet = encode_row(&row).unwrap();
        assert_eq!(packet[0], ALLIANCE_COMMAND);
        for (slot, &value) in row.iter().enumerate() {
            assert_eq!(relation(&packet, slot), value);
        }
        assert_eq!(packet[4], 0);
    }

    #[test]
    fn toggles_only_target_computer_and_preserves_concurrent_human_edits_and_high_bits() {
        let (game, roster) = fixture();
        let value = snapshot(&game, &roster).unwrap();
        let mut packet = value.command().unwrap();
        // The human row was changed in the native dialog after our snapshot.
        let mut mask = u32::from_le_bytes(packet[1..].try_into().unwrap());
        mask &= !(3 << 2);
        mask |= 0xa5 << 24;
        packet[1..].copy_from_slice(&mask.to_le_bytes());
        let enabled = merge_toggle(&packet, &value, 2, true).unwrap();
        assert_eq!(relation(&enabled, 2), 1);
        assert_eq!(enabled[4], 0xa5);
        assert_eq!(relation(&enabled, 1), 0);
        for slot in 0..ALLIANCE_SLOTS {
            if slot != 2 { assert_eq!(relation(&enabled, slot), relation(&packet, slot)); }
        }
        let disabled = merge_toggle(&enabled, &value, 2, false).unwrap();
        assert_eq!(disabled, packet);
    }

    #[test]
    fn check_preserves_existing_allied_victory_and_reserved_two_bit_state() {
        let (game, roster) = fixture();
        let value = snapshot(&game, &roster).unwrap();
        let packet = value.command().unwrap();
        for slot in [3, 4, 5] {
            assert_eq!(merge_toggle(&packet, &value, slot, true).unwrap(), packet);
            let off = merge_toggle(&packet, &value, slot, false).unwrap();
            assert_eq!(relation(&off, slot as usize), 0);
        }
    }

    #[test]
    fn cannot_toggle_owner_human_or_any_slot_outside_eight_playable_slots() {
        let (game, roster) = fixture();
        let value = snapshot(&game, &roster).unwrap();
        let packet = value.command().unwrap();
        for target in [0, 1, 8, 9, 10, 11, 12, 255] {
            assert!(merge_toggle(&packet, &value, target, true).is_none());
        }
        let wrong = [0x0d, 0, 0, 0, 0];
        assert!(merge_toggle(&wrong, &value, 2, true).is_none());
    }
}
