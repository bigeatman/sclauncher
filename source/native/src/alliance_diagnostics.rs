//! Bounded, owned diagnostics for diplomacy state transitions.
//!
//! Only fixed phase codes and numeric facts are stored. This module has no
//! process, file, memory-reader or UI access. A caller may publish render()
//! through the launcher's existing owned-log writer.

use std::collections::VecDeque;
use std::fmt::Write as _;

pub(crate) const MAX_RECORDS: usize = 64;
/// Numeric field widths and the fixed phase codes keep every rendering below
/// this limit, including the largest possible values of the field types.
pub(crate) const MAX_RENDER_BYTES: usize = 8192;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Phase {
    Waiting,
    CoreUnavailable,
    PlayersUnavailable,
    StateChanged,
    DialogUnavailable,
    DialogClosed,
    NoComputers,
    LayoutUnavailable,
    PolicyUnavailable,
    PolicyBlocked,
    Ready,
}

impl Phase {
    pub(crate) const fn as_code(self) -> &'static str {
        match self {
            Self::Waiting => "WAITING",
            Self::CoreUnavailable => "CORE_UNAVAILABLE",
            Self::PlayersUnavailable => "PLAYERS_UNAVAILABLE",
            Self::StateChanged => "STATE_CHANGED",
            Self::DialogUnavailable => "DIALOG_UNAVAILABLE",
            Self::DialogClosed => "DIALOG_CLOSED",
            Self::NoComputers => "NO_COMPUTERS",
            Self::LayoutUnavailable => "LAYOUT_UNAVAILABLE",
            Self::PolicyUnavailable => "POLICY_UNAVAILABLE",
            Self::PolicyBlocked => "POLICY_BLOCKED",
            Self::Ready => "READY",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Record {
    pub(crate) phase: Phase,
    pub(crate) session: u64,
    /// The first frame at which this transition was observed. Advancing frames
    /// alone neither append nor overwrite a stored transition.
    pub(crate) frame: u32,
    /// Playable local-player slot, or 255 while ownership is unavailable.
    pub(crate) owner: u8,
    pub(crate) other_humans: u8,
    pub(crate) computers: u8,
    pub(crate) policy_available: bool,
    /// 0 waiting, 1 unavailable, 2 unchanged, 3 shown, 4 hidden, 5 refused.
    pub(crate) button_state: u8,
}

impl Record {
    fn same_facts(self, other: Self) -> bool {
        self.phase == other.phase
            && self.session == other.session
            && self.owner == other.owner
            && self.other_humans == other.other_humans
            && self.computers == other.computers
            && self.policy_available == other.policy_available
            && self.button_state == other.button_state
    }
}

#[derive(Debug)]
pub(crate) struct History {
    records: VecDeque<Record>,
}

impl History {
    pub(crate) const fn new() -> Self {
        Self { records: VecDeque::new() }
    }

    /// Returns true only when the retained history changes. Coalescing is
    /// consecutive: returning to a prior state is a new transition.
    pub(crate) fn observe(&mut self, record: Record) -> bool {
        if self.records.back().is_some_and(|previous| previous.same_facts(record)) {
            return false;
        }
        if self.records.len() == MAX_RECORDS {
            self.records.pop_front();
        }
        self.records.push_back(record);
        true
    }

    /// SCALLYDIAG1 header: marker, PID, retained transition count.
    /// D rows: phase, session, frame, owner, other humans, computers,
    /// policy available (0/1), button state. All fields are bounded ASCII.
    pub(crate) fn render(&self, pid: u32) -> String {
        let mut output = String::with_capacity(64 + self.records.len() * 96);
        let _ = writeln!(output, "SCALLYDIAG1\t{pid}\t{}", self.records.len());
        for record in &self.records {
            let _ = writeln!(output, "D\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                record.phase.as_code(), record.session, record.frame, record.owner,
                record.other_humans, record.computers,
                u8::from(record.policy_available), record.button_state);
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(phase: Phase, frame: u32) -> Record {
        Record {
            phase, session: 3, frame, owner: 0, other_humans: 0,
            computers: 2, policy_available: true, button_state: 2,
        }
    }

    #[test]
    fn empty_history_has_a_bounded_protocol_header() {
        assert_eq!(History::new().render(123), "SCALLYDIAG1\t123\t0\n");
    }

    #[test]
    fn game_end_preserves_previous_game_transitions() {
        let mut history = History::new();
        assert!(history.observe(record(Phase::PlayersUnavailable, 10)));
        assert!(history.observe(record(Phase::Ready, 20)));
        let mut ended = record(Phase::Waiting, 0);
        ended.owner = 255;
        ended.computers = 0;
        ended.policy_available = false;
        ended.button_state = 0;
        assert!(history.observe(ended));
        assert_eq!(history.records.len(), 3);
        let rendered = history.render(123);
        assert!(rendered.contains("D\tPLAYERS_UNAVAILABLE\t3\t10\t"));
        assert!(rendered.contains("D\tREADY\t3\t20\t"));
        assert!(rendered.contains("D\tWAITING\t3\t0\t255\t0\t0\t0\t0\n"));
    }

    #[test]
    fn advancing_frames_do_not_spam_or_replace_transition_frame() {
        let mut history = History::new();
        assert!(history.observe(record(Phase::Ready, 10)));
        for frame in 11..10_000 {
            assert!(!history.observe(record(Phase::Ready, frame)));
        }
        assert_eq!(history.records.len(), 1);
        assert_eq!(history.records.front().unwrap().frame, 10);
    }

    #[test]
    fn each_non_frame_fact_can_trigger_a_transition() {
        let initial = record(Phase::Ready, 10);
        let mut variants = [initial; 7];
        variants[0].phase = Phase::DialogClosed;
        variants[1].session += 1;
        variants[2].owner += 1;
        variants[3].other_humans += 1;
        variants[4].computers += 1;
        variants[5].policy_available = false;
        variants[6].button_state = 3;
        for changed in variants {
            let mut history = History::new();
            assert!(history.observe(initial));
            assert!(history.observe(changed));
            assert_eq!(history.records.len(), 2);
        }
    }

    #[test]
    fn repeated_state_after_another_state_is_retained() {
        let mut history = History::new();
        assert!(history.observe(record(Phase::Ready, 10)));
        assert!(history.observe(record(Phase::DialogClosed, 11)));
        assert!(history.observe(record(Phase::Ready, 12)));
        assert_eq!(history.records.len(), 3);
    }

    #[test]
    fn only_the_latest_sixty_four_distinct_transitions_are_retained() {
        let mut history = History::new();
        for session in 0..100 {
            let mut current = record(Phase::Ready, session as u32);
            current.session = session;
            assert!(history.observe(current));
            assert!(history.records.len() <= MAX_RECORDS);
        }
        assert_eq!(history.records.len(), MAX_RECORDS);
        assert_eq!(history.records.front().unwrap().session, 36);
        assert_eq!(history.records.back().unwrap().session, 99);
        let rendered = history.render(123);
        assert_eq!(rendered.lines().count(), MAX_RECORDS + 1);
        assert!(rendered.starts_with("SCALLYDIAG1\t123\t64\n"));
    }

    #[test]
    fn all_phase_codes_are_fixed_unique_ascii_tokens() {
        let phases = [Phase::Waiting, Phase::CoreUnavailable, Phase::PlayersUnavailable,
            Phase::StateChanged, Phase::DialogUnavailable, Phase::DialogClosed,
            Phase::NoComputers, Phase::LayoutUnavailable, Phase::PolicyUnavailable,
            Phase::PolicyBlocked, Phase::Ready];
        for (index, phase) in phases.iter().enumerate() {
            let code = phase.as_code();
            assert!(!code.is_empty());
            assert!(code.bytes().all(|byte| byte.is_ascii_uppercase() || byte == b'_'));
            assert!(phases[..index].iter().all(|previous| previous.as_code() != code));
        }
    }

    #[test]
    fn render_is_bounded_even_at_numeric_type_limits() {
        let mut history = History::new();
        for offset in 0..MAX_RECORDS {
            assert!(history.observe(Record {
                phase: Phase::PlayersUnavailable,
                session: u64::MAX - offset as u64,
                frame: u32::MAX, owner: u8::MAX, other_humans: u8::MAX,
                computers: u8::MAX, policy_available: true, button_state: u8::MAX,
            }));
        }
        let rendered = history.render(u32::MAX);
        assert!(rendered.len() < MAX_RENDER_BYTES);
        assert!(rendered.is_ascii());
        assert!(rendered.lines().skip(1).all(|line| line.split('\t').count() == 9));
    }
}