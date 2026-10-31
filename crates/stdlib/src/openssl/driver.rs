//! Ownership rules for nonblocking OpenSSL retries.
//!
//! In particular, SSL_write returning WANT_WRITE forbids *any* other I/O on
//! that SSL until the same write is retried. Its buffer must survive Python
//! timeout, signal and nonblocking returns as well as ordinary OS waits.
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug)]
pub(super) struct WriteOwner(Arc<AtomicBool>);

impl WriteOwner {
    pub(super) fn new() -> Self {
        Self(Arc::new(AtomicBool::new(true)))
    }

    pub(super) fn release(&self) {
        self.0.store(false, Ordering::Release);
    }

    fn is_active(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    fn is(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Needs {
    Read,
    Write,
}

#[derive(Debug)]
struct PendingWrite {
    bytes: Box<[u8]>,
    owner: WriteOwner,
    needs: Option<Needs>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Permit {
    Ready,
    Wait,
    RetryOriginalWrite,
    InvalidRetry,
}

#[derive(Default, Debug)]
pub(super) struct Driver {
    pending: Option<PendingWrite>,
    pub(super) terminal: bool,
}

impl Driver {
    pub(super) fn other_io(&self) -> Permit {
        match &self.pending {
            Some(pending) if pending.needs == Some(Needs::Write) => {
                if pending.owner.is_active() {
                    Permit::Wait
                } else {
                    Permit::RetryOriginalWrite
                }
            }
            _ => Permit::Ready,
        }
    }

    pub(super) fn write(
        &mut self,
        owner: &WriteOwner,
        bytes: &mut Option<Box<[u8]>>,
        queued: &mut bool,
    ) -> Permit {
        // An invocation can fail before entering native SSL. There is no
        // continuation in that case, so its abandoned buffer is disposable.
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.needs.is_none() && !pending.owner.is_active())
        {
            self.pending = None;
        }
        if let Some(pending) = &mut self.pending {
            if pending.owner.is_active() {
                if pending.owner.is(owner) {
                    return Permit::Ready;
                }
                *queued = true;
                return Permit::Wait;
            }
            // A write already queued behind another call is a distinct write,
            // even when its contents happen to match the abandoned operation.
            if *queued || bytes.as_deref() != Some(pending.bytes.as_ref()) {
                return Permit::InvalidRetry;
            }
            pending.owner = owner.clone();
            // Retain the original allocation, not the new invocation's copy.
            *bytes = None;
            return Permit::Ready;
        }
        self.pending = Some(PendingWrite {
            bytes: bytes.take().expect("new write owns its buffer"),
            owner: owner.clone(),
            needs: None,
        });
        Permit::Ready
    }

    pub(super) fn bytes(&self, owner: &WriteOwner) -> &[u8] {
        let pending = self.pending.as_ref().expect("active write");
        assert!(pending.owner.is(owner) && pending.owner.is_active());
        &pending.bytes
    }

    pub(super) fn settle_write(&mut self, owner: &WriteOwner, needs: Option<Needs>) -> bool {
        let pending = self.pending.as_mut().expect("active write");
        assert!(pending.owner.is(owner) && pending.owner.is_active());
        if let Some(needs) = needs {
            pending.needs = Some(needs);
        } else {
            self.pending = None;
        }
        needs.is_some()
    }

    pub(super) fn fail(&mut self) {
        self.terminal = true;
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> Box<[u8]> {
        Box::from(&b"stable bytes"[..])
    }

    #[test]
    fn want_write_is_exclusive_and_keeps_original_allocation() {
        let mut driver = Driver::default();
        let owner = WriteOwner::new();
        let mut bytes = Some(payload());
        let ptr = bytes.as_ref().unwrap().as_ptr();
        assert_eq!(driver.write(&owner, &mut bytes, &mut false), Permit::Ready);
        driver.settle_write(&owner, Some(Needs::Write));
        assert_eq!(driver.other_io(), Permit::Wait);
        assert_eq!(driver.bytes(&owner).as_ptr(), ptr);
        owner.release();
        assert_eq!(driver.other_io(), Permit::RetryOriginalWrite);
        let next = WriteOwner::new();
        assert_eq!(
            driver.write(&next, &mut Some(payload()), &mut false),
            Permit::Ready
        );
        assert_eq!(driver.bytes(&next).as_ptr(), ptr);
        assert!(
            !driver.settle_write(&next, None),
            "completion disarms cleanup"
        );
        assert_eq!(driver.other_io(), Permit::Ready);
    }

    #[test]
    fn want_read_allows_progress_but_not_a_different_write() {
        let mut driver = Driver::default();
        let owner = WriteOwner::new();
        driver.write(&owner, &mut Some(payload()), &mut false);
        driver.settle_write(&owner, Some(Needs::Read));
        assert_eq!(driver.other_io(), Permit::Ready);
        owner.release();
        assert_eq!(driver.other_io(), Permit::Ready);
        let other = WriteOwner::new();
        let mut different = Some(Box::from(&b"different"[..]));
        assert_eq!(
            driver.write(&other, &mut different, &mut false),
            Permit::InvalidRetry
        );
        assert_eq!(
            driver.write(&other, &mut Some(payload()), &mut false),
            Permit::Ready
        );
        assert_eq!(driver.bytes(&other), b"stable bytes");
    }

    #[test]
    fn concurrent_equal_writes_are_distinct() {
        let mut driver = Driver::default();
        let first = WriteOwner::new();
        let second = WriteOwner::new();
        driver.write(&first, &mut Some(payload()), &mut false);
        driver.settle_write(&first, Some(Needs::Write));
        let mut bytes = Some(payload());
        let ptr = bytes.as_ref().unwrap().as_ptr();
        let mut queued = false;
        assert_eq!(driver.write(&second, &mut bytes, &mut queued), Permit::Wait);
        driver.settle_write(&first, None);
        assert_eq!(
            driver.write(&second, &mut bytes, &mut queued),
            Permit::Ready
        );
        assert_eq!(driver.bytes(&second).as_ptr(), ptr);
    }

    #[test]
    fn queued_writer_cannot_acknowledge_an_abandoned_write() {
        let mut driver = Driver::default();
        let first = WriteOwner::new();
        let second = WriteOwner::new();
        driver.write(&first, &mut Some(payload()), &mut false);
        driver.settle_write(&first, Some(Needs::Read));
        let mut bytes = Some(payload());
        let mut queued = false;
        assert_eq!(driver.write(&second, &mut bytes, &mut queued), Permit::Wait);
        first.release();
        assert_eq!(
            driver.write(&second, &mut bytes, &mut queued),
            Permit::InvalidRetry
        );
        assert!(driver.pending.is_some());
    }

    #[test]
    fn stale_release_cannot_orphan_an_adopted_writer() {
        let mut driver = Driver::default();
        let first = WriteOwner::new();
        driver.write(&first, &mut Some(payload()), &mut false);
        assert!(driver.settle_write(&first, Some(Needs::Write)));
        first.release();
        let next = WriteOwner::new();
        assert_eq!(
            driver.write(&next, &mut Some(payload()), &mut false),
            Permit::Ready
        );
        first.release();
        assert_eq!(driver.other_io(), Permit::Wait);
        assert_eq!(driver.bytes(&next), b"stable bytes");
        assert!(!driver.settle_write(&next, None));
    }

    #[test]
    fn release_before_attempt_and_terminal_error_leave_no_retry() {
        let mut driver = Driver::default();
        let owner = WriteOwner::new();
        driver.write(&owner, &mut Some(payload()), &mut false);
        owner.release();
        let owner = WriteOwner::new();
        let mut different = Some(Box::from(&b"new operation"[..]));
        assert_eq!(
            driver.write(&owner, &mut different, &mut false),
            Permit::Ready
        );
        assert_eq!(driver.bytes(&owner), b"new operation");
        driver.settle_write(&owner, Some(Needs::Write));
        driver.fail();
        assert!(driver.terminal);
        owner.release();
        assert!(driver.pending.is_none());
    }
}
