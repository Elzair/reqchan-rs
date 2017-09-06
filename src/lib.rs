use std::cell::UnsafeCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

// mod request;
// mod response;

pub struct Requester<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Requester<T> {
    pub fn try_request(&self) -> Result<RequestMonitor<T>, RequestError> {
        match self.inner.try_flag_request() {
            true => Ok(RequestMonitor {
                inner: self.inner.clone(),
                done: false,
            }),
            false => Err(RequestError::ExistingRequest),
        }
    }
}

pub struct RequestMonitor<T> {
    inner: Arc<Inner<T>>,
    done: bool,
}

impl<T> RequestMonitor<T> {
    pub fn receive(&mut self) -> T {
        loop {
            if let Ok(data) = self.try_receive() {
                return data;
            }
        }
    }

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

    pub fn try_unsend(&self) -> bool {
        self.inner.try_unflag_request()
    }
}

impl<T> Drop for RequestMonitor<T> {
    fn drop(&mut self) {
        if !self.done {
            panic!("Dropping RequestMonitor without receiving value");
        }
    }
}

#[derive(Clone)]
pub struct Responder<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Responder<T> {
    pub fn try_send(self, data: T) -> TrySendResult<T> {
        self.inner.try_set_data(data)
    }
}

struct Inner<T> {
    has_request: AtomicBool,
    has_data: AtomicBool,
    data: UnsafeCell<Option<T>>,
}

impl<T> Inner<T> {
    #[inline]
    fn try_flag_request(&self) -> bool {
        let (old, new) = (false, true);

        self.has_request.compare_and_swap(old,
                                          new,
                                          Ordering::SeqCst) == old
    }

    #[inline]
    fn try_unflag_request(&self) -> bool {
        let (old, new) = (true, false);

        self.has_request.compare_and_swap(old,
                                          new,
                                          Ordering::SeqCst) == old
    }

    #[inline]
    fn try_set_data(&self, data: T) -> TrySendResult<T> {
        // First atomically check for a request and signal a response to it.
        // If no request exists, return the data.
        if !self.try_unflag_request() {
            return TrySendResult::NoRequest(data);
        }
        
        // Next update actual data.
        unsafe {
            // // Check if unclaimed data exists.
            // // This should not happen.
            // // TODO: Remove if we can prove this cannot happen.
            // if let Some(_) = *self.data.get() {
            //     return TrySendResult::UnreceivedValue(data);
            // }

            *self.data.get() = Some(data);
        }

        // Then indicate the presence of new data.
        self.has_data.store(true, Ordering::SeqCst);

        TrySendResult::Ok
    }
    
    #[inline]
    fn try_get_data(&self) -> Result<T, TryReceiveError> {
        // First check to see if data exists.
        let (old, new) = (true, false);

        if self.has_data.compare_and_swap(old,
                                          new,
                                          Ordering::SeqCst) == new {
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
}

#[derive(Debug)]
pub enum RequestError {
    ExistingRequest,
}

#[derive(Debug)]
pub enum TryReceiveError {
    Empty,
    Done,
}

pub enum TrySendResult<T> {
    Ok,
    NoRequest(T),
    UnreceivedValue(T),
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
    }
}
