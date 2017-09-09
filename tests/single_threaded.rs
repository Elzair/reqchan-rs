use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

extern crate reqgetchan;
use reqgetchan::*;

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
fn test_multiple_requests() {
    let (rqst, resp) = reqgetchan::channel::<Task>();

    let var = Arc::new(AtomicUsize::new(0));
    let var2 = var.clone();
    let var3 = var.clone();

    // Scope of first set of contracts
    {
        let mut rqst_con = rqst.try_request().unwrap();
        let mut resp_con = resp.try_respond().unwrap();

        resp_con.try_send(Box::new(move || {
            var2.fetch_add(1, Ordering::SeqCst);
        }) as Task).ok().unwrap();

        match rqst_con.try_receive() {
            Ok(task) => {
                task.call_box();
            },
            _ => { assert!(false); },
        }
        // `resp_con` should drop here freeing up `resp` 
        // `rqst_con` should drop here freeing up `rqst` 
    }

    assert_eq!(var.load(Ordering::SeqCst), 1);

    // Scope of second set of contracts
    {
        let mut rqst_con = rqst.try_request().unwrap();
        let mut resp_con = resp.try_respond().unwrap();

        resp_con.try_send(Box::new(move || {
            var3.fetch_add(1, Ordering::SeqCst);
        }) as Task).ok().unwrap();

        match rqst_con.try_receive() {
            Ok(task) => {
                task.call_box();
            },
            _ => { assert!(false); },
        }
        // `resp_con` should drop here freeing up `resp` 
        // `rqst_con` should drop here freeing up `rqst` 
    }

    assert_eq!(var.load(Ordering::SeqCst), 2);
}

#[test]
fn test_multiple_responders() {
    let (rqst, resp) = channel::<Task>();
    let resp2 = resp.clone();

    let var = Arc::new(AtomicUsize::new(0));
    let var2 = var.clone();
    
    let mut rqst_con = rqst.try_request().unwrap();
    let mut resp_con = resp.try_respond().unwrap();

    match resp2.try_respond() {
        Err(TryRespondError::Locked) => {},
        _ => { assert!(false); },
    }

    resp_con.try_send(Box::new(move || {
        var2.fetch_add(1, Ordering::SeqCst);
    }) as Task).ok().unwrap();

    match rqst_con.try_receive() {
        Ok(task) => {
            task.call_box();
        },
        _ => { assert!(false); },
    }

    assert_eq!(var.load(Ordering::SeqCst), 1);
}
