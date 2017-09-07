use std::cell::UnsafeCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

// mod request;
// mod response;

pub fn channel<T>() -> (Requester<T>, Responder<T>) {
    let inner = Arc::new(Inner {
        has_monitor: AtomicBool::new(false),
        has_request: AtomicBool::new(false),
        has_data: AtomicBool::new(false),
        data: UnsafeCell::new(None),
    });

    (
        Requester { inner: inner.clone() },
        Responder { inner: inner.clone() },
    )
}

pub struct Requester<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Requester<T> {
    pub fn try_request(&self) -> Result<RequestMonitor<T>, TryRequestError> {
        match self.inner.try_flag_request() {
            true => Ok(RequestMonitor {
                inner: self.inner.clone(),
                done: false,
            }),
            false => Err(TryRequestError::ExistingRequest),
        }
    }
}

unsafe impl<T> Send for Requester<T> {}

pub struct RequestMonitor<T> {
    inner: Arc<Inner<T>>,
    done: bool,
}

impl<T> RequestMonitor<T> {
    // pub fn receive(&mut self) -> T {
    //     loop {
    //         if let Ok(data) = self.try_receive() {
    //             return data;
    //         }
    //     }
    // }

    pub fn try_receive(&mut self) -> Result<T, TryReceiveError> {
        // Do not try to receive anything if the monitor is done.
        if self.done {
            return Err(TryReceiveError::Done);
        }

        match self.inner.try_get_data() {
            Ok(data) => {
                self.done = true;
                Ok(data)
            },
            Err(err) => Err(err),
        }
    } 

    pub fn try_unsend(&mut self) -> bool {
        // Do not try to unsend if the monitor is done.
        if self.done {
            return false;
        }

        match self.inner.try_unflag_request() {
            true => {
                self.done = true;
                true
            },
            false => false,
        }
    }
}

impl<T> Drop for RequestMonitor<T> {
    fn drop(&mut self) {
        if !self.done {
            panic!("Dropping RequestMonitor without receiving value");
        }

        self.inner.drop_monitor();
    }
}

#[derive(Clone)]
pub struct Responder<T> {
    inner: Arc<Inner<T>>,
}

unsafe impl<T> Send for Responder<T> {}

impl<T> Responder<T> {
    pub fn try_respond(&self, data: T) -> Result<(), TryRespondError<T>> {
        self.inner.try_set_data(data)
    }
}

struct Inner<T> {
    has_monitor: AtomicBool,
    has_request: AtomicBool,
    has_data: AtomicBool,
    data: UnsafeCell<Option<T>>,
}

unsafe impl<T> Sync for Inner<T> {}

impl<T> Inner<T> {
    #[inline]
    fn try_flag_request(&self) -> bool {
        // Ensure there are no existing requests.
        match self.try_add_monitor() {
            true => {
                self.has_request.store(true, Ordering::SeqCst);
                true
            },
            false => false,
        }
    }

    #[inline]
    fn try_unflag_request(&self) -> bool {
        let (old, new) = (true, false);

        self.has_request.compare_and_swap(old,
                                          new,
                                          Ordering::SeqCst) == old
    }

    #[inline]
    fn try_set_data(&self, data: T) -> Result<(), TryRespondError<T>> {
        // First atomically check for a request and signal a response to it.
        // If no request exists, return the data.
        if !self.try_unflag_request() {
            return Err(TryRespondError::NoRequest(data));
        }
        
        // Next update actual data.
        unsafe {
            *self.data.get() = Some(data);
        }

        // Then indicate the presence of new data.
        self.has_data.store(true, Ordering::SeqCst);

        Ok(())
    }
    
    #[inline]
    fn try_get_data(&self) -> Result<T, TryReceiveError> {
        // First check to see if data exists.
        let (old, new) = (true, false);

        if self.has_data.compare_and_swap(old,
                                          new,
                                          Ordering::SeqCst) == old {
            // If so, retrieve the data and unwrap it from its Option container.
            // Invariant: self.data.get() == Some(_) iff self.has_data == TRUE 
            unsafe {
                Ok((*self.data.get()).take().unwrap())
            }
        }
        else {
            Err(TryReceiveError::Empty)
        }
    }

    #[inline]
    fn try_add_monitor(&self) -> bool {
        let (old, new) = (false, true);

        self.has_monitor.compare_and_swap(old, new, Ordering::SeqCst) == old
    }

    #[inline]
    fn drop_monitor(&self) {
        self.has_monitor.store(false, Ordering::SeqCst);
    }
}

#[derive(Debug)]
pub enum TryRequestError {
    ExistingRequest,
}

#[derive(Debug)]
pub enum TryReceiveError {
    Empty,
    Done,
}

pub enum TryRespondError<T> {
    NoRequest(T),
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    
    use super::*;

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
    fn test_channel() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();
    }
   
    #[test]
    fn test_inner_try_add_monitor() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        assert_eq!(rqst.inner.try_add_monitor(), true);
        assert_eq!(resp.inner.has_monitor.load(Ordering::SeqCst), true);

        assert_eq!(rqst.inner.try_add_monitor(), false);
    }
    
    #[test]
    fn test_inner_try_drop_monitor() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        rqst.inner.has_monitor.store(true, Ordering::SeqCst);

        rqst.inner.drop_monitor();
        assert_eq!(resp.inner.has_monitor.load(Ordering::SeqCst), false);
    }

    #[test]
    fn test_inner_try_flag_request() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        assert_eq!(rqst.inner.try_flag_request(), true);
        assert_eq!(resp.inner.has_request.load(Ordering::SeqCst), true);
        assert_eq!(resp.inner.has_monitor.load(Ordering::SeqCst), true);

        assert_eq!(rqst.inner.try_flag_request(), false);
    }
  
    #[test]
    fn test_inner_try_unflag_request() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        resp.inner.has_request.store(true, Ordering::SeqCst);

        assert_eq!(rqst.inner.try_unflag_request(), true);
        assert_eq!(resp.inner.has_request.load(Ordering::SeqCst), false);

        assert_eq!(rqst.inner.try_unflag_request(), false);
    }
   
    #[test]
    fn test_inner_try_set_data_with_request() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        let task = Box::new(move || { println!("Hello World!"); }) as Task;

        rqst.inner.has_request.store(true, Ordering::SeqCst);

        match resp.inner.try_set_data(task) {
            Ok(_) => {},
            _ => { assert!(false); },
        }

        assert_eq!(resp.inner.has_data.load(Ordering::SeqCst), true);
    }
   
    #[test]
    fn test_inner_try_set_data_no_request() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        let var = Arc::new(AtomicUsize::new(0));
        let var2 = var.clone();

        let task = Box::new(move || {
            var2.fetch_add(1, Ordering::SeqCst);
        }) as Task;

        match resp.inner.try_set_data(task) {
            Err(TryRespondError::NoRequest(task)) => {
                task.call_box();
                assert_eq!(var.load(Ordering::SeqCst), 1);
            },
            _ => { assert!(false); },
        }

        assert_eq!(resp.inner.has_data.load(Ordering::SeqCst), false);
    }
 
    #[test]
    fn test_inner_try_get_data_with_data() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        let var = Arc::new(AtomicUsize::new(0));
        let var2 = var.clone();
        
        let task = Box::new(move || {
            var2.fetch_add(1, Ordering::SeqCst);
        }) as Task;

        unsafe {
            *resp.inner.data.get() = Some(task);
        }
        resp.inner.has_data.store(true, Ordering::SeqCst);
             
        match rqst.inner.try_get_data() {
            Ok(t) => {
                t.call_box();
                assert_eq!(var.load(Ordering::SeqCst), 1);
            },
            _ => { assert!(false); },
        }
    }
 
    #[test]
    fn test_inner_try_get_data_no_data() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();
        
        match rqst.inner.try_get_data() {
            Err(TryReceiveError::Empty) => {}
            _ => { assert!(false); },
        }
    }

    #[test]
    fn test_requester_try_request() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        let mut monitor = rqst.try_request().unwrap();

        monitor.done = true;
    }

    #[test]
    fn test_requester_try_request_disallow_multiple() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        let mut monitor = rqst.try_request().unwrap();

        match rqst.try_request() {
            Err(TryRequestError::ExistingRequest) => {},
            _ => { assert!(false); },
        }

        monitor.done = true;
    }

    #[test]
    fn test_responder_try_respond() {
        let (rqst, resp) = channel::<Task>();

        let task = Box::new(move || { println!("Hello World!"); }) as Task;
        
        rqst.inner.try_flag_request();

        resp.try_respond(task).ok();
    }

    #[test]
    fn test_responder_try_respond_no_request() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        let var = Arc::new(AtomicUsize::new(0));
        let var2 = var.clone();

        let task = Box::new(move || {
            var2.fetch_add(1, Ordering::SeqCst);
        }) as Task;

        match resp.try_respond(task) {
            Err(TryRespondError::NoRequest(task)) => {
                task.call_box();
                assert_eq!(var.load(Ordering::SeqCst), 1);
            },
            _ => { assert!(false); },
        }
    }

    #[test]
    fn test_monitor_try_receive() {
        let (rqst, resp) = channel::<Task>();

        let var = Arc::new(AtomicUsize::new(0));
        let var2 = var.clone();

        let task = Box::new(move || {
            var2.fetch_add(1, Ordering::SeqCst);
        }) as Task;

        let mut monitor = rqst.try_request().unwrap();

        resp.inner.try_set_data(task).ok();

        match monitor.try_receive() {
            Ok(task) => {
                task.call_box();
                assert_eq!(var.load(Ordering::SeqCst), 1);
            },
            _ => { assert!(false); },
        }

        assert_eq!(monitor.done, true);
    }

    #[test]
    fn test_monitor_try_receive_no_data() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        let mut monitor = rqst.try_request().unwrap();

        match monitor.try_receive() {
            Err(TryReceiveError::Empty) => {},
            _ => { assert!(false); },
        }

        assert_eq!(monitor.done, false);

        monitor.done = true;
    }
    
    #[test]
    fn test_monitor_try_receive_done() {
        let (rqst, resp) = channel::<Task>();

        let task = Box::new(move || { println!("Hello World!"); }) as Task;

        let mut monitor = rqst.try_request().unwrap();

        resp.inner.try_set_data(task).ok();

        monitor.try_receive().ok();

        match monitor.try_receive() {
            Err(TryReceiveError::Done) => {},
            _ => { assert!(false); },
        }
    }

    #[test]
    fn test_monitor_try_unsend() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        let mut monitor = rqst.try_request().unwrap();

        assert_eq!(monitor.try_unsend(), true);
    }

    #[test]
    fn test_monitor_try_unsend_too_late() {
        let (rqst, resp) = channel::<Task>();

        let task = Box::new(move || { println!("Hello World!"); }) as Task;
        
        let mut monitor = rqst.try_request().unwrap();

        resp.inner.try_set_data(task).ok();

        assert_eq!(monitor.try_unsend(), false);

        assert_eq!(monitor.done, false);

        monitor.done = true;
    }

    #[test]
    fn test_monitor_try_unsend_done() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        let mut monitor = rqst.try_request().unwrap();

        monitor.try_unsend();

        assert_eq!(monitor.try_unsend(), false);
    }

    #[test]
    #[should_panic]
    fn test_monitor_drop_without_receiving_data() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        #[allow(unused_variables)]
        let monitor = rqst.try_request().unwrap();
    }

    #[test]
    fn test_request_receive_threaded() {
        let (rqst, resp) = channel::<Task>();

        let var = Arc::new(AtomicUsize::new(0));
        let var2 = var.clone();
        let var3 = var.clone();

        let handle = thread::spawn(move || {
            let mut tasks = vec![Box::new(move || {
                var2.fetch_add(1, Ordering::SeqCst);
            }) as Task];
            
            loop {
                match resp.try_respond(tasks.pop().unwrap()) {
                    Err(TryRespondError::NoRequest(task)) => {
                        tasks.push(task);
                    },
                    Ok(_) => {
                        break;
                    },
                }
            }
        });

        let mut monitor = rqst.try_request().unwrap();

        loop {
            match monitor.try_receive() {
                Ok(task) => {
                    task.call_box();
                    assert_eq!(var3.load(Ordering::SeqCst), 1);
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
        let var3 = var.clone();

        let handle = thread::spawn(move || {
            let mut monitor = rqst.try_request().unwrap();

            loop {
                match monitor.try_receive() {
                    Ok(task) => {
                        task.call_box();
                        assert_eq!(var2.load(Ordering::SeqCst), 1);
                        break;
                    },
                    Err(TryReceiveError::Empty) => {},
                    Err(TryReceiveError::Done) => { assert!(false); },
                }
            }
        });

        let mut tasks = vec![Box::new(move || {
            var3.fetch_add(1, Ordering::SeqCst);
        }) as Task];
        
        loop {
            match resp.try_respond(tasks.pop().unwrap()) {
                Err(TryRespondError::NoRequest(task)) => {
                    tasks.push(task);
                },
                Ok(_) => {
                    break;
                },
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
        let var3 = var.clone();

        let handle1 = thread::spawn(move || {
            let mut monitor = rqst.try_request().unwrap();

            loop {
                match monitor.try_receive() {
                    Ok(task) => {
                        task.call_box();
                        assert_eq!(var3.load(Ordering::SeqCst), 1);
                        break;
                    },
                    Err(TryReceiveError::Empty) => {},
                    Err(TryReceiveError::Done) => { assert!(false); },
                }
            }
        });

        let handle2 = thread::spawn(move || {
            let mut tasks = vec![Box::new(move || {
                var2.fetch_add(1, Ordering::SeqCst);
            }) as Task];
            
            loop {
                match resp.try_respond(tasks.pop().unwrap()) {
                    Err(TryRespondError::NoRequest(task)) => {
                        tasks.push(task);
                    },
                    Ok(_) => {
                        break;
                    },
                }
            }
        });

        handle1.join().unwrap();
        handle2.join().unwrap();

        assert_eq!(var.load(Ordering::SeqCst), 1);
    }
}
