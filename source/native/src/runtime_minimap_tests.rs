// Minimap regression fixtures use only owned bytes. No callback installation,
// game discovery, process access, input delivery or native sender runs here.

fn minimap_attack_packet(queued: bool) -> Vec<u8> {
    // Engine-resolved world coordinates and unit/type target are opaque here.
    // The observer must preserve them rather than substitute screen coordinates.
    vec![0x61, 0x34, 0x12, 0x78, 0x16, 0x09, 0x20, 0, 0, 228, 0, 14,
        u8::from(queued)]
}

fn run_owned_minimap_event(
    fixture: &std::cell::RefCell<Fixture>, runtime: &Runtime,
    state: &std::cell::RefCell<State>, kind: Option<usize>, outer: bool,
    output: &[u8], order: &std::cell::RefCell<Vec<&'static str>>,
) -> u32 {
    observe_minimap_call(kind, outer,
        || {
            order.borrow_mut().push("begin");
            Some(capture_owned_callback(&fixture.borrow(), runtime, &state.borrow()))
        },
        || {
            order.borrow_mut().push("original");
            append_owned_callback(&mut fixture.borrow_mut(), output);
            0x2468_1357
        },
        |capture| {
            order.borrow_mut().push(if capture.is_some() { "finish" } else { "finish-none" });
            if let Some(capture) = capture {
                complete_captured_control(runtime, &mut state.borrow_mut(), capture, false, 7);
            }
        })
}

#[test]
fn minimap_attack_preserves_native_world_target_and_excludes_all_originals_in_every_owner_slot() {
    use std::cell::RefCell;
    for owner in 0..8 {
        for queued in [false, true] {
            let mut f = Fixture::new(32, owner, 37);
            for (slot, index) in [0, 5, 10].into_iter().enumerate() {
                f.selection[slot] = f.pointer(index);
            }
            // Exactly 27 live same-kind owned units, plus five rejected candidates.
            f.write_u8(27, 0x68, (owner + 1) % 8);
            f.write_u16(28, 0x8c, 38);
            f.write_u32(29, 0x10, 0);
            f.write_u32(30, 0x140, 1 | 0x40);
            f.write_u32(31, 0x140, 0);
            let runtime = f.runtime();
            let selection = *f.selection; let units = f.units.to_vec();
            let mut initial = f.state(); initial.last_selection = ids(&[0, 5, 10], 1, 13);
            let state = RefCell::new(initial); let fixture = RefCell::new(f);
            let order = RefCell::new(Vec::new());
            let command = minimap_attack_packet(queued);
            assert_eq!(run_owned_minimap_event(&fixture, &runtime, &state, Some(5), true,
                &command, &order), 0x2468_1357);
            assert_eq!(order.into_inner(), ["begin", "original", "finish"]);
            let state = state.borrow();
            assert!(state.active, "owner={owner}, queued={queued}: {}", state.message);
            assert_eq!(state.count, 27); assert_eq!(state.pending.len(), 2); assert_eq!(state.sent, 0);
            let expected: Vec<_> = (0..27).filter(|index| ![0, 5, 10].contains(index)).collect();
            let copied: Vec<_> = state.pending.iter().flat_map(|job| job.ids.iter().copied()).collect();
            assert_eq!(copied, ids(&expected, 1, 13));
            for job in &state.pending {
                assert_eq!(job.restore, ids(&[0, 5, 10], 1, 13));
                assert_eq!(job.command, command);
                assert_eq!(job.ids.len(), 12);
                assert!(job.ids.iter().all(|id| runtime.valid_id(*id, owner, 37, 13)));
                let wire = crate::batch::plan_one(&job.ids, &job.restore, &job.command, 480).unwrap();
                assert_eq!(wire[1], command);
                assert_eq!(capture_selection(&wire[0]), Some(job.ids.clone()));
                assert_eq!(capture_selection(&wire[2]), Some(job.restore.clone()));
            }
            let fixture = fixture.borrow();
            assert_eq!(*fixture.selection, selection); assert_eq!(fixture.units.as_ref(), units.as_slice());
            assert_eq!(fixture.globals[9], command.len());
            assert_eq!(&fixture.outgoing_buffer[..command.len()], command);
        }
    }
}

#[test]
fn minimap_original_receives_same_owned_control_event_once_and_returns_its_result() {
    use std::cell::{Cell, RefCell};
    let f = Fixture::new(4, 3, 37); let runtime = f.runtime();
    let state = RefCell::new(f.state()); let fixture = RefCell::new(f);
    let control = Box::new([0u8; 0x78]); let event = Box::new([0u8; 0x20]);
    let arguments = (control.as_ptr() as usize, event.as_ptr() as usize);
    let observed_arguments = Cell::new(None); let original_calls = Cell::new(0);
    let command = minimap_attack_packet(false);
    let result = observe_minimap_call(Some(5), true,
        || Some(capture_owned_callback(&fixture.borrow(), &runtime, &state.borrow())),
        || {
            original_calls.set(original_calls.get() + 1);
            observed_arguments.set(Some(arguments));
            append_owned_callback(&mut fixture.borrow_mut(), &command);
            0xffff_0135
        },
        |capture| complete_captured_control(&runtime, &mut state.borrow_mut(), capture.unwrap(), false, 7));
    assert_eq!(original_calls.get(), 1); assert_eq!(observed_arguments.get(), Some(arguments));
    assert_eq!(result, 0xffff_0135); assert_eq!(state.borrow().pending.len(), 1);
    assert!(!crate::control_capture::active());
}

#[test]
fn minimap_camera_click_with_no_native_command_keeps_group_without_fabricating_attack() {
    use std::cell::RefCell;
    let f = Fixture::new(27, 6, 37); let runtime = f.runtime(); let selection = *f.selection;
    let state = RefCell::new(f.state()); let fixture = RefCell::new(f);
    let order = RefCell::new(Vec::new());
    assert_eq!(run_owned_minimap_event(&fixture, &runtime, &state, Some(5), true, &[], &order),
        0x2468_1357);
    assert_eq!(order.into_inner(), ["begin", "original", "finish"]);
    assert!(state.borrow().active); assert!(state.borrow().pending.is_empty());
    assert_eq!(state.borrow().last_selection, ids(&[0], 1, 13));
    assert_eq!(fixture.borrow().globals[9], 0); assert_eq!(*fixture.borrow().selection, selection);
}

#[test]
fn minimap_right_click_and_rally_keep_native_payload_and_existing_dispatch_policy() {
    use std::cell::RefCell;
    for (kind, command, copies) in [(37, click_order(false), 3), (37, click_order(true), 3),
        (111, rally_packet(39, false), 26), (111, rally_packet(40, true), 26),
        (131, rally_packet(40, false), 26)] {
        let mut f = Fixture::new(27, 4, kind);
        if kind == 111 { f.metadata(kind, 1, 0x22); }
        if kind == 131 { f.metadata(kind, 1, 0x21); }
        let runtime = f.runtime(); let selection = *f.selection;
        let state = RefCell::new(building_state(&f, kind)); let fixture = RefCell::new(f);
        let order = RefCell::new(Vec::new());
        assert_eq!(run_owned_minimap_event(&fixture, &runtime, &state, Some(7), true,
            &command, &order), 0x2468_1357);
        assert_eq!(order.into_inner(), ["begin", "original", "finish"]);
        let state = state.borrow();
        assert!(state.active, "kind={kind}: {}", state.message);
        assert_eq!(state.count, 27); assert_eq!(state.pending.len(), copies);
        if kind == 37 {
            assert_eq!(state.pending.iter().map(|job| job.ids.len()).collect::<Vec<_>>(), [12, 12, 2]);
        }
        for job in &state.pending {
            assert_eq!(job.command, command); assert_eq!(job.restore, ids(&[0], 1, 13));
            assert!(!job.ids.contains(&ids(&[0], 1, 13)[0]));
            if kind != 37 { assert_eq!(job.ids.len(), 1); }
        }
        assert_eq!(fixture.borrow().globals[9], command.len());
        assert_eq!(*fixture.borrow().selection, selection);
    }
}

#[test]
fn minimap_manual_selection_malformed_and_multiple_native_records_clear_without_copies() {
    use std::cell::RefCell;
    let attack = minimap_attack_packet(false);
    let mut bad_queue = attack.clone(); bad_queue[12] = 2;
    let mut unsupported = attack.clone(); unsupported[11] = 33;
    let selected = selection_record(&ids(&[0], 1, 13)).unwrap();
    for output in [selected.clone(), [selected, attack.clone()].concat(), attack[..12].to_vec(),
        [attack.clone(), attack.clone()].concat(), bad_queue, unsupported] {
        let f = Fixture::new(27, 5, 37); let runtime = f.runtime(); let selection = *f.selection;
        let state = RefCell::new(f.state()); let fixture = RefCell::new(f);
        let order = RefCell::new(Vec::new());
        assert_eq!(run_owned_minimap_event(&fixture, &runtime, &state, Some(5), true,
            &output, &order), 0x2468_1357);
        assert_eq!(order.into_inner(), ["begin", "original", "finish"]);
        assert!(!state.borrow().active); assert!(state.borrow().pending.is_empty());
        assert_eq!(state.borrow().sent, 0); assert_eq!(fixture.borrow().globals[9], output.len());
        assert_eq!(*fixture.borrow().selection, selection);
    }
}

#[test]
fn minimap_capture_keeps_context_owner_pause_epoch_and_selection_uid_guards() {
    use std::cell::RefCell;
    for change in 0..12 {
        let f = Fixture::new(27, 5, 37); let runtime = f.runtime();
        let state = RefCell::new(f.state()); let fixture = RefCell::new(f);
        let command = minimap_attack_packet(false);
        let result = observe_minimap_call(Some(5), true,
            || Some(capture_owned_callback(&fixture.borrow(), &runtime, &state.borrow())),
            || {
                let mut f = fixture.borrow_mut();
                append_owned_callback(&mut f, &command);
                match change {
                    0 => f.globals[8] += 1,
                    1 => f.globals[3] = 6,
                    2 => f.globals[7] = 1,
                    3 => f.globals[5] = 4,
                    4 => f.globals[6] = 0,
                    5 => f.globals[4] = 1,
                    6 => f.selection[0] = f.pointer(1),
                    7 => f.write_u8(0, 0xe9, 2),
                    8 => f.write_u8(0, 0x68, 6),
                    9 => f.write_u16(0, 0x8c, 38),
                    10 => f.globals[10] = 480,
                    11 => (), // Only the binding epoch changes at completion.
                    _ => unreachable!(),
                }
                0x3141
            },
            |capture| complete_captured_control(&runtime, &mut state.borrow_mut(), capture.unwrap(),
                false, if change == 11 { 8 } else { 7 }));
        assert_eq!(result, 0x3141); assert!(!state.borrow().active, "change={change}");
        assert!(state.borrow().pending.is_empty(), "change={change}");
        assert_eq!(state.borrow().sent, 0); assert_eq!(fixture.borrow().globals[9], command.len());
        assert!(!crate::control_capture::active());
    }
}

#[test]
fn minimap_uid_and_owner_changes_of_other_candidates_never_dispatch_to_stale_targets() {
    use std::cell::RefCell;
    let f = Fixture::new(27, 5, 37); let runtime = f.runtime();
    let state = RefCell::new(f.state()); let fixture = RefCell::new(f);
    let command = minimap_attack_packet(false);
    observe_minimap_call(Some(5), true,
        || Some(capture_owned_callback(&fixture.borrow(), &runtime, &state.borrow())),
        || {
            let mut f = fixture.borrow_mut();
            append_owned_callback(&mut f, &command);
            f.write_u8(1, 0xe9, 2); // Slot reused while original handler ran.
            f.write_u8(2, 0x68, 6); // No longer owned by the local player.
            17
        },
        |capture| complete_captured_control(&runtime, &mut state.borrow_mut(), capture.unwrap(), false, 7));
    let state = state.borrow();
    assert!(state.active, "{}", state.message);
    let copied: Vec<_> = state.pending.iter().flat_map(|job| job.ids.iter().copied()).collect();
    assert!(!copied.contains(&ids(&[1], 1, 13)[0]));
    assert!(copied.contains(&ids(&[1], 2, 13)[0])); // Fresh matching snapshot uses current generation.
    assert!(!copied.contains(&ids(&[2], 1, 13)[0]));
    assert!(copied.iter().all(|id| runtime.valid_id(*id, 5, 37, 13)));
}

#[test]
fn outer_capture_and_minimap_delegation_observe_attack_once_but_distinct_input_revokes_it() {
    use std::cell::{Cell, RefCell};
    for same_event in [true, false] {
        let f = Fixture::new(27, 5, 37); let runtime = f.runtime(); let selection = *f.selection;
        let state = RefCell::new(f.state()); let fixture = RefCell::new(f);
        let event = Box::new([0u8; 0x20]); let other = Box::new([1u8; 0x20]);
        let event_pointer = event.as_ptr() as usize;
        let nested_pointer = if same_event { event_pointer } else { other.as_ptr() as usize };
        let outer_calls = Cell::new(0); let minimap_calls = Cell::new(0);
        let nested_changed = Cell::new(false); let command = minimap_attack_packet(false);
        let _event = crate::control_capture::EventScope::enter(event_pointer);
        let result = observe_panel_call(Some(5), None, true,
            || Some(capture_owned_callback(&fixture.borrow(), &runtime, &state.borrow())),
            || {
                outer_calls.set(outer_calls.get() + 1);
                assert!(crate::control_capture::active());
                nested_changed.set(panel_nested_input_changed(Some(5), nested_pointer));
                let nested = observe_minimap_call::<Capture>(Some(5), true,
                    || panic!("A delegated minimap event must not own a second snapshot"),
                    || {
                        minimap_calls.set(minimap_calls.get() + 1);
                        append_owned_callback(&mut fixture.borrow_mut(), &command);
                        0x1234
                    },
                    |capture| assert!(capture.is_none()));
                assert_eq!(nested, 0x1234);
                0x5678
            },
            |capture| complete_captured_control(&runtime, &mut state.borrow_mut(), capture.unwrap(),
                nested_changed.get(), 7));
        assert_eq!(result, 0x5678); assert_eq!(outer_calls.get(), 1); assert_eq!(minimap_calls.get(), 1);
        assert_eq!(state.borrow().active, same_event);
        assert_eq!(state.borrow().pending.len(), if same_event { 3 } else { 0 });
        assert_eq!(fixture.borrow().globals[9], command.len());
        assert_eq!(*fixture.borrow().selection, selection); assert!(!crate::control_capture::active());
    }
}

#[test]
fn noncapturing_parent_allows_minimap_target_callback_to_own_one_native_append() {
    use std::cell::RefCell;
    let f = Fixture::new(27, 1, 37); let runtime = f.runtime();
    let state = RefCell::new(f.state()); let fixture = RefCell::new(f);
    let order = RefCell::new(Vec::new()); let command = minimap_attack_packet(false);
    let result = observe_panel_call::<Capture>(Some(0xe), Some(2), true,
        || None,
        || {
            assert!(!crate::control_capture::active());
            run_owned_minimap_event(&fixture, &runtime, &state, Some(5), true, &command, &order)
        },
        |capture| assert!(capture.is_none()));
    assert_eq!(result, 0x2468_1357);
    assert_eq!(order.into_inner(), ["begin", "original", "finish"]);
    assert!(state.borrow().active); assert_eq!(state.borrow().pending.len(), 3);
    assert_eq!(fixture.borrow().globals[9], command.len());
}

#[test]
fn minimap_owned_wrapper_requires_exact_identity_and_verified_saved_native_provider() {
    let wrapper = 0x90000;
    let is_code = |pointer: usize| (0x40000..0x50000).contains(&pointer);
    assert!(owned_minimap_provider(wrapper, 0x45000, wrapper, is_code));
    for (callback, original) in [(wrapper, 0), (wrapper, wrapper), (wrapper, 0x50000),
        (wrapper, 0x3ffff), (0, 0x45000), (0x91000, 0x45000), (0x45000, 0x45000)] {
        assert!(!owned_minimap_provider(callback, original, wrapper, is_code),
            "callback={callback:#x}, original={original:#x}");
    }
}

#[test]
fn minimap_owned_snapshot_survives_identical_global_delegation_but_not_new_input() {
    use std::cell::{Cell, RefCell};
    for same_event in [true, false] {
        let f = Fixture::new(27, 5, 37); let runtime = f.runtime();
        let state = RefCell::new(f.state()); let fixture = RefCell::new(f);
        let event = Box::new([0u8; 0x20]); let other = Box::new([1u8; 0x20]);
        let pointer = event.as_ptr() as usize;
        let delegated = if same_event { pointer } else { other.as_ptr() as usize };
        let changed = Cell::new(false); let native_calls = Cell::new(0);
        let command = minimap_attack_packet(false);
        let _scope = crate::control_capture::EventScope::enter(pointer);
        let result = observe_minimap_call(Some(5), true,
            || Some(capture_owned_callback(&fixture.borrow(), &runtime, &state.borrow())),
            || {
                // A global callback reached by the minimap's native handler
                // delegates the same event without owning a second snapshot.
                changed.set(global_nested_input_changed(delegated));
                native_calls.set(native_calls.get() + 1);
                append_owned_callback(&mut fixture.borrow_mut(), &command);
                0x5678
            },
            |capture| complete_captured_control(&runtime, &mut state.borrow_mut(), capture.unwrap(),
                changed.get(), 7));
        assert_eq!(result, 0x5678); assert_eq!(native_calls.get(), 1);
        assert_eq!(state.borrow().active, same_event);
        assert_eq!(state.borrow().pending.len(), if same_event { 3 } else { 0 });
        assert_eq!(fixture.borrow().globals[9], command.len());
        assert!(!crate::control_capture::active());
    }
}

#[test]
fn global_delegation_without_capture_lease_or_during_sender_never_accepts_minimap_snapshot() {
    struct RestoreOriginal(bool);
    impl Drop for RestoreOriginal {
        fn drop(&mut self) { IN_ORIGINAL.with(|flag| flag.set(self.0)); }
    }
    for sending in [false, true] {
        let mut f = Fixture::new(27, 5, 37); let runtime = f.runtime(); let mut state = f.state();
        let capture = capture_owned_callback(&f, &runtime, &state);
        let event = Box::new([0u8; 0x20]); let pointer = event.as_ptr() as usize;
        let _scope = crate::control_capture::EventScope::enter(pointer);
        let _lease = sending.then(crate::control_capture::Lease::enter);
        let previous = IN_ORIGINAL.with(|flag| flag.replace(sending));
        let _restore = RestoreOriginal(previous);
        append_owned_callback(&mut f, &minimap_attack_packet(false));
        complete_captured_control(&runtime, &mut state, capture, global_nested_input_changed(pointer), 7);
        assert!(!state.active); assert!(state.pending.is_empty()); assert_eq!(state.sent, 0);
        assert_eq!(f.globals[9], 13);
    }
    assert!(!crate::control_capture::active()); assert!(!IN_ORIGINAL.with(Cell::get));
}
