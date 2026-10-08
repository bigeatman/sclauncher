// Hydralisk/Lurker integration fixtures are owned arrays in this test process.
// They observe callback completion and plan copies; they never run a sender,
// initialize hooks, inspect another process, or alter installed game files.

#[test]
fn hydralisk_lurker_capture_copies_every_other_owned_hydralisk_for_all_local_slots() {
    for owner in 0..8 {
        let mut f = Fixture::new(27, owner, 38);
        let mut runtime = f.runtime();
        runtime.metadata = None; // Optional DAT/visual discovery is unnecessary.
        assert!(runtime.visual.is_none());
        let mut state = building_state(&f, 38);
        let capture = capture_owned_callback(&f, &runtime, &state);
        let original = capture.ids.clone();
        let command = vec![0x23, 103, 0];
        append_owned_callback(&mut f, &command);
        f.write_u16(0, 0x8c, 97); // Same generation becomes incomplete Lurker Egg.
        f.write_u32(0, 0x140, 0);
        let native_selection = *f.selection;
        let occupied_buffer = f.outgoing_buffer.to_vec();

        complete_captured_control(&runtime, &mut state, capture, false, 7);

        assert!(state.active, "owner {owner}: {}", state.message);
        assert_eq!(state.count, 27);
        assert_eq!(state.pending.iter().map(|p| p.ids.len()).collect::<Vec<_>>(), [12, 12, 2]);
        let copied: Vec<_> = state.pending.iter().flat_map(|p| p.ids.iter().copied()).collect();
        assert_eq!(copied, ids(&(1..27).collect::<Vec<_>>(), 1, 13));
        assert_eq!(active_selection_ids(&runtime, &state), Some(original.clone()));
        for job in &state.pending {
            assert_eq!(job.command, command);
            assert_eq!(job.restore, original);
            assert!(job.ids.iter().all(|id| runtime.valid_id(*id, owner, 38, 13)));
            assert!(job.restore.iter().all(|id| runtime.valid_morph_restore(*id, owner, 13, 38)));
            assert!(validate_pending_reference(&runtime, &state, job).is_ok());
            let records = crate::batch::plan_one(&job.ids, &job.restore, &job.command, 480).unwrap();
            assert_eq!(capture_selection(&records[0]), Some(job.ids.clone()));
            assert_eq!(records[1], [0x23, 103, 0]);
            assert_eq!(capture_selection(&records[2]), Some(original.clone()));
            assert!(!job.ids.iter().any(|id| original.contains(id)));
        }
        assert_eq!(*f.selection, native_selection);
        assert_eq!(f.outgoing_buffer.as_ref(), occupied_buffer.as_slice());
        assert_eq!(f.globals[9], 3); // Only the fixture's original command exists.
        assert_eq!(state.sent, 0); // Queue construction does not invoke the sender.
    }
}

#[test]
fn hydralisk_lurker_capture_excludes_multiple_originals_before_any_type_change() {
    let mut f = Fixture::new(27, 3, 38);
    f.metadata(38, 0, 1);
    for (slot, index) in [5, 0, 10].into_iter().enumerate() {
        f.selection[slot] = f.pointer(index);
    }
    let runtime = f.runtime();
    let mut state = building_state(&f, 38);
    state.last_selection = ids(&[5, 0, 10], 1, 13);
    let capture = capture_owned_callback(&f, &runtime, &state);
    append_owned_callback(&mut f, &[0x23, 103, 0]);
    let selection = *f.selection;

    complete_captured_control(&runtime, &mut state, capture, false, 7);

    assert!(state.active, "{}", state.message);
    assert_eq!(state.count, 27);
    assert_eq!(state.pending.iter().map(|p| p.ids.len()).collect::<Vec<_>>(), [12, 12]);
    let wanted: Vec<_> = (0..27).filter(|index| ![5, 0, 10].contains(index)).collect();
    assert_eq!(state.pending.iter().flat_map(|p| p.ids.iter().copied()).collect::<Vec<_>>(),
        ids(&wanted, 1, 13));
    for job in &state.pending {
        assert_eq!(job.restore, ids(&[5, 0, 10], 1, 13));
        assert_eq!(job.command, [0x23, 103, 0]);
        assert!(validate_pending_reference(&runtime, &state, job).is_ok());
    }
    assert_eq!(*f.selection, selection);
}

#[test]
fn hydralisk_original_group_keeps_order_when_first_member_becomes_lurker_egg() {
    let mut f = Fixture::new(30, 5, 38);
    f.selection[0] = f.pointer(2);
    f.selection[1] = f.pointer(5);
    f.selection[2] = f.pointer(10);
    let runtime = f.runtime();
    let mut state = building_state(&f, 38);
    state.last_selection = ids(&[2, 5, 10], 1, 13);
    let capture = capture_owned_callback(&f, &runtime, &state);
    append_owned_callback(&mut f, &[0x23, 103, 0]);
    f.write_u16(2, 0x8c, 97);
    f.write_u32(2, 0x140, 0);

    complete_captured_control(&runtime, &mut state, capture, false, 7);

    assert!(state.active, "{}", state.message);
    assert_eq!(state.count, 30);
    assert_eq!(active_selection_ids(&runtime, &state), Some(ids(&[2, 5, 10], 1, 13)));
    assert_eq!(state.pending.iter().map(|p| p.ids.len()).collect::<Vec<_>>(), [12, 12, 3]);
    for job in &state.pending {
        assert_eq!(job.restore, ids(&[2, 5, 10], 1, 13));
        assert!(!job.ids.iter().any(|id| job.restore.contains(id)));
        assert!(validate_pending_reference(&runtime, &state, job).is_ok());
    }
}

#[test]
fn hydralisk_lurker_restore_is_source_specific_and_generation_identical() {
    let mut f = Fixture::new(2, 2, 38);
    let runtime = f.runtime();
    let original = ids(&[0], 1, 13)[0];
    assert!(runtime.valid_morph_restore(original, 2, 13, 38));
    assert!(!runtime.valid_morph_restore(original, 2, 13, 35));
    f.write_u32(0, 0x140, 0); // An incomplete Hydralisk is not a control source.
    assert!(!runtime.valid_morph_restore(original, 2, 13, 38));
    f.write_u16(0, 0x8c, 97);
    assert!(runtime.valid_morph_restore(original, 2, 13, 38));
    assert!(!runtime.valid_morph_restore(original, 2, 13, 35));
    assert_eq!(runtime.morph_selection_ids(2, 13, 38), Some(vec![original]));
    f.write_u8(0, 0xe9, 2);
    assert!(!runtime.valid_morph_restore(original, 2, 13, 38));
    assert_eq!(runtime.morph_selection_ids(2, 13, 38), Some(ids(&[0], 2, 13)));
    f.write_u8(0, 0xe9, 1);
    f.write_u16(0, 0x8c, 36); // Ordinary Egg belongs only to the Larva path.
    assert!(!runtime.valid_morph_restore(original, 2, 13, 38));
    assert!(runtime.valid_morph_restore(original, 2, 13, 35));
    f.write_u16(0, 0x8c, 103); // A finished Lurker is not a pending reference.
    f.write_u32(0, 0x140, 1);
    assert!(!runtime.valid_morph_restore(original, 2, 13, 38));
    assert!(runtime.morph_selection_ids(2, 13, 38).is_none());
    for source_kind in [36, 37, 97, 103, 111, u16::MAX] {
        assert!(!runtime.valid_morph_restore(original, 2, 13, source_kind));
    }
}

#[test]
fn every_original_hydralisk_can_become_multiselectable_lurker_egg_before_restore() {
    // Lurker Egg is multi-selectable, unlike the separate Larva/Egg path.
    for transformed in [vec![1usize], vec![0usize, 1]] {
        let mut f = Fixture::new(15, 7, 38);
        f.selection[1] = f.pointer(1);
        let runtime = f.runtime();
        let mut state = building_state(&f, 38);
        state.last_selection = ids(&[0, 1], 1, 13);
        let capture = capture_owned_callback(&f, &runtime, &state);
        append_owned_callback(&mut f, &[0x23, 103, 0]);
        for index in transformed {
            f.write_u16(index, 0x8c, 97);
            f.write_u32(index, 0x140, 0);
        }
        assert_eq!(runtime.morph_selection_ids(7, 13, 38), Some(state.last_selection.clone()));

        complete_captured_control(&runtime, &mut state, capture, false, 7);

        assert!(state.active, "{}", state.message);
        assert_eq!(state.count, 15);
        assert_eq!(state.pending.iter().map(|p| p.ids.len()).collect::<Vec<_>>(), [12, 1]);
        assert_eq!(state.pending.iter().flat_map(|p| p.ids.iter().copied()).collect::<Vec<_>>(),
            ids(&(2..15).collect::<Vec<_>>(), 1, 13));
        for job in &state.pending {
            assert_eq!(job.restore, ids(&[0, 1], 1, 13));
            assert!(validate_pending_reference(&runtime, &state, job).is_ok());
            let records = crate::batch::plan_one(&job.ids, &job.restore, &job.command, 480).unwrap();
            assert_eq!(capture_selection(&records[2]), Some(ids(&[0, 1], 1, 13)));
        }
        assert_eq!(state.sent, 0);
    }
}

#[test]
fn multi_original_lurker_morph_rejects_mixed_eggs_cocoons_foreign_owners_and_reused_ids() {
    for failure in 0..4 {
        let mut f = Fixture::new(15, 7, 38);
        f.selection[1] = f.pointer(1);
        let runtime = f.runtime();
        let mut state = building_state(&f, 38);
        state.last_selection = ids(&[0, 1], 1, 13);
        let capture = capture_owned_callback(&f, &runtime, &state);
        append_owned_callback(&mut f, &[0x23, 103, 0]);
        for index in 0..2 {
            f.write_u16(index, 0x8c, 97);
            f.write_u32(index, 0x140, 0);
        }
        match failure {
            0 => f.write_u16(1, 0x8c, 36),
            1 => f.write_u16(1, 0x8c, 59),
            2 => f.write_u8(1, 0x68, 6),
            3 => f.write_u8(1, 0xe9, 2),
            _ => unreachable!(),
        }

        complete_captured_control(&runtime, &mut state, capture, false, 7);

        assert!(!state.active, "failure {failure}");
        assert!(state.pending.is_empty());
        assert_eq!(state.sent, 0);
    }
}

#[test]
fn hydralisk_sync_selection_then_lurker_morph_uses_only_exact_original_ids() {
    let mut f = Fixture::new(27, 0, 38);
    f.selection[1] = f.pointer(5);
    f.selection[2] = f.pointer(10);
    append_owned_callback(&mut f, &[5, 7]); // Pre-existing occupied prefix.
    let runtime = f.runtime();
    let mut state = building_state(&f, 38);
    state.last_selection = ids(&[0, 5, 10], 1, 13);
    let capture = capture_owned_callback(&f, &runtime, &state);
    let mut command = selection_record(&capture.ids).unwrap();
    command.extend([0x23, 103, 0]);
    append_owned_callback(&mut f, &command);
    f.write_u16(0, 0x8c, 97);
    f.write_u32(0, 0x140, 0);

    complete_captured_control(&runtime, &mut state, capture, false, 7);

    assert!(state.active, "{}", state.message);
    assert_eq!(state.pending.len(), 2);
    assert_eq!(state.pending.iter().flat_map(|p| p.ids.iter()).count(), 24);
    assert!(state.pending.iter().all(|p| p.command == [0x23, 103, 0]));
    assert!(state.pending.iter().all(|p| p.restore == ids(&[0, 5, 10], 1, 13)));
    assert_eq!(f.globals[9], 2 + command.len());
    assert_eq!(&f.outgoing_buffer[..2], &[5, 7]);
}

#[test]
fn hydralisk_sync_morph_rejects_reordered_reused_or_extra_records() {
    let originals = ids(&[0, 1], 1, 13);
    let original_selection = selection_record(&originals).unwrap();
    let mut swapped = original_selection.clone();
    swapped[2..6].copy_from_slice(&originals[1].to_le_bytes());
    swapped[6..10].copy_from_slice(&originals[0].to_le_bytes());
    let mut reused = original_selection.clone();
    reused[2..6].copy_from_slice(&(originals[0] + (1 << 13)).to_le_bytes());
    for command in [
        [swapped, vec![0x23, 103, 0]].concat(),
        [reused, vec![0x23, 103, 0]].concat(),
        [original_selection.clone(), vec![0x23, 103, 0, 0]].concat(),
        [original_selection.clone(), vec![0x23, 103, 0, 0x23, 103, 0]].concat(),
        [original_selection.clone(), click_order(false)].concat(),
        [original_selection.clone(), vec![0x23, 103]].concat(),
    ] {
        let mut f = Fixture::new(15, 4, 38);
        f.selection[1] = f.pointer(1);
        let runtime = f.runtime();
        let mut state = building_state(&f, 38);
        state.last_selection = originals.clone();
        let capture = capture_owned_callback(&f, &runtime, &state);
        append_owned_callback(&mut f, &command);
        f.write_u16(0, 0x8c, 97);
        f.write_u32(0, 0x140, 0);

        complete_captured_control(&runtime, &mut state, capture, false, 7);

        assert!(!state.active, "command {command:?}");
        assert!(state.pending.is_empty());
    }
}

#[test]
fn hydralisk_transition_requires_one_proven_exact_lurker_morph_append() {
    for command in [
        vec![], train_packet(103), click_order(false), vec![0x23, 103],
        vec![0x23, 103, 0, 0], vec![0x23, 41, 0], vec![0x23, 96, 0],
        vec![0x23, 103, 0, 0x23, 103, 0],
        selection_record(&ids(&[0], 1, 13)).unwrap(),
    ] {
        let mut f = Fixture::new(15, 6, 38);
        let runtime = f.runtime();
        let mut state = building_state(&f, 38);
        let capture = capture_owned_callback(&f, &runtime, &state);
        append_owned_callback(&mut f, &command);
        f.write_u16(0, 0x8c, 97);
        f.write_u32(0, 0x140, 0);

        complete_captured_control(&runtime, &mut state, capture, false, 7);

        assert!(!state.active, "command {command:?}");
        assert!(state.pending.is_empty());
        assert_eq!(state.sent, 0);
    }
}

#[test]
fn hydralisk_morph_capture_preserves_owner_generation_session_and_callback_guards() {
    for failure in 0..20 {
        let mut f = Fixture::new(15, 3, 38);
        let runtime = f.runtime();
        let mut state = building_state(&f, 38);
        let capture = capture_owned_callback(&f, &runtime, &state);
        append_owned_callback(&mut f, &[0x23, 103, 0]);
        f.write_u16(0, 0x8c, 97);
        f.write_u32(0, 0x140, 0);
        match failure {
            0 => f.write_u8(0, 0x68, 6),
            1 => f.write_u8(0, 0xe9, 2),
            2 => f.selection[0] = f.pointer(1),
            3 => f.selection[0] = 0,
            4 => f.globals[3] = 6,
            5 => f.globals[4] = 1,
            6 => f.globals[8] = 101,
            7 => f.globals[7] = 1,
            8 => f.write_u32(0, 0x10, 0),
            9 => f.selection[1] = f.pointer(1),
            10 => f.globals[5] = 2,
            11 => f.globals[6] = 0,
            12 => state.owner = 6,
            13 => state.kind = 37,
            14 => state.shift = 11,
            15 => state.last_selection = ids(&[1], 1, 13),
            16 => f.globals[10] = 488,
            17 => f.write_u32(0, 0x140, 0x40),
            _ => (),
        }

        complete_captured_control(&runtime, &mut state, capture, failure == 18,
            if failure == 19 { 8 } else { 7 });

        assert!(!state.active, "failure {failure}: {}", state.message);
        assert!(state.pending.is_empty());
        assert_eq!(state.sent, 0);
        assert_eq!(f.globals[9], 3);
    }
}

#[test]
fn hydralisk_lurker_copies_filter_foreign_incomplete_dead_and_other_unit_types() {
    let mut f = Fixture::new(8, 1, 38);
    f.write_u8(1, 0x68, 2);
    f.write_u32(2, 0x140, 0);
    f.write_u16(3, 0x8c, 37);
    f.write_u32(4, 0x10, 0);
    f.write_u64(5, 0x18, 0);
    f.write_u32(6, 0x140, 0x41);
    let runtime = f.runtime();
    let mut state = building_state(&f, 38);
    let capture = capture_owned_callback(&f, &runtime, &state);
    append_owned_callback(&mut f, &[0x23, 103, 0]);
    f.write_u16(0, 0x8c, 97);
    f.write_u32(0, 0x140, 0);

    complete_captured_control(&runtime, &mut state, capture, false, 7);

    assert!(state.active, "{}", state.message);
    assert_eq!(state.count, 2);
    assert_eq!(state.pending.len(), 1);
    assert_eq!(state.pending[0].ids, ids(&[7], 1, 13));
    assert_eq!(state.pending[0].restore, ids(&[0], 1, 13));
    assert_eq!(state.pending[0].command, [0x23, 103, 0]);
}

#[test]
fn hydralisk_pending_reference_rechecks_selection_and_identity_before_dispatch() {
    for failure in 0..6 {
        let mut f = Fixture::new(15, 7, 38);
        let runtime = f.runtime();
        let mut state = building_state(&f, 38);
        let capture = capture_owned_callback(&f, &runtime, &state);
        append_owned_callback(&mut f, &[0x23, 103, 0]);
        f.write_u16(0, 0x8c, 97);
        f.write_u32(0, 0x140, 0);
        complete_captured_control(&runtime, &mut state, capture, false, 7);
        assert!(state.active);
        let job = state.pending.front().unwrap();
        assert!(validate_pending_reference(&runtime, &state, job).is_ok());
        match failure {
            0 => f.selection[0] = f.pointer(1),
            1 => f.write_u8(0, 0xe9, 2),
            2 => f.write_u8(0, 0x68, 6),
            3 => f.write_u32(0, 0x10, 0),
            4 => f.write_u16(0, 0x8c, 103),
            5 => f.selection[0] = 0,
            _ => unreachable!(),
        }
        assert!(validate_pending_reference(&runtime, &state, job).is_err(), "failure {failure}");
        assert_ne!(active_selection_ids(&runtime, &state), Some(state.last_selection.clone()));
        assert_eq!(f.globals[9], 3);
        assert_eq!(state.sent, 0);
    }
}

#[test]
fn hydralisk_lurker_hud_reports_actual_egg_and_session_boundary_clears_old_requests() {
    let mut f = Fixture::new(15, 0, 38);
    let runtime = f.runtime();
    let mut state = building_state(&f, 38);
    let capture = capture_owned_callback(&f, &runtime, &state);
    append_owned_callback(&mut f, &[0x23, 103, 0]);
    f.write_u16(0, 0x8c, 97);
    f.write_u32(0, 0x140, 0);
    complete_captured_control(&runtime, &mut state, capture, false, 7);
    assert!(state.active);
    let display = hud_selection(Some(&runtime), &state, true);
    assert_eq!(display, DisplaySelection { control_active: false, in_game: true, count: 1, kind: 97 });
    let wire = encode_status(99, &state, "READY", None, display);
    let fields: Vec<_> = wire.trim().split('\t').collect();
    assert_eq!(&fields[2..4], &["0", "0"]);
    assert_eq!(&fields[8..11], &["1", "1", "97"]);
    let pending = state.pending.len();
    assert!(pending > 0); // Passive display must not cancel the planned morphs.

    reset_state_for_session(&mut state, 1, true);

    assert!(!state.active);
    assert!(state.pending.is_empty());
    assert!(state.last_selection.is_empty());
    assert_eq!(state.count, 0);
    assert!(state.last_key);
    assert!(state.visual_frame.is_none());
    f.selection.fill(0);
    assert_eq!(hud_selection(Some(&runtime), &state, true), DisplaySelection::empty(true));
}

#[test]
fn one_original_hydralisk_morph_without_remaining_sources_completes_without_duplicates() {
    let mut f = Fixture::new(1, 4, 38);
    let runtime = f.runtime();
    let mut state = building_state(&f, 38);
    let capture = capture_owned_callback(&f, &runtime, &state);
    append_owned_callback(&mut f, &[0x23, 103, 0]);
    f.write_u16(0, 0x8c, 97);
    f.write_u32(0, 0x140, 0);

    complete_captured_control(&runtime, &mut state, capture, false, 7);

    assert!(!state.active);
    assert!(state.pending.is_empty());
    assert_eq!(state.count, 0);
    assert_eq!(state.sent, 0);
    assert_eq!(f.globals[9], 3);
    assert_eq!(hud_selection(Some(&runtime), &state, true),
        DisplaySelection { control_active: false, in_game: true, count: 1, kind: 97 });
}
