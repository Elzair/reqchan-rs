use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
fn test_multi_threaded_one_requester_two_responders() {
    let (rqst, resp) = channel::<Task>();
    let resp2 = resp.clone();

    let exit = Arc::new(AtomicBool::new(false));
    let exit1 = exit.clone();
    let exit2 = exit.clone();

    let var = Arc::new(AtomicUsize::new(0));
    let var2 = var.clone();
    let var3 = var.clone();

    let handle1 = thread::spawn(move || {
        let mut contract = rqst.try_request().unwrap();

        loop {
            match contract.try_receive() {
                Ok(task) => {
                    task.call_box();
                    exit.store(true, Ordering::SeqCst);
                    break;
                },
                Err(TryReceiveError::Empty) => {},
                Err(TryReceiveError::Done) => { assert!(false); },
            }
        }
    });

    let handle2 = thread::spawn(move || {
        loop {
            if exit1.load(Ordering::SeqCst) {
                break;
            }
            
            match resp.try_respond() {
                Ok(mut contract) => {
                    contract.try_send(Box::new(move || {
                        var2.fetch_add(1, Ordering::SeqCst);
                    }) as Task).ok().unwrap();
                    break;
                },
                Err(TryRespondError::NoRequest) => {},
                Err(TryRespondError::Locked) => { break; },
            }
        }
    });

    let handle3 = thread::spawn(move || {
        loop {
            if exit2.load(Ordering::SeqCst) {
                break;
            }
            
            match resp2.try_respond() {
                Ok(mut contract) => {
                    contract.try_send(Box::new(move || {
                        var3.fetch_add(2, Ordering::SeqCst);
                    }) as Task).ok().unwrap();
                    break;
                },
                Err(TryRespondError::NoRequest) => {},
                Err(TryRespondError::Locked) => { break; },
            }
        }
    });

    handle1.join().unwrap();
    handle2.join().unwrap();
    handle3.join().unwrap();

    let num = var.load(Ordering::SeqCst);
    assert!(num > 0);
}
