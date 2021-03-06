//! This crate defines a channel for requesting and receiving data. Each
//! channel has only one requesting end, but it can have multiple responding
//! ends. It is useful for implementing work sharing.
//!
//! The two ends of the channel are asynchronous with respect to each other,
//! so it is kinda nonblocking. However, if multiple responding ends try to 
//! respond to the same request, only one will succeed; the rest will
//! return errors.  
//!
//! # Design
//!
//! ## Overview
//!
//! `reqchan` is built around the two halves of the channel: `Requester`
//! and `Responder`. Both implement methods, `Requester::try_request()` and
//! `Responder::try_respond()`, that, when succesful, lock their corresponding
//! side of the channel and return contracts. `RequestContract` **requires** the
//! user to either successfully receive a datum or cancel the request.
//! `ResponseContract` requires the user to send a datum. These requirements
//! prevent the system from losing data sent through the channel.
//!
//! ## Locking 
//!
//! `Responder::try_response()` locks the responding side to prevent other
//! potential responders from responding to the same request. However,
//! `Requester::try_request()` locks the requesting side of the channel
//! to prevent the user from trying to issue multiple outstanding requests.
//! Both locks are dropped when their corresponding contract object is dropped.
//!
//! ## Contracts 
//!
//! `Requester::try_request()` has to issue a `RequestContract` so the
//! thread of execution does not block waiting for a response. However,
//! that reason does not apply to `Responder::try_response()`. I originally
//! made `Responder::try_response()` send the datum. However, that required
//! the user to have the datum available to send even if it could not be sent,
//! and it required the user to handle the returned datum if it could not be
//! sent. If the datum was, say, half the contents of a `Vec`, this might entail
//! lots of expensive memory allocation. Therefore, I made `Responder::try_response()`
//! return a `ResponseContract` indicating that the responder *could* and *would*
//! respond to the request. This way the user only has to perform the necessary
//! steps to send the datum if the datum must be sent.
//!
//! # Examples 
//! 
//! ## Simple Example
//! 
//! This simple, single-threaded example demonstrates most of the API.
//! The only thing left out is `RequestContract::try_cancel()`.
//!
//! ```rust
//! extern crate reqchan;
//!
//! // Create channel.
//! let (requester, responder) = reqchan::channel::<u32>(); 
//!
//! // Issue request.
//! let mut request_contract = requester.try_request().ok().unwrap();
//!
//! // Respond with number.
//! responder.try_respond().ok().unwrap().send(5);
//!
//! // Receive and print number.
//! println!("Number is {}", request_contract.try_receive().ok().unwrap());
//! ```
//!
//! ## More Complex Example 
//!
//! This more complex example demonstrates more "real-world" usage.
//! One thread requests a 'task' (i.e. a closure to run), and the
//! other two threads fight over who gets to respond with their
//! own personal task. Meanwhile, the requesting thread is polling
//! for a task, and if it gets one in time, it runs it. Regardless of
//! whether or not the receiver got a task or timed out, the receiver
//! notifies other threads to stop running, and stops itself.
//!
//! ```rust
//! extern crate reqchan as chan;
//! 
//! use std::sync::Arc;
//! use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
//! use std::thread;
//! use std::time::{Duration, Instant};
//! 
//! // Stuff to make it easier to pass around closures.
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
//! let test_var = Arc::new(AtomicUsize::new(0));
//! let test_var2 = test_var.clone();
//! let test_var3 = test_var.clone();
//! 
//! // Variable needed to stop `responder` thread if `requester` times out
//! let should_exit = Arc::new(AtomicBool::new(false));
//! let should_exit_copy_1 = should_exit.clone();
//! let should_exit_copy_2 = should_exit.clone();
//! 
//! let (requester, responder) = chan::channel::<Task>();
//! let responder2 = responder.clone();
//! 
//! // requesting thread
//! let requester_handle = thread::spawn(move || {
//!     let start_time = Instant::now();
//!     let timeout = Duration::new(0, 1000000);
//!     
//!     let mut contract = requester.try_request().ok().unwrap();
//! 
//!     loop {
//!         // Try to cancel request and stop threads if runtime
//!         // has exceeded `timeout`.
//!         if start_time.elapsed() >= timeout {
//!             // Try to cancel request.
//!             // This should only fail if `responder` has started responding.
//!             if let Ok(_) = contract.try_cancel() {
//!                 // Notify other threads to stop.
//!                 should_exit.store(true, Ordering::SeqCst);
//!                 break;
//!             }
//!         }
//! 
//!         // Try getting `task` from `responder`.
//!         match contract.try_receive() {
//!             // `contract` received `task`.
//!             Ok(task) => {
//!                 task.call_box();
//!                 // Notify other threads to stop.
//!                 should_exit.store(true, Ordering::SeqCst);
//!                 break;
//!             },
//!             // Continue looping if `responder` has not yet sent `task`.
//!             Err(chan::Error::Empty) => {},
//!             // The only other error is `chan::Error::Done`.
//!             // This only happens if we call `contract.try_receive()`
//!             // after either receiving data or cancelling the request.
//!             _ => unreachable!(),
//!         }
//!     }
//! });
//! 
//! // responding thread 1
//! let responder_1_handle = thread::spawn(move || {
//!     let mut tasks = vec![Box::new(move || {
//!         test_var2.fetch_add(1, Ordering::SeqCst);
//!     }) as Task];
//!     
//!     loop {
//!         // Exit loop if `receiver` has timed out.
//!         if should_exit_copy_1.load(Ordering::SeqCst) {
//!             break;
//!         }
//!         
//!         // Send `task` to `receiver` if it has issued a request.
//!         match responder2.try_respond() {
//!             // `responder2` can respond to request.
//!             Ok(contract) => {
//!                 contract.send(tasks.pop().unwrap());
//!                 break;
//!             },
//!             // Either `requester` has not yet made a request,
//!             // or `responder2` already handled the request.
//!             Err(chan::Error::NoRequest) => {},
//!             // `responder2` is processing request..
//!             Err(chan::Error::AlreadyLocked) => { break; },
//!             _ => unreachable!(),
//!         }
//!     }
//! });
//! 
//! // responding thread 2
//! let responder_2_handle = thread::spawn(move || {
//!     let mut tasks = vec![Box::new(move || {
//!         test_var3.fetch_add(2, Ordering::SeqCst);
//!     }) as Task];
//!     
//!     loop {
//!         // Exit loop if `receiver` has timed out.
//!         if should_exit_copy_2.load(Ordering::SeqCst) {
//!             break;
//!         }
//!         
//!         // Send `task` to `receiver` if it has issued a request.
//!         match responder.try_respond() {
//!             // `responder2` can respond to request.
//!             Ok(contract) => {
//!                 contract.send(tasks.pop().unwrap());
//!                 break;
//!             },
//!             // Either `requester` has not yet made a request,
//!             // or `responder` already handled the request.
//!             Err(chan::Error::NoRequest) => {},
//!             // `responder` is processing request.
//!             Err(chan::Error::AlreadyLocked) => { break; },
//!             _ => unreachable!(),
//!         }
//!     }
//! });
//! 
//! requester_handle.join().unwrap();
//! responder_1_handle.join().unwrap();
//! responder_2_handle.join().unwrap();
//!
//! // `num` can be 0, 1 or 2.
//! let num = test_var.load(Ordering::SeqCst);
//! println!("Number is {}", num);
//! ```

use std::cell::UnsafeCell;
use std::result;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// This function creates a `reqchan` and returns a tuple containing the
/// two ends of this bidirectional request->response channel.
///
/// # Example
/// 
/// ```rust
/// extern crate reqchan;
///
/// #[allow(unused_variables)]
/// let (requester, responder) = reqchan::channel::<u32>(); 
/// ```
pub fn channel<T>() -> (Requester<T>, Responder<T>) {
    let inner = Arc::new(Inner {
        has_request_lock: AtomicBool::new(false),
        has_response_lock: AtomicBool::new(false),
        has_request: AtomicBool::new(false),
        has_datum: AtomicBool::new(false),
        datum: UnsafeCell::new(None),
    });

    (
        Requester { inner: inner.clone() },
        Responder { inner: inner.clone() },
    )
}

/// This end of the channel requests and receives data from its `Responder`(s).
pub struct Requester<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Requester<T> {
    /// This methods tries to request item(s) from one or more `Responder`(s).
    /// If successful, it returns a `RequestContract` to either poll for data or
    /// cancel the request.
    ///
    /// # Warning
    ///
    /// Only **one** `RequestContract` may be active at a time.
    ///
    /// # Example
    /// 
    /// ```rust
    /// extern crate reqchan as chan;
    ///
    /// let (requester, responder) = chan::channel::<u32>(); 
    ///
    /// // Create request.
    /// let mut request_contract = requester.try_request().ok().unwrap();
    /// 
    /// // We have to wait for `request_contract` to go out of scope
    /// // before we can make another request.
    /// // match requester.try_request() {
    /// //     Err(chan::Error::AlreadyLocked) => {
    /// //         println!("We already have a request contract!");
    /// //     },
    /// //     _ => unreachable!(),
    /// // }
    ///
    /// responder.try_respond().ok().unwrap().send(5);
    /// println!("Got number {}", request_contract.try_receive().ok().unwrap());
    /// ```
    pub fn try_request(&self) -> Result<RequestContract<T>> {
        // First, try to lock the requesting side.
        let _ = self.inner.try_lock_request()?;

        // Next, flag a request.
        self.inner.flag_request();

        // Then return a `RequestContract`.
        Ok(RequestContract {
            inner: self.inner.clone(),
            done: false,
        })
    }
}

/// This is the contract returned by a successful `Requester::try_request()`.
/// It represents the caller's exclusive access to the requesting side of
/// the channel. The user can either try to get a datum from the responding side
/// or *attempt* to cancel the request. To prevent data loss, `RequestContract`
/// will panic if the user has not received a datum or cancelled the request.
pub struct RequestContract<T> {
    inner: Arc<Inner<T>>,
    done: bool,
}

impl<T> RequestContract<T> {
    /// This method attempts to receive a datum from one or more responder(s).
    ///
    /// # Warning
    ///
    /// It returns `Err(Error::Done)` if the user called it
    /// after either receiving a datum or cancelling the request.
    ///
    /// # Example
    /// 
    /// ```rust
    /// extern crate reqchan as chan;
    ///
    /// let (requester, responder) = chan::channel::<u32>(); 
    ///
    /// let mut request_contract = requester.try_request().ok().unwrap();
    ///
    /// // The responder has not responded yet. 
    /// match request_contract.try_receive() {
    ///     Err(chan::Error::Empty) => { println!("No Data yet!"); },
    ///     _ => unreachable!(),
    /// }
    /// 
    /// responder.try_respond().ok().unwrap().send(6);
    /// 
    /// // The responder has responded now.
    /// match request_contract.try_receive() {
    ///     Ok(num) => { println!("Number: {}", num); },
    ///     _ => unreachable!(),
    /// }
    ///
    /// // We need to issue another request to receive more data.
    /// match request_contract.try_receive() {
    ///     Err(chan::Error::Done) => {
    ///         println!("We already received data!");
    ///     },
    ///     _ => unreachable!(),
    /// }
    /// ```
    pub fn try_receive(&mut self) -> Result<T> {
        // Do not try to receive anything if the contract already received data.
        if self.done {
            return Err(Error::Done);
        }

        let datum = self.inner.try_get_datum()?;
        self.done = true;

        Ok(datum)
    } 

    /// This method attempts to cancel a request. This is useful for
    /// implementing a timeout.
    ///
    /// # Warning
    ///
    /// It also returns `false` if the user called it after
    /// either receiving a datum or cancelling the request.
    ///
    /// # Example
    /// 
    /// ```rust
    /// extern crate reqchan as chan;
    ///
    /// let (requester, responder) = chan::channel::<u32>(); 
    ///
    /// {
    ///     let mut contract = requester.try_request().ok().unwrap();
    ///
    ///     // We can cancel the request since `responder` has not
    ///     // yet responded to it.
    ///     contract.try_cancel().ok().unwrap();
    ///
    ///     // Both contracts go out of scope here
    /// }
    ///
    /// {
    ///     let mut request_contract = requester.try_request().ok().unwrap();
    ///
    ///     responder.try_respond().ok().unwrap().send(7);
    ///
    ///     // It is too late to cancel the request!
    ///     match request_contract.try_cancel() {
    ///         Err(chan::Error::TooLate) => {
    ///             println!("Number: {}", request_contract.try_receive().unwrap());
    ///         },
    ///         _ => unimplemented!(),
    ///     }
    ///
    ///     // Both contracts go out of scope here
    /// }
    /// ```
    pub fn try_cancel(&mut self) -> Result<()> {
        // Do not try to unsend if the contract already received data.
        if self.done {
            return Err(Error::Done);
        }

        match self.inner.try_unflag_request() {
            Ok(()) => {
                self.done = true;
                Ok(())
            },
            Err(Error::NoRequest) => {
                Err(Error::TooLate)
            },
            _ => unreachable!(),
        }
    }
}

impl<T> Drop for RequestContract<T> {
    fn drop(&mut self) {
        if !self.done {
            panic!("Dropping RequestContract without receiving data!");
        }

        self.inner.unlock_request();
    }
}

/// This end of the channel sends data in response to requests from
/// its `Requester`.
pub struct Responder<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Responder<T> {
    /// This method signals the intent of `Responder` to respond to a request.
    /// If successful, it returns a `ResponseContract` to ensure the user sends
    /// a datum.
    ///
    /// # Warning
    ///
    /// Only **one** `ResponseContract` may be active at a time.
    ///
    /// # Example
    /// 
    /// ```
    /// extern crate reqchan as chan;
    ///
    /// let (requester, responder) = chan::channel::<u32>(); 
    ///
    /// // `requester` has not yet issued a request.
    /// match responder.try_respond() {
    ///     Err(chan::Error::NoRequest) => {
    ///         println!("There is no request!");
    ///     },
    ///     _ => unreachable!(),
    /// }
    ///
    /// let mut request_contract = requester.try_request().ok().unwrap();
    ///
    /// // `requester` has issued a request.
    /// let response_contract = responder.try_respond().ok().unwrap();
    ///
    /// // We cannot issue another response to the request.
    /// match responder.try_respond() {
    ///     Err(chan::Error::AlreadyLocked) => {
    ///         println!("We cannot issue multiple responses to a request!");
    ///     },
    ///     _ => unreachable!(),
    /// }
    ///
    /// response_contract.send(8);
    /// 
    /// println!("Number is {}", request_contract.try_receive().ok().unwrap());
    /// ```
    pub fn try_respond(&self) -> Result<ResponseContract<T>> {
        // First try to lock the responding side.
        let _ = self.inner.try_lock_response()?;
        
        // Next, atomically check for a request and signal a response to it.
        // If no request exists, drop the lock and return the data.
        match self.inner.try_unflag_request() {
            Ok(_) => {
                Ok(ResponseContract {
                    inner: self.inner.clone(),
                    done: false,
                })
            },
            Err(err) => {
                self.inner.unlock_response();
                Err(err)
            },
        }
    }
}

impl<T> Clone for Responder<T> {
    fn clone(&self) -> Self {
        Responder {
            inner: self.inner.clone(),
        }
    }
}

/// This is the contract returned by a successful `Responder::try_response()`.
/// It represents the caller's exclusive access to the responding side of
/// the channel. It ensures the user sends a datum by panicking if they have not.
pub struct ResponseContract<T> {
    inner: Arc<Inner<T>>,
    done: bool,
}

impl<T> ResponseContract<T> {
    /// This method tries to send a datum to the requesting end of the channel.
    /// It will then consume itself, thereby freeing the responding side of
    /// the channel.
    ///
    /// # Arguments
    ///
    /// * `datum` - The item(s) to send
    ///
    /// # Example
    /// 
    /// ```
    /// extern crate reqchan as chan;
    ///
    /// let (requester, responder) = chan::channel::<u32>(); 
    ///
    /// let mut request_contract = requester.try_request().ok().unwrap();
    ///
    /// let mut response_contract = responder.try_respond().ok().unwrap();
    ///
    /// // We send data to the requesting end.
    /// response_contract.send(9);
    ///
    /// println!("Number is {}", request_contract.try_receive().unwrap());
    /// ```
    pub fn send(mut self, datum: T) {
        self.inner.set_datum(datum);
        self.done = true;
    }
}

impl<T> Drop for ResponseContract<T> {
    fn drop(&mut self) {
        if !self.done {
            panic!("Dropping ResponseContract without sending data!");
        }

        self.inner.unlock_response();
    }
}

#[derive(Debug)]
pub enum Error {
    AlreadyLocked,
    Done,
    Empty,
    NoRequest,
    TooLate,
}

pub type Result<T> = result::Result<T, Error>;

#[doc(hidden)]
struct Inner<T> {
    has_request_lock: AtomicBool,
    has_response_lock: AtomicBool,
    has_request: AtomicBool,
    has_datum: AtomicBool,
    datum: UnsafeCell<Option<T>>,
}

unsafe impl<T> Sync for Inner<T> {}

#[doc(hidden)]
impl<T> Inner<T> {
    /// This method indicates that the requesting side has made a request.
    ///
    /// # Warning
    ///
    /// **ONLY** the requesting side of the channel should call it.
    ///
    /// # Invariant
    ///
    /// * self.has_request_lock == true
    #[inline]
    fn flag_request(&self) {
        self.has_request.store(true, Ordering::SeqCst);
    }

    /// This method atomically checks to see if the requesting end
    /// issued a request and unflag the request.
    #[inline]
    fn try_unflag_request(&self) -> Result<()> {
        let (old, new) = (true, false);

        let res = self.has_request.compare_and_swap(old,
                                                    new,
                                                    Ordering::SeqCst);
        if res == old {
            Ok(())
        }
        else {
            Err(Error::NoRequest)
        }
    }

    /// This method sets the inner datum to the specified value.
    ///
    /// # Arguments
    ///
    /// * datum - The datum to set
    ///
    /// # Warning
    ///
    /// **ONLY** the responding side of the channel should call it.
    ///
    /// # Invariant
    ///
    /// * self.has_response_lock == true
    ///
    /// * (*self.datum.get()).is_none() == true
    ///
    /// * self.has_datum == false
    #[inline]
    fn set_datum(&self, data: T) {
        // First update inner datum.
        unsafe {
            *self.datum.get() = Some(data);
        }

        // Then indicate the presence of a new datum.
        self.has_datum.store(true, Ordering::SeqCst);
    }
    
    /// This method tries to get the datum out of `Inner`.
    ///
    /// # Warning
    ///
    /// **ONLY** the requesting side of the channel should call it.
    ///
    /// # Invariant
    ///
    /// * self.has_request_lock == true
    ///
    /// * if self.has_datum == true then (*self.datum.get()).is_some() == true
    #[inline]
    fn try_get_datum(&self) -> Result<T> {
        // First check to see if data exists.
        let (old, new) = (true, false);

        if self.has_datum.compare_and_swap(old,
                                           new,
                                           Ordering::SeqCst) == old {
            // If so, retrieve the data and unwrap it from its Option container.
            unsafe {
                Ok((*self.datum.get()).take().unwrap())
            }
        }
        else {
            Err(Error::Empty)
        }
    }

    // TODO: Make locks Acquire and Release
    
    /// This method tries to lock the requesting side of the channel.
    /// It returns a `boolean` indicating whether or not it succeeded.
    #[inline]
    fn try_lock_request(&self) -> Result<()> {
        let (old, new) = (false, true);

        let res = self.has_request_lock.compare_and_swap(old,
                                                         new,
                                                         Ordering::SeqCst);

        if res == old {
            Ok(())
        }
        else {
            Err(Error::AlreadyLocked)
        }
    }

    /// This method unlocks the requesting side of the channel.
    #[inline]
    fn unlock_request(&self) {
        self.has_request_lock.store(false, Ordering::SeqCst);
    }

    /// This method tries to lock the responding side of the channel.
    /// It returns a `boolean` indicating whether or not it succeeded.
    #[inline]
    fn try_lock_response(&self) -> Result<()> {
        let (old, new) = (false, true);

        let res = self.has_response_lock.compare_and_swap(old,
                                                          new,
                                                          Ordering::SeqCst);

        if res == old {
            Ok(())
        }
        else {
            Err(Error::AlreadyLocked)
        }
    }

    /// This method unlocks the responding side of the channel.
    #[inline]
    fn unlock_response(&self) {
        self.has_response_lock.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    
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
    fn test_inner_try_lock_request() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        match rqst.inner.try_lock_request() {
            Ok(()) => {},
            _ => { assert!(false); },
        }

        assert_eq!(resp.inner.has_request_lock.load(Ordering::SeqCst), true);
    }
       
    #[test]
    fn test_inner_try_lock_request_multiple() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        rqst.inner.try_lock_request().ok().unwrap();

        match rqst.inner.try_lock_request() {
            Err(Error::AlreadyLocked) => {},
            _ => { assert!(false); },
        }
    }

    #[test]
    fn test_inner_try_unlock_request() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        rqst.inner.has_request_lock.store(true, Ordering::SeqCst);

        rqst.inner.unlock_request();
        
        assert_eq!(resp.inner.has_request_lock.load(Ordering::SeqCst), false);
    }
      
    #[test]
    fn test_inner_try_lock_response() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        match rqst.inner.try_lock_response() {
            Ok(()) => {},
            _ => { assert!(false); },
        }

        assert_eq!(resp.inner.has_response_lock.load(Ordering::SeqCst), true);
    }
       
    #[test]
    fn test_inner_try_lock_response_multiple() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        rqst.inner.try_lock_response().ok().unwrap();

        match rqst.inner.try_lock_response() {
            Err(Error::AlreadyLocked) => {},
            _ => { assert!(false); },
        }
    }

    #[test]
    fn test_inner_try_unlock_response() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        rqst.inner.has_response_lock.store(true, Ordering::SeqCst);

        rqst.inner.unlock_response();

        assert_eq!(resp.inner.has_response_lock.load(Ordering::SeqCst), false);
    }

    #[test]
    fn test_inner_flag_request() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        rqst.inner.flag_request();

        assert_eq!(resp.inner.has_request.load(Ordering::SeqCst), true);
    }

    #[test]
    fn test_inner_try_unflag_request() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        resp.inner.has_request.store(true, Ordering::SeqCst);

        match rqst.inner.try_unflag_request() {
            Ok(()) => {},
            _ => { assert!(false); },
        }

        assert_eq!(resp.inner.has_request.load(Ordering::SeqCst), false);

        match rqst.inner.try_unflag_request() {
            Err(Error::NoRequest) => {},
            _ => { assert!(false); },
        }
    }
   
    #[test]
    fn test_inner_try_unflag_request_multiple() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        resp.inner.has_request.store(true, Ordering::SeqCst);

        rqst.inner.try_unflag_request().ok().unwrap();

        match rqst.inner.try_unflag_request() {
            Err(Error::NoRequest) => {},
            _ => { assert!(false); },
        }
    }

    #[test]
    fn test_inner_set_datum() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        let task = Box::new(move || { println!("Hello World!"); }) as Task;

        resp.inner.set_datum(task);

        assert_eq!(resp.inner.has_datum.load(Ordering::SeqCst), true);
    }
  
    #[test]
    fn test_inner_try_get_datum_with_data() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        let var = Arc::new(AtomicUsize::new(0));
        let var2 = var.clone();
        
        let task = Box::new(move || {
            var2.fetch_add(1, Ordering::SeqCst);
        }) as Task;

        unsafe {
            *resp.inner.datum.get() = Some(task);
        }
        resp.inner.has_datum.store(true, Ordering::SeqCst);
             
        match rqst.inner.try_get_datum() {
            Ok(t) => {
                t.call_box();
                assert_eq!(var.load(Ordering::SeqCst), 1);
            },
            _ => { assert!(false); },
        }
    }
 
    #[test]
    fn test_inner_try_get_datum_no_data() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();
        
        match rqst.inner.try_get_datum() {
            Err(Error::Empty) => {}
            _ => { assert!(false); },
        }
    }

    #[test]
    fn test_requester_try_request() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        let mut contract = rqst.try_request().ok().unwrap();

        contract.done = true;
    }

    #[test]
    fn test_requester_try_request_multiple() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        rqst.inner.try_lock_request().ok().unwrap();

        match rqst.try_request() {
            Err(Error::AlreadyLocked) => {},
            _ => { assert!(false); },
        }
    }

    #[test]
    fn test_request_contract_try_receive() {
        let (rqst, resp) = channel::<Task>();

        let var = Arc::new(AtomicUsize::new(0));
        let var2 = var.clone();

        let task = Box::new(move || {
            var2.fetch_add(1, Ordering::SeqCst);
        }) as Task;

        let mut contract = rqst.try_request().ok().unwrap();

        resp.inner.set_datum(task);

        match contract.try_receive() {
            Ok(task) => {
                task.call_box();
            },
            _ => { assert!(false); },
        }

        assert_eq!(contract.done, true);
        assert_eq!(var.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_request_contract_try_receive_no_data() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        let mut contract = rqst.try_request().ok().unwrap();

        match contract.try_receive() {
            Err(Error::Empty) => {},
            _ => { assert!(false); },
        }

        assert_eq!(contract.done, false);

        contract.done = true;
    }
    
    #[test]
    fn test_request_contract_try_receive_done() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        let mut contract = rqst.try_request().ok().unwrap();

        contract.done = true;

        match contract.try_receive() {
            Err(Error::Done) => {},
            _ => { assert!(false); },
        }
    }

    #[test]
    fn test_request_contract_try_cancel() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        let mut contract = rqst.try_request().ok().unwrap();

        match contract.try_cancel() {
            Ok(()) => {},
            _ => { assert!(false); },
        }
    }

    #[test]
    fn test_request_contract_try_cancel_too_late() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();
        
        let mut contract = rqst.try_request().ok().unwrap();

        rqst.inner.try_unflag_request().ok().unwrap();

        match contract.try_cancel() {
            Err(Error::TooLate) => {},
            _ => { assert!(false); },
        }

        assert_eq!(contract.done, false);

        contract.done = true;
    }

    #[test]
    fn test_request_contract_try_cancel_done() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        let mut contract = rqst.try_request().ok().unwrap();

        contract.done = true;

        match contract.try_cancel() {
            Err(Error::Done) => {},
            _ => { assert!(false); },
        }
    }

    #[test]
    #[should_panic]
    fn test_request_contract_drop_without_receiving_data() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        #[allow(unused_variables)]
        let contract = rqst.try_request().ok().unwrap();
    }

    #[test]
    fn test_responder_try_respond() {
        let (rqst, resp) = channel::<Task>();
        
        rqst.inner.flag_request();

        let mut contract = resp.try_respond().ok().unwrap();

        contract.done = true;
    }

    #[test]
    fn test_responder_try_respond_no_request() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();
        
        match resp.try_respond() {
            Err(Error::NoRequest) => {},
            _ => { assert!(false); },
        }
    }

    #[test]
    fn test_responder_try_respond_multiple() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        let _ = resp.inner.try_lock_response().ok().unwrap();
        
        match resp.try_respond() {
            Err(Error::AlreadyLocked) => {},
            _ => { assert!(false); },
        }
    }

    #[test]
    fn test_response_contract_send() {
        let (rqst, resp) = channel::<Task>();

        rqst.inner.flag_request();

        let contract = resp.try_respond().ok().unwrap();

        contract.send(Box::new(move || { println!("Hello World!"); }) as Task);
    }

    #[test]
    #[should_panic]
    fn test_response_contract_drop_without_sending_data() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        #[allow(unused_variables)]
        let contract = resp.try_respond().ok().unwrap();
    }
}
