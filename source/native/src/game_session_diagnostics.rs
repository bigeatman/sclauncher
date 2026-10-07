//! Bounded numeric observations of game/session readiness transitions.
//!
//! Every fact is supplied by the caller. This module has no process, game,
//! memory, file, clock or UI access and stores no pointers, names, code or
//! command contents. Game end does not clear already retained observations.

use std::collections::VecDeque;
use std::fmt::Write as _;

pub const MAX_RECORDS: usize = 64;
/// Decimal field widths and the retained record limit bound every rendering.
pub const MAX_RENDER_BYTES: usize = 8192;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Record {
    pub session: u64,
    pub boundary_epoch: u64,
    pub frame: u32,
    pub owner: u8,
    pub active: bool,
    pub ready: bool,
    pub thread_bound: bool,
    pub fault: bool,
}

impl Record {
    fn same_key(self, other: Self) -> bool {
        self.session == other.session
            && self.boundary_epoch == other.boundary_epoch
            && self.owner == other.owner
            && self.active == other.active
            && self.ready == other.ready
            && self.thread_bound == other.thread_bound
            && self.fault == other.fault
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Entry {
    record: Record,
    first_frame: u32,
    /// Total consecutive observations, including the first, saturating at MAX.
    repeats: u32,
}

#[derive(Debug)]
pub struct History {
    records: VecDeque<Entry>,
}

impl History {
    pub const fn new() -> Self {
        Self { records: VecDeque::new() }
    }

    /// Consecutive equal non-frame facts share one transition row. Frame-only
    /// progress updates its last frame and saturating observation count without
    /// appending rows. Returning to an earlier state is a new transition.
    /// Returns true exactly when the retained rendering changes.
    pub fn observe(&mut self, record: Record) -> bool {
        if let Some(last) = self.records.back_mut().filter(|entry| entry.record.same_key(record)) {
            let changed = last.record.frame != record.frame || last.repeats != u32::MAX;
            last.record.frame = record.frame;
            last.repeats = last.repeats.saturating_add(1);
            return changed;
        }
        if self.records.len() == MAX_RECORDS {
            self.records.pop_front();
        }
        self.records.push_back(Entry { record, first_frame: record.frame, repeats: 1 });
        true
    }

    /// SCGAMESESSION1 header: marker, PID, retained transition count.
    /// S rows: session, boundary epoch, first frame, last frame, owner,
    /// active (0/1), ready (0/1), thread bound (0/1), fault (0/1), repeats.
    /// Rendering is fixed numeric ASCII, at most MAX_RENDER_BYTES bytes.
    pub fn render(&self, pid: u32) -> String {
        let mut output = String::with_capacity(64 + self.records.len() * 112);
        let _ = writeln!(output, "SCGAMESESSION1\t{pid}\t{}", self.records.len());
        for entry in &self.records {
            let record = entry.record;
            let _ = writeln!(output, "S\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                record.session, record.boundary_epoch, entry.first_frame,
                record.frame, record.owner, u8::from(record.active),
                u8::from(record.ready), u8::from(record.thread_bound),
                u8::from(record.fault), entry.repeats);
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(session: u64, frame: u32) -> Record {
        Record { session, boundary_epoch: session, frame, owner: 0,
            active: true, ready: true, thread_bound: true, fault: false }
    }

    #[test]
    fn empty_history_has_the_fixed_header_and_const_constructor() {
        const EMPTY: History = History::new();
        assert_eq!(EMPTY.render(123), "SCGAMESESSION1\t123\t0\n");
    }

    #[test]
    fn two_games_with_waiting_and_menu_keep_each_transition() {
        let mut history = History::new();
        assert!(history.observe(ready(1, 10)));
        assert!(history.observe(ready(1, 100)));
        let waiting = Record { ready: false, frame: 110, ..ready(1, 110) };
        assert!(history.observe(waiting));
        let menu = Record { session: 1, boundary_epoch: 2, frame: 0, owner: 255,
            active: false, ready: false, thread_bound: false, fault: false };
        assert!(history.observe(menu));
        let second = Record { boundary_epoch: 3, owner: 2, ..ready(2, 1) };
        assert!(history.observe(second));
        assert_eq!(history.records.len(), 4);
        assert_eq!(history.records[0].first_frame, 10);
        assert_eq!(history.records[0].record.frame, 100);
        assert_eq!(history.records[0].repeats, 2);
        assert_eq!(history.records[1].record, waiting);
        assert_eq!(history.records[2].record, menu);
        assert_eq!(history.records[3].record, second);
    }

    #[test]
    fn fault_history_survives_game_end_and_next_game() {
        let mut history = History::new();
        assert!(history.observe(ready(1, 10)));
        let fault = Record { ready: false, fault: true, ..ready(1, 11) };
        assert!(history.observe(fault));
        let ended = Record { boundary_epoch: 2, frame: 0, owner: 255,
            active: false, thread_bound: false, ..fault };
        assert!(history.observe(ended));
        assert!(history.observe(Record { boundary_epoch: 3, ..ready(2, 1) }));
        assert_eq!(history.records.len(), 4);
        assert_eq!(history.records[1].record, fault);
        assert_eq!(history.records[2].record, ended);
        assert!(history.records[1].record.fault);
        assert!(!history.records[3].record.fault);
        let output = history.render(123);
        assert!(output.contains("S\t1\t1\t11\t11\t0\t1\t0\t1\t1\t1\n"));
        assert!(output.contains("S\t1\t2\t0\t0\t255\t0\t0\t0\t1\t1\n"));
    }

    #[test]
    fn first_live_game_is_distinct_from_process_start_and_later_session_ids() {
        let mut history = History::new();
        assert!(history.observe(Record { session: 0, boundary_epoch: 0, frame: 0,
            owner: 255, active: false, ready: false, thread_bound: false, fault: false }));
        assert!(history.observe(ready(1, 1)));
        assert!(history.observe(Record { session: 2, ..ready(1, 1) }));
        assert!(history.observe(Record { boundary_epoch: 2, ..ready(1, 1) }));
        assert_eq!(history.records.len(), 4);
        assert_eq!(history.records[0].record.session, 0);
        assert_eq!(history.records[1].record.session, 1);
        assert_eq!(history.records[2].record.session, 2);
        assert_eq!(history.records[3].record.session, 1);
        assert_eq!(history.records[3].record.boundary_epoch, 2);
    }

    #[test]
    fn every_non_frame_fact_starts_a_separate_transition() {
        let initial = ready(1, 10);
        let mut variants = [initial; 7];
        variants[0].session += 1;
        variants[1].boundary_epoch += 1;
        variants[2].owner += 1;
        variants[3].active = false;
        variants[4].ready = false;
        variants[5].thread_bound = false;
        variants[6].fault = true;
        for variant in variants {
            let mut history = History::new();
            assert!(history.observe(initial));
            assert!(history.observe(variant));
            assert_eq!(history.records.len(), 2);
            assert_eq!(history.records.back().unwrap().record, variant);
        }
    }

    #[test]
    fn frame_progress_coalesces_with_first_last_and_total_observations() {
        let mut history = History::new();
        for frame in 10..10_000 {
            assert!(history.observe(ready(1, frame)));
        }
        assert_eq!(history.records.len(), 1);
        let entry = history.records.back().unwrap();
        assert_eq!(entry.first_frame, 10);
        assert_eq!(entry.record.frame, 9_999);
        assert_eq!(entry.repeats, 9_990);
    }

    #[test]
    fn a_return_to_ready_after_waiting_is_a_new_transition() {
        let mut history = History::new();
        assert!(history.observe(ready(1, 10)));
        assert!(history.observe(Record { ready: false, ..ready(1, 11) }));
        assert!(history.observe(ready(1, 12)));
        assert_eq!(history.records.len(), 3);
        assert_eq!(history.records.front().unwrap().record.frame, 10);
        assert_eq!(history.records.back().unwrap().first_frame, 12);
    }

    #[test]
    fn observation_count_saturates_and_unchanged_render_reports_false() {
        let mut history = History::new();
        assert!(history.observe(ready(1, 10)));
        history.records.back_mut().unwrap().repeats = u32::MAX - 1;
        assert!(history.observe(ready(1, 10)));
        assert_eq!(history.records.back().unwrap().repeats, u32::MAX);
        assert!(!history.observe(ready(1, 10)));
        assert!(history.observe(ready(1, 11)));
        assert_eq!(history.records.back().unwrap().first_frame, 10);
        assert_eq!(history.records.back().unwrap().repeats, u32::MAX);
        assert!(!history.observe(ready(1, 11)));
    }

    #[test]
    fn only_the_latest_sixty_four_transitions_are_retained() {
        let mut history = History::new();
        for session in 0..100 {
            assert!(history.observe(ready(session, session as u32)));
            assert!(history.records.len() <= MAX_RECORDS);
        }
        assert_eq!(history.records.len(), MAX_RECORDS);
        assert_eq!(history.records.front().unwrap().record.session, 36);
        assert_eq!(history.records.back().unwrap().record.session, 99);
        assert_eq!(history.render(123).lines().count(), MAX_RECORDS + 1);
    }

    #[test]
    fn protocol_fields_are_numeric_and_render_fits_at_every_type_limit() {
        let mut history = History::new();
        for offset in 0..MAX_RECORDS {
            assert!(history.observe(Record {
                session: u64::MAX - offset as u64, boundary_epoch: u64::MAX,
                frame: u32::MAX, owner: u8::MAX, active: true, ready: true,
                thread_bound: true, fault: true,
            }));
            history.records.back_mut().unwrap().repeats = u32::MAX;
        }
        let output = history.render(u32::MAX);
        assert!(output.is_ascii());
        assert!(output.len() <= MAX_RENDER_BYTES);
        assert_eq!(output.lines().count(), MAX_RECORDS + 1);
        assert_eq!(output.lines().next().unwrap(), "SCGAMESESSION1\t4294967295\t64");
        for row in output.lines().skip(1) {
            let fields: Vec<_> = row.split('\t').collect();
            assert_eq!(fields.len(), 11);
            assert_eq!(fields[0], "S");
            assert!(fields[1..].iter().all(|field| !field.is_empty()
                && field.bytes().all(|byte| byte.is_ascii_digit())));
        }
    }

    #[test]
    fn rendering_uses_fixed_field_order_and_boolean_digits() {
        let mut history = History::new();
        let current = Record { session: 11, boundary_epoch: 22, frame: 33,
            owner: 4, active: true, ready: false, thread_bound: true, fault: false };
        assert!(history.observe(current));
        assert!(history.observe(Record { frame: 44, ..current }));
        assert_eq!(history.render(55),
            "SCGAMESESSION1\t55\t1\nS\t11\t22\t33\t44\t4\t1\t0\t1\t0\t2\n");
    }
}
