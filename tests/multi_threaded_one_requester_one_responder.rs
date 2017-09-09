use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

extern crate reqchan;
use reqchan::*;

trait FnBox {
    fn call_box(self: Box<Self>);
}

impl<F: FnOnce()> FnBox for F {
    fn call_box(self: Box<F>) {
        (*self)()
    }
}

type Task = Box<FnBox + Send + 'static>;

#[test]
fn test_request_receive_threaded() {
    let (rqst, resp) = channel::<Task>();

    let var = Arc::new(AtomicUsize::new(0));
    let var2 = var.clone();

    let handle = thread::spawn(move || {
        loop {
            match resp.try_respond() {
                Ok(mut contract) => {
                    contract.try_send(Box::new(move || {
                        var2.fetch_add(1, Ordering::SeqCst);
                    }) as Task).ok().unwrap();
                    break;
                },
                Err(TryRespondError::NoRequest) => {},
                Err(TryRespondError::Locked) => { assert!(false); },
            }
        }
    });

    let mut contract = rqst.try_request().unwrap();

    loop {
        match contract.try_receive() {
            Ok(task) => {
                task.call_box();
                break;
            },
            Err(TryReceiveError::Empty) => {},
            Err(TryReceiveError::Done) => { assert!(false); },
        }
    }

    handle.join().unwrap();

    assert_eq!(var.load(Ordering::SeqCst), 1);
}

#[test]
fn test_request_threaded_receive() {
    let (rqst, resp) = channel::<Task>();

    let var = Arc::new(AtomicUsize::new(0));
    let var2 = var.clone();

    let handle = thread::spawn(move || {
        let mut contract = rqst.try_request().unwrap();

        loop {
            match contract.try_receive() {
                Ok(task) => {
                    task.call_box();
                    break;
                },
                Err(TryReceiveError::Empty) => {},
                Err(TryReceiveError::Done) => { assert!(false); },
            }
        }
    });

    loop {
        match resp.try_respond() {
            Ok(mut contract) => {
                contract.try_send(Box::new(move || {
                    var2.fetch_add(1, Ordering::SeqCst);
                }) as Task).ok().unwrap();
                break;
            },
            Err(TryRespondError::NoRequest) => {},
            Err(TryRespondError::Locked) => { assert!(false); },
        }
    }

    handle.join().unwrap();

    assert_eq!(var.load(Ordering::SeqCst), 1);
}

#[test]
fn test_request_threaded_receive_threaded() {
    let (rqst, resp) = channel::<Task>();

    let var = Arc::new(AtomicUsize::new(0));
    let var2 = var.clone();

    let handle1 = thread::spawn(move || {
        let mut contract = rqst.try_request().unwrap();

        loop {
            match contract.try_receive() {
                Ok(task) => {
                    task.call_box();
                    break;
                },
                Err(TryReceiveError::Empty) => {},
                Err(TryReceiveError::Done) => { assert!(false); },
            }
        }
    });

    let handle2 = thread::spawn(move || {
        loop {
            match resp.try_respond() {
                Ok(mut contract) => {
                    contract.try_send(Box::new(move || {
                        var2.fetch_add(1, Ordering::SeqCst);
                    }) as Task).ok().unwrap();
                    break;
                },
                Err(TryRespondError::NoRequest) => {},
                Err(TryRespondError::Locked) => { assert!(false); },
            }
        }
    });

    handle1.join().unwrap();
    handle2.join().unwrap();

    assert_eq!(var.load(Ordering::SeqCst), 1);
}
