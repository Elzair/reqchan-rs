//! This crate defines a channel for requesting and receiving data.
//!
//! It is useful for requesting and then receiving a datum from other threads.
//!
//! Each channel has only one `Requester`, but it can have multiple `Responder`s.
//!
//! # Examples 
//! 
//! ## Simple Example
//! 
//! This simple, single-threaded example demonstrates most of the API.
//! The only thing left out is `RequestContract::try_cancel()`.
//!
//! ```
//! extern crate reqchan;
//!
//! // Create channel.
//! let (requester, responder) = reqchan::channel::<u32>(); 
//!
//! // Issue request.
//! let mut request_contract = requester.try_request().unwrap();
//!
//! // Respond with number.
//! responder.try_respond().unwrap().try_send(5).ok().unwrap();
//!
//! // Receive and print number.
//! println!("Number is {}", request_contract.try_receive().unwrap());
//! ```
//!
//! ## More Complex Example 
//!
//! This more complex example demonstrates more "real-world" usage.
//!
//! ```
//! extern crate reqchan as chan;
//! 
//! use std::sync::Arc;
//! use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
//!     let mut contract = requester.try_request().unwrap();
//! 
//!     loop {
//!         // Try to cancel request and stop threads if runtime
//!         // has exceeded `timeout`.
//!         if start_time.elapsed() >= timeout {
//!             // Try to cancel request.
//!             // This should only fail if `responder` has started responding.
//!             if contract.try_cancel() {
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
//!             Err(chan::TryReceiveError::Empty) => {},
//!             // The only other error is `chan::TryReceiveError::Done`.
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
//!             Ok(mut contract) => {
//!                 contract.try_send(tasks.pop().unwrap()).ok().unwrap();
//!                 break;
//!             },
//!             // Either `requester` has not yet made a request,
//!             // or `responder2` already handled the request.
//!             Err(chan::TryRespondError::NoRequest) => {},
//!             // `responder2` is processing request..
//!             Err(chan::TryRespondError::Locked) => { break; },
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
//!             Ok(mut contract) => {
//!                 contract.try_send(tasks.pop().unwrap()).ok().unwrap();
//!                 break;
//!             },
//!             // Either `requester` has not yet made a request,
//!             // or `responder` already handled the request.
//!             Err(chan::TryRespondError::NoRequest) => {},
//!             // `responder` is processing request.
//!             Err(chan::TryRespondError::Locked) => { break; },
//!         }
//!     }
//! });
//! 
//! requester_handle.join().unwrap();
//! responder_1_handle.join().unwrap();
//! responder_2_handle.join().unwrap();
//!
//! // Ensure receiving thread executed the task.
//! let num = test_var.load(Ordering::SeqCst);
//! assert!(num > 0);
//! println!("Number is {}", num);
//! ```

use std::cell::UnsafeCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// This function creates a `reqchan` and returns a tuple containing the
/// two ends of this bidirectional request->response channel.
///
/// # Example
/// 
/// ```
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
    /// ```
    /// extern crate reqchan as chan;
    ///
    /// let (requester, responder) = chan::channel::<u32>(); 
    ///
    /// // Create request.
    /// let mut request_contract = requester.try_request().unwrap();
    /// 
    /// // We have to wait for `request_contract` to go out of scope
    /// // before we can make another request.
    /// // match requester.try_request() {
    /// //     Err(chan::TryRequestError::Locked) => {
    /// //         println!("We already have a request contract!");
    /// //     },
    /// //     _ => unreachable!(),
    /// // }
    ///
    /// responder.try_respond().unwrap().try_send(5).ok().unwrap();
    /// println!("Got number {}", request_contract.try_receive().unwrap());
    /// ```
    pub fn try_request(&self) -> Result<RequestContract<T>, TryRequestError> {
        // First, try to lock the requesting side.
        if !self.inner.try_lock_request() {
            return Err(TryRequestError::Locked);
        }

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
    /// It returns `Err(TryReceiveError::Done)` if the user called it
    /// after either receiving a datum or cancelling the request.
    ///
    /// # Example
    /// 
    /// ```
    /// extern crate reqchan as chan;
    ///
    /// let (requester, responder) = chan::channel::<u32>(); 
    ///
    /// let mut request_contract = requester.try_request().unwrap();
    ///
    /// // The responder has not responded yet. 
    /// match request_contract.try_receive() {
    ///     Err(chan::TryReceiveError::Empty) => { println!("No Data yet!"); },
    ///     _ => unreachable!(),
    /// }
    /// 
    /// responder.try_respond().unwrap().try_send(5).ok().unwrap();
    /// 
    /// // The responder has responded now.
    /// match request_contract.try_receive() {
    ///     Ok(num) => { println!("Number: {}", num); },
    ///     _ => unreachable!(),
    /// }
    ///
    /// // We need to issue another request to receive more data.
    /// match request_contract.try_receive() {
    ///     Err(chan::TryReceiveError::Done) => {
    ///         println!("We already received data!");
    ///     },
    ///     _ => unreachable!(),
    /// }
    /// ```
    pub fn try_receive(&mut self) -> Result<T, TryReceiveError> {
        // Do not try to receive anything if the contract already received data.
        if self.done {
            return Err(TryReceiveError::Done);
        }

        match self.inner.try_get_datum() {
            Ok(datum) => {
                self.done = true;
                Ok(datum)
            },
            Err(err) => Err(err),
        }
    } 

    /// This method attempts to cancel a request. This is useful for
    /// implementing a timeout.
    ///
    /// # Return
    ///
    /// * `true` - Cancelled request
    /// * `false` - `Responder` started processing request first
    ///
    /// # Warning
    ///
    /// It also returns `false` if the user called it after
    /// either receiving a datum or cancelling the request.
    ///
    /// # Example
    /// 
    /// ```
    /// extern crate reqchan as chan;
    ///
    /// let (requester, responder) = chan::channel::<u32>(); 
    ///
    /// {
    ///     let mut contract = requester.try_request().unwrap();
    ///
    ///     // We can cancel the request since `responder` has not
    ///     // yet responded to it.
    ///     assert_eq!(contract.try_cancel(), true);
    ///
    ///     // Both contracts go out of scope here
    /// }
    ///
    /// {
    ///     let mut request_contract = requester.try_request().unwrap();
    ///
    ///     responder.try_respond().unwrap().try_send(5).ok().unwrap();
    ///
    ///     // It is too late to cancel the request!
    ///     if !request_contract.try_cancel() {
    ///         println!("Number: {}", request_contract.try_receive().unwrap());
    ///     }
    ///
    ///     // Both contracts go out of scope here
    /// }
    /// ```
    pub fn try_cancel(&mut self) -> bool {
        // Do not try to unsend if the contract already received data.
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
    ///     Err(chan::TryRespondError::NoRequest) => {
    ///         println!("There is no request!");
    ///     },
    ///     _ => unreachable!(),
    /// }
    ///
    /// let mut request_contract = requester.try_request().unwrap();
    ///
    /// // `requester` has issued a request.
    /// let mut response_contract = responder.try_respond().unwrap();
    ///
    /// // We cannot issue another response to the request.
    /// match responder.try_respond() {
    ///     Err(chan::TryRespondError::Locked) => {
    ///         println!("We cannot issue multiple responses to a request!");
    ///     },
    ///     _ => unreachable!(),
    /// }
    ///
    /// response_contract.try_send(5).ok().unwrap();
    /// 
    /// println!("Number is {}", request_contract.try_receive().unwrap());
    /// ```
    pub fn try_respond(&self) -> Result<ResponseContract<T>,
                                        TryRespondError> {

        // First try to lock the responding side.
        if !self.inner.try_lock_response() {
            return Err(TryRespondError::Locked);
        }
        
        // Next, atomically check for a request and signal a response to it.
        // If no request exists, drop the lock and return the data.
        if !self.inner.try_unflag_request() {
            self.inner.unlock_response();
            return Err(TryRespondError::NoRequest);
        }
     
        Ok(ResponseContract {
            inner: self.inner.clone(),
            done: false,
        })
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
    ///
    /// # Arguments
    ///
    /// * `datum` - The item(s) to send
    ///
    /// # Warning
    ///
    /// It returns `Err(TrySendError::Done(datum))` if the user called it
    /// after sending data once. The error also contains the datum the user
    /// tried to send. Therefore, the system does not "eat" unsent data.
    ///
    /// # Example
    /// 
    /// ```
    /// extern crate reqchan as chan;
    ///
    /// let (requester, responder) = chan::channel::<u32>(); 
    ///
    /// let mut request_contract = requester.try_request().unwrap();
    ///
    /// let mut response_contract = responder.try_respond().unwrap();
    ///
    /// // We send data to the requesting end.
    /// response_contract.try_send(9).ok().unwrap();
    ///
    /// // The response contract returns an error if we try to send
    /// // data more than once. It also returns the data
    /// match response_contract.try_send(10) {
    ///     Err(chan::TrySendError::Done(num)) => {
    ///         println!("We can send data only once per request!");
    ///         println!("The number we tried to send is {}", num);
    ///     },
    ///     _ => unreachable!(),
    /// }
    /// 
    /// println!("Number is {}", request_contract.try_receive().unwrap());
    /// ```
    pub fn try_send(&mut self, datum: T) -> Result<(), TrySendError<T>>  {
        // Do not try to send data if the contract already sent a datum.
        if self.done {
            return Err(TrySendError::Done(datum));
        }

        self.inner.set_datum(datum);
        self.done = true;

        Ok(())
    }
}

impl<T> Drop for ResponseContract<T> {
    fn drop(&mut self) {
        if !self.done {
            panic!("Dropping ResponseContract without sending data");
        }

        self.inner.unlock_response();
    }
}

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
    fn try_unflag_request(&self) -> bool {
        let (old, new) = (true, false);

        self.has_request.compare_and_swap(old,
                                          new,
                                          Ordering::SeqCst) == old
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
    /// * self.datum.get().is_none() == true
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
    /// * if self.has_datum == true then self.datum.get().is_some() == true
    #[inline]
    fn try_get_datum(&self) -> Result<T, TryReceiveError> {
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
            Err(TryReceiveError::Empty)
        }
    }

    /// This method tries to lock the requesting side of the channel.
    /// It returns a `boolean` indicating whether or not it succeeded.
    #[inline]
    fn try_lock_request(&self) -> bool {
        let (old, new) = (false, true);

        self.has_request_lock.compare_and_swap(old, new, Ordering::SeqCst) == old
    }

    /// This method unlocks the requesting side of the channel.
    #[inline]
    fn unlock_request(&self) {
        self.has_request_lock.store(false, Ordering::SeqCst);
    }

    /// This method tries to lock the responding side of the channel.
    /// It returns a `boolean` indicating whether or not it succeeded.
    #[inline]
    fn try_lock_response(&self) -> bool {
        let (old, new) = (false, true);

        self.has_response_lock.compare_and_swap(old, new, Ordering::SeqCst) == old
    }

    /// This method unlocks the responding side of the channel.
    #[inline]
    fn unlock_response(&self) {
        self.has_response_lock.store(false, Ordering::SeqCst);
    }
}

#[derive(Debug)]
pub enum TryRequestError {
    Locked,
}

#[derive(Debug)]
pub enum TryReceiveError {
    Empty,
    Done,
}

#[derive(Debug)]
pub enum TryRespondError {
    NoRequest,
    Locked,
}

pub enum TrySendError<T> {
    Done(T),
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

        assert_eq!(rqst.inner.try_lock_request(), true);
        assert_eq!(resp.inner.has_request_lock.load(Ordering::SeqCst), true);
    }
       
    #[test]
    fn test_inner_try_lock_request_multiple() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        rqst.inner.try_lock_request();

        assert_eq!(rqst.inner.try_lock_request(), false);
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

        assert_eq!(rqst.inner.try_lock_response(), true);
        assert_eq!(resp.inner.has_response_lock.load(Ordering::SeqCst), true);
    }
       
    #[test]
    fn test_inner_try_lock_response_multiple() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        rqst.inner.try_lock_response();

        assert_eq!(rqst.inner.try_lock_response(), false);
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

        assert_eq!(rqst.inner.try_unflag_request(), true);
        assert_eq!(resp.inner.has_request.load(Ordering::SeqCst), false);

        assert_eq!(rqst.inner.try_unflag_request(), false);
    }
   
    #[test]
    fn test_inner_try_unflag_request_multiple() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        resp.inner.has_request.store(true, Ordering::SeqCst);

        rqst.inner.try_unflag_request();

        assert_eq!(rqst.inner.try_unflag_request(), false);
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
            Err(TryReceiveError::Empty) => {}
            _ => { assert!(false); },
        }
    }

    #[test]
    fn test_requester_try_request() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        let mut contract = rqst.try_request().unwrap();

        contract.done = true;
    }

    #[test]
    fn test_requester_try_request_multiple() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        rqst.inner.try_lock_request();

        match rqst.try_request() {
            Err(TryRequestError::Locked) => {},
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

        let mut contract = rqst.try_request().unwrap();

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

        let mut contract = rqst.try_request().unwrap();

        match contract.try_receive() {
            Err(TryReceiveError::Empty) => {},
            _ => { assert!(false); },
        }

        assert_eq!(contract.done, false);

        contract.done = true;
    }
    
    #[test]
    fn test_request_contract_try_receive_done() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        let mut contract = rqst.try_request().unwrap();

        contract.done = true;

        match contract.try_receive() {
            Err(TryReceiveError::Done) => {},
            _ => { assert!(false); },
        }
    }

    #[test]
    fn test_request_contract_try_cancel() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        let mut contract = rqst.try_request().unwrap();

        assert_eq!(contract.try_cancel(), true);
    }

    #[test]
    fn test_request_contract_try_cancel_too_late() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();
        
        let mut contract = rqst.try_request().unwrap();

        rqst.inner.try_unflag_request();

        assert_eq!(contract.try_cancel(), false);
        assert_eq!(contract.done, false);

        contract.done = true;
    }

    #[test]
    fn test_request_contract_try_cancel_done() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        let mut contract = rqst.try_request().unwrap();

        contract.done = true;

        assert_eq!(contract.try_cancel(), false);
    }

    #[test]
    #[should_panic]
    fn test_request_contract_drop_without_receiving_data() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        #[allow(unused_variables)]
        let contract = rqst.try_request().unwrap();
    }

    #[test]
    fn test_responder_try_respond() {
        let (rqst, resp) = channel::<Task>();
        
        rqst.inner.flag_request();

        let mut contract = resp.try_respond().unwrap();

        contract.done = true;
    }

    #[test]
    fn test_responder_try_respond_no_request() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();
        
        match resp.try_respond() {
            Err(TryRespondError::NoRequest) => {},
            _ => { assert!(false); },
        }
    }

    #[test]
    fn test_responder_try_respond_multiple() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        resp.inner.try_lock_response();
        
        match resp.try_respond() {
            Err(TryRespondError::Locked) => {},
            _ => { assert!(false); },
        }
    }

    #[test]
    fn test_response_contract_try_send() {
        let (rqst, resp) = channel::<Task>();

        rqst.inner.flag_request();

        let mut contract = resp.try_respond().unwrap();

        contract.try_send(Box::new(move || { println!("Hello World!"); }) as Task).ok().unwrap();

        assert_eq!(contract.done, true);
    }

    #[test]
    fn test_response_contract_try_send_already_sent() {
        let (rqst, resp) = channel::<Task>();

        let var = Arc::new(AtomicUsize::new(0));
        let var2 = var.clone();

        let task1 = Box::new(move || { println!("Hello World!"); }) as Task;
        let task2 = Box::new(move || {
            var2.fetch_add(1, Ordering::SeqCst);
        }) as Task;

        rqst.inner.flag_request();

        let mut contract = resp.try_respond().unwrap();

        contract.try_send(task1).ok().unwrap();

        match contract.try_send(task2) {
            Err(TrySendError::Done(task)) => {
                task.call_box();
            },
            _ => { assert!(false); },
        }

        assert_eq!(var.load(Ordering::SeqCst), 1);
    }

    #[test]
    #[should_panic]
    fn test_response_contract_drop_without_sending_data() {
        #[allow(unused_variables)]
        let (rqst, resp) = channel::<Task>();

        #[allow(unused_variables)]
        let contract = resp.try_respond().unwrap();
    }
}
