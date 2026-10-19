//! A connected platform account shared by the UI and the background fetches, which renew its
//! token and report the change so the app can save it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub struct Session<A>(Arc<Inner<A>>);

struct Inner<A> {
    account: Mutex<Option<A>>,
    /// Set when a renewal changed the account (or signed it out).
    changed: AtomicBool,
}

impl<A> Clone for Session<A> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<A: Clone + PartialEq> Session<A> {
    pub fn new(account: Option<A>) -> Self {
        Self(Arc::new(Inner {
            account: Mutex::new(account),
            changed: AtomicBool::new(false),
        }))
    }

    pub fn set(&self, account: Option<A>) {
        if let Ok(mut a) = self.0.account.lock() {
            *a = account;
        }
    }

    pub fn account(&self) -> Option<A> {
        self.0.account.lock().ok()?.clone()
    }

    /// The account after a background renewal (`Some(None)`: the sign-in expired), once.
    pub fn take_change(&self) -> Option<Option<A>> {
        self.0
            .changed
            .swap(false, Ordering::AcqRel)
            .then(|| self.account())
    }

    /// Stores a renewed account (`None`: renewal failed, signed out) in place of `old`, unless
    /// the user disconnected or reconnected meanwhile. Returns the account now in use.
    pub fn renewed(&self, old: &A, renewed: Option<A>) -> Option<A> {
        let mut current = self.0.account.lock().ok()?;
        if current.as_ref() != Some(old) {
            return current.clone();
        }
        *current = renewed.clone();
        self.0.changed.store(true, Ordering::Release);
        renewed
    }
}
