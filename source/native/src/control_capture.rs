//! One causal command observer per thread, shared by global and control callbacks.
//! A lease exists only after a caller actually obtained its before snapshot.

use std::cell::Cell;
use std::marker::PhantomData;
use std::rc::Rc;

thread_local! {
    static DEPTH: Cell<usize> = const { Cell::new(0) };
    static EVENT: Cell<usize> = const { Cell::new(0) };
}

pub(crate) fn active() -> bool {
    DEPTH.with(|depth| depth.get() != 0)
}

/// Compare identities only; the event address is never dereferenced here.
pub(crate) fn same_event(event: usize) -> bool {
    event != 0 && EVENT.with(|current| current.get() == event)
}

/// Identifies raw input delegated within an existing native callback scope.
pub(crate) struct EventScope {
    previous: usize,
    _thread: PhantomData<Rc<()>>,
}

impl EventScope {
    pub(crate) fn enter(event: usize) -> Self {
        let previous = EVENT.with(|current| current.replace(event));
        Self { previous, _thread: PhantomData }
    }
}

impl Drop for EventScope {
    fn drop(&mut self) {
        EVENT.with(|current| current.set(self.previous));
    }
}

/// Restores the prior observation scope, including during panic unwinding.
/// The marker prevents moving or sharing a lease across its owning thread.
pub(crate) struct Lease {
    previous: usize,
    _thread: PhantomData<Rc<()>>,
}

impl Lease {
    pub(crate) fn enter() -> Self {
        let previous = DEPTH.with(|depth| {
            let previous = depth.get();
            depth.set(previous.saturating_add(1));
            previous
        });
        Self { previous, _thread: PhantomData }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        DEPTH.with(|depth| depth.set(self.previous));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_thread_is_inactive_before_and_after_a_lease() {
        assert!(!active());
        let lease = Lease::enter();
        assert!(active());
        drop(lease);
        assert!(!active());
    }

    #[test]
    fn nested_leases_restore_the_parent_observation_scope() {
        let parent = Lease::enter();
        let child = Lease::enter();
        let grandchild = Lease::enter();
        assert!(active());
        drop(grandchild);
        assert!(active());
        drop(child);
        assert!(active());
        drop(parent);
        assert!(!active());
    }

    #[test]
    fn panic_unwind_restores_inactive_or_existing_parent_scope() {
        let result = std::panic::catch_unwind(|| {
            let _lease = Lease::enter();
            assert!(active());
            panic!("owned observation fixture unwind");
        });
        assert!(result.is_err());
        assert!(!active());

        let parent = Lease::enter();
        let result = std::panic::catch_unwind(|| {
            let _child = Lease::enter();
            assert!(active());
            panic!("owned descendant fixture unwind");
        });
        assert!(result.is_err());
        assert!(active());
        drop(parent);
        assert!(!active());
    }

    #[test]
    fn observation_scope_is_isolated_between_threads() {
        let parent = Lease::enter();
        std::thread::spawn(|| {
            assert!(!active());
            let worker = Lease::enter();
            assert!(active());
            drop(worker);
            assert!(!active());
        }).join().unwrap();
        assert!(active());
        drop(parent);
        assert!(!active());
    }

    #[test]
    fn failed_parent_snapshot_leaves_child_free_to_observe() {
        // A failed before snapshot supplies no lease. A later child with its
        // own valid snapshot can therefore own the causal observation scope.
        let parent: Option<Lease> = None;
        assert!(!active());
        let child = (!active()).then(Lease::enter);
        assert!(child.is_some());
        assert!(active());
        drop(child);
        assert!(!active());
        drop(parent);
        assert!(!active());
    }

    #[test]
    fn event_identity_follows_same_pointer_delegation_and_restores_nested_scopes() {
        let first = 0x10000;
        let second = 0x20000;
        assert!(!same_event(first));
        let parent = EventScope::enter(first);
        assert!(same_event(first));
        assert!(!same_event(second));
        let delegated = EventScope::enter(first);
        assert!(same_event(first));
        drop(delegated);
        assert!(same_event(first));

        let new_input = EventScope::enter(second);
        assert!(same_event(second));
        assert!(!same_event(first));
        drop(new_input);
        assert!(same_event(first));
        assert!(!same_event(second));
        drop(parent);
        assert!(!same_event(first));
        assert!(!same_event(second));
    }

    #[test]
    fn zero_is_never_an_observed_event_identity() {
        assert!(!same_event(0));
        let empty = EventScope::enter(0);
        assert!(!same_event(0));
        assert!(!same_event(0x10000));
        drop(empty);
        assert!(!same_event(0));
    }

    #[test]
    fn event_scope_unwind_restores_parent_identity_or_inactive_identity() {
        let first = 0x10000;
        let second = 0x20000;
        let result = std::panic::catch_unwind(|| {
            let _event = EventScope::enter(first);
            assert!(same_event(first));
            panic!("owned event identity fixture unwind");
        });
        assert!(result.is_err());
        assert!(!same_event(first));

        let parent = EventScope::enter(first);
        let result = std::panic::catch_unwind(|| {
            let _event = EventScope::enter(second);
            assert!(same_event(second));
            assert!(!same_event(first));
            panic!("owned descendant event identity fixture unwind");
        });
        assert!(result.is_err());
        assert!(same_event(first));
        assert!(!same_event(second));
        drop(parent);
        assert!(!same_event(first));
    }

    #[test]
    fn event_identity_is_isolated_between_threads() {
        let first = 0x10000;
        let second = 0x20000;
        let parent = EventScope::enter(first);
        std::thread::spawn(move || {
            assert!(!same_event(first));
            assert!(!same_event(second));
            let worker = EventScope::enter(second);
            assert!(same_event(second));
            assert!(!same_event(first));
            drop(worker);
            assert!(!same_event(first));
            assert!(!same_event(second));
        }).join().unwrap();
        assert!(same_event(first));
        assert!(!same_event(second));
        drop(parent);
        assert!(!same_event(first));
    }
}
