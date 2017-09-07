//! This crate defines a bidirectional request->response channel.
//!
//! It is useful for requesting and then receiving an item from other threads.
//!
//! Each channel has only one `Requester`, but it can have multiple `Responder`s.
//!
//! # Example
//!
//! ```
//! extern crate reqgetchan as chan;
//! 
//! use std::sync::Arc;
//! use std::sync::atomic::{AtomicBool, Ordering};
//! use std::thread;
//! use std::time::{Duration, Instant};
//! 
//! // Stuff required to pass functions around.
//! trait FnBox {
//!     fn call_box(self: Box<Self>);
//! }
//! impl<F: FnOnce()> FnBox for F {
//!     fn call_box(self: Box<F>) {
//!         (*self)()
//!     }
//! }
//! type Task = Box<FnBox + Send + 'static>;
//! 
//! // Variable used to test calling a `Task` sent between threads.
//! let test_var = Arc::new(AtomicBool::new(false));
//! let test_var2 = test_var.clone();
//! 
//! // Variable needed to stop `responder` thread if `requester` times out
//! let requester_timeout = Arc::new(AtomicBool::new(false));
//! let requester_timeout2 = requester_timeout.clone();
//! let requester_timeout3 = requester_timeout.clone();
//! 
//! let (requester, responder) = chan::channel::<Task>();
//! 
//! // `requester` thread
//! let requester_handle = thread::spawn(move || {
//!     let start_time = Instant::now();
//!     let timeout = Duration::new(0, 1000000);
//!     
//!     let mut monitor = requester.try_request().unwrap();
//! 
//!     loop {
//!         // Try to cancel request and stop threads if runtime
//!         // has exceeded `timeout`.
//!         if start_time.elapsed() >= timeout {
//!             // Try to cancel request.
//!             // This should only fail if `responder` has started responding.
//!             if monitor.try_cancel() {
//!                 // Notify other thread to stop.
//!                 requester_timeout2.store(true, Ordering::SeqCst);
//!                 break;
//!             }
//!         }
//! 
//!         // Try getting `task` from `responder`.
//!         match monitor.try_receive() {
//!             // `monitor` received `task`.
//!             Ok(task) => {
//!                 task.call_box();
//!                 break;
//!             },
//!             // Continue looping if `responder` has not yet sent `task`.
//!             Err(chan::TryReceiveError::Empty) => {},
//!             // The only other error is `chan::TryReceiveError::Done`.
//!             // This only happens if we call `monitor.try_receive()`
//!             // after either receiving data or cancelling the request.
//!             _ => unreachable!(),
//!         }
//!     }
//! });
//! 
//! // `responder` thread
//! let responder_handle = thread::spawn(move || {
//!     let mut tasks = vec![Box::new(move || {
//!         test_var2.store(true, Ordering::SeqCst);
//!     }) as Task];
//!     
//!     loop {
//!         // Exit loop if `receiver` has timed out.
//!         if requester_timeout3.load(Ordering::SeqCst) {
//!             break;
//!         }
//!         
//!         // Send `task` to `receiver` if it has issued a request.
//!         match responder.try_respond(tasks.pop().unwrap()) {
//!             // If `responder` tries to respond before `requester.try_request()`,
//!             // put `task` back and continue looping.
//!             Err(chan::TryRespondError::NoRequest(task)) => {
//!                 tasks.push(task);
//!             },
//!             // `responder` successfully sent response.
//!             Ok(_) => {
//!                 break;
//!             },
//!         }
//!     }
//! });
//! 
//! requester_handle.join().unwrap();
//! responder_handle.join().unwrap();
//!
//! // Ensure `test_var` has been set.
//! assert_eq!(test_var.load(Ordering::SeqCst), true);
//! ```

use std::cell::UnsafeCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// This function returns a tuple containing the two ends of this
/// bidirectional request->response channel.
///
/// # Example
/// 
/// ```
/// extern crate reqgetchan;
///
/// #[allow(unused_variables)]
/// let (requester, responder) = reqgetchan::channel::<u32>(); 
/// ```
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

/// This end of the channel requests item(s)
pub struct Requester<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Requester<T> {
    /// This methods tries to request item(s) from one or more `Responder`(s).
    /// It returns a `RequestMonitor` to poll for data or cancel the request.
    ///
    /// # Warning
    ///
    /// Only **one** `RequestMonitor` may be active at a time.
    ///
    /// # Example
    /// 
    /// ```
    /// extern crate reqgetchan as chan;
    ///
    /// let (requester, responder) = chan::channel::<u32>(); 
    ///
    /// let mut monitor = requester.try_request().unwrap();
    ///
    /// match monitor.try_receive() {
    ///     Err(chan::TryReceiveError::Empty) => { println!("No Data yet!"); },
    ///     _ => unreachable!(),
    /// }
    /// 
    /// responder.try_respond(5).ok().unwrap();
    /// 
    /// match monitor.try_receive() {
    ///     Ok(num) => { println!("Number: {}", num); },
    ///     _ => unreachable!(),
    /// }
    /// ```
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

/// This monitors the existing request issued by `Requester`.
pub struct RequestMonitor<T> {
    inner: Arc<Inner<T>>,
    done: bool,
}

impl<T> RequestMonitor<T> {
    /// This method attempts to receive data from one or more `Responder`(s).
    ///
    /// # Warning
    ///
    /// It will return `Err(TryReceiveError::Done)` if the user calls it
    /// after either receiving data or cancelling the request.
    ///
    /// # Example
    /// 
    /// ```
    /// extern crate reqgetchan as chan;
    ///
    /// let (requester, responder) = chan::channel::<u32>(); 
    ///
    /// let mut monitor = requester.try_request().unwrap();
    ///
    /// // The responder has not responded yet.
    /// match monitor.try_receive() {
    ///     Err(chan::TryReceiveError::Empty) => { println!("No Data yet!"); },
    ///     _ => unreachable!(),
    /// }
    /// 
    /// responder.try_respond(5).ok().unwrap();
    /// 
    /// // The responder has responded now.
    /// match monitor.try_receive() {
    ///     Ok(num) => { println!("Number: {}", num); },
    ///     _ => unreachable!(),
    /// }
    /// ```
    pub fn try_receive(&mut self) -> Result<T, TryReceiveError> {
        // Do not try to receive anything if the monitor already received data.
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

    /// This method attempts to cancel the request.
    ///
    /// # Return
    ///
    /// * `true` - Cancelled request
    /// * `false` - Responder started processing request first
    ///
    /// # Warning
    ///
    /// It will also return `false` if the user calls it after
    /// either receiving data or cancelling the request.
    ///
    /// # Example
    /// 
    /// ```
    /// extern crate reqgetchan as chan;
    ///
    /// let (requester, responder) = chan::channel::<u32>(); 
    ///
    /// {
    ///     let mut monitor = requester.try_request().unwrap();
    ///     assert_eq!(monitor.try_cancel(), true);
    /// }
    ///
    /// {
    ///     let mut monitor = requester.try_request().unwrap();
    ///     responder.try_respond(5).ok().unwrap();
    ///     if !monitor.try_cancel() {
    ///         println!("Number: {}", monitor.try_receive().unwrap());
    ///     }
    /// }
    /// ```
    pub fn try_cancel(&mut self) -> bool {
        // Do not try to unsend if the monitor already received data.
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

/// This end of the channel tries to respond to requests from its `Requester`.
#[derive(Clone)]
pub struct Responder<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Responder<T> {
    /// This method sends item(s) to the `RequestMonitor` created by
    /// its `Requester` *if and only if* that `Requester` issued a request. 
    ///
    /// # Arguments
    ///
    /// * `data` - The item(s) to send
    ///
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
        // Ensure inner has no existing requests.
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
    fn test_monitor_try_cancel() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        let mut monitor = rqst.try_request().unwrap();

        assert_eq!(monitor.try_cancel(), true);
    }

    #[test]
    fn test_monitor_try_cancel_too_late() {
        let (rqst, resp) = channel::<Task>();

        let task = Box::new(move || { println!("Hello World!"); }) as Task;
        
        let mut monitor = rqst.try_request().unwrap();

        resp.inner.try_set_data(task).ok();

        assert_eq!(monitor.try_cancel(), false);

        assert_eq!(monitor.done, false);

        monitor.done = true;
    }

    #[test]
    fn test_monitor_try_cancel_done() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        let mut monitor = rqst.try_request().unwrap();

        monitor.try_cancel();

        assert_eq!(monitor.try_cancel(), false);
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
