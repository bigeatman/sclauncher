//! Pure startup/reconnection policy. These decisions never access a game process.
use crate::callback_binding::{BindingError, Slot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObservedSlot {
    pub slot: Slot,
    pub actual: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Ready,
    Waiting {
        initialized: usize,
        pending_mask: u8,
    },
    Foreign {
        index: usize,
        actual: usize,
    },
}
pub fn classify(observed: &[ObservedSlot; 4]) -> Gate {
    let mut initialized = 0;
    let mut pending_mask = 0;
    for (index, item) in observed.iter().enumerate() {
        if item.actual == 0 {
            pending_mask |= 1 << index;
        } else if item.actual == item.slot.original || item.actual == item.slot.replacement {
            initialized += 1;
        } else {
            return Gate::Foreign {
                index,
                actual: item.actual,
            };
        }
    }
    if pending_mask == 0 {
        Gate::Ready
    } else {
        Gate::Waiting {
            initialized,
            pending_mask,
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum Attempt {
    Ready {
        changed: Vec<Slot>,
    },
    Waiting {
        initialized: usize,
        pending_mask: u8,
    },
    WaitingRace {
        address: usize,
    },
}
/// The installer is not invoked unless the complete observation is ready. A
/// strict original-CAS race to null remains a retry, never a null-to-own write.
pub fn attempt(
    observed: &[ObservedSlot; 4],
    install: impl FnOnce() -> Result<Vec<Slot>, BindingError>,
) -> Result<Attempt, BindingError> {
    match classify(observed) {
        Gate::Foreign { index, actual } => Err(BindingError::Mismatch {
            address: observed[index].slot.address,
            expected: observed[index].slot.original,
            actual,
        }),
        Gate::Waiting {
            initialized,
            pending_mask,
        } => Ok(Attempt::Waiting {
            initialized,
            pending_mask,
        }),
        Gate::Ready => match install() {
            Ok(changed) => Ok(Attempt::Ready { changed }),
            Err(BindingError::Mismatch {
                address, actual: 0, ..
            }) => Ok(Attempt::WaitingRace { address }),
            Err(error) => Err(error),
        },
    }
}
#[derive(Default)]
pub struct WaitPolicy {
    active: Option<(u64, u32, u8)>,
}
impl WaitPolicy {
    /// Menus/lobbies/frame-zero never consume the live-game timeout. Owner,
    /// frame reset or backwards time starts a new live-game waiting interval.
    pub fn expired(
        &mut self,
        now_ms: u64,
        pending: bool,
        in_game: bool,
        frame: u32,
        owner: u8,
    ) -> bool {
        if !pending || !in_game || frame == 0 || owner >= 8 {
            self.active = None;
            return false;
        }
        let (start, last_frame, previous_owner) = self.active.unwrap_or((now_ms, frame, owner));
        let start = if owner != previous_owner || frame < last_frame || now_ms < start {
            now_ms
        } else {
            start
        };
        self.active = Some((start, frame, owner));
        now_ms - start >= 10_000
    }
    /// The 10-second deadline diagnoses initial live attachment only. Once a
    /// complete table was installed, known-null teardown remains reconnectable
    /// even while stale live flags are still set. Foreign entries are rejected
    /// separately by `attempt`, regardless of this deadline.
    pub fn expired_after_install(
        &mut self, now_ms:u64, pending:bool, in_game:bool, frame:u32, owner:u8, installed:bool,
    )->bool {
        self.expired(now_ms,pending&&!installed,in_game,frame,owner)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    fn observations(actual: [usize; 4]) -> [ObservedSlot; 4] {
        std::array::from_fn(|i| ObservedSlot {
            slot: Slot {
                address: 0x1000 + i * 8,
                original: 0x2000 + i * 8,
                replacement: 0x3000 + i * 8,
            },
            actual: actual[i],
        })
    }
    fn original() -> [usize; 4] {
        [0x2000, 0x2008, 0x2010, 0x2018]
    }
    #[test]
    fn zero_and_partial_tables_wait_without_invoking_installer() {
        for actual in [
            [0; 4],
            [0x2000, 0, 0x2010, 0x2018],
            [0x3000, 0x2008, 0, 0x3018],
        ] {
            let observed = observations(actual);
            let called = Cell::new(false);
            let outcome = attempt(&observed, || {
                called.set(true);
                Ok(vec![])
            })
            .unwrap();
            assert!(matches!(outcome, Attempt::Waiting { .. }));
            assert!(!called.get());
            assert_eq!(
                observed.iter().map(|s| s.actual).collect::<Vec<_>>(),
                actual
            );
        }
        assert_eq!(
            classify(&observations([0; 4])),
            Gate::Waiting {
                initialized: 0,
                pending_mask: 15
            }
        );
    }
    #[test]
    fn foreign_is_prioritized_over_any_null_slot() {
        for actual in [[0, 0, 0x9999, 0], [0x9999, 0, 0, 0]] {
            let called = Cell::new(false);
            assert!(matches!(
                attempt(&observations(actual), || {
                    called.set(true);
                    Ok(vec![])
                }),
                Err(BindingError::Mismatch { actual: 0x9999, .. })
            ));
            assert!(!called.get());
        }
    }
    #[test]
    fn all_original_or_own_accepts_only_the_installer_result() {
        for actual in [
            original(),
            [0x3000, 0x3008, 0x3010, 0x3018],
            [0x2000, 0x3008, 0x2010, 0x3018],
        ] {
            let observed = observations(actual);
            assert_eq!(classify(&observed), Gate::Ready);
            assert_eq!(
                attempt(&observed, || Ok(vec![observed[0].slot])).unwrap(),
                Attempt::Ready {
                    changed: vec![observed[0].slot]
                }
            );
        }
    }
    #[test]
    fn zero_cas_race_retries_but_foreign_cas_race_fails() {
        let observed = observations(original());
        let error = |actual| BindingError::Mismatch {
            address: observed[0].slot.address,
            expected: observed[0].slot.original,
            actual,
        };
        assert_eq!(
            attempt(&observed, || Err(error(0))).unwrap(),
            Attempt::WaitingRace { address: 0x1000 }
        );
        assert!(matches!(
            attempt(&observed, || Err(error(0x9999))),
            Err(BindingError::Mismatch { actual: 0x9999, .. })
        ));
    }
    #[test]
    fn null_table_becomes_ready_and_can_wait_again_after_teardown() {
        let called = Cell::new(0);
        let installer = || {
            called.set(called.get() + 1);
            Ok(vec![])
        };
        assert!(matches!(
            attempt(&observations([0; 4]), installer).unwrap(),
            Attempt::Waiting { .. }
        ));
        assert_eq!(called.get(), 0);
        assert!(matches!(
            attempt(&observations(original()), installer).unwrap(),
            Attempt::Ready { .. }
        ));
        assert_eq!(called.get(), 1);
        assert!(matches!(
            attempt(&observations([0x3000, 0, 0, 0]), installer).unwrap(),
            Attempt::Waiting { .. }
        ));
        assert_eq!(called.get(), 1);
        assert!(matches!(
            attempt(&observations(original()), installer).unwrap(),
            Attempt::Ready { .. }
        ));
        assert_eq!(called.get(), 2);
    }
    #[test]
    fn menu_wait_is_unlimited_and_frame_zero_does_not_time_out() {
        let mut policy = WaitPolicy::default();
        for now in [0, 10_000, 1_000_000, u64::MAX] {
            assert!(!policy.expired(now, true, false, 100, 0));
            assert!(!policy.expired(now, true, true, 0, 0));
        }
    }
    #[test]
    fn installed_connection_survives_long_live_teardown_and_recovers_owned_table() {
        let mut policy=WaitPolicy::default();
        let mut actual=original();
        assert!(matches!(attempt(&observations(actual),||Ok(vec![])).unwrap(),Attempt::Ready{..}));
        for (now,slots) in [(0,[0;4]),(15_000,[0x3000,0,0x3010,0]),(120_000,[0;4])] {
            actual=slots;
            assert!(matches!(attempt(&observations(actual),||panic!("null table must stay unwritten")).unwrap(),Attempt::Waiting{..}));
            assert!(!policy.expired_after_install(now,true,true,100,0,true));
        }
        actual=original();
        assert!(matches!(attempt(&observations(actual),||Ok(vec![])).unwrap(),Attempt::Ready{..}));
        assert!(!policy.expired_after_install(120_100,false,true,1,0,true));
    }
    #[test]
    fn reconnection_deadline_does_not_allow_foreign_callbacks() {
        let mut policy=WaitPolicy::default();
        assert!(!policy.expired_after_install(u64::MAX,true,true,12,0,true));
        assert!(matches!(attempt(&observations([0,0x9999,0,0]),||Ok(vec![])),Err(BindingError::Mismatch{actual:0x9999,..})));
    }
    #[test]
    fn initial_install_deadline_still_expires_but_never_carries_into_reconnection() {
        let mut policy=WaitPolicy::default();
        assert!(!policy.expired_after_install(100,true,true,10,0,false));
        assert!(policy.expired_after_install(10_100,true,true,11,0,false));
        assert!(!policy.expired_after_install(10_101,true,true,12,0,true));
        assert!(!policy.expired_after_install(1_000_000,true,true,13,0,true));
    }
    #[test]
    fn live_empty_table_times_out_only_after_ten_continuous_seconds() {
        let mut policy = WaitPolicy::default();
        assert!(!policy.expired(100, true, true, 1, 0));
        assert!(!policy.expired(10_099, true, true, 2, 0));
        assert!(policy.expired(10_100, true, true, 3, 0));
        assert!(!policy.expired(10_200, true, false, 3, 0));
        assert!(!policy.expired(10_300, true, true, 4, 0));
        assert!(!policy.expired(20_299, true, true, 5, 0));
        assert!(policy.expired(20_300, true, true, 6, 0));
    }
    #[test]
    fn readiness_owner_and_frame_resets_cancel_old_timeout() {
        let mut policy = WaitPolicy::default();
        assert!(!policy.expired(0, true, true, 100, 0));
        assert!(!policy.expired(9_999, true, true, 101, 1));
        assert!(!policy.expired(10_000, true, true, 1, 1));
        assert!(!policy.expired(19_999, false, true, 2, 1));
        assert!(!policy.expired(20_000, true, true, 3, 1));
        assert!(!policy.expired(29_999, true, true, 4, 1));
        assert!(policy.expired(30_000, true, true, 5, 1));
    }
    #[cfg(windows)]
    fn owned_table(
        actual: [usize; 4],
    ) -> (Box<[std::sync::atomic::AtomicUsize; 4]>, [ObservedSlot; 4]) {
        use std::sync::atomic::AtomicUsize;
        let table = Box::new(actual.map(AtomicUsize::new));
        let observed = std::array::from_fn(|index| ObservedSlot {
            slot: Slot {
                address: &table[index] as *const AtomicUsize as usize,
                original: 0x2000 + index * 8,
                replacement: 0x3000 + index * 8,
            },
            actual: actual[index],
        });
        (table, observed)
    }
    #[cfg(windows)]
    #[test]
    fn owned_zero_table_stays_unwritten_then_original_table_installs() {
        use std::sync::atomic::Ordering;
        let (table, mut observed) = owned_table([0; 4]);
        let slots = observed.map(|item| item.slot);
        assert!(matches!(
            attempt(&observed, || unsafe {
                crate::callback_binding::maintain_report(&slots)
            })
            .unwrap(),
            Attempt::Waiting { .. }
        ));
        assert!(table.iter().all(|entry| entry.load(Ordering::SeqCst) == 0));
        for index in 0..4 {
            table[index].store(slots[index].original, Ordering::SeqCst);
            observed[index].actual = slots[index].original;
        }
        assert!(
            matches!(attempt(&observed, || unsafe { crate::callback_binding::maintain_report(&slots) }).unwrap(), Attempt::Ready { changed } if changed.len() == 4)
        );
        for index in 0..4 {
            assert_eq!(
                table[index].load(Ordering::SeqCst),
                slots[index].replacement
            );
        }
        table[1].store(0, Ordering::SeqCst);
        for index in 0..4 {
            observed[index].actual = table[index].load(Ordering::SeqCst);
        }
        assert!(matches!(
            attempt(&observed, || unsafe {
                crate::callback_binding::maintain_report(&slots)
            })
            .unwrap(),
            Attempt::Waiting {
                pending_mask: 2,
                ..
            }
        ));
        assert_eq!(table[1].load(Ordering::SeqCst), 0);
        for index in 0..4 {
            table[index].store(slots[index].original, Ordering::SeqCst);
            observed[index].actual = slots[index].original;
        }
        assert!(matches!(
            attempt(&observed, || unsafe {
                crate::callback_binding::maintain_report(&slots)
            })
            .unwrap(),
            Attempt::Ready { .. }
        ));
    }
    #[cfg(windows)]
    #[test]
    fn owned_zero_cas_race_rolls_back_prior_changes_and_waits() {
        use std::sync::atomic::Ordering;
        let (table, observed) = owned_table(original());
        let slots = observed.map(|item| item.slot);
        // A modeled engine reset occurs after the complete ready observation.
        table[1].store(0, Ordering::SeqCst);
        assert_eq!(
            attempt(&observed, || unsafe {
                crate::callback_binding::maintain_report(&slots)
            })
            .unwrap(),
            Attempt::WaitingRace {
                address: slots[1].address
            }
        );
        assert_eq!(table[0].load(Ordering::SeqCst), slots[0].original);
        assert_eq!(table[1].load(Ordering::SeqCst), 0);
        assert_eq!(table[2].load(Ordering::SeqCst), slots[2].original);
    }
    #[cfg(windows)]
    #[test]
    fn owned_foreign_cas_race_is_preserved_and_fails() {
        use std::sync::atomic::Ordering;
        let (table, observed) = owned_table(original());
        let slots = observed.map(|item| item.slot);
        table[1].store(0x9999, Ordering::SeqCst);
        assert!(matches!(
            attempt(&observed, || unsafe {
                crate::callback_binding::maintain_report(&slots)
            }),
            Err(BindingError::Mismatch { actual: 0x9999, .. })
        ));
        assert_eq!(table[0].load(Ordering::SeqCst), slots[0].original);
        assert_eq!(table[1].load(Ordering::SeqCst), 0x9999);
    }
}
