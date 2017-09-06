use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub fn channel() -> (Sender, Receiver) {
    let inner = Arc::new(AtomicBool::new(false));
    
    (
        Sender { inner: inner.clone() },
        Receiver { inner: inner.clone() },
    )
}

pub struct Sender {
    inner: Arc<AtomicBool>,
}

impl Sender {
    pub fn send(&self) -> Result<(), SendError> {
        match self.inner.compare_and_swap(false, true, Ordering::Relaxed) {
            false => Ok(()),
            true => Err(SendError::UnreceivedValue),
        }
    }
    
    pub fn try_unsend(&self) -> Result<(), TryUnsendError> {
        match self.inner.compare_and_swap(true, false, Ordering::Relaxed) {
            true => Ok(()),
            false => Err(TryUnsendError::TooLate),
        }
    }
}

pub struct Receiver {
    inner: Arc<AtomicBool>,
}

impl Receiver {
    pub fn receive(&self) -> bool {
        self.inner.compare_and_swap(true, false, Ordering::Relaxed)
    }
}

impl Clone for Receiver {
    fn clone(&self) -> Self {
        Receiver {
            inner: self.inner.clone(),
        }
    }
}

#[derive(Debug)]
pub enum SendError {
    UnreceivedValue,
}

#[derive(Debug)]
pub enum TryUnsendError {
    TooLate,
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use super::*;

    #[test]
    fn test_channel() {
        let (tx, rx) = channel();

        assert_eq!(tx.inner.load(Ordering::SeqCst), false);
        assert_eq!(rx.inner.load(Ordering::SeqCst), false);
    }

    #[test]
    fn test_sender_send() {
        #[allow(unused_variables)]
        let (tx, rx) = channel();

        tx.send().unwrap();

        assert_eq!(tx.inner.load(Ordering::SeqCst), true);
    }

    #[test]
    fn test_sender_send_twice() {
        #[allow(unused_variables)]
        let (tx, rx) = channel();

        tx.send().unwrap();

        match tx.send() {
            Err(SendError::UnreceivedValue) => {},
            _ => { assert!(false); },
        };
    }

    #[test]
    fn test_sender_try_unsend() {
        #[allow(unused_variables)]
        let (tx, rx) = channel();

        tx.inner.store(true, Ordering::SeqCst);

        tx.try_unsend().unwrap();
    }

    #[test]
    fn test_sender_try_unsend_too_late() {
        #[allow(unused_variables)]
        let (tx, rx) = channel();

        match tx.try_unsend() {
            Err(TryUnsendError::TooLate) => {},
            _ => { assert!(false); }
        }
    }

    #[test]
    fn test_receiver_recv() {
        #[allow(unused_variables)]
        let (tx, rx) = channel();

        rx.inner.store(true, Ordering::SeqCst);

        assert_eq!(rx.receive(), true);
        assert_eq!(rx.inner.load(Ordering::SeqCst), false);
    }

    #[test]
    fn test_receiver_recv_no_sent() {
        #[allow(unused_variables)]
        let (tx, rx) = channel();

        assert_eq!(rx.receive(), false);
        assert_eq!(rx.inner.load(Ordering::SeqCst), false);
    }

    #[test]
    fn test_send_receive() {
        let (tx, rx) = channel();

        tx.send().unwrap();

        assert_eq!(rx.receive(), true);
    }

    #[test]
    fn test_send_try_unsend() {
        let (tx, rx) = channel();

        tx.send().unwrap();

        tx.try_unsend().unwrap();

        assert_eq!(rx.receive(), false);
    }

    #[test]
    fn test_send_receive_try_unsend() {
        let (tx, rx) = channel();

        tx.send().unwrap();

        assert_eq!(rx.receive(), true);

        match tx.try_unsend() {
            Err(TryUnsendError::TooLate) => {},
            _ => { assert!(false); },
        };
    }

    #[test]
    fn test_send_multiple_receive() {
        let (tx, rx1) = channel();
        let rx2 = rx1.clone();

        tx.send().unwrap();

        assert_eq!(rx1.receive(), true);
        assert_eq!(rx2.receive(), false);

        tx.send().unwrap();

        assert_eq!(rx2.receive(), true);
        assert_eq!(rx1.receive(), false);
    }
}
