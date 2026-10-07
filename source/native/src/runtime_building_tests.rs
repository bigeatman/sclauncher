// Integration fixtures use only this test process's owned byte arrays. They
// never initialize callbacks, discover a game, invoke a sender, or issue input.

fn building_state(fixture: &Fixture, kind: u16) -> State {
    let mut state = fixture.state();
    state.kind = kind;
    state
}
fn rally_packet(order: u8, queued: bool) -> Vec<u8> {
    // Preserve both position and an extended unit target in the real record.
    vec![0x61, 16, 0, 32, 0, 9, 32, 0, 0, 228, 0, order, u8::from(queued)]
}
fn train_packet(unit: u16) -> Vec<u8> {
    let bytes = unit.to_le_bytes();
    vec![0x1f, bytes[0], bytes[1]]
}
fn morph_packet(unit: u16) -> Vec<u8> {
    let bytes = unit.to_le_bytes();
    vec![0x23, bytes[0], bytes[1]]
}

#[test]
fn building_production_one_click_queues_one_request_per_other_owned_building() {
    for (kind, race, unit) in [(111, 2, 0), (160, 4, 65), (154, 4, 64)] {
        let mut f = Fixture::new(15, 7, kind);
        f.metadata(kind, 0x8000_0001, race | 0x30);
        let runtime = f.runtime();
        assert!(runtime.visual.is_none());
        let mut state = building_state(&f, kind);
        let command = train_packet(unit);
        let original = state.last_selection.clone();
        assert!(queue_control(&command, &original, &mut state, &runtime));
        assert_eq!(state.count, 15);
        assert_eq!(state.pending.len(), 14);
        for (index, job) in state.pending.iter().enumerate() {
            assert_eq!(job.ids, ids(&[index as u32 + 1], 1, 13));
            assert_eq!(job.restore, original);
            assert_eq!(job.command, command);
            assert_eq!(crate::batch::plan_one(&job.ids, &job.restore, &job.command, 480).unwrap()
                .iter().map(Vec::len).sum::<usize>(), 15);
        }
    }
}
#[test]
fn building_right_click_and_explicit_unit_or_ground_rally_all_use_singletons() {
    let mut f = Fixture::new(4, 7, 154);
    f.metadata(154, 0x8000_0001, 0x34);
    let runtime = f.runtime();
    for command in [click_order(false), rally_packet(39, false), rally_packet(40, false),
        rally_packet(40, true)] {
        let mut state = building_state(&f, 154);
        let original = state.last_selection.clone();
        assert!(queue_control(&command, &original, &mut state, &runtime));
        assert_eq!(state.pending.len(), 3);
        for (index, job) in state.pending.iter().enumerate() {
            assert_eq!(job.ids, ids(&[index as u32 + 1], 1, 13));
            assert_eq!(job.restore, original);
            assert_eq!(job.command, command);
        }
    }
}
#[test]
fn building_train_wire_keeps_original_once_and_serializes_each_copy_exactly() {
    let mut f = Fixture::new(3, 7, 154);
    f.metadata(154, 0x8000_0001, 0x34);
    let runtime = f.runtime();
    let mut state = building_state(&f, 154);
    let command = train_packet(64);
    let prefix = vec![0x05, 0x37, 0x05];
    let mut after = prefix.clone();
    after.extend(&command);
    let crate::event_capture::Observation::Command(observed) =
        crate::event_capture::observe_append(&prefix, &after, 480) else { panic!("exact production append not captured"); };
    assert_eq!(observed, [0x1f, 64, 0]);
    assert!(queue_control(&observed, &state.last_selection.clone(), &mut state, &runtime));
    let first = &state.pending[0];
    let bytes: Vec<u8> = crate::batch::plan_one(&first.ids, &first.restore, &first.command, 480)
        .unwrap().into_iter().flatten().collect();
    assert_eq!(bytes, vec![0x63, 1, 2, 32, 0, 0, 0x1f, 64, 0, 0x63, 1, 1, 32, 0, 0]);
    assert_eq!(state.pending[1].ids, ids(&[2], 1, 13));
    assert!(!state.pending.iter().any(|job| job.ids.contains(&state.last_selection[0])));
    assert_eq!(f.globals[9], 0);
    assert!(f.outgoing_buffer.iter().all(|byte| *byte == 0));
    assert_eq!(f.selection[0], f.pointer(0));
    // A buffer delta containing two records must not become two production clicks.
    after.extend(&command);
    assert!(matches!(crate::event_capture::observe_append(&prefix, &after, 480),
        crate::event_capture::Observation::Clear(crate::event_capture::ClearReason::UnsupportedOrIncompleteRecord)));
}
#[test]
fn building_rally_capture_preserves_target_coordinates_order_and_queue_bytes() {
    let mut f = Fixture::new(2, 0, 111);
    f.metadata(111, 0x8000_0001, 0x32);
    let runtime = f.runtime();
    let mut state = building_state(&f, 111);
    let command = rally_packet(39, true);
    let crate::event_capture::Observation::Command(observed) =
        crate::event_capture::observe_append(&[5], &[vec![5], command.clone()].concat(), 480)
        else { panic!("rally append not captured"); };
    assert!(queue_control(&observed, &state.last_selection.clone(), &mut state, &runtime));
    let job = state.pending.front().unwrap();
    let records = crate::batch::plan_one(&job.ids, &job.restore, &job.command, 480).unwrap();
    assert_eq!(records[0], vec![0x63, 1, 2, 32, 0, 0]);
    assert_eq!(records[1], command);
    assert_eq!(records[2], vec![0x63, 1, 1, 32, 0, 0]);
}
#[test]
fn building_production_filters_unowned_incomplete_other_type_dead_and_missing_sprite() {
    let mut f = Fixture::new(7, 7, 160);
    f.metadata(160, 0x8000_0001, 0x34);
    f.write_u8(1, 0x68, 0);
    f.write_u32(2, 0x140, 0);
    f.write_u16(3, 0x8c, 154);
    f.write_u32(4, 0x10, 0);
    f.write_u64(5, 0x18, 0);
    let runtime = f.runtime();
    let mut state = building_state(&f, 160);
    assert!(queue_control(&train_packet(65), &state.last_selection.clone(), &mut state, &runtime));
    assert_eq!(state.count, 2);
    assert_eq!(state.pending.len(), 1);
    assert_eq!(state.pending[0].ids, ids(&[6], 1, 13));
}
#[test]
fn building_native_selection_requires_exactly_one_and_never_rewrites_client_selection() {
    let mut f = Fixture::new(3, 7, 111);
    f.metadata(111, 0x8000_0001, 0x32);
    f.selection[1] = f.pointer(1);
    let runtime = f.runtime();
    let mut state = building_state(&f, 111);
    state.last_selection = ids(&[0, 1], 1, 13);
    state.active = false;
    assert_eq!(activate_same_type(&runtime, &mut state, 7, 100),
        Err("Select one completed owned building first"));
    assert!(!state.active);
    state.active = true;
    assert!(!queue_control(&train_packet(0), &state.last_selection.clone(), &mut state, &runtime));
    assert!(state.pending.is_empty());
    assert_eq!(f.selection[0], f.pointer(0));
    assert_eq!(f.selection[1], f.pointer(1));
}
#[test]
fn repeated_building_train_clicks_are_additive_and_rally_keeps_previous_production() {
    let mut f = Fixture::new(4, 7, 154);
    f.metadata(154, 0x8000_0001, 0x34);
    let runtime = f.runtime();
    let mut state = building_state(&f, 154);
    let original = state.last_selection.clone();
    assert!(queue_control(&train_packet(64), &original, &mut state, &runtime));
    assert!(queue_control(&train_packet(64), &original, &mut state, &runtime));
    assert_eq!(state.pending.len(), 6);
    assert!(queue_control(&rally_packet(40, false), &original, &mut state, &runtime));
    assert_eq!(state.pending.len(), 9);
    assert!(state.pending.iter().take(6).all(|job| job.command == train_packet(64)));
    assert!(state.pending.iter().skip(6).all(|job| job.command == rally_packet(40, false)));
    assert!(queue_control(&rally_packet(39, false), &original, &mut state, &runtime));
    assert_eq!(state.pending.len(), 9);
    assert!(state.pending.iter().take(6).all(|job| job.command == train_packet(64)));
    assert!(state.pending.iter().skip(6).all(|job| job.command == rally_packet(39, false)));
}
#[test]
fn research_upgrade_building_morph_and_fighter_production_are_never_queued() {
    for command in [vec![0x30, 1], vec![0x32, 1], vec![0x35, 132, 0], vec![0x27],
        train_packet(106), morph_packet(106)] {
        let mut f = Fixture::new(3, 7, 154);
        f.metadata(154, 0x8000_0001, 0x34);
        let runtime = f.runtime();
        let mut state = building_state(&f, 154);
        assert!(!queue_control(&command, &state.last_selection.clone(), &mut state, &runtime), "{command:?}");
        assert!(!state.active);
        assert!(state.pending.is_empty());
        assert_eq!(f.globals[9], 0);
    }
}
#[test]
fn zerg_building_train_rally_and_nonproducer_tp_train_rally_fail_closed() {
    for (kind, flags, race, command) in [
        (131, 0x8000_0001, 1, train_packet(37)),
        (131, 0x8000_0001, 1, rally_packet(40, false)),
        (156, 1, 4, train_packet(64)),
        (156, 1, 4, rally_packet(40, false)),
        (111, 0x8000_0001, 2, morph_packet(37)),
    ] {
        let mut f = Fixture::new(2, 7, kind);
        f.metadata(kind, flags, race);
        let runtime = f.runtime();
        let mut state = building_state(&f, kind);
        assert!(!queue_control(&command, &state.last_selection.clone(), &mut state, &runtime));
        assert!(state.pending.is_empty());
    }
}
#[test]
fn missing_metadata_keeps_command_center_scv_requests_and_singleton_restore() {
    for count in [3, 5, 7, 15] {
        for failure in 0..3 {
            let mut f = Fixture::new(count, 7, 106);
            f.metadata(106, 0xc400_1021, 0x32);
            let mut runtime = f.runtime();
            match failure {
                0 => runtime.metadata = None,
                1 => runtime.metadata.as_mut().unwrap().stride = 12,
                _ => { f.dat[22].width = 8; std::hint::black_box(&f.dat); },
            }
            let mut state = building_state(&f, 106);
            let original = state.last_selection.clone();
            let scv = train_packet(7);
            assert!(queue_control(&scv, &original, &mut state, &runtime));
            assert!(state.active);
            assert_eq!(state.count, count);
            assert_eq!(state.pending.len(), count - 1);
            for job in &state.pending {
                assert_eq!(job.ids.len(), 1);
                assert!(!job.ids.contains(&original[0]));
                assert_eq!(job.restore, original);
                assert_eq!(job.command, [0x1f, 7, 0]);
                assert_eq!(crate::batch::plan_one(&job.ids, &job.restore, &job.command, 480)
                    .unwrap().iter().map(Vec::len).sum::<usize>(), 15);
            }
            assert_eq!(f.globals[9], 0); // no actual command dispatch in this fixture
        }
    }
}
#[test]
fn supported_producers_and_larva_work_with_no_optional_dat_table() {
    for (kind, unit) in [(111,0),(113,2),(114,8),(154,64),(155,69),(160,65),(167,70),(35,37)] {
        let f = Fixture::new(3, 7, kind);
        let mut runtime = f.runtime(); runtime.metadata = None;
        let mut state = building_state(&f, kind);
        let command = if kind == 35 { morph_packet(unit) } else { train_packet(unit) };
        assert!(queue_control(&command, &state.last_selection.clone(), &mut state, &runtime));
        assert_eq!(state.pending.len(), if kind == 35 { 1 } else { 2 });
    }
}
#[test]
fn fallback_does_not_override_readable_metadata_or_expand_to_other_buildings() {
    let mut f = Fixture::new(3, 7, 106);
    f.metadata(106, 1, 0x12); // readable metadata deliberately marks non-producer
    let runtime = f.runtime(); let mut state = building_state(&f, 106);
    assert!(!queue_control(&train_packet(7), &state.last_selection.clone(), &mut state, &runtime));
    for kind in [107,109,112,134,156,200] {
        let f = Fixture::new(3, 7, kind); let mut runtime = f.runtime(); runtime.metadata = None;
        assert!(runtime.command_metadata(kind).is_none());
        let mut state = building_state(&f, kind);
        assert!(!queue_control(&train_packet(7), &state.last_selection.clone(), &mut state, &runtime));
        assert!(state.pending.is_empty());
    }
}
#[test]
fn larva_morph_excludes_every_original_and_copies_remaining_in_twelve_unit_batches() {
    for original_indices in [vec![0u32], vec![0u32, 5, 10]] {
        let mut f = Fixture::new(27, 7, 35);
        f.metadata(35, 0, 1);
        for (slot, index) in original_indices.iter().enumerate() {
            f.selection[slot] = f.pointer(*index as usize);
        }
        let runtime = f.runtime();
        let mut state = building_state(&f, 35);
        let original = ids(&original_indices, 1, 13);
        state.last_selection = original.clone();
        assert!(queue_control(&morph_packet(37), &original, &mut state, &runtime));
        assert_eq!(state.count, 27);
        let expected: Vec<_> = ids(&(0..27).collect::<Vec<_>>(), 1, 13)
            .into_iter().filter(|id| !original.contains(id)).collect();
        let copied: Vec<_> = state.pending.iter().flat_map(|job| job.ids.iter().copied()).collect();
        assert_eq!(copied, expected);
        assert_eq!(state.pending.len(), expected.len().div_ceil(12));
        for job in &state.pending {
            assert!(job.ids.len() <= 12);
            assert_eq!(job.restore, original);
            assert_eq!(job.command, morph_packet(37));
        }
    }
}
#[test]
fn new_larva_morph_replaces_stale_movement_but_keeps_earlier_morph_requests() {
    let mut f = Fixture::new(15, 7, 35);
    f.metadata(35, 0, 1);
    let runtime = f.runtime();
    let mut state = building_state(&f, 35);
    let original = state.last_selection.clone();
    assert!(queue_control(&click_order(false), &original, &mut state, &runtime));
    assert_eq!(state.pending.len(), 2);
    assert!(queue_control(&morph_packet(37), &original, &mut state, &runtime));
    assert_eq!(state.pending.len(), 2);
    assert!(state.pending.iter().all(|job| job.command == morph_packet(37)));
    assert!(queue_control(&morph_packet(41), &original, &mut state, &runtime));
    assert_eq!(state.pending.len(), 4);
    assert!(state.pending.iter().take(2).all(|job| job.command == morph_packet(37)));
    assert!(state.pending.iter().skip(2).all(|job| job.command == morph_packet(41)));
}
#[test]
fn one_original_larva_can_become_one_incomplete_egg_while_copy_jobs_remain() {
    let mut f = Fixture::new(15, 7, 35);
    f.metadata(35, 0, 1);
    let runtime = f.runtime();
    let mut state = building_state(&f, 35);
    let original = state.last_selection.clone();
    assert!(queue_control(&morph_packet(37), &original, &mut state, &runtime));
    f.write_u16(0, 0x8c, 36);
    f.write_u32(0, 0x140, 0); // Eggs are intentionally incomplete.
    assert!(selected_ids(&runtime, 7, 35, 13).is_none());
    assert_eq!(runtime.morph_selection_ids(7, 13), Some(original.clone()));
    assert_eq!(active_selection_ids(&runtime, &state), Some(original.clone()));
    assert!(runtime.valid_morph_restore(original[0], 7, 13));
    let job = state.pending.front().unwrap();
    assert!(job.ids.iter().all(|id| runtime.valid_id(*id, 7, 35, 13)));
    let records = crate::batch::plan_one(&job.ids, &job.restore, &job.command, 480).unwrap();
    assert_eq!(capture_selection(records.last().unwrap()), Some(original));
}
#[test]
fn later_or_multiple_eggs_cannot_be_silently_restored_as_a_different_selection() {
    for transformed in [vec![1usize], vec![0usize, 1]] {
        let mut f = Fixture::new(15, 7, 35);
        f.metadata(35, 0, 1);
        f.selection[1] = f.pointer(1);
        let runtime = f.runtime();
        let mut state = building_state(&f, 35);
        state.last_selection = ids(&[0, 1], 1, 13);
        assert!(queue_control(&morph_packet(37), &state.last_selection.clone(), &mut state, &runtime));
        assert_eq!(runtime.morph_selection_ids(7, 13), Some(state.last_selection.clone()));
        for index in transformed {
            f.write_u16(index, 0x8c, 36);
            f.write_u32(index, 0x140, 0);
        }
        assert!(runtime.morph_selection_ids(7, 13).is_none());
        assert!(active_selection_ids(&runtime, &state).is_none());
        // Individual IDs remain alive, but exact multi-selection restoration
        // is unavailable; the dispatch guard must reject the entire selection.
        assert!(state.last_selection.iter().all(|id| runtime.valid_morph_restore(*id, 7, 13)));
    }
}
#[test]
fn morph_restore_identity_rejects_reused_generation_foreign_owner_and_other_unit_kind() {
    let mut f = Fixture::new(2, 7, 35);
    let runtime = f.runtime();
    let original = ids(&[0], 1, 13)[0];
    f.write_u16(0, 0x8c, 36);
    f.write_u32(0, 0x140, 0);
    assert!(runtime.valid_morph_restore(original, 7, 13));
    f.write_u8(0, 0xe9, 2);
    assert!(!runtime.valid_morph_restore(original, 7, 13));
    assert_eq!(runtime.morph_selection_ids(7, 13), Some(ids(&[0], 2, 13)));
    f.write_u8(0, 0xe9, 1);
    f.write_u8(0, 0x68, 0);
    assert!(!runtime.valid_morph_restore(original, 7, 13));
    assert!(runtime.morph_selection_ids(7, 13).is_none());
    f.write_u8(0, 0x68, 7);
    f.write_u16(0, 0x8c, 37);
    assert!(!runtime.valid_morph_restore(original, 7, 13));
}
#[test]
fn larva_morph_capture_never_reinterprets_a_queued_movement_or_selection_record() {
    let morph = morph_packet(37);
    let prefix = [0x05, 0x05];
    let after = [prefix.as_slice(), morph.as_slice()].concat();
    assert_eq!(crate::event_capture::observe_append(&prefix, &after, 480),
        crate::event_capture::Observation::Command(morph.clone()));
    let mut two_records = after;
    two_records.extend(click_order(true));
    assert!(matches!(crate::event_capture::observe_append(&prefix, &two_records, 480),
        crate::event_capture::Observation::Clear(crate::event_capture::ClearReason::UnsupportedOrIncompleteRecord)));
    assert_eq!(crate::event_capture::observe_append(&prefix, &prefix, 480),
        crate::event_capture::Observation::NoObservedAppend);
}

#[test]
fn missing_command_metadata_preserves_previous_mobile_control_without_building_fallback() {
    let f = Fixture::new(15, 7, 37);
    let mut runtime = f.runtime();
    runtime.metadata = None;
    let mut state = f.state();
    assert!(queue_control(&click_order(false), &state.last_selection.clone(), &mut state, &runtime));
    assert_eq!(state.count, 15);
    assert_eq!(state.pending.len(), 2);
    assert_eq!(state.pending[0].ids.len(), 12);
    assert_eq!(state.pending[1].ids.len(), 2);
    assert_eq!(f.globals[9], 0);
}

fn capture_owned_callback(f: &Fixture, runtime: &Runtime, state: &State) -> Capture {
    let actual = selected_ids(runtime, state.owner, state.kind, state.shift).unwrap();
    assert_eq!(actual, state.last_selection);
    assert_eq!(f.globals[8], state.frame as usize);
    Capture { buffer: buffer_snapshot(runtime).unwrap(), ids: actual,
        context: runtime.context().unwrap(), session: session_identity(runtime).unwrap(),
        owner: state.owner, kind: state.kind, shift: state.shift, epoch: 7 }
}
fn append_owned_callback(f: &mut Fixture, command: &[u8]) {
    let start = f.globals[9];
    f.outgoing_buffer[start..start + command.len()].copy_from_slice(command);
    f.globals[9] += command.len();
    std::hint::black_box(&f.outgoing_buffer);
    std::hint::black_box(&f.globals);
}

#[test]
fn actual_morph_completion_accepts_original_egg_and_excludes_it_from_every_copy() {
    for command in [morph_packet(37), morph_packet(41), morph_packet(42)] {
        let mut f = Fixture::new(63, 7, 35);
        let mut runtime = f.runtime(); runtime.metadata = None;
        let mut state = building_state(&f, 35);
        let capture = capture_owned_callback(&f, &runtime, &state);
        let original = capture.ids.clone();
        append_owned_callback(&mut f, &command);
        f.write_u16(0, 0x8c, 36);
        f.write_u32(0, 0x140, 0);
        let after = f.outgoing_buffer.to_vec();
        let selection = *f.selection;
        complete_captured_control(&runtime, &mut state, capture, false, 7);
        assert!(state.active, "{}", state.message);
        assert_eq!(state.count, 63);
        assert_eq!(state.pending.len(), 6);
        let copied: Vec<_> = state.pending.iter().flat_map(|p| p.ids.iter().copied()).collect();
        assert_eq!(copied, ids(&(1..63).collect::<Vec<_>>(), 1, 13));
        assert_eq!(active_selection_ids(&runtime, &state), Some(original.clone()));
        for job in &state.pending {
            assert_eq!(job.restore, original);
            assert_eq!(job.command, command);
            assert!(job.ids.iter().all(|id| runtime.valid_id(*id, 7, 35, 13)));
            assert!(job.restore.iter().all(|id| runtime.valid_morph_restore(*id, 7, 13)));
            let plan = crate::batch::plan_one(&job.ids, &job.restore, &job.command, 480).unwrap();
            assert_eq!(capture_selection(&plan[2]), Some(original.clone()));
            assert_eq!(plan[1], command);
        }
        assert_eq!(*f.selection, selection);
        assert_eq!(f.outgoing_buffer.as_ref(), after.as_slice());
        assert_eq!(f.globals[9], command.len()); // only the fixture's original append
    }
}

#[test]
fn larva_capture_accepts_only_exact_original_selection_then_single_morph() {
    let mut f = Fixture::new(27, 7, 35);
    for (slot, index) in [0, 5, 10].into_iter().enumerate() { f.selection[slot] = f.pointer(index); }
    let runtime = f.runtime();
    let mut state = building_state(&f, 35); state.last_selection = ids(&[0, 5, 10], 1, 13);
    let capture = capture_owned_callback(&f, &runtime, &state);
    let mut output = selection_record(&capture.ids).unwrap(); output.extend(morph_packet(37));
    append_owned_callback(&mut f, &output);
    complete_captured_control(&runtime, &mut state, capture, false, 7);
    assert!(state.active, "{}", state.message);
    assert_eq!(state.pending.len(), 2);
    assert_eq!(state.pending.iter().flat_map(|p| p.ids.iter()).count(), 24);
    assert!(state.pending.iter().all(|p| p.command == morph_packet(37)));
    assert!(state.pending.iter().all(|p| !p.ids.iter().any(|id| state.last_selection.contains(id))));
}

#[test]
fn original_larva_group_keeps_its_order_after_first_member_becomes_egg() {
    let mut f = Fixture::new(27, 7, 35);
    f.selection[1] = f.pointer(5); f.selection[2] = f.pointer(10);
    let runtime = f.runtime();
    let mut state = building_state(&f, 35); state.last_selection = ids(&[0, 5, 10], 1, 13);
    let capture = capture_owned_callback(&f, &runtime, &state);
    append_owned_callback(&mut f, &morph_packet(37));
    f.write_u16(0, 0x8c, 36); f.write_u32(0, 0x140, 0);
    complete_captured_control(&runtime, &mut state, capture, false, 7);
    assert!(state.active, "{}", state.message);
    assert_eq!(state.count, 27);
    assert_eq!(state.pending.len(), 2);
    assert_eq!(active_selection_ids(&runtime, &state), Some(state.last_selection.clone()));
    for p in &state.pending {
        assert_eq!(p.restore, ids(&[0, 5, 10], 1, 13));
        assert!(![0, 5, 10].iter().any(|index| p.ids.contains(&ids(&[*index], 1, 13)[0])));
    }
}

#[test]
fn morph_transition_is_never_accepted_without_one_proven_supported_morph_append() {
    for command in [vec![], train_packet(37), click_order(false), vec![0x23, 37],
        [morph_packet(37), morph_packet(41)].concat(), selection_record(&ids(&[0], 1, 13)).unwrap()] {
        let mut f = Fixture::new(15, 7, 35); let runtime = f.runtime();
        let mut state = building_state(&f, 35);
        let capture = capture_owned_callback(&f, &runtime, &state);
        append_owned_callback(&mut f, &command);
        f.write_u16(0, 0x8c, 36); f.write_u32(0, 0x140, 0);
        complete_captured_control(&runtime, &mut state, capture, false, 7);
        assert!(!state.active, "command {:?}", command);
        assert!(state.pending.is_empty());
    }
}

#[test]
fn morph_capture_rejects_changed_owner_generation_selection_session_and_callback_guards() {
    for failure in 0..12 {
        let mut f = Fixture::new(15, 7, 35); let runtime = f.runtime();
        let mut state = building_state(&f, 35);
        let capture = capture_owned_callback(&f, &runtime, &state);
        append_owned_callback(&mut f, &morph_packet(37));
        f.write_u16(0, 0x8c, 36); f.write_u32(0, 0x140, 0);
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
            _ => (),
        }
        complete_captured_control(&runtime, &mut state, capture, failure == 10,
            if failure == 11 { 8 } else { 7 });
        assert!(!state.active, "failure {failure}");
        assert!(state.pending.is_empty());
    }
}

#[test]
fn larva_sync_selection_cannot_change_ids_order_generation_or_contain_extra_records() {
    let originals = ids(&[0, 1], 1, 13);
    let selection = selection_record(&originals).unwrap();
    let morph = morph_packet(37);
    let good = [selection.clone(), morph.clone()].concat();
    assert_eq!(crate::event_capture::observe_control_append(&[5], &[vec![5], good.clone()].concat(), 480,
        Some(&originals)), crate::event_capture::Observation::Command(morph.clone()));
    let mut swapped = selection.clone(); swapped[2..6].copy_from_slice(&originals[1].to_le_bytes());
    swapped[6..10].copy_from_slice(&originals[0].to_le_bytes());
    let mut generation = selection.clone(); generation[2..6].copy_from_slice(&(originals[0] + (1 << 13)).to_le_bytes());
    for bad in [ [swapped, morph.clone()].concat(), [generation, morph.clone()].concat(),
        [selection.clone(), morph.clone(), morph.clone()].concat(),
        [selection.clone(), click_order(false)].concat(),
        [selection.clone(), vec![0x23, 37]].concat(),
        [selection.clone(), morph.clone(), vec![0]].concat() ] {
        assert!(!matches!(crate::event_capture::observe_control_append(&[], &bad, 480, Some(&originals)),
            crate::event_capture::Observation::Command(_)));
    }
    assert_eq!(crate::event_capture::observe_control_append(&[], &good, 480, None),
        crate::event_capture::Observation::Clear(crate::event_capture::ClearReason::ManualSelection));
    assert!(matches!(crate::event_capture::observe_control_append(&[1], &good, 480, Some(&originals)),
        crate::event_capture::Observation::Ambiguous(_)));
}

#[test]
fn capture_to_queue_rally_handles_every_stock_producer_and_never_dispatches_to_reference_twice() {
    for kind in [106, 111, 113, 114, 130, 131, 132, 133, 154, 155, 160, 167] {
        for command in [click_order(false), rally_packet(39, false), rally_packet(40, true)] {
            let mut f = Fixture::new(7, 7, kind); let mut runtime = f.runtime(); runtime.metadata = None;
            let mut state = building_state(&f, kind);
            let selection = *f.selection;
            let capture = capture_owned_callback(&f, &runtime, &state);
            append_owned_callback(&mut f, &command);
            complete_captured_control(&runtime, &mut state, capture, false, 7);
            assert!(state.active, "kind {kind}: {}", state.message);
            assert_eq!(state.count, 7); assert_eq!(state.pending.len(), 6);
            for (index, job) in state.pending.iter().enumerate() {
                assert_eq!(job.ids, ids(&[index as u32 + 1], 1, 13));
                assert_eq!(job.restore, ids(&[0], 1, 13));
                assert_eq!(job.command, command);
            }
            assert_eq!(*f.selection, selection);
        }
    }
}

#[test]
fn larva_morph_hud_falls_back_to_actual_egg_then_clears_stale_virtual_count() {
    let mut f = Fixture::new(15, 7, 35); let runtime = f.runtime();
    let mut state = building_state(&f, 35);
    let capture = capture_owned_callback(&f, &runtime, &state);
    append_owned_callback(&mut f, &morph_packet(37));
    f.write_u16(0, 0x8c, 36); f.write_u32(0, 0x140, 0);
    complete_captured_control(&runtime, &mut state, capture, false, 7);
    let display = hud_selection(Some(&runtime), &state, true);
    assert_eq!(display, DisplaySelection { control_active: false, in_game: true, count: 1, kind: 36 });
    let wire = encode_status(99, &state, "READY", None, display);
    let fields: Vec<_> = wire.trim().split('\t').collect();
    assert_eq!(&fields[2..4], &["0", "0"]);
    assert_eq!(&fields[8..11], &["1", "1", "36"]);
    state.stop("test finished");
    f.selection.fill(0);
    let display = hud_selection(Some(&runtime), &state, true);
    assert_eq!(display, DisplaySelection::empty(true));
    assert!(state.pending.is_empty()); assert!(state.visual_frame.is_none());
}

#[test]
fn numeric_control_trace_is_bounded_and_does_not_log_packet_or_memory_content() {
    let f = Fixture::new(15, 7, 35); let runtime = f.runtime();
    let mut state = building_state(&f, 35);
    for _ in 0..100 { trace_control(&runtime, &mut state, "fixture", Some(3), Some(0x23)); }
    assert_eq!(state.control_traces, 24);
    assert_eq!(state.events.len(), 24);
    for row in &state.events {
        assert!(row.contains("native_count=1; native_type=35"));
        assert!(row.contains("delta_len=3; opcode=35"));
        assert!(!row.contains("0x")); assert!(!row.contains("pointer"));
    }
}
#[test]
fn single_original_larva_morph_with_no_remaining_larvae_clears_virtual_mode_normally() {
    let mut f = Fixture::new(1, 7, 35); let runtime = f.runtime();
    let mut state = building_state(&f, 35);
    let capture = capture_owned_callback(&f, &runtime, &state);
    append_owned_callback(&mut f, &morph_packet(37));
    f.write_u16(0, 0x8c, 36); f.write_u32(0, 0x140, 0);
    complete_captured_control(&runtime, &mut state, capture, false, 7);
    assert!(!state.active); assert!(state.pending.is_empty());
    assert_eq!(state.count, 0); assert_eq!(state.sent, 0);
    assert_eq!(state.message, "Original larvae already received production; group cleared");
    let display = hud_selection(Some(&runtime), &state, true);
    assert_eq!(display.kind, 36); assert_eq!(display.count, 1); assert!(!display.control_active);
}

#[test]
fn actual_dispatch_reference_guard_accepts_one_egg_but_rejects_manual_selection_before_send() {
    let mut f = Fixture::new(15, 7, 35); let runtime = f.runtime();
    let mut state = building_state(&f, 35);
    let capture = capture_owned_callback(&f, &runtime, &state);
    append_owned_callback(&mut f, &morph_packet(37));
    f.write_u16(0, 0x8c, 36); f.write_u32(0, 0x140, 0);
    complete_captured_control(&runtime, &mut state, capture, false, 7);
    let job = state.pending.front().unwrap();
    assert!(validate_pending_reference(&runtime, &state, job).is_ok());
    f.selection[0] = f.pointer(1);
    assert!(validate_pending_reference(&runtime, &state, job).is_err());
    assert!(active_selection_ids(&runtime, &state).as_deref() != Some(state.last_selection.as_slice()));
    assert_eq!(f.globals[9], 3); assert_eq!(state.sent, 0);
}