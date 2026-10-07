//! Pure lifetime checks for one native alliance output transaction.
//!
//! This module neither accesses a game nor reads process/window/clock state.
//! The caller supplies every observation. Ordinary descendant UI callbacks and
//! unit-control binding generations are deliberately absent from this model.
//! A separate transaction interference counter represents actual unauthorized
//! appends/cross-thread activity; normal same-thread native delivery is allowed.
//!
//! A zero result establishes only these context checks. The caller separately
//! proves native packet identity, complete immutable outgoing-buffer prefix,
//! buffer/capacity equality, current player roster/core and authorization scope
//! before replacing any already queued bytes.

pub(crate) const MAX_CAPTURE_MS: u64 = 1000;
pub(crate) const REJECT_THREAD: u32 = 1 << 0;
pub(crate) const REJECT_GAME: u32 = 1 << 1;
pub(crate) const REJECT_OWNER: u32 = 1 << 2;
pub(crate) const REJECT_FRAME: u32 = 1 << 3;
pub(crate) const REJECT_IDENTITY: u32 = 1 << 4;
pub(crate) const REJECT_CLOCK: u32 = 1 << 5;
pub(crate) const REJECT_TIMEOUT: u32 = 1 << 6;
pub(crate) const REJECT_INTERFERENCE: u32 = 1 << 7;
pub(crate) const REJECT_INSTALLED: u32 = 1 << 8;
pub(crate) const REJECT_FAULT: u32 = 1 << 9;
pub(crate) const REJECT_FOREGROUND: u32 = 1 << 10;
pub(crate) const REJECT_HEARTBEAT: u32 = 1 << 11;
pub(crate) const REJECT_IN_ORIGINAL: u32 = 1 << 12;
pub(crate) const REJECT_PAUSED: u32 = 1 << 13;
pub(crate) const REJECT_POLICY: u32 = 1 << 14;
pub(crate) const REJECT_STOP: u32 = 1 << 15;
pub(crate) const ALL_REJECTION_BITS: u32 = (1 << 16) - 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Authorization {
    pub ui_thread: u32,
    pub before: (bool, u32, u8),
    pub identity: [usize; 3],
    pub start_tick: u64,
    pub interference: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Observation {
    pub ui_thread: u32,
    pub context: Option<(bool, u32, u8)>,
    pub identity: Option<[usize; 3]>,
    pub now_tick: u64,
    pub interference: u64,
    pub installed: bool,
    pub fault: bool,
    pub foreground: bool,
    pub heartbeat: bool,
    pub in_original: bool,
    pub paused_clear: bool,
    pub policy: bool,
    pub stop_clear: bool,
}

/// Fixed independent reason bits; zero means this context is still authorized.
/// Frame advance is normal, but rollback, changed owner/session, more than one
/// second elapsed, or any loss of the caller-supplied guards rejects the output.
/// Unit CALLBACK_NESTED/BINDING_EPOCH are intentionally not inputs.
pub(crate) fn rejection_mask(auth: &Authorization, observed: &Observation) -> u32 {
    let mut mask = 0;
    if auth.ui_thread == 0 || observed.ui_thread != auth.ui_thread { mask |= REJECT_THREAD; }
    if !auth.before.0 { mask |= REJECT_GAME; }
    if auth.before.2 >= 8 { mask |= REJECT_OWNER; }
    match observed.context {
        Some((live, frame, owner)) => {
            if !live { mask |= REJECT_GAME; }
            if owner >= 8 || owner != auth.before.2 { mask |= REJECT_OWNER; }
            if frame < auth.before.1 { mask |= REJECT_FRAME; }
        }
        None => mask |= REJECT_GAME,
    }
    if observed.identity != Some(auth.identity) { mask |= REJECT_IDENTITY; }
    match observed.now_tick.checked_sub(auth.start_tick) {
        None => mask |= REJECT_CLOCK,
        Some(elapsed) if elapsed > MAX_CAPTURE_MS => mask |= REJECT_TIMEOUT,
        Some(_) => (),
    }
    if observed.interference != auth.interference { mask |= REJECT_INTERFERENCE; }
    if !observed.installed { mask |= REJECT_INSTALLED; }
    if observed.fault { mask |= REJECT_FAULT; }
    if !observed.foreground { mask |= REJECT_FOREGROUND; }
    if !observed.heartbeat { mask |= REJECT_HEARTBEAT; }
    if observed.in_original { mask |= REJECT_IN_ORIGINAL; }
    if !observed.paused_clear { mask |= REJECT_PAUSED; }
    if !observed.policy { mask |= REJECT_POLICY; }
    if !observed.stop_clear { mask |= REJECT_STOP; }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authorization() -> Authorization {
        Authorization { ui_thread: 17, before: (true, 50, 0), identity: [0x10000, 7, 0],
            start_tick: 1000, interference: 9 }
    }
    fn observation() -> Observation {
        Observation { ui_thread: 17, context: Some((true, 50, 0)), identity: Some([0x10000, 7, 0]),
            now_tick: 1250, interference: 9, installed: true, fault: false,
            foreground: true, heartbeat: true, in_original: false,
            paused_clear: true, policy: true, stop_clear: true }
    }

    #[test]
    fn native_descendant_delivery_and_normal_frame_advance_keep_same_transaction() {
        let auth = authorization();
        // Root/child/periodic delivery within this one UI transaction may advance
        // the frame. It does not represent an unauthorized append/interference.
        for frame in [50, 51, 52, 60, u32::MAX] {
            let mut observed = observation(); observed.context = Some((true, frame, 0));
            assert_eq!(rejection_mask(&auth, &observed), 0);
        }
        assert_eq!(rejection_mask(&auth, &observation()), 0);
    }

    #[test]
    fn owner_frame_game_and_missing_context_fail_independently() {
        let auth = authorization();
        for (context, expected) in [
            (Some((false, 50, 0)), REJECT_GAME),
            (Some((true, 49, 0)), REJECT_FRAME),
            (Some((true, 50, 1)), REJECT_OWNER),
            (Some((true, 50, 8)), REJECT_OWNER),
            (Some((true, 50, 255)), REJECT_OWNER),
            (None, REJECT_GAME),
        ] {
            let mut observed = observation(); observed.context = context;
            assert_eq!(rejection_mask(&auth, &observed), expected);
        }
        let mut invalid = auth; invalid.before.0 = false;
        assert_eq!(rejection_mask(&invalid, &observation()), REJECT_GAME);
        invalid = auth; invalid.before.2 = 8;
        assert_eq!(rejection_mask(&invalid, &observation()), REJECT_OWNER);
    }

    #[test]
    fn identity_session_and_ui_thread_changes_reject_without_unit_capture_state() {
        let auth = authorization();
        for field in 0..3 {
            let mut observed = observation(); let mut identity = auth.identity;
            identity[field] += 1; observed.identity = Some(identity);
            assert_eq!(rejection_mask(&auth, &observed), REJECT_IDENTITY);
        }
        let mut observed = observation(); observed.identity = None;
        assert_eq!(rejection_mask(&auth, &observed), REJECT_IDENTITY);
        for thread in [0, 18, u32::MAX] {
            observed = observation(); observed.ui_thread = thread;
            assert_eq!(rejection_mask(&auth, &observed), REJECT_THREAD);
        }
        let mut invalid = auth; invalid.ui_thread = 0;
        observed = observation(); observed.ui_thread = 0;
        assert_eq!(rejection_mask(&invalid, &observed), REJECT_THREAD);
    }

    #[test]
    fn time_window_is_inclusive_monotonic_and_handles_overflow_without_saturation() {
        let auth = authorization();
        for elapsed in [0, 1, MAX_CAPTURE_MS] {
            let mut observed = observation(); observed.now_tick = auth.start_tick + elapsed;
            assert_eq!(rejection_mask(&auth, &observed), 0);
        }
        let mut observed = observation(); observed.now_tick = auth.start_tick + MAX_CAPTURE_MS + 1;
        assert_eq!(rejection_mask(&auth, &observed), REJECT_TIMEOUT);
        observed.now_tick = auth.start_tick - 1;
        assert_eq!(rejection_mask(&auth, &observed), REJECT_CLOCK);
        let mut high = auth; high.start_tick = u64::MAX - 100;
        observed.now_tick = u64::MAX;
        assert_eq!(rejection_mask(&high, &observed), 0);
        observed.now_tick = 0;
        assert_eq!(rejection_mask(&high, &observed), REJECT_CLOCK);
    }

    #[test]
    fn every_operational_guard_has_one_fixed_distinct_reason_bit() {
        let auth = authorization();
        let mut failures = Vec::new();
        let mut o = observation(); o.interference += 1; failures.push((o, REJECT_INTERFERENCE));
        let mut o = observation(); o.installed = false; failures.push((o, REJECT_INSTALLED));
        let mut o = observation(); o.fault = true; failures.push((o, REJECT_FAULT));
        let mut o = observation(); o.foreground = false; failures.push((o, REJECT_FOREGROUND));
        let mut o = observation(); o.heartbeat = false; failures.push((o, REJECT_HEARTBEAT));
        let mut o = observation(); o.in_original = true; failures.push((o, REJECT_IN_ORIGINAL));
        let mut o = observation(); o.paused_clear = false; failures.push((o, REJECT_PAUSED));
        let mut o = observation(); o.policy = false; failures.push((o, REJECT_POLICY));
        let mut o = observation(); o.stop_clear = false; failures.push((o, REJECT_STOP));
        for (observed, expected) in failures {
            assert!(expected.is_power_of_two());
            assert_eq!(rejection_mask(&auth, &observed), expected);
        }
    }

    #[test]
    fn simultaneous_rejections_retain_all_reasons_and_do_not_alias_known_bits() {
        let auth = authorization();
        let mut observed = Observation { ui_thread: 18, context: Some((false, 49, 1)), identity: None,
            now_tick: 2001, interference: 10, installed: false, fault: true,
            foreground: false, heartbeat: false, in_original: true,
            paused_clear: false, policy: false, stop_clear: false };
        assert_eq!(rejection_mask(&auth, &observed), ALL_REJECTION_BITS & !REJECT_CLOCK);
        observed.now_tick = 999;
        assert_eq!(rejection_mask(&auth, &observed), ALL_REJECTION_BITS & !REJECT_TIMEOUT);
        assert_eq!(ALL_REJECTION_BITS, 0xffff);
    }
}