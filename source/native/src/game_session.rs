//! Pure game-session and native callback thread ownership policy.
//!
//! Every observation is supplied by the caller. No process, clock, memory,
//! callback-table, client heartbeat or UI readiness state is read here. The
//! runtime must serialize a fresh context read and the following operation
//! under the same lifecycle mutex. Every `enter` must have one matching `leave`,
//! including forwarding, nested and wrong-thread native calls.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Context {
    pub active: bool,
    pub frame: u32,
    pub owner: u8,
    pub identity: [usize; 3],
}

impl Context {
    fn verified(self) -> bool {
        !self.active || self.owner < 8
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Change {
    pub generation: u64,
    pub invalidated: bool,
    pub thread_released: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Decision {
    /// This is a known active session on its claimed UI thread. The caller
    /// separately checks callback nesting, current bindings and write policy.
    Active,
    /// Always invoke the exact original; do not capture or send owned commands.
    ForwardOnly,
    /// A different thread entered the same established active session.
    WrongThread,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Entry {
    pub change: Change,
    pub decision: Decision,
}

pub(crate) struct Tracker {
    last: Option<Context>,
    generation: u64,
    ui_thread: u32,
    active_callbacks: usize,
    reset_pending: bool,
    // A lost nesting count cannot establish that every original has returned.
    // This practically unreachable condition permanently forwards only.
    depth_overflow: bool,
}

impl Tracker {
    pub(crate) const fn new() -> Self {
        Self {
            last: None,
            generation: 0,
            ui_thread: 0,
            active_callbacks: 0,
            reset_pending: false,
            depth_overflow: false,
        }
    }

    pub(crate) fn generation(&self) -> u64 { self.generation }
    pub(crate) fn ui_thread(&self) -> u32 { self.ui_thread }
    pub(crate) fn active_callbacks(&self) -> usize { self.active_callbacks }

    /// Additional fence for an existing wrapper after its original returns.
    /// The runtime also verifies a fresh readable active context and the entry
    /// decision; a prior known context cannot authorize an unknown current one.
    pub(crate) fn control_allowed(&self, thread: u32) -> bool {
        self.last.is_some_and(|context| context.active && context.verified())
            && !self.reset_pending && !self.depth_overflow && self.generation != u64::MAX
            && thread != 0 && thread == self.ui_thread
    }

    fn change(&self, invalidated: bool, thread_released: bool) -> Change {
        Change { generation: self.generation, invalidated, thread_released }
    }

    fn release_if_idle(&mut self) -> bool {
        if !self.reset_pending || self.active_callbacks != 0 || self.depth_overflow {
            return false;
        }
        let released = self.ui_thread != 0;
        self.ui_thread = 0;
        self.reset_pending = false;
        released
    }

    /// Unknown context preserves the last known session. Verified game end
    /// invalidates immediately; the next active game advances its generation.
    /// Owner/core identity changes or frame rollback also start a generation.
    /// Ownership release waits until all original callback calls have returned.
    pub(crate) fn observe(&mut self, context: Option<Context>) -> Change {
        let Some(context) = context.filter(|context| context.verified()) else {
            return self.change(false, false);
        };
        let start = context.active && self.last.is_none_or(|last|
            !last.active || last.owner != context.owner || last.identity != context.identity
                || context.frame < last.frame);
        let ended = !context.active && self.last.is_some_and(|last| last.active);
        let invalidated = start || ended;
        if start {
            // Reserve the final value as a permanent forwarding fence rather
            // than wrap or reuse an authorization generation after exhaustion.
            self.generation = self.generation.saturating_add(1);
        }
        if invalidated { self.reset_pending = true; }
        self.last = Some(context);
        let thread_released = self.release_if_idle();
        self.change(invalidated, thread_released)
    }

    /// Observe against the callbacks already in flight, then count this call
    /// before returning. A first live callback can claim an idle new session;
    /// a boundary beneath another native call can only forward until it exits.
    pub(crate) fn enter(&mut self, context: Option<Context>, thread: u32) -> Entry {
        let change = self.observe(context);
        match self.active_callbacks.checked_add(1) {
            Some(count) => self.active_callbacks = count,
            None => { self.depth_overflow = true; self.reset_pending = true; },
        }
        let decision = if self.reset_pending || self.depth_overflow
            || self.generation == u64::MAX || thread == 0
            || context.is_none_or(|context| !context.active || !context.verified())
        {
            Decision::ForwardOnly
        } else if self.ui_thread == 0 {
            self.ui_thread = thread;
            Decision::Active
        } else if self.ui_thread == thread {
            Decision::Active
        } else {
            Decision::WrongThread
        };
        Entry { change, decision }
    }

    pub(crate) fn leave(&mut self) -> Change {
        if let Some(count) = self.active_callbacks.checked_sub(1) {
            self.active_callbacks = count;
        }
        let thread_released = self.release_if_idle();
        self.change(false, thread_released)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live(frame: u32) -> Context {
        Context { active: true, frame, owner: 0, identity: [0, 1, 1] }
    }
    fn menu() -> Context {
        Context { active: false, frame: 0, owner: 8, identity: [0, 0, 0] }
    }

    #[test]
    fn first_live_callback_claims_one_generation_and_menu_cannot_claim() {
        const EMPTY: Tracker = Tracker::new();
        let mut tracker = EMPTY;
        assert_eq!(tracker.generation(), 0);
        assert_eq!(tracker.enter(Some(menu()), 11).decision, Decision::ForwardOnly);
        assert_eq!(tracker.active_callbacks(), 1);
        assert_eq!(tracker.ui_thread(), 0);
        tracker.leave();
        let entry = tracker.enter(Some(live(1)), 11);
        assert_eq!(entry.decision, Decision::Active);
        assert_eq!(entry.change, Change { generation: 1, invalidated: true, thread_released: false });
        assert_eq!(tracker.ui_thread(), 11);
        tracker.leave();
        assert_eq!(tracker.observe(Some(live(2))), Change { generation: 1, invalidated: false, thread_released: false });
    }

    #[test]
    fn second_game_same_owner_reclaims_a_different_ui_thread() {
        let mut tracker = Tracker::new();
        assert_eq!(tracker.enter(Some(live(100)), 11).decision, Decision::Active);
        tracker.leave();
        assert_eq!(tracker.observe(Some(menu())), Change { generation: 1, invalidated: true, thread_released: true });
        assert_eq!(tracker.ui_thread(), 0);
        assert_eq!(tracker.observe(Some(menu())).generation, 1);
        let entry = tracker.enter(Some(live(1)), 22);
        assert_eq!(entry.change.generation, 2);
        assert!(entry.change.invalidated);
        assert_eq!(entry.decision, Decision::Active);
        assert_eq!(tracker.ui_thread(), 22);
        tracker.leave();
    }

    #[test]
    fn a_different_thread_within_one_live_session_is_rejected_and_counted() {
        let mut tracker = Tracker::new();
        tracker.enter(Some(live(1)), 11);
        let nested = tracker.enter(Some(live(2)), 22);
        assert_eq!(nested.decision, Decision::WrongThread);
        assert!(!nested.change.invalidated);
        assert_eq!(tracker.active_callbacks(), 2);
        tracker.leave(); tracker.leave();
        assert_eq!(tracker.ui_thread(), 11);
        assert_eq!(tracker.generation(), 1);
    }

    #[test]
    fn boundary_inside_original_defers_release_until_every_call_returns() {
        let mut tracker = Tracker::new();
        tracker.enter(Some(live(50)), 11);
        tracker.enter(Some(live(51)), 11);
        let end = tracker.observe(Some(menu()));
        assert!(end.invalidated); assert!(!end.thread_released);
        assert_eq!(tracker.ui_thread(), 11);
        // A new match/thread beneath the old original is a transition, not a
        // within-match wrong-thread fault. Its native call is still counted.
        let next = tracker.enter(Some(live(1)), 22);
        assert_eq!(next.decision, Decision::ForwardOnly);
        assert_eq!(next.change.generation, 2);
        assert_eq!(tracker.active_callbacks(), 3);
        assert!(!tracker.leave().thread_released);
        assert!(!tracker.leave().thread_released);
        assert!(tracker.leave().thread_released);
        assert_eq!(tracker.ui_thread(), 0);
        assert_eq!(tracker.enter(Some(live(2)), 22).decision, Decision::Active);
        tracker.leave();
    }

    #[test]
    fn owner_identity_and_frame_rollback_each_invalidate_the_session() {
        for changed in [
            Context { owner: 1, ..live(51) },
            Context { identity: [0, 1, 2], ..live(51) },
            live(49),
        ] {
            let mut tracker = Tracker::new();
            tracker.enter(Some(live(50)), 11); tracker.leave();
            let entry = tracker.enter(Some(changed), 22);
            assert_eq!(entry.change.generation, 2);
            assert!(entry.change.invalidated); assert!(entry.change.thread_released);
            assert_eq!(entry.decision, Decision::Active);
            tracker.leave();
        }
    }

    #[test]
    fn unknown_context_preserves_session_but_only_forwards_and_counts_calls() {
        let mut tracker = Tracker::new();
        tracker.enter(Some(live(50)), 11); tracker.leave();
        assert_eq!(tracker.observe(None), Change { generation: 1, invalidated: false, thread_released: false });
        assert_eq!(tracker.enter(None, 22).decision, Decision::ForwardOnly);
        assert_eq!(tracker.active_callbacks(), 1);
        assert_eq!(tracker.ui_thread(), 11);
        assert!(!tracker.leave().thread_released);
        assert_eq!(tracker.enter(Some(live(51)), 11).decision, Decision::Active);
        assert_eq!(tracker.generation(), 1); tracker.leave();
        // An unverified active owner is also unknown, never a claim or reset.
        assert_eq!(tracker.enter(Some(Context { owner: 255, ..live(52) }), 22).decision, Decision::ForwardOnly);
        tracker.leave(); assert_eq!(tracker.generation(), 1);
    }

    #[test]
    fn unknown_native_call_also_delays_boundary_release() {
        let mut tracker = Tracker::new();
        tracker.enter(Some(live(50)), 11); tracker.leave();
        tracker.enter(None, 0);
        assert!(!tracker.observe(Some(menu())).thread_released);
        assert_eq!(tracker.ui_thread(), 11);
        assert!(tracker.leave().thread_released);
    }

    #[test]
    fn control_fence_closes_under_an_old_original_until_new_game_claims() {
        let mut tracker = Tracker::new();
        tracker.enter(Some(live(50)), 11);
        assert!(tracker.control_allowed(11));
        assert!(!tracker.control_allowed(0)); assert!(!tracker.control_allowed(22));
        tracker.observe(Some(menu()));
        assert!(!tracker.control_allowed(11));
        tracker.observe(Some(live(1)));
        assert!(!tracker.control_allowed(11)); assert!(!tracker.control_allowed(22));
        assert!(tracker.leave().thread_released);
        assert!(!tracker.control_allowed(11)); assert!(!tracker.control_allowed(22));
        tracker.enter(Some(live(2)), 22);
        assert!(tracker.control_allowed(22)); assert!(!tracker.control_allowed(11));
        tracker.leave();
        assert!(tracker.control_allowed(22));
        // The last known active session remains available, but an unknown
        // current entry is ForwardOnly and its separate runtime read must fail.
        assert_eq!(tracker.enter(None, 22).decision, Decision::ForwardOnly);
        tracker.leave();
    }

    #[test]
    fn zero_thread_cannot_claim_and_balanced_leave_cannot_underflow() {
        let mut tracker = Tracker::new();
        assert_eq!(tracker.enter(Some(live(1)), 0).decision, Decision::ForwardOnly);
        assert_eq!(tracker.ui_thread(), 0); tracker.leave(); tracker.leave();
        assert_eq!(tracker.active_callbacks(), 0);
        assert_eq!(tracker.enter(Some(live(2)), 11).decision, Decision::Active);
        tracker.leave();
    }

    #[test]
    fn readiness_and_heartbeat_flaps_do_not_start_a_game_session() {
        let mut tracker = Tracker::new();
        tracker.enter(Some(live(100)), 11); tracker.leave();
        // External connection readiness is deliberately absent from Context.
        // Both unavailable and restored states submit the same engine context.
        for _ in 0..1000 {
            let change = tracker.observe(Some(live(100)));
            assert!(!change.invalidated); assert_eq!(change.generation, 1);
            assert_eq!(tracker.ui_thread(), 11);
        }
    }

    #[test]
    fn same_callback_table_restoration_is_independent_of_game_reclamation() {
        let mut tracker = Tracker::new();
        tracker.enter(Some(live(500)), 11); tracker.leave();
        // Exact original/own callback table restoration is checked separately
        // by callback_startup. It must not hide this actual match boundary.
        tracker.observe(Some(menu()));
        tracker.observe(Some(live(1)));
        assert_eq!(tracker.generation(), 2);
        assert_eq!(tracker.enter(Some(live(2)), 22).decision, Decision::Active);
        tracker.leave();
    }

    #[test]
    fn state_is_bounded_and_frame_advance_does_not_accumulate_history() {
        assert!(std::mem::size_of::<Tracker>() <= 128);
        let mut tracker = Tracker::new();
        for frame in 1..10_000 {
            assert_eq!(tracker.enter(Some(live(frame)), 11).decision, Decision::Active);
            tracker.leave();
        }
        assert_eq!(tracker.generation(), 1);
        assert_eq!(tracker.active_callbacks(), 0);
    }

    #[test]
    fn generation_and_callback_count_exhaustion_never_wrap_authorization() {
        let mut tracker = Tracker::new();
        tracker.generation = u64::MAX - 1;
        assert_eq!(tracker.enter(Some(live(1)), 11).decision, Decision::ForwardOnly);
        assert_eq!(tracker.generation(), u64::MAX); tracker.leave();
        tracker.observe(Some(menu()));
        assert_eq!(tracker.enter(Some(live(1)), 22).decision, Decision::ForwardOnly);
        assert_eq!(tracker.generation(), u64::MAX); tracker.leave();
        let mut tracker = Tracker::new();
        tracker.enter(Some(live(1)), 11);
        tracker.active_callbacks = usize::MAX;
        assert_eq!(tracker.enter(Some(live(2)), 11).decision, Decision::ForwardOnly);
        assert_eq!(tracker.active_callbacks(), usize::MAX);
        assert!(!tracker.observe(Some(menu())).thread_released);
        tracker.active_callbacks = 0;
        assert!(!tracker.leave().thread_released);
        assert_eq!(tracker.enter(Some(live(1)), 22).decision, Decision::ForwardOnly);
    }
}
