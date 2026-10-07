//! Production and rally policy, independent of camera/overlay discovery.
//! Protocol evidence: BWAPI BW/OrderTypes.h; GPTP recv_commands/train_cmd_receive.cpp
//! and CMDRECV_Morph.cpp. Buildings require singleton network selections, while
//! UnitMorph applies to every selected larva. See TEST-REPORT for source links.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Metadata {
    pub building: bool,
    pub produces: bool,
    /// Low three bits are Zerg=1, Terran=2, Protoss=4.
    pub race_bits: u8,
}

/// Bounded fallback for the supported stock producers when optional DAT
/// discovery is unavailable. Verified against installed arr/units.dat SHA-256
/// da2ee6f116b77329048ec6be9e02721a527fde23d4b05b1b1336e634490abcba.
/// Readable live metadata takes precedence; engine command checks remain in force.
pub fn standard_production_metadata(kind: u16) -> Option<Metadata> {
    let race_bits = match kind {
        106 | 111 | 113 | 114 => 2, // Command Center, Barracks, Factory, Starport
        130..=133 => 1, // Infested Command Center, Hatchery, Lair, Hive: rally only
        154 | 155 | 160 | 167 => 4, // Nexus, Robotics Facility, Gateway, Stargate
        35 => return Some(Metadata { building: false, produces: false, race_bits: 1 }),
        _ => return None,
    };
    Some(Metadata { building: true, produces: true, race_bits })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    RightClick { queued: bool },
    Attack { queued: bool },
    Stop { queued: bool },
    Rally { queued: bool },
    Train { unit: u16 },
    Morph { unit: u16 },
}
impl Action {
    pub fn is_production(self) -> bool {
        matches!(self, Self::Train { .. } | Self::Morph { .. })
    }
    pub fn queued(self) -> bool {
        match self {
            Self::RightClick { queued } | Self::Attack { queued }
            | Self::Stop { queued } | Self::Rally { queued } => queued,
            // Train/Morph have a u16 unit ID, no queue flag. They are additive
            // regardless of the high byte of that ID.
            Self::Train { .. } | Self::Morph { .. } => false,
        }
    }
}

pub fn classify(command: &[u8]) -> Option<Action> {
    let queued = |offset| match command.get(offset) {
        Some(&0) => Some(false),
        Some(&1) => Some(true),
        _ => None,
    };
    match command.first()? {
        0x60 if command.len() == 12 => Some(Action::RightClick { queued: queued(11)? }),
        0x61 if command.len() == 13 => match command[11] {
            8..=12 | 14 | 53 | 59 | 134 | 135 => Some(Action::Attack { queued: queued(12)? }),
            39 | 40 => Some(Action::Rally { queued: queued(12)? }),
            _ => None,
        },
        0x1a if command.len() == 2 => Some(Action::Stop { queued: queued(1)? }),
        0x1b | 0x1c if command.len() == 1 => Some(Action::Stop { queued: false }),
        0x1f | 0x23 if command.len() == 3 => {
            let unit = u16::from_le_bytes([command[1], command[2]]);
            // The original receiver accepts types up through Disruption Web,
            // before the building IDs. Captured UI commands still undergo the
            // game's own prerequisites, resources, supply, and queue checks.
            if unit >= 106 { return None; }
            if command[0] == 0x1f { Some(Action::Train { unit }) }
            else { Some(Action::Morph { unit }) }
        }
        _ => None,
    }
}

pub fn allowed(action: Action, kind: u16, metadata: Metadata) -> bool {
    let tp_producer = metadata.building && metadata.produces
        && matches!(metadata.race_bits & 7, 2 | 4);
    // Zerg factory metadata also appears on other modified unit types. Enable
    // the additional rally path only for these four stock producer buildings.
    let zerg_producer_type = matches!(kind, 130..=133);
    let zerg_rally = zerg_producer_type && metadata.building && metadata.produces
        && metadata.race_bits & 7 == 1;
    match action {
        // Zerg production remains the separate larva-morph path, even if a
        // modified metadata table gives one of these buildings T/P race bits.
        Action::Train { unit } => tp_producer && !zerg_producer_type && unit < 106,
        Action::Rally { .. } => tp_producer || zerg_rally,
        Action::Morph { unit } => kind == 35 && unit < 106,
        Action::RightClick { .. } | Action::Attack { .. } | Action::Stop { .. } => true,
    }
}

/// Selection packets reject additional buildings, regardless of the command.
pub fn dispatch_batch_limit(metadata: Metadata) -> usize {
    if metadata.building { 1 } else { 12 }
}

/// A new movement/rally command can supersede earlier movement, but must not
/// discard production requests that have already been observed and accepted.
pub fn retains_pending(new_action: Action, pending_command: &[u8]) -> bool {
    let production=classify(pending_command).is_some_and(Action::is_production);
    // Morph supersedes old larva movement before its type becomes Egg.
    // Otherwise a deferred old movement job could cancel all accepted morphs.
    if matches!(new_action,Action::Morph{..}) {return production;}
    new_action.is_production() || new_action.queued() || production
}

/// Reads fields 22 (u32 ability flags) and 44 (u8 group/race flags) from the
/// x64 DatTable array: pointer at +0, element width at +8, entry count at +12.
/// The caller supplies bounded reads; this function neither dereferences raw
/// pointers nor writes anything. Metadata never depends on visual discovery.
pub fn read_metadata(
    table: usize,
    stride: usize,
    kind: u16,
    mut reader: impl FnMut(usize, usize) -> Option<usize>,
) -> Option<Metadata> {
    if table == 0 || stride != 16 { return None; }
    let mut field = |index: usize, size: usize| -> Option<usize> {
        let header = table.checked_add(index.checked_mul(stride)?)?;
        let data = reader(header, 8)?;
        let width = reader(header.checked_add(8)?, 4)?;
        let entries = reader(header.checked_add(12)?, 4)?;
        if data == 0 || width != size || entries == 0 || entries > 65536
            || kind as usize >= entries { return None; }
        let address = data.checked_add((kind as usize).checked_mul(size)?)?;
        reader(address, size)
    };
    let flags = u32::try_from(field(22, 4)?).ok()?;
    let groups = u8::try_from(field(44, 1)?).ok()?;
    Some(Metadata {
        building: flags & 1 != 0,
        // The factory group includes Starport/Stargate, which lack ability bit 31.
        produces: groups & 0x20 != 0,
        race_bits: groups & 7,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn producer(race_bits: u8) -> Metadata {
        Metadata { building: true, produces: true, race_bits }
    }
    #[test]
    fn zerg_rally_does_not_expand_to_other_buildings_or_invalid_metadata() {
        let rally = Action::Rally { queued: false };
        for kind in [35, 36, 129, 134, 135, 138, 139, 141, 146, 147, 148, 149] {
            assert!(!allowed(rally, kind, producer(1)));
            assert!(standard_production_metadata(kind).is_none() || kind == 35);
        }
        for kind in 130..=133 {
            for meta in [Metadata { building: false, ..producer(1) },
                Metadata { produces: false, ..producer(1) }, producer(0),
                producer(3), producer(6), producer(7)]
            {
                assert!(!allowed(rally, kind, meta));
            }
        }
    }
    #[test]
    fn stock_tp_production_and_rally_support_remain_available() {
        for kind in [106, 111, 113, 114, 154, 155, 160, 167] {
            let meta = standard_production_metadata(kind).unwrap();
            assert!(allowed(Action::Train { unit: 7 }, kind, meta));
            assert!(allowed(Action::Rally { queued: false }, kind, meta));
            assert!(allowed(Action::RightClick { queued: false }, kind, meta));
            assert_eq!(dispatch_batch_limit(meta), 1);
        }
        assert!(!allowed(Action::Rally { queued: false }, 35,
            standard_production_metadata(35).unwrap()));
        assert!(allowed(Action::Morph { unit: 37 }, 35,
            standard_production_metadata(35).unwrap()));
    }
    #[test]
    fn air_factory_metadata_uses_factory_group_without_ground_producer_ability_bit() {
        let (table, mut data) = dat_fixture();
        for (kind, flags, groups) in [(114, 0x4400_0021, 0x32), (167, 0x4408_0001, 0x34)] {
            data.insert((0x20_0000 + kind * 4, 4), flags);
            data.insert((0x30_0000 + kind, 1), groups);
            let meta = metadata(table, kind as u16, &data).unwrap();
            assert!(meta.produces);
            assert!(allowed(Action::Train { unit: 8 }, kind as u16, meta));
        }
    }
    fn target(order: u8, queued: u8) -> Vec<u8> {
        let mut packet = vec![0x61, 16, 0, 32, 0, 0, 0, 0, 0, 228, 0, order, queued];
        packet.shrink_to_fit();
        packet
    }
    fn dat_fixture() -> (usize, BTreeMap<(usize, usize), usize>) {
        // The addresses deliberately do not point to actual memory; all loader
        // reads must come from this independently described x64 wire fixture.
        let table = 0x10_0000;
        let mut data = BTreeMap::new();
        for (index, pointer, width) in [(22, 0x20_0000, 4), (44, 0x30_0000, 1)] {
            data.insert((table + index * 16, 8), pointer);
            data.insert((table + index * 16 + 8, 4), width);
            data.insert((table + index * 16 + 12, 4), 228);
        }
        // Barracks, Gateway, Hatchery, Larva, Pylon. Building and ProducesUnits
        // are distinct, and race/group high bits must not affect race matching.
        for (kind, flags, groups) in [
            (111, 0x8000_0001, 0x32),
            (160, 0x8000_0001, 0x34),
            (131, 0x8000_0001, 0x31),
            (35, 0, 0x09),
            (156, 0x0000_0001, 0x14),
        ] {
            data.insert((0x20_0000 + kind * 4, 4), flags);
            data.insert((0x30_0000 + kind, 1), groups);
        }
        (table, data)
    }
    fn metadata(table: usize, kind: u16, data: &BTreeMap<(usize, usize), usize>) -> Option<Metadata> {
        read_metadata(table, 16, kind, |address, size| data.get(&(address, size)).copied())
    }

    #[test]
    fn captured_train_and_larva_packets_have_u16_payload_not_queue_flag() {
        assert_eq!(classify(&[0x1f, 0, 0]), Some(Action::Train { unit: 0 }));
        assert_eq!(classify(&[0x1f, 64, 0]), Some(Action::Train { unit: 64 }));
        assert_eq!(classify(&[0x23, 37, 0]), Some(Action::Morph { unit: 37 }));
        assert!(!classify(&[0x1f, 64, 0]).unwrap().queued());
        assert!(classify(&[0x1f, 105, 0]).is_some());
        for packet in [&[0x1f, 106, 0][..], &[0x23, 228, 0], &[0x1f, 0, 1],
            &[0x1f, 64], &[0x23, 37, 0, 0], &[0x1f], &[]] {
            assert!(classify(packet).is_none(), "{packet:?}");
        }
    }
    #[test]
    fn only_exact_supported_records_are_classified() {
        for opcode in [0x1a, 0x60, 0x61] {
            let mut packet = match opcode {
                0x1a => vec![opcode, 0],
                0x60 => { let mut p = vec![0; 12]; p[0] = opcode; p },
                _ => target(14, 0),
            };
            assert!(classify(&packet).is_some());
            *packet.last_mut().unwrap() = 1;
            assert!(classify(&packet).unwrap().queued());
            *packet.last_mut().unwrap() = 2;
            assert!(classify(&packet).is_none());
            packet.pop();
            assert!(classify(&packet).is_none());
        }
        for opcode in [0x1b, 0x1c] {
            assert_eq!(classify(&[opcode]), Some(Action::Stop { queued: false }));
            assert!(classify(&[opcode, 0]).is_none());
        }
        for opcode in [0x27, 0x30, 0x32, 0x35, 0x20, 0x19, 0x63] {
            assert!(classify(&[opcode, 37, 0]).is_none());
        }
    }
    #[test]
    fn rally_and_attack_orders_are_distinct_and_abilities_are_not_copied() {
        for order in [39, 40] {
            assert_eq!(classify(&target(order, 1)), Some(Action::Rally { queued: true }));
        }
        for order in [8, 9, 10, 11, 12, 14, 53, 59, 134, 135] {
            assert_eq!(classify(&target(order, 0)), Some(Action::Attack { queued: false }));
        }
        for order in [0, 33, 38, 41, 43, 44, 60, 255] {
            assert!(classify(&target(order, 0)).is_none());
        }
    }
    #[test]
    fn x64_dat_fixture_distinguishes_tp_producers_and_zerg() {
        let (table, data) = dat_fixture();
        assert_eq!(metadata(table, 111, &data), Some(producer(2)));
        assert_eq!(metadata(table, 160, &data), Some(producer(4)));
        assert_eq!(metadata(table, 131, &data), Some(producer(1)));
        assert_eq!(metadata(table, 35, &data), Some(Metadata { building: false, produces: false, race_bits: 1 }));
        assert_eq!(metadata(table, 156, &data), Some(Metadata { building: true, produces: false, race_bits: 4 }));
    }
    #[test]
    fn malformed_dat_headers_and_overflow_are_rejected_without_unbounded_reads() {
        let (table, mut data) = dat_fixture();
        for (field, offset, value) in [(22, 0, 0), (22, 8, 8), (22, 12, 111),
            (44, 0, 0), (44, 8, 4), (44, 12, 65537)] {
            let size = if offset == 0 { 8 } else { 4 };
            let key = (table + field * 16 + offset, size);
            let old = data.insert(key, value).unwrap();
            assert!(metadata(table, 111, &data).is_none());
            data.insert(key, old);
        }
        data.remove(&(0x30_0000 + 111, 1));
        assert!(metadata(table, 111, &data).is_none());
        let mut reads = 0;
        for (base, stride) in [(0, 16), (table, 12), (usize::MAX - 8, 16)] {
            assert!(read_metadata(base, stride, 111, |_, _| { reads += 1; Some(0) }).is_none());
        }
        assert_eq!(reads, 0);
        let (table, mut data) = dat_fixture();
        data.insert((table + 22 * 16, 8), usize::MAX - 1);
        assert!(metadata(table, 111, &data).is_none());
    }
    #[test]
    fn production_eligibility_follows_race_and_producer_flags_not_player_race() {
        let train = Action::Train { unit: 64 };
        let rally = Action::Rally { queued: false };
        for race in [2, 4] {
            assert!(allowed(train, 154, producer(race)));
            assert!(allowed(rally, 154, producer(race)));
        }
        for meta in [producer(1), producer(0), producer(3), producer(6), producer(7),
            Metadata { building: false, ..producer(4) },
            Metadata { produces: false, ..producer(4) }] {
            assert!(!allowed(train, 154, meta));
            assert!(!allowed(rally, 154, meta));
        }
        assert!(!allowed(Action::Train { unit: 106 }, 154, producer(4)));
        for kind in [35, 38, 43, 131, 132, 133] {
            assert_eq!(allowed(Action::Morph { unit: 37 }, kind, Metadata::default()), kind == 35);
        }
        assert!(!allowed(Action::Morph { unit: 106 }, 35, Metadata::default()));
    }
    #[test]
    fn existing_control_policy_is_unchanged_when_metadata_is_unavailable() {
        for action in [Action::RightClick { queued: false }, Action::Attack { queued: true },
            Action::Stop { queued: false }] {
            for kind in [0, 35, 64, 106, 131, 160] {
                assert!(allowed(action, kind, Metadata::default()));
            }
        }
    }
    #[test]
    fn any_building_uses_singleton_dispatch_even_when_not_a_producer() {
        for meta in [producer(2), producer(4), producer(1),
            Metadata { produces: false, ..producer(4) }] {
            assert_eq!(dispatch_batch_limit(meta), 1);
        }
        assert_eq!(dispatch_batch_limit(Metadata::default()), 12);
    }
    #[test]
    fn train_clicks_are_additive_and_rally_does_not_drop_previous_production() {
        let pending = [vec![0x1f, 64, 0], vec![0x23, 37, 0], target(40, 0), vec![0x1a, 0]];
        for new_action in [Action::Train { unit: 64 },
            Action::RightClick { queued: true }, Action::Rally { queued: true }] {
            assert!(pending.iter().all(|packet| retains_pending(new_action, packet)));
        }
        for new_action in [Action::Morph { unit: 37 }, Action::RightClick { queued: false }, Action::Attack { queued: false },
            Action::Stop { queued: false }, Action::Rally { queued: false }] {
            assert!(retains_pending(new_action, &pending[0]));
            assert!(retains_pending(new_action, &pending[1]));
            assert!(!retains_pending(new_action, &pending[2]));
            assert!(!retains_pending(new_action, &pending[3]));
            assert!(!retains_pending(new_action, &[0x35, 132, 0]));
        }
    }
}
