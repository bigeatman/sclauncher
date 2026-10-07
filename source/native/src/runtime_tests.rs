//! Tests operate only on owned byte arrays in this test process. No game lookup,
//! hooks, input events, outgoing sender, or network calls are used.
use super::*;

#[test]
fn status_publication_replaces_delete_shared_reader_and_keeps_old_file_on_denial() {
    use std::io::Read;
    use std::os::windows::fs::OpenOptionsExt;
    use winapi::um::winnt::{FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE};
    let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let folder = std::env::temp_dir().join(format!("sc-owned-publication-{}-{unique}", std::process::id()));
    fs::create_dir(&folder).unwrap();
    let file = folder.join("status.tsv");
    atomic_write_text(&file, "old complete status\n").unwrap();
    let mut held = fs::OpenOptions::new().read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).open(&file).unwrap();
    atomic_write_text(&file, "new complete status\n").unwrap();
    let mut prior = String::new(); held.read_to_string(&mut prior).unwrap();
    assert_eq!(prior, "old complete status\n");
    assert_eq!(fs::read_to_string(&file).unwrap(), "new complete status\n");
    drop(held);
    let restrictive = fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ).open(&file).unwrap();
    assert!(atomic_write_text(&file, "must never become partial\n").is_err());
    assert_eq!(fs::read_to_string(&file).unwrap(), "new complete status\n");
    drop(restrictive);
    atomic_write_text(&file, "recovered complete status\n").unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "recovered complete status\n");
    fs::remove_dir_all(&folder).unwrap();
}

#[repr(C)]
#[derive(Clone, Copy)]
struct DatHeader { data: usize, width: u32, entries: u32 }

struct Fixture {
    units: Box<[u8]>,
    selection: Box<[usize; 12]>,
    globals: Box<[usize; 12]>,
    outgoing_buffer: Box<[u8; 512]>,
    dat: Box<[DatHeader; 45]>,
    dat_flags: Box<[u32; 228]>,
    dat_groups: Box<[u8; 228]>,
}

impl Fixture {
    fn new(count: usize, owner: u8, kind: u16) -> Self {
        assert!(count > 0 && count <= MAX_UNITS);
        let mut this = Self {
            units: vec![0; count * UNIT_SIZE].into_boxed_slice(),
            selection: Box::new([0; 12]),
            globals: Box::new([0; 12]),
            outgoing_buffer: Box::new([0; 512]),
            dat: Box::new([DatHeader {data:0,width:0,entries:0};45]),
            dat_flags: Box::new([0;228]),
            dat_groups: Box::new([0;228]),
        };
        for index in 0..count {
            this.write_u32(index, 0x10, 25600); // positive fixed-point hitpoints
            this.write_u64(index, 0x18, 0x10000); // non-null sprite, never dereferenced
            this.write_u8(index, 0x68, owner);
            this.write_u8(index, 0x69, 3); // live order
            this.write_u16(index, 0x8c, kind);
            this.write_u8(index, 0xe9, 1);
            this.write_u32(index, 0x140, 1); // completed, not in transport
            if index + 1 < count {
                this.write_u64(index, 8, this.pointer(index + 1) as u64);
            }
        }
        this.dat[22]=DatHeader{data:this.dat_flags.as_ptr() as usize,width:4,entries:228};
        this.dat[44]=DatHeader{data:this.dat_groups.as_ptr() as usize,width:1,entries:228};
        this.selection[0] = this.pointer(0);
        this.globals[0] = this.units.as_ptr() as usize;
        this.globals[1] = this.pointer(0);
        this.globals[2] = this.selection.as_ptr() as usize;
        this.globals[3] = owner as usize;
        this.globals[4] = 0; // replay
        this.globals[5] = 1; // live
        this.globals[6] = 1; // continuing
        this.globals[7] = 0; // paused
        this.globals[8] = 100; // frame
        this.globals[9] = 0; // outgoing bytes
        this.globals[10] = 496; // dynamic outgoing capacity
        this.globals[11] = this.outgoing_buffer.as_ptr() as usize;
        this
    }
    fn metadata(&mut self,kind:u16,flags:u32,groups:u8) {
        self.dat_flags[kind as usize]=flags;self.dat_groups[kind as usize]=groups;
        std::hint::black_box(&self.dat_flags);std::hint::black_box(&self.dat_groups);
    }
    fn pointer(&self, index: usize) -> usize {
        assert!(index * UNIT_SIZE < self.units.len());
        self.units.as_ptr() as usize + index * UNIT_SIZE
    }
    fn write(&mut self, index: usize, offset: usize, bytes: &[u8]) {
        let begin = index * UNIT_SIZE + offset;
        self.units[begin..begin + bytes.len()].copy_from_slice(bytes);
    }
    fn write_u8(&mut self, index: usize, offset: usize, value: u8) {
        self.write(index, offset, &[value]);
    }
    fn write_u16(&mut self, index: usize, offset: usize, value: u16) {
        self.write(index, offset, &value.to_le_bytes());
    }
    fn write_u32(&mut self, index: usize, offset: usize, value: u32) {
        self.write(index, offset, &value.to_le_bytes());
    }
    fn write_u64(&mut self, index: usize, offset: usize, value: u64) {
        self.write(index, offset, &value.to_le_bytes());
    }
    fn operand(&self, index: usize) -> Value {
        Value::Memory(
            Box::new(Value::Constant(self.globals.as_ptr() as usize)),
            index * 8,
            8,
        )
    }
    fn runtime(&self) -> Runtime {
        let ctx = OperandContext::new();
        // Reproduce samase_scarf GameScreenRClickAnalyzer: derive the array
        // address from slot 11's memory expression with Scarf's actual helper.
        let last_slot = ctx.mem64(ctx.constant(self.selection.as_ptr() as u64), 11 * 8);
        let selection_address = ctx.mem_sub_const_op(last_slot.if_memory().unwrap(), 11 * 8);
        Runtime {
            alliance: None,
            visual: None,
            gui: None,
            metadata: Some(UnitMetadataSource{table:Value::Constant(self.dat.as_ptr() as usize),stride:16}),
            unit_vectors: Vec::new(),
            units: self.operand(0),
            first: self.operand(1),
            selection: Value::from_operand(selection_address, 0, 0, 0, 0).unwrap(),
            local: self.operand(3),
            replay: self.operand(4),
            live: self.operand(5),
            continuing: self.operand(6),
            paused: self.operand(7),
            frame: self.operand(8),
            outgoing: self.operand(9),
            outgoing_capacity: self.operand(10),
            outgoing_buffer: self.operand(11),
        }
    }
    fn state(&self) -> State {
        State {
            active: true,
            count: 1,
            kind: 37,
            owner: self.globals[3] as u8,
            shift: 13,
            frame: self.globals[8] as u32,
            sent: 0,
            last_key: false,
            last_selection: vec![1 | (1 << 13)],
            pending: VecDeque::new(),
            message: "test",
            events: VecDeque::new(),
            last_scan: self.globals[8] as u32,
            visual_frame: None,
            visual_tick: None,
            panel_diagnostic: None,
            control_traces: 0,
            session_boundary: 0,
        }
    }
}

fn click_order(queued: bool) -> Vec<u8> {
    let mut out = vec![0u8; 12];
    out[0] = 0x60;
    out[11] = u8::from(queued);
    out
}
#[test]
fn game_boundary_revokes_old_group_pending_commands_and_held_backtick() {
    let f=Fixture::new(3,7,37);let mut state=f.state();
    state.pending.push_back(Pending{ids:vec![1],restore:vec![1],command:vec![0x1a,0],created:Instant::now()});
    state.frame=900;state.last_scan=900;state.panel_diagnostic=Some("old panel".into());state.control_traces=7;
    reset_state_for_session(&mut state,1,true);
    assert!(!state.active);assert_eq!(state.count,0);assert!(state.pending.is_empty()&&state.last_selection.is_empty());
    assert!(state.last_key);assert_eq!(state.frame,0);assert_eq!(state.last_scan,0);
    assert!(state.panel_diagnostic.is_none());assert_eq!(state.control_traces,0);assert_eq!(state.session_boundary,1);
}
#[test]
fn second_game_rebuilds_group_from_new_unit_storage_instead_of_reusing_same_ids() {
    let first=Fixture::new(3,7,37);let mut r1=first.runtime();let _v1=attach_vector(&mut r1,first.pointer(0),3,3);
    let mut state=first.state();state.active=false;
    activate_same_type(&r1,&mut state,7,100).unwrap();assert_eq!(state.count,3);
    let before1=*first.selection;let old_ids=state.last_selection.clone();
    reset_state_for_session(&mut state,1,false);assert!(!state.active);
    let mut second=Fixture::new(15,7,37);second.globals[8]=1;
    let mut r2=second.runtime();let _v2=attach_vector(&mut r2,second.pointer(0),15,15);
    let before2=*second.selection;
    assert_ne!(r1.units.read(),r2.units.read());
    activate_same_type(&r2,&mut state,7,1).unwrap();
    assert!(state.active);assert_eq!(state.count,15);assert_eq!(state.last_selection,old_ids);
    assert_eq!(*first.selection,before1);assert_eq!(*second.selection,before2);assert!(state.pending.is_empty());
}
#[test]
fn same_game_observation_does_not_repeatedly_clear_newly_activated_group() {
    let f=Fixture::new(3,7,37);let mut state=f.state();reset_state_for_session(&mut state,1,false);
    state.active=true;state.count=3;state.last_selection=vec![1];state.last_key=false;
    reset_state_for_session(&mut state,1,true);
    assert!(state.active);assert_eq!(state.count,3);assert_eq!(state.last_selection,vec![1]);assert!(!state.last_key);
}
#[test]
fn coherent_session_reads_owned_menu_and_next_match_flags_each_time() {
    let mut f=Fixture::new(3,7,37);let r=f.runtime();
    assert_eq!(coherent_session_context(&r),Some(crate::game_session::Context{active:true,frame:100,owner:7,identity:[0,1,1]}));
    f.globals[5]=0;f.globals[6]=0;f.globals[8]=0;f.globals[3]=8;
    assert_eq!(coherent_session_context(&r),Some(crate::game_session::Context{active:false,frame:0,owner:8,identity:[0,0,0]}));
    f.globals[5]=1;f.globals[6]=1;f.globals[8]=1;f.globals[3]=7;
    assert_eq!(coherent_session_context(&r),Some(crate::game_session::Context{active:true,frame:1,owner:7,identity:[0,1,1]}));
}
#[test]
fn one_owned_process_survives_game_menu_long_teardown_then_another_game() {
    use crate::callback_startup::{ObservedSlot,Attempt,WaitPolicy};
    use crate::game_session::{Tracker,Decision};
    let table=Box::new(std::array::from_fn::<_,4,_>(|i|AtomicUsize::new(0x20000+i*8)));
    let slots=std::array::from_fn::<_,4,_>(|i|crate::callback_binding::Slot{
        address:&table[i] as *const AtomicUsize as usize,original:0x20000+i*8,replacement:0x30000+i*8,
    });
    let observe=||std::array::from_fn::<_,4,_>(|i|ObservedSlot{slot:slots[i],actual:table[i].load(Ordering::Acquire)});
    assert!(matches!(crate::callback_startup::attempt(&observe(),||unsafe{crate::callback_binding::maintain_report(&slots)}).unwrap(),Attempt::Ready{..}));
    let first=Fixture::new(3,7,37);let mut r1=first.runtime();let _v1=attach_vector(&mut r1,first.pointer(0),3,3);
    let mut tracker=Tracker::new();let mut state=first.state();
    assert_eq!(tracker.enter(coherent_session_context(&r1),11).decision,Decision::Active);
    assert!(tracker.control_allowed(11));reset_state_for_session(&mut state,1,false);
    activate_same_type(&r1,&mut state,7,100).unwrap();assert_eq!(state.count,3);tracker.leave();
    for entry in table.iter(){entry.store(0,Ordering::Release);}
    tracker.observe(Some(crate::game_session::Context{active:false,frame:0,owner:8,identity:[0,0,0]}));
    reset_state_for_session(&mut state,2,true);assert!(!state.active&&state.pending.is_empty());
    let mut wait=WaitPolicy::default();
    for now in [0,15_000,120_000] {
        assert!(matches!(crate::callback_startup::attempt(&observe(),||panic!("do not install into null slots")).unwrap(),Attempt::Waiting{..}));
        assert!(!wait.expired_after_install(now,true,true,900,7,true));
        assert!(!tracker.control_allowed(11));assert_eq!(tracker.generation(),1);
    }
    let mut second=Fixture::new(15,7,37);second.globals[8]=1;
    let mut r2=second.runtime();let _v2=attach_vector(&mut r2,second.pointer(0),15,15);
    for (i,entry) in table.iter().enumerate(){entry.store(slots[i].original,Ordering::Release);}
    assert!(matches!(crate::callback_startup::attempt(&observe(),||unsafe{crate::callback_binding::maintain_report(&slots)}).unwrap(),Attempt::Ready{..}));
    let entry=tracker.enter(coherent_session_context(&r2),22);
    assert_eq!(entry.decision,Decision::Active);assert_eq!(entry.change.generation,2);assert!(tracker.control_allowed(22));
    reset_state_for_session(&mut state,3,false);activate_same_type(&r2,&mut state,7,1).unwrap();
    assert!(state.active);assert_eq!(state.count,15);assert_eq!(state.session_boundary,3);
    assert!(table.iter().enumerate().all(|(i,v)|v.load(Ordering::Acquire)==slots[i].replacement));tracker.leave();
}
fn ids(indices: &[u32], generation: u32, shift: u8) -> Vec<u32> {
    indices
        .iter()
        .map(|index| (index + 1) | (generation << shift))
        .collect()
}

#[test]
fn matching_excludes_enemy_type_dead_and_transport_units() {
    let mut f = Fixture::new(7, 7, 37);
    f.write_u8(1, 0x68, 0); // enemy
    f.write_u16(2, 0x8c, 0); // another type
    f.write_u32(3, 0x10, 0); // dead hitpoints
    f.write_u8(4, 0x69, 0); // death order
    f.write_u32(5, 0x140, 1 | 0x40); // inside transport
    let r = f.runtime();
    assert_eq!(r.matching(7, 37, 13).unwrap(), ids(&[0, 6], 1, 13));
    assert_eq!(r.matching(0, 37, 13).unwrap(), ids(&[1], 1, 13));
    assert_eq!(r.matching(7, 0, 13).unwrap(), ids(&[2], 1, 13));
}
#[test]
fn missing_sprite_incomplete_invalid_type_and_invalid_owner_are_excluded() {
    let mut f = Fixture::new(5, 7, 37);
    f.write_u64(0, 0x18, 0);
    f.write_u32(1, 0x140, 0);
    f.write_u16(2, 0x8c, 228);
    f.write_u8(3, 0x68, 8);
    assert_eq!(f.runtime().matching(7, 37, 13).unwrap(), ids(&[4], 1, 13));
}
#[test]
fn selection_reads_owned_local_buffer_and_rejects_duplicates() {
    let mut f = Fixture::new(2, 7, 37);
    f.selection[1] = f.pointer(1);
    let r = f.runtime();
    let selected = r.selected().unwrap();
    assert_eq!(selected.len(), 2);
    assert_eq!(selected[0].owner, 7);
    assert_eq!(selected[1].uid(13), Some(2 | (1 << 13)));
    f.selection[1] = f.pointer(0);
    assert!(r.selected().is_none());
}
#[test]
fn invalid_selection_pointer_is_rejected_without_dereferencing() {
    let mut f = Fixture::new(1, 7, 37);
    f.selection[0] = f.pointer(0) + 1;
    assert!(f.runtime().selected().is_none());
    f.selection[0] = 0;
    assert!(f.runtime().selected().unwrap().is_empty());
}
#[test]
fn cyclic_and_misaligned_lists_fail_whole_scan() {
    let mut f = Fixture::new(2, 7, 37);
    f.write_u64(1, 8, f.pointer(0) as u64);
    assert!(f.runtime().matching(7, 37, 13).is_none());
    f.write_u64(1, 8, f.pointer(0) as u64 + 1);
    assert!(f.runtime().matching(7, 37, 13).is_none());
}
#[test]
fn regenerated_unit_id_invalidates_queued_identity() {
    let mut f = Fixture::new(1, 7, 37);
    let r = f.runtime();
    let old = 1 | (1 << 13);
    assert!(r.valid_id(old, 7, 37, 13));
    f.write_u8(0, 0xe9, 2);
    assert!(!r.valid_id(old, 7, 37, 13));
    assert!(r.valid_id(1 | (2 << 13), 7, 37, 13));
    assert!(!r.valid_id(0, 7, 37, 13));
    assert!(!r.valid_id(1 | (2 << 13), 0, 37, 13));
    assert!(!r.valid_id(1 | (2 << 13), 7, 0, 13));
}
#[test]
fn context_uses_current_values_and_excludes_replay_lobby_exit_and_observer() {
    let mut f = Fixture::new(1, 7, 37);
    let r = f.runtime();
    assert_eq!(r.context(), Some((true, 100, 7)));
    for (index, value) in [(4, 1), (5, 0), (6, 0), (3, 8)] {
        let previous = f.globals[index];
        f.globals[index] = value;
        assert!(!r.context().unwrap().0);
        f.globals[index] = previous;
    }
    f.globals[8] = 101;
    assert_eq!(r.context(), Some((true, 101, 7)));
}
#[test]
fn slot_eight_preserves_one_original_then_queues_twelve_plus_two() {
    let f = Fixture::new(15, 7, 37);
    let r = f.runtime();
    let mut state = f.state();
    let command = click_order(false);
    assert!(queue_control(&command, &ids(&[0], 1, 13), &mut state, &r));
    assert_eq!(state.count, 15);
    assert_eq!(state.pending.len(), 2);
    assert_eq!(
        state.pending[0].ids,
        ids(&(1..13).collect::<Vec<_>>(), 1, 13)
    );
    assert_eq!(state.pending[1].ids, ids(&[13, 14], 1, 13));
    for pending in &state.pending {
        assert_eq!(pending.restore, ids(&[0], 1, 13));
        assert_eq!(pending.command, command);
    }
}
#[test]
fn current_owner_change_clears_previously_queued_jobs() {
    let mut f = Fixture::new(15, 7, 37);
    let r = f.runtime();
    let mut state = f.state();
    assert!(queue_control(
        &click_order(false),
        &ids(&[0], 1, 13),
        &mut state,
        &r
    ));
    f.globals[3] = 0;
    assert!(!queue_control(
        &click_order(false),
        &ids(&[0], 1, 13),
        &mut state,
        &r
    ));
    assert!(!state.active);
    assert!(state.pending.is_empty());
    assert_eq!(state.count, 0);
}
#[test]
fn frame_reset_and_replay_clear_pending_jobs() {
    for which in [0, 1] {
        let mut f = Fixture::new(15, 7, 37);
        let r = f.runtime();
        let mut state = f.state();
        assert!(queue_control(
            &click_order(false),
            &ids(&[0], 1, 13),
            &mut state,
            &r
        ));
        if which == 0 {
            f.globals[8] = 1;
        } else {
            f.globals[4] = 1;
        }
        assert!(!queue_control(
            &click_order(false),
            &ids(&[0], 1, 13),
            &mut state,
            &r
        ));
        assert!(!state.active);
        assert!(state.pending.is_empty());
    }
}
#[test]
fn changed_or_removed_reference_cancels_control_without_queueing() {
    for which in [0, 1, 2, 3] {
        let mut f = Fixture::new(2, 7, 37);
        let r = f.runtime();
        let mut state = f.state();
        match which {
            0 => f.write_u8(0, 0x68, 0),
            1 => f.write_u16(0, 0x8c, 0),
            2 => f.write_u32(0, 0x10, 0),
            _ => f.selection[0] = 0,
        }
        assert!(!queue_control(
            &click_order(false),
            &ids(&[0], 1, 13),
            &mut state,
            &r
        ));
        assert!(!state.active);
        assert!(state.pending.is_empty());
    }
}
#[test]
fn nonqueued_order_replaces_pending_and_queued_order_appends() {
    let f = Fixture::new(15, 7, 37);
    let r = f.runtime();
    let mut state = f.state();
    assert!(queue_control(
        &click_order(false),
        &ids(&[0], 1, 13),
        &mut state,
        &r
    ));
    assert_eq!(state.pending.len(), 2);
    assert!(queue_control(
        &click_order(true),
        &ids(&[0], 1, 13),
        &mut state,
        &r
    ));
    assert_eq!(state.pending.len(), 4);
    assert!(queue_control(
        &click_order(false),
        &ids(&[0], 1, 13),
        &mut state,
        &r
    ));
    assert_eq!(state.pending.len(), 2);
    assert!(state.pending.iter().all(|p| p.command.last() == Some(&0)));
}
#[test]
fn pending_queue_limit_fails_closed_instead_of_partial_append() {
    let f = Fixture::new(15, 7, 37);
    let r = f.runtime();
    let mut state = f.state();
    for _ in 0..MAX_PENDING {
        state.pending.push_back(Pending {
            ids: vec![1 | (1 << 13)],
            restore: vec![1 | (1 << 13)],
            command: click_order(true),
            created: Instant::now(),
        });
    }
    assert!(!queue_control(
        &click_order(true),
        &ids(&[0], 1, 13),
        &mut state,
        &r
    ));
    assert!(!state.active);
    assert!(state.pending.is_empty());
}
#[test]
fn neutral_encoding_accepts_only_equal_short_and_long_ids() {
    let mut f = Fixture::new(1, 7, 37);
    f.write_u8(0, 0xe9, 0);
    let unit = read_unit(f.pointer(0), f.pointer(0)).unwrap();
    assert_eq!(unit.uid(11), Some(1));
    assert_eq!(unit.uid(13), Some(1));
    assert_eq!(unit.uid(0), Some(1));
    assert_eq!(infer_shift(&unit, &[1]), Some(0));
    assert_eq!(f.runtime().matching(7, 37, 0), Some(vec![1]));
    assert!(f.runtime().valid_id(1, 7, 37, 0));
    f.write_u8(0, 0xe9, 1);
    let unit = read_unit(f.pointer(0), f.pointer(0)).unwrap();
    assert_ne!(unit.uid(11), unit.uid(13));
    assert_eq!(unit.uid(0), None);
    assert!(!f.runtime().valid_id(1, 7, 37, 0));
}
#[test]
fn neutral_encoding_rejects_mixed_generation_group_whole_scan() {
    let mut f = Fixture::new(2, 7, 37);
    f.write_u8(0, 0xe9, 0);
    f.write_u8(1, 0xe9, 0);
    assert_eq!(f.runtime().matching(7, 37, 0), Some(vec![1, 2]));
    f.write_u8(1, 0xe9, 1);
    assert!(f.runtime().matching(7, 37, 0).is_none());
}

fn attach_vector(
    runtime: &mut Runtime,
    data: usize,
    length: usize,
    capacity: usize,
) -> Box<[usize; 3]> {
    let vector = Box::new([data, length, capacity]);
    runtime
        .unit_vectors
        .push(Value::Constant(vector.as_ptr() as usize));
    vector
}
#[test]
fn authoritative_vector_uses_length_boundary_for_id_mode() {
    for (count, shift) in [(1700, 11), (1701, 13), (3400, 13)] {
        let mut f = Fixture::new(count, 7, 37);
        f.write_u8(0, 0xe9, 0);
        let mut runtime = f.runtime();
        let _vector = attach_vector(&mut runtime, f.pointer(0), count, count);
        assert_eq!(
            runtime.unit_layout(),
            Some(UnitLayout {
                base: f.pointer(0),
                count: Some(count),
                shift: Some(shift)
            })
        );
        let reference = read_unit(f.pointer(0), f.pointer(0)).unwrap();
        // Generation zero no longer forces the fallback neutral encoding.
        assert_eq!(runtime.selection_shift(&reference, &[1]), Some(shift));
        assert_eq!(runtime.selection_shift(&reference, &[2]), None);
        let matching = runtime.matching(7, 37, shift).unwrap();
        assert_eq!(matching.len(), count);
        assert!(runtime.valid_id(*matching.last().unwrap(), 7, 37, shift));
    }
}
#[test]
fn unmatched_or_unreadable_candidates_preserve_only_observed_id_fallback() {
    let f = Fixture::new(1, 7, 37);
    let mut runtime = f.runtime();
    let _unmatched = attach_vector(&mut runtime, f.pointer(0) + UNIT_SIZE, 100, 100);
    runtime.unit_vectors.push(Value::Constant(1));
    assert_eq!(
        runtime.unit_layout(),
        Some(UnitLayout {
            base: f.pointer(0),
            count: None,
            shift: None
        })
    );
    let reference = read_unit(f.pointer(0), f.pointer(0)).unwrap();
    assert_eq!(
        runtime.selection_shift(&reference, &[1 | (1 << 13)]),
        Some(13)
    );
    assert_eq!(
        runtime.selection_shift(&reference, &[1 | (1 << 11)]),
        Some(11)
    );
    assert_eq!(runtime.selection_shift(&reference, &[1]), None);
}
#[test]
fn malformed_matching_vector_fails_closed_instead_of_falling_back() {
    let f = Fixture::new(2, 7, 37);
    for (length, capacity) in [
        (0, 2),
        (3, 2),
        (MAX_UNITS, MAX_UNITS),
        (1, MAX_UNITS + 1),
        (usize::MAX, usize::MAX),
    ] {
        let mut runtime = f.runtime();
        let _vector = attach_vector(&mut runtime, f.pointer(0), length, capacity);
        assert_eq!(runtime.unit_layout(), None);
        assert!(runtime.selected().is_none());
        assert!(runtime.matching(7, 37, 13).is_none());
        assert!(!runtime.valid_id(1 | (1 << 13), 7, 37, 13));
    }
}
#[test]
fn equal_data_candidates_require_consistent_lengths() {
    let f = Fixture::new(2, 7, 37);
    let mut runtime = f.runtime();
    let _first = attach_vector(&mut runtime, f.pointer(0), 2, 2);
    let mut second = attach_vector(&mut runtime, f.pointer(0), 2, 2);
    assert_eq!(runtime.unit_layout().unwrap().count, Some(2));
    second[1] = 1;
    assert_eq!(runtime.unit_layout(), None);
}
#[test]
fn matching_vector_bounds_apply_before_selected_or_linked_unit_reads() {
    let mut f = Fixture::new(2, 7, 37);
    let mut runtime = f.runtime();
    let _vector = attach_vector(&mut runtime, f.pointer(0), 1, 2);
    f.selection[0] = f.pointer(1);
    assert!(runtime.selected().is_none());
    assert!(runtime.matching(7, 37, 11).is_none()); // next points beyond length
    assert!(!runtime.valid_id(2 | (1 << 11), 7, 37, 11));
    f.selection[0] = f.pointer(0);
    f.write_u64(0, 8, 0);
    assert_eq!(runtime.selected().unwrap().len(), 1);
    assert_eq!(runtime.matching(7, 37, 11).unwrap(), ids(&[0], 1, 11));
}
#[test]
fn authoritative_mode_rejects_observed_wrong_mode_and_out_of_range_reference() {
    let f = Fixture::new(2, 7, 37);
    let mut runtime = f.runtime();
    let _vector = attach_vector(&mut runtime, f.pointer(0), 1, 2);
    let reference = read_unit(f.pointer(0), f.pointer(0)).unwrap();
    assert_eq!(runtime.selection_shift(&reference, &[1 | (1 << 13)]), None);
    assert_eq!(
        runtime.selection_shift(&reference, &[1 | (1 << 11)]),
        Some(11)
    );
    assert_eq!(runtime.selection_shift(&reference, &[]), None);
    assert!(runtime.matching(7, 37, 13).is_none());
    assert!(!runtime.valid_id(1 | (1 << 13), 7, 37, 13));
    let outside = read_unit(f.pointer(1), f.pointer(0)).unwrap();
    assert_eq!(runtime.selection_shift(&outside, &[2 | (1 << 11)]), None);
}
#[test]
fn invalid_shift_values_fail_without_overflow_or_reading_units() {
    let f = Fixture::new(1, 7, 37);
    let runtime = f.runtime();
    for shift in [1, 12, 32, 64, 255] {
        assert!(!runtime.valid_id(1, 7, 37, shift));
    }
}

#[test]
fn client_heartbeat_expires_and_never_accepts_zero_or_future_time() {
    assert!(client_present(10000, 10000));
    assert!(client_present(12999, 10000));
    assert!(!client_present(13000, 10000));
    assert!(!client_present(20000, 10000));
    assert!(!client_present(10000, 0));
    assert!(!client_present(10000, 10001));
}

#[test]
fn single_matching_unit_receives_no_copy_of_its_original_command() {
    let f = Fixture::new(1, 7, 37);
    let runtime = f.runtime();
    let mut state = f.state();
    assert!(queue_control(
        &click_order(false),
        &ids(&[0], 1, 13),
        &mut state,
        &runtime
    ));
    assert!(state.pending.is_empty());
    assert!(state.active);
    assert_eq!(state.count, 1);
}
#[test]
fn every_original_selected_unit_is_excluded_from_copies() {
    let mut f = Fixture::new(15, 7, 37);
    f.selection[1] = f.pointer(1);
    let runtime = f.runtime();
    let mut state = f.state();
    let original = ids(&[0, 1], 1, 13);
    assert!(queue_control(
        &click_order(true),
        &original,
        &mut state,
        &runtime
    ));
    assert_eq!(state.pending.len(), 2);
    assert_eq!(
        state.pending[0].ids,
        ids(&(2..14).collect::<Vec<_>>(), 1, 13)
    );
    assert_eq!(state.pending[1].ids, ids(&[14], 1, 13));
    for job in state.pending {
        assert_eq!(job.restore, original);
    }
}
#[test]
fn same_kind_but_changed_selection_is_rejected_before_queueing() {
    let mut f = Fixture::new(15, 7, 37);
    let runtime = f.runtime();
    let mut state = f.state();
    f.selection[0] = f.pointer(1);
    assert!(!queue_control(
        &click_order(false),
        &ids(&[0], 1, 13),
        &mut state,
        &runtime
    ));
    assert!(!state.active);
    assert!(state.pending.is_empty());
}
#[test]
fn owned_outgoing_snapshot_reads_exact_occupied_prefix_and_capacity() {
    let mut f = Fixture::new(1, 7, 37);
    f.outgoing_buffer[..3].copy_from_slice(&[7, 8, 9]);
    f.globals[9] = 3;
    let snapshot = buffer_snapshot(&f.runtime()).unwrap();
    assert_eq!(snapshot.bytes, [7, 8, 9]);
    assert_eq!(snapshot.capacity, 496);
    assert_eq!(snapshot.buffer, f.outgoing_buffer.as_ptr() as usize);
    f.globals[10] = 400;
    assert_eq!(buffer_snapshot(&f.runtime()).unwrap().capacity, 400);
}
#[test]
fn owned_outgoing_snapshot_rejects_length_capacity_and_pointer_errors() {
    let mut f = Fixture::new(1, 7, 37);
    for capacity in [0, 63, 513, usize::MAX] {
        f.globals[10] = capacity;
        assert!(buffer_snapshot(&f.runtime()).is_none());
    }
    f.globals[10] = 496;
    for length in [481, 497, usize::MAX] {
        f.globals[9] = length;
        assert!(buffer_snapshot(&f.runtime()).is_none());
    }
    f.globals[9] = 100;
    f.globals[10] = 64;
    assert!(buffer_snapshot(&f.runtime()).is_none());
    f.globals[9] = 0;
    f.globals[10] = 496;
    f.globals[11] = 0;
    assert!(buffer_snapshot(&f.runtime()).is_none());
}


#[test]
fn resolver_selection_address_matches_actual_mem_sub_const_contract() {
    let f = Fixture::new(5, 7, 37);
    let ctx = OperandContext::new();
    let address = f.selection.as_ptr() as u64;
    let last_slot = ctx.mem64(ctx.constant(address), 11 * 8);
    let op = ctx.mem_sub_const_op(last_slot.if_memory().unwrap(), 11 * 8);
    assert_eq!(op.if_constant(), Some(address));
    let value = Value::from_operand(op, 0, 0, 0, 0).unwrap();
    assert_eq!(value.read(), Some(address as usize));
    assert_ne!(value.read(), Some(f.pointer(0)));
    let r = f.runtime();
    assert_eq!(r.selected().unwrap().len(), 1);
    assert_eq!(r.selected().unwrap()[0].pointer, f.pointer(0));
}
#[test]
fn resolver_selection_address_rebases_constant_and_keeps_empty_array() {
    let mut f = Fixture::new(5, 7, 37);
    let ctx = OperandContext::new();
    let preferred = 0x140000000;
    let actual = f.selection.as_ptr() as usize;
    let last_slot = ctx.mem64(ctx.const_0(), preferred + 11 * 8);
    let op = ctx.mem_sub_const_op(last_slot.if_memory().unwrap(), 11 * 8);
    let mut r = f.runtime();
    r.selection = Value::from_operand(op, preferred, preferred + 0x1000, actual, 0).unwrap();
    assert_eq!(r.selection.read(), Some(actual));
    assert_eq!(r.selected().unwrap().len(), 1);
    f.selection.fill(0);
    assert_eq!(r.selection.read(), Some(actual));
    assert!(r.selected().unwrap().is_empty());
}
#[test]
fn resolver_selection_address_preserves_dynamic_base_without_extra_dereference() {
    let f = Fixture::new(5, 7, 37);
    let ctx = OperandContext::new();
    let holder = Box::new(f.selection.as_ptr() as usize);
    let base = ctx.mem64(ctx.constant(holder.as_ref() as *const usize as u64), 0);
    let last_slot = ctx.mem64(base, 11 * 8);
    let op = ctx.mem_sub_const_op(last_slot.if_memory().unwrap(), 11 * 8);
    assert!(op.if_memory().is_some()); // this load yields the ARRAY base
    let mut r = f.runtime();
    r.selection = Value::from_operand(op, 0, 0, 0, 0).unwrap();
    assert_eq!(r.selection.read(), Some(f.selection.as_ptr() as usize));
    assert_eq!(r.selected().unwrap().len(), 1);
    assert_eq!(r.selected().unwrap()[0].pointer, f.pointer(0));
}
#[test]
fn resolver_selection_address_preserves_dynamic_base_plus_offset() {
    let f = Fixture::new(5, 7, 37);
    let ctx = OperandContext::new();
    let actual = f.selection.as_ptr() as usize;
    let holder = Box::new(actual - 32);
    let base = ctx.mem64(ctx.constant(holder.as_ref() as *const usize as u64), 0);
    let last_slot = ctx.mem64(base, 32 + 11 * 8);
    let op = ctx.mem_sub_const_op(last_slot.if_memory().unwrap(), 11 * 8);
    assert!(op.if_arithmetic_add().is_some());
    let mut r = f.runtime();
    r.selection = Value::from_operand(op, 0, 0, 0, 0).unwrap();
    assert_eq!(r.selection.read(), Some(actual));
    assert_eq!(r.selected().unwrap().len(), 1);
}
#[test]
fn same_type_activation_accepts_one_five_and_twelve_without_changing_native_selection() {
    for selected_count in [1, 5, 12] {
        let mut f = Fixture::new(15, 7, 37);
        for i in 0..selected_count { f.selection[i] = f.pointer(i); }
        let before = *f.selection;
        let units_before = f.units.to_vec();
        let mut r = f.runtime();
        let _vector = attach_vector(&mut r, f.pointer(0), 15, 15);
        let mut state = f.state(); state.active = false; state.last_selection.clear();
        activate_same_type(&r, &mut state, 7, 100).unwrap();
        assert!(state.active); assert_eq!(state.count, 15); assert_eq!(state.shift, 11);
        assert_eq!(state.last_selection, ids(&(0..selected_count as u32).collect::<Vec<_>>(), 1, 11));
        assert_eq!(*f.selection, before); assert_eq!(f.units.as_ref(), units_before);
        assert!(state.pending.is_empty()); assert_eq!(state.sent, 0);
    }
}
#[test]
fn same_type_activation_rejects_empty_mixed_and_other_owner_without_state_change() {
    for condition in [0, 1, 2, 3] {
        let mut f = Fixture::new(5, 7, 37);
        f.selection[1] = f.pointer(1);
        let expected = match condition {
            0 => { f.selection.fill(0); "Select owned same-type units first" },
            1 => { f.write_u16(1, 0x8c, 38); "Selected units have different types" },
            2 => { f.write_u8(1, 0x68, 0); "Selected units are not all owned by local player" },
            _ => { f.selection[1] = f.pointer(0); "Selected unit pointers are duplicated" },
        };
        let before = *f.selection;
        let mut r = f.runtime(); let _vector = attach_vector(&mut r, f.pointer(0), 5, 5);
        let mut state = f.state(); state.active = false; state.last_selection.clear();
        assert_eq!(activate_same_type(&r, &mut state, 7, 100), Err(expected));
        assert!(!state.active); assert!(state.pending.is_empty());
        assert!(state.last_selection.is_empty()); assert_eq!(*f.selection, before);
    }
}
#[test]
fn five_selected_out_of_fifteen_exclude_all_originals_and_restore_original_order() {
    let mut f = Fixture::new(15, 7, 37);
    let originals = [7, 3, 11, 0, 14];
    for (i, index) in originals.iter().enumerate() { f.selection[i] = f.pointer(*index); }
    let before = *f.selection;
    let mut r = f.runtime(); let _vector = attach_vector(&mut r, f.pointer(0), 15, 15);
    let mut state = f.state(); state.active = false; state.last_selection.clear();
    activate_same_type(&r, &mut state, 7, 100).unwrap();
    let original_ids = ids(&originals.map(|x| x as u32), 1, 11);
    assert_eq!(state.last_selection, original_ids);
    assert!(queue_control(&click_order(false), &original_ids, &mut state, &r));
    let expected = ids(&(0..15).filter(|x| !originals.contains(&(*x as usize)))
        .collect::<Vec<_>>(), 1, 11);
    assert_eq!(state.pending.len(), 1);
    assert_eq!(state.pending[0].ids, expected);
    assert_eq!(state.pending[0].restore, original_ids);
    assert_eq!(state.count, 15); assert_eq!(*f.selection, before);
    assert_eq!(selection_record(&state.pending[0].restore).unwrap()[1], 5);
}
#[test]
fn twelve_selected_out_of_twenty_nine_preserve_twelve_then_copy_twelve_and_five() {
    let mut f = Fixture::new(29, 7, 37);
    for i in 0..12 { f.selection[i] = f.pointer(i); }
    let mut r = f.runtime(); let _vector = attach_vector(&mut r, f.pointer(0), 29, 29);
    let mut state = f.state(); state.active = false; state.last_selection.clear();
    activate_same_type(&r, &mut state, 7, 100).unwrap();
    let original_ids = ids(&(0..12).collect::<Vec<_>>(), 1, 11);
    assert!(queue_control(&click_order(false), &original_ids, &mut state, &r));
    assert_eq!(state.pending.len(), 2);
    assert_eq!(state.pending[0].ids.len(), 12); assert_eq!(state.pending[1].ids.len(), 5);
    for batch in &state.pending {
        assert_eq!(batch.restore, original_ids);
        assert!(!batch.ids.iter().any(|x| original_ids.contains(x)));
    }
}
#[test]
fn group_encoding_uses_every_observed_id_and_rejects_mixed_or_unknown_modes() {
    let mut f = Fixture::new(5, 7, 37);
    for i in 0..5 { f.selection[i] = f.pointer(i); }
    let r = f.runtime(); let selected = r.selected().unwrap();
    assert_eq!(infer_group_shift(&selected, &ids(&(0..5).collect::<Vec<_>>(), 1, 11)), Some(11));
    assert_eq!(infer_group_shift(&selected, &ids(&(0..5).collect::<Vec<_>>(), 1, 13)), Some(13));
    assert_eq!(infer_group_shift(&selected, &[]), None);
    let mut mixed = ids(&(0..5).collect::<Vec<_>>(), 1, 11); mixed[4] = 5 | (1 << 13);
    assert_eq!(infer_group_shift(&selected, &mixed), None);
    let mut state = f.state(); state.active = false; state.last_selection = mixed;
    assert_eq!(activate_same_type(&r, &mut state, 7, 100),
        Err("Selection ID encoding unavailable; click selected units again"));
    assert!(!state.active);
}
#[test]
fn group_activation_requires_selected_units_in_live_matching_list() {
    let mut f = Fixture::new(5, 7, 37);
    f.selection[0] = f.pointer(4);
    let mut r = f.runtime(); let _vector = attach_vector(&mut r, f.pointer(0), 5, 5);
    f.write_u64(0, 8, 0); // selected unit exists but is absent from active list
    let mut state = f.state(); state.active = false;
    assert_eq!(activate_same_type(&r, &mut state, 7, 100),
        Err("Selected units are missing from matching unit list"));
    assert!(!state.active); assert!(state.pending.is_empty());
}


#[test]
fn completed_x64_unit_is_accepted_with_zero_at_previous_wrong_offset() {
    let mut f = Fixture::new(5, 7, 37);
    for i in 0..5 {
        f.selection[i] = f.pointer(i);
        f.write_u32(i, 0x138, 0); // unrelated field previously mistaken for flags
        f.write_u32(i, 0x140, 1);
    }
    let r = f.runtime();
    assert_eq!(r.selected_checked().unwrap().len(), 5);
    let mut r = f.runtime(); let _vector = attach_vector(&mut r, f.pointer(0), 5, 5);
    let mut state = f.state(); state.active = false; state.last_selection.clear();
    activate_same_type(&r, &mut state, 7, 100).unwrap();
    assert!(state.active); assert_eq!(state.count, 5);
}
#[test]
fn unrelated_old_offset_never_overrides_actual_completed_or_transport_flags() {
    let mut f = Fixture::new(1, 7, 37);
    f.write_u32(0, 0x138, 1);
    f.write_u32(0, 0x140, 0);
    assert_eq!(read_unit_checked(f.pointer(0), f.pointer(0)).err().unwrap().message,
        "Selected unit is not completed");
    f.write_u32(0, 0x138, 0);
    f.write_u32(0, 0x140, 1 | 0x40);
    assert_eq!(read_unit_checked(f.pointer(0), f.pointer(0)).err().unwrap().message,
        "Selected unit is inside transport");
    f.write_u32(0, 0x138, 1 | 0x40);
    f.write_u32(0, 0x140, 1);
    assert!(read_unit(f.pointer(0), f.pointer(0)).is_some());
}
#[test]
fn selection_diagnostic_identifies_slot_and_actual_x64_flags_field() {
    let mut f = Fixture::new(2, 7, 37);
    f.selection[1] = f.pointer(1);
    f.write_u32(1, 0x140, 0);
    let error = f.runtime().selected_checked().err().unwrap();
    assert_eq!(error.message, "Selected unit is not completed");
    assert!(error.detail.contains("slot=1"));
    assert!(error.detail.contains("flags@0x140=0x0"));
    assert!(error.detail.contains("owner=7; kind=37"));
    assert!(error.detail.contains("array=0x"));
}
#[test]
fn selection_diagnostic_distinguishes_duplicate_address_range_and_slot_read() {
    let mut f = Fixture::new(2, 7, 37);
    f.selection[1] = f.pointer(0);
    assert_eq!(f.runtime().selected_checked().err().unwrap().message,
        "Selected unit pointers are duplicated");
    f.selection[1] = 0;
    f.selection[0] = f.pointer(0) + 1;
    let error = f.runtime().selected_checked().err().unwrap();
    assert_eq!(error.message, "Selected unit pointer is outside unit array");
    assert!(error.detail.contains("stride=0x1e8"));
    let mut r = f.runtime(); r.selection = Value::Constant(1);
    assert_eq!(r.selected_checked().err().unwrap().message,
        "Selection slot memory read failed");
}
#[test]
fn failed_activation_keeps_mode_off_and_records_selection_failure_detail() {
    let mut f = Fixture::new(1, 7, 37);
    f.write_u32(0, 0x140, 0);
    let r = f.runtime(); let mut state = f.state(); state.active = false;
    assert_eq!(activate_same_type(&r, &mut state, 7, 321), Err("Selected unit is not completed"));
    assert!(!state.active); assert!(state.pending.is_empty()); assert_eq!(state.sent, 0);
    let record = state.events.back().unwrap();
    assert!(record.starts_with("321\t"));
    assert!(record.contains("Selection diagnostic; reason=Selected unit is not completed"));
    assert!(record.contains("flags@0x140=0x0"));
}

#[test]
fn requested_controls_are_exact_and_attack_specific() {
    for opcode in [0x1b,0x1c] { assert!(callback_control_supported(&[opcode])); assert!(!callback_control_supported(&[opcode,0])); }
    assert!(callback_control_supported(&[0x1a,0]));
    assert!(callback_control_supported(&[0x1a,1]));
    assert!(!callback_control_supported(&[0x1a,2]));
    for order in [8,9,10,11,12,14,53,59,134,135] {
        let mut attack=vec![0;13];attack[0]=0x61;attack[11]=order;
        assert!(callback_control_supported(&attack));
        attack[12]=1;assert!(callback_control_supported(&attack));
        attack[12]=2;assert!(!callback_control_supported(&attack));
    }
    let mut ability=vec![0;13];ability[0]=0x61;ability[11]=33;
    assert!(!callback_control_supported(&ability));
    assert!(!callback_control_supported(&[0x2b,0]));
}
#[test]
fn stop_and_attack_replicate_to_others_and_restore_all_original_units() {
    let f=Fixture::new(15,7,37); let runtime=f.runtime();
    let mut attack=vec![0;13];attack[0]=0x61;attack[1..5].copy_from_slice(&[40,0,60,0]);attack[11]=14;
    for command in [vec![0x1a,0],vec![0x1b],vec![0x1c],attack] {
        let mut state=f.state();
        assert!(queue_control(&command,&state.last_selection.clone(),&mut state,&runtime));
        assert_eq!(state.pending.len(),2);
        let all:Vec<_>=state.pending.iter().flat_map(|p|p.ids.iter().copied()).collect();
        assert_eq!(all,ids(&(1..15).collect::<Vec<_>>(),1,13));
        for job in &state.pending { assert_eq!(job.command,command);assert_eq!(job.restore,ids(&[0],1,13)); }
    }
}
#[test]
fn ordinary_same_selection_ui_reset_preserves_mode_but_session_or_selection_changes_do_not() {
    let mut f=Fixture::new(15,7,37); let state=f.state();
    assert!(can_preserve_ui_reset(&f.runtime(),&state,(true,100,7)));
    assert!(!can_preserve_ui_reset(&f.runtime(),&state,(false,100,7)));
    assert!(!can_preserve_ui_reset(&f.runtime(),&state,(true,99,7)));
    assert!(!can_preserve_ui_reset(&f.runtime(),&state,(true,100,6)));
    f.selection[0]=f.pointer(1);
    assert!(!can_preserve_ui_reset(&f.runtime(),&state,(true,100,7)));
}

include!("runtime_building_tests.rs");

#[test]
fn hud_reports_one_five_or_twelve_ordinary_units_without_control_or_selection_writes() {
    for count in [1, 5, 12] {
        let mut f = Fixture::new(15, 7, 41);
        for index in 0..count { f.selection[index] = f.pointer(index); }
        let before_units = f.units.to_vec();
        let before_selection = *f.selection;
        let mut state = f.state();
        state.active = false;
        state.count = 987;
        state.kind = 0;
        let display = hud_selection(Some(&f.runtime()), &state, true);
        assert_eq!(display, DisplaySelection { control_active: false, in_game: true, count, kind: 41 });
        assert_eq!(*f.selection, before_selection);
        assert_eq!(f.units.as_ref(), before_units.as_slice());
        assert_eq!(state.count, 987);
        assert!(state.pending.is_empty());
        assert_eq!(f.globals[9], 0);
    }
}

#[test]
fn hud_empty_and_mixed_selection_have_unambiguous_name_sentinel() {
    let mut f = Fixture::new(3, 7, 37);
    let mut state = f.state(); state.active = false;
    f.selection.fill(0);
    assert_eq!(hud_selection(Some(&f.runtime()), &state, true), DisplaySelection::empty(true));
    f.selection[0] = f.pointer(0);
    f.selection[1] = f.pointer(1);
    f.write_u16(1, 0x8c, 41);
    assert_eq!(hud_selection(Some(&f.runtime()), &state, true),
        DisplaySelection { control_active: false, in_game: true, count: 2, kind: NO_DISPLAY_UNIT });
}

#[test]
fn hud_accepts_enemy_neutral_unfinished_and_morphing_selection_without_widening_control() {
    for (owner, kind, flags) in [(0, 37, 1), (11, 41, 1), (7, 106, 0), (7, 36, 0), (7, 59, 0)] {
        let mut f = Fixture::new(1, 7, kind);
        f.write_u8(0, 0x68, owner);
        f.write_u32(0, 0x140, flags);
        let mut state = f.state(); state.active = false;
        assert_eq!(hud_selection(Some(&f.runtime()), &state, true),
            DisplaySelection { control_active: false, in_game: true, count: 1, kind });
        assert!(activate_same_type(&f.runtime(), &mut state, 7, 100).is_err());
        assert!(!state.active);
        assert!(state.pending.is_empty());
    }
}

#[test]
fn hud_clears_malformed_duplicate_dead_or_unreadable_selection() {
    for scenario in 0..9 {
        let mut f = Fixture::new(2, 7, 37);
        let mut state = f.state(); state.active = false;
        let mut runtime = f.runtime();
        match scenario {
            0 => { f.selection[1] = f.pointer(0); },
            1 => { f.selection[0] = f.pointer(0) + 1; },
            2 => { f.selection[0] = f.units.as_ptr() as usize - UNIT_SIZE; },
            3 => { f.write_u32(0, 0x10, 0); },
            4 => { f.write_u32(0, 0x10, u32::MAX); },
            5 => { f.write_u64(0, 0x18, 0); },
            6 => { f.write_u16(0, 0x8c, 228); },
            7 => { f.write_u8(0, 0x68, 12); },
            _ => { runtime.selection = Value::Constant(1); },
        }
        assert_eq!(hud_selection(Some(&runtime), &state, true), DisplaySelection::empty(true),
            "scenario {scenario}");
    }
}

#[test]
fn hud_hides_for_menu_exit_replay_failed_status_or_missing_context_but_not_pause() {
    let mut f = Fixture::new(1, 7, 37);
    let mut state = f.state(); state.active = false;
    f.globals[7] = 1; // Paused match: passive selected-unit HUD remains available.
    assert_eq!(hud_selection(Some(&f.runtime()), &state, true).count, 1);
    assert_eq!(hud_selection(Some(&f.runtime()), &state, false), DisplaySelection::empty(false));
    assert_eq!(hud_selection(None, &state, true), DisplaySelection::empty(false));
    for (index, value) in [(4, 1), (5, 0), (6, 0), (3, 8)] {
        let saved = f.globals[index];
        f.globals[index] = value;
        assert_eq!(hud_selection(Some(&f.runtime()), &state, true), DisplaySelection::empty(false));
        f.globals[index] = saved;
    }
}

#[test]
fn hud_active_virtual_count_can_exceed_twelve_but_changed_selection_uses_ordinary_count() {
    let mut f = Fixture::new(15, 7, 37);
    for index in 0..5 { f.selection[index] = f.pointer(index); }
    let mut state = f.state();
    state.count = 15;
    state.shift = 11;
    state.last_selection = ids(&[0, 1, 2, 3, 4], 1, 11);
    assert_eq!(hud_selection(Some(&f.runtime()), &state, true),
        DisplaySelection { control_active: true, in_game: true, count: 15, kind: 37 });
    f.selection.fill(0);
    f.selection[0] = f.pointer(10);
    assert_eq!(hud_selection(Some(&f.runtime()), &state, true),
        DisplaySelection { control_active: false, in_game: true, count: 1, kind: 37 });
    assert_eq!(state.count, 15); // Publishing never changes command state.
}

#[test]
fn hud_ignores_stale_virtual_context_and_authoritative_unit_vector_bounds() {
    let mut f = Fixture::new(2, 7, 37);
    let mut state = f.state();
    state.count = 15;
    state.shift = 11;
    state.last_selection = ids(&[0], 1, 11);
    state.owner = 0;
    assert_eq!(hud_selection(Some(&f.runtime()), &state, true).count, 1);
    state.owner = 7; state.frame = 101;
    assert_eq!(hud_selection(Some(&f.runtime()), &state, true).count, 1);
    state.active = false;
    let vector = Box::new([f.units.as_ptr() as usize, 1usize, 2usize]);
    let mut runtime = f.runtime();
    runtime.unit_vectors.push(Value::Constant(vector.as_ptr() as usize));
    f.selection[0] = f.pointer(1);
    assert_eq!(hud_selection(Some(&runtime), &state, true), DisplaySelection::empty(true));
}

#[test]
fn scmulti2_preserves_control_count_and_appends_passive_hud_fields() {
    let f = Fixture::new(1, 7, 37);
    let mut state = f.state(); state.active = false; state.count = 15; state.sent = 9;
    let text = encode_status(123, &state, "READY", None,
        DisplaySelection { control_active: false, in_game: true, count: 1, kind: 41 });
    assert_eq!(text.trim_end().split('\t').collect::<Vec<_>>(),
        ["SCMULTI2", "123", "0", "0", "37", "9", "READY", "test", "1", "1", "41"]);
    state.active = true;
    let text = encode_status(123, &state, "READY", Some("safe\tmessage\n"),
        DisplaySelection { control_active: true, in_game: true, count: 15, kind: 37 });
    assert_eq!(text.trim_end().split('\t').collect::<Vec<_>>(),
        ["SCMULTI2", "123", "1", "15", "37", "9", "READY", "safemessage", "1", "15", "37"]);
}

#[test]
fn scmulti2_fallback_snapshot_is_inactive_and_preserves_passive_hud_contract() {
    for scenario in 0..7 {
        let mut f = Fixture::new(15, 7, 37);
        let mut state = f.state();
        state.count = 15;
        state.shift = 11;
        state.last_selection = ids(&[0], 1, 11);
        let ready = scenario != 6;
        match scenario {
            0 => { f.selection[0] = f.pointer(10); }, // Different native selection.
            1 => { state.owner = 0; }, // Local owner changed.
            2 => { state.frame = 101; }, // Previous match/frame.
            3 => { f.globals[4] = 1; }, // Replay.
            4 => { f.globals[5] = 0; }, // Menu.
            5 => { f.globals[6] = 0; }, // Match ended.
            _ => {}, // Failed/waiting module status.
        }
        let display = hud_selection(Some(&f.runtime()), &state, ready);
        let encoded = encode_status(123, &state, if ready { "READY" } else { "FAILED" }, None, display);
        let fields = encoded.trim_end().split('\t').collect::<Vec<_>>();
        assert_eq!(fields.len(), 11);
        assert_eq!((fields[2], fields[3]), ("0", "0"), "scenario {scenario}");
        if scenario <= 2 {
            assert_eq!((fields[8], fields[9], fields[10]), ("1", "1", "37"));
        } else {
            assert_eq!((fields[8], fields[9], fields[10]), ("0", "0", "65535"));
        }
        assert!(state.active); // Serialization never mutates command state.
        assert_eq!(state.count, 15);
        assert!(state.pending.is_empty());
    }
    // Matching passive metadata alone is not proof of virtual-group validation.
    let f = Fixture::new(1, 7, 37);
    let state = f.state();
    let passive = DisplaySelection { control_active: false, in_game: true, count: 1, kind: 37 };
    let encoded = encode_status(123, &state, "READY", None, passive);
    let fields = encoded.trim_end().split('\t').collect::<Vec<_>>();
    assert_eq!((fields[2], fields[3], fields[9], fields[10]), ("0", "0", "1", "37"));
}
