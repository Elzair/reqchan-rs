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
                Ok(contract) => {
                    contract.send(Box::new(move || {
                        var2.fetch_add(1, Ordering::SeqCst);
                    }) as Task);
                    break;
                },
                Err(Error::NoRequest) => {},
                Err(Error::AlreadyLocked) => { assert!(false); },
                _ => unreachable!(),
            }
        }
    });

    let mut contract = rqst.try_request().ok().unwrap();

    loop {
        match contract.try_receive() {
            Ok(task) => {
                task.call_box();
                break;
            },
            Err(Error::Empty) => {},
            Err(Error::Done) => { assert!(false); },
            _ => unreachable!(),
        }
    }

    handle.join().unwrap();

    assert_eq!(var.load(Ordering::SeqCst), 1);
}

//#[test]
fn test_request_threaded_receive() {
    let (rqst, resp) = channel::<Task>();

    let var = Arc::new(AtomicUsize::new(0));
    let var2 = var.clone();

    let handle = thread::spawn(move || {
        let mut contract = rqst.try_request().ok().unwrap();

        loop {
            match contract.try_receive() {
                Ok(task) => {
                    task.call_box();
                    break;
                },
                Err(Error::Empty) => {},
                Err(Error::Done) => { assert!(false); },
                _ => unreachable!(),
            }
        }
    });

    loop {
        match resp.try_respond() {
            Ok(contract) => {
                contract.send(Box::new(move || {
                    var2.fetch_add(1, Ordering::SeqCst);
                }) as Task);
                break;
            },
            Err(Error::NoRequest) => {},
            Err(Error::AlreadyLocked) => { assert!(false); },
            _ => unreachable!(),
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
        let mut contract = rqst.try_request().ok().unwrap();

        loop {
            match contract.try_receive() {
                Ok(task) => {
                    task.call_box();
                    break;
                },
                Err(Error::Empty) => {},
                Err(Error::Done) => { assert!(false); },
                _ => unreachable!(),
            }
        }
    });

    let handle2 = thread::spawn(move || {
        loop {
            match resp.try_respond() {
                Ok(contract) => {
                    contract.send(Box::new(move || {
                        var2.fetch_add(1, Ordering::SeqCst);
                    }) as Task);
                    break;
                },
                Err(Error::NoRequest) => {},
                Err(Error::AlreadyLocked) => { assert!(false); },
                _ => unreachable!(),
            }
        }
    });

    handle1.join().unwrap();
    handle2.join().unwrap();

    assert_eq!(var.load(Ordering::SeqCst), 1);
}
