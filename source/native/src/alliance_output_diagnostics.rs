//! Bounded numeric history for guarded native alliance output decisions.
//!
//! Callers supply all facts. This module has no game, process, file, memory,
//! clock or UI access, and retains no names, pointers, code or packet bytes.

use std::collections::VecDeque;
use std::fmt::Write as _;

pub const MAX_RECORDS: usize = 32;
/// Fixed decimal field widths keep every rendering below this byte limit.
pub const MAX_RENDER_BYTES: usize = 8192;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Record {
    pub request_id: u64,
    pub session: u64,
    pub generation: u64,
    /// 1: root native callback; 2: outer native callback.
    pub source: u8,
    pub before_frame: u32,
    pub after_frame: u32,
    /// Fixed reason bits supplied by the output guard; zero means accepted.
    pub mask: u32,
    pub nested: bool,
    pub reentry_changed: bool,
    pub binding_changed: bool,
}

impl Record {
    fn same_key(self, other: Self) -> bool {
        self.request_id == other.request_id
            && self.session == other.session
            && self.generation == other.generation
            && self.source == other.source
            && self.mask == other.mask
            && self.nested == other.nested
            && self.reentry_changed == other.reentry_changed
            && self.binding_changed == other.binding_changed
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Entry {
    record: Record,
    /// Total observations of this retained combination, including the first.
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

    /// Coalesces equal request/session/generation/source/reason/flag facts.
    /// Updates both frames to the latest observation and moves that combination
    /// to the back. Returns true exactly when the rendered history changes.
    pub fn observe(&mut self, record: Record) -> bool {
        if let Some(index) = self.records.iter().position(|entry| entry.record.same_key(record)) {
            let was_latest = index + 1 == self.records.len();
            let mut entry = self.records.remove(index).expect("located retained output record");
            let changed = !was_latest
                || entry.record.before_frame != record.before_frame
                || entry.record.after_frame != record.after_frame
                || entry.repeats != u32::MAX;
            entry.record = record;
            entry.repeats = entry.repeats.saturating_add(1);
            self.records.push_back(entry);
            return changed;
        }
        if self.records.len() == MAX_RECORDS {
            self.records.pop_front();
        }
        self.records.push_back(Entry { record, repeats: 1 });
        true
    }

    /// SCALLYOUTPUT1 header: marker, PID, retained combination count.
    /// O rows: request ID, session, generation, source, before frame, after
    /// frame, reason mask, nested (0/1), reentry changed (0/1), binding changed
    /// (0/1), repeats (total observations, saturating at u32::MAX).
    /// The fixed protocol and numeric fields are ASCII and at most 8192 bytes.
    pub fn render(&self, pid: u32) -> String {
        let mut output = String::with_capacity(64 + self.records.len() * 128);
        let _ = writeln!(output, "SCALLYOUTPUT1\t{pid}\t{}", self.records.len());
        for entry in &self.records {
            let record = entry.record;
            let _ = writeln!(output, "O\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                record.request_id, record.session, record.generation, record.source,
                record.before_frame, record.after_frame, record.mask,
                u8::from(record.nested), u8::from(record.reentry_changed),
                u8::from(record.binding_changed), entry.repeats);
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(request_id: u64) -> Record {
        Record { request_id, session: 3, generation: 7, source: 1,
            before_frame: 10, after_frame: 12, mask: 0,
            nested: false, reentry_changed: false, binding_changed: false }
    }

    #[test]
    fn empty_history_has_the_fixed_protocol_header() {
        const EMPTY: History = History::new();
        assert_eq!(EMPTY.render(123), "SCALLYOUTPUT1\t123\t0\n");
    }

    #[test]
    fn repeats_coalesce_and_keep_the_latest_frame_pair() {
        let mut history = History::new();
        let mut current = record(11);
        assert!(history.observe(current));
        for frame in 13..100 {
            current.before_frame = frame;
            current.after_frame = frame + 2;
            assert!(history.observe(current));
        }
        assert_eq!(history.records.len(), 1);
        let entry = history.records.back().unwrap();
        assert_eq!(entry.record.before_frame, 99);
        assert_eq!(entry.record.after_frame, 101);
        assert_eq!(entry.repeats, 88);
        // The supplied pair is retained even when its numbers decrease.
        current.before_frame = 4;
        current.after_frame = 2;
        assert!(history.observe(current));
        assert_eq!(history.records.back().unwrap().record, current);
        assert_eq!(history.records.back().unwrap().repeats, 89);
    }

    #[test]
    fn every_non_frame_field_is_part_of_the_coalescing_key() {
        let initial = record(11);
        let mut variants = [initial; 8];
        variants[0].request_id += 1;
        variants[1].session += 1;
        variants[2].generation += 1;
        variants[3].source = 2;
        variants[4].mask = 1;
        variants[5].nested = true;
        variants[6].reentry_changed = true;
        variants[7].binding_changed = true;
        for changed in variants {
            let mut history = History::new();
            assert!(history.observe(initial));
            assert!(history.observe(changed));
            assert_eq!(history.records.len(), 2);
            assert_eq!(history.records.front().unwrap().record, initial);
            assert_eq!(history.records.back().unwrap().record, changed);
        }
    }

    #[test]
    fn a_repeated_combination_moves_to_the_back_without_duplicating_it() {
        let mut history = History::new();
        assert!(history.observe(record(1)));
        assert!(history.observe(record(2)));
        assert!(history.observe(record(3)));
        assert!(history.observe(record(1)));
        let retained: Vec<_> = history.records.iter().map(|entry| entry.record.request_id).collect();
        assert_eq!(retained, [2, 3, 1]);
        assert_eq!(history.records.back().unwrap().repeats, 2);
        assert_eq!(history.records.len(), 3);
    }

    #[test]
    fn repeats_saturate_and_only_actual_changes_report_an_update() {
        let mut history = History::new();
        let mut current = record(1);
        assert!(history.observe(current));
        history.records.back_mut().unwrap().repeats = u32::MAX - 1;
        assert!(history.observe(current));
        assert_eq!(history.records.back().unwrap().repeats, u32::MAX);
        assert!(!history.observe(current));
        current.before_frame += 1;
        assert!(history.observe(current));
        current.after_frame += 1;
        assert!(history.observe(current));
        assert_eq!(history.records.back().unwrap().repeats, u32::MAX);
        assert!(history.observe(record(2)));
        assert!(history.observe(current));
        assert_eq!(history.records.back().unwrap().record, current);
        assert_eq!(history.records.back().unwrap().repeats, u32::MAX);
        assert!(!history.observe(current));
    }

    #[test]
    fn capacity_evicts_the_least_recently_observed_combination() {
        let mut history = History::new();
        for request_id in 0..MAX_RECORDS as u64 {
            assert!(history.observe(record(request_id)));
            assert!(history.records.len() <= MAX_RECORDS);
        }
        assert!(history.observe(record(0)));
        assert_eq!(history.records.len(), MAX_RECORDS);
        assert!(history.observe(record(MAX_RECORDS as u64)));
        assert_eq!(history.records.len(), MAX_RECORDS);
        assert_eq!(history.records.front().unwrap().record.request_id, 2);
        assert!(history.records.iter().any(|entry| entry.record.request_id == 0));
        assert!(!history.records.iter().any(|entry| entry.record.request_id == 1));
        for request_id in 33..100 {
            assert!(history.observe(record(request_id)));
            assert!(history.records.len() <= MAX_RECORDS);
        }
        assert_eq!(history.records.front().unwrap().record.request_id, 68);
        assert_eq!(history.records.back().unwrap().record.request_id, 99);
    }

    #[test]
    fn rendering_preserves_all_numeric_fields_in_fixed_order() {
        let mut history = History::new();
        let current = Record { request_id: 11, session: 22, generation: 33, source: 2,
            before_frame: 44, after_frame: 55, mask: 66,
            nested: true, reentry_changed: false, binding_changed: true };
        assert!(history.observe(current));
        assert!(history.observe(current));
        assert_eq!(history.render(77),
            "SCALLYOUTPUT1\t77\t1\nO\t11\t22\t33\t2\t44\t55\t66\t1\t0\t1\t2\n");
    }

    #[test]
    fn maximum_numeric_fields_and_repeats_fit_the_ascii_byte_bound() {
        let mut history = History::new();
        for offset in 0..MAX_RECORDS {
            assert!(history.observe(Record {
                request_id: u64::MAX - offset as u64, session: u64::MAX,
                generation: u64::MAX, source: u8::MAX,
                before_frame: u32::MAX, after_frame: u32::MAX, mask: u32::MAX,
                nested: true, reentry_changed: true, binding_changed: true,
            }));
            history.records.back_mut().unwrap().repeats = u32::MAX;
        }
        let output = history.render(u32::MAX);
        assert!(output.is_ascii());
        assert!(output.len() <= MAX_RENDER_BYTES);
        assert_eq!(output.lines().count(), MAX_RECORDS + 1);
        assert_eq!(output.lines().next().unwrap(), "SCALLYOUTPUT1\t4294967295\t32");
        for row in output.lines().skip(1) {
            let fields: Vec<_> = row.split('\t').collect();
            assert_eq!(fields.len(), 12);
            assert_eq!(fields[0], "O");
            assert!(fields[1..].iter().all(|field| !field.is_empty()
                && field.bytes().all(|byte| byte.is_ascii_digit())));
        }
    }
}
