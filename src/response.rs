use std::cell::UnsafeCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let inner = Arc::new(Inner {
        has_data: AtomicBool::new(false),
        data: UnsafeCell::new(None),
    });
    
    (
        Sender { inner: inner.clone() },
        Receiver {
            inner: inner.clone(),
        },
    )
}

#[derive(Clone)]
pub struct Sender<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Sender<T> {
    pub fn send(&self, data: T) -> Result<(), SendError> {
        // First update actual data.
        unsafe {
            // Check if Receiver has not claimed previous data.
            // This should not happen.
            if let Some(_) = *self.inner.data.get() {
                return Err(SendError::UnreceivedValue);
            }

            *self.inner.data.get() = Some(data);
        }

        // Then indicate data has been sent.
        self.inner.has_data.store(true, Ordering::SeqCst);

        Ok(())
    }
}

pub struct Receiver<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Receiver<T> {
    #[inline]
    pub fn try_receive(&self) -> Result<T, TryReceiveError> {
        // First check to see if any data has been sent.
        match self.inner.has_data.compare_and_swap(true, false, Ordering::SeqCst) {
            true => {
                // If so, retrieve the data and unwrap it from its Option container.
                // Invariant: *self.inner.data.get() == Some(_) if self.inner.has_data == true
                unsafe {
                    Ok((*self.inner.data.get()).take().unwrap())
                }
            },
            false => {
                Err(TryReceiveError::Empty)
            },
        }
    }

    pub fn receive(&self) -> T {
        loop {
            if let Ok(data) = self.try_receive() {
                return data;
            }
        }
    }
}

struct Inner<T> {
    has_data: AtomicBool,
    data: UnsafeCell<Option<T>>,
}

unsafe impl<T> Send for Inner<T> {}
unsafe impl<T> Sync for Inner<T> {}

#[derive(Debug)]
pub enum SendError {
    UnreceivedValue,
}

#[derive(Debug)]
pub enum TryReceiveError {
    Empty,
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
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

    type Task = FnBox + Send + 'static;
    
    enum TaskData {
        NoTasks,
        OneTask(Box<Task>),
        ManyTasks(VecDeque<Box<Task>>),
    }

    #[test]
    fn test_creation() {
        #[allow(unused_variables)]
        let (tx, rx) = channel::<TaskData>();
    }

    #[test]
    fn test_sender_send_no_tasks() {
        #[allow(unused_variables)]
        let (tx, rx) = channel();

        tx.send(TaskData::NoTasks).unwrap();

        unsafe {
            match *tx.inner.data.get() {
                Some(TaskData::NoTasks) => {},
                _ => { assert!(false); },
            }
        }
    }

    #[test]
    fn test_sender_send_one_task() {
        #[allow(unused_variables)]
        let (tx, rx) = channel();

        let data = TaskData::OneTask(Box::new(|| { println!("Hello world!"); }));
        tx.send(data).unwrap();

        unsafe {
            match *tx.inner.data.get() {
                Some(TaskData::OneTask(_)) => {},
                _ => { assert!(false); },
            }
        }
    }

    #[test]
    fn test_sender_send_many_tasks() {
        #[allow(unused_variables)]
        let (tx, rx) = channel();

        let data1 = Box::new(|| { println!("Hello world!"); });
        let data2 = Box::new(|| { println!("Hello again!"); });
        let mut data = VecDeque::<Box<Task>>::new();
        data.push_back(data1);
        data.push_back(data2);

        tx.send(TaskData::ManyTasks(data)).unwrap();

        unsafe {
            match *tx.inner.data.get() {
                Some(TaskData::ManyTasks(_)) => {},
                _ => { assert!(false); },
            }
        }
    }
    
    #[test]
    fn test_sender_send_unreceived() {
        #[allow(unused_variables)]
        let (tx, rx) = channel();

        tx.send(TaskData::NoTasks).unwrap();

        match tx.send(TaskData::NoTasks) {
            Err(SendError::UnreceivedValue) => {},
            _ => { assert!(false); },
        };
    }

    #[test]
    fn test_receiver_try_receive_no_tasks() {
        let (tx, rx) = channel();

        tx.send(TaskData::NoTasks).unwrap();

        match rx.try_receive() {
            Ok(TaskData::NoTasks) => {},
            _ => { assert!(false); },
        };

        unsafe {
            match *tx.inner.data.get() {
                None => {},
                _ => { assert!(false); },
            }
        }
    }

    #[test]
    fn test_receiver_try_receive_one_task() {
        let (tx, rx) = channel();

        let var = Arc::new(AtomicUsize::new(0));
        let var2 = var.clone();

        let data = TaskData::OneTask(Box::new(move || {
            var2.fetch_add(1, Ordering::SeqCst);
        }));

        tx.send(data).unwrap();

        match rx.try_receive() {
            Ok(TaskData::OneTask(task)) => {
                task.call_box();
            },
            _ => { assert!(false); },
        };

        assert_eq!(var.load(Ordering::SeqCst), 1);

        unsafe {
            match *tx.inner.data.get() {
                None => {},
                _ => { assert!(false); },
            }
        }
    }

    #[test]
    fn test_receiver_try_receive_many_tasks() {
        let (tx, rx) = channel();

        let var = Arc::new(AtomicUsize::new(0));
        let var1 = var.clone();
        let var2 = var.clone();

        let task1 = Box::new(move || {
            var1.fetch_add(1, Ordering::SeqCst);
        });
        let task2 = Box::new(move || {
            var2.fetch_add(1, Ordering::SeqCst);
        });

        let mut vd: VecDeque<Box<Task>> = VecDeque::new();
        vd.push_back(task1);
        vd.push_back(task2);

        let data = TaskData::ManyTasks(vd);

        tx.send(data).unwrap();

        match rx.try_receive() {
            Ok(TaskData::ManyTasks(tasks)) => {
                for task in tasks.into_iter() {
                    task.call_box();
                }
            },
            _ => { assert!(false); },
        };

        assert_eq!(var.load(Ordering::SeqCst), 2);

        unsafe {
            match *tx.inner.data.get() {
                None => {},
                _ => { assert!(false); },
            }
        }
    }
    
    #[test]
    fn test_receiver_try_receive_empty() {
        #[allow(unused_variables)]
        let (tx, rx) = channel::<TaskData>();

        match rx.try_receive() {
            Err(TryReceiveError::Empty) => {},
            _ => { assert!(false); },
        };
    }

    #[test]
    fn test_receiver_receive() {
        let (tx, rx) = channel();

        tx.send(TaskData::NoTasks).unwrap();

        #[allow(unused_variables)]
        match rx.receive() {
            TaskData::NoTasks => {},
            _ => { assert!(false); },
        };
    }

    #[test]
    fn test_send_receive_threaded() {
        let (tx, rx) = channel();

        let var = Arc::new(AtomicUsize::new(0));
        let var2 = var.clone();

        tx.send(TaskData::OneTask(Box::new(move || {
            var2.fetch_add(1, Ordering::SeqCst);
        }))).unwrap();
        
        let handle = thread::spawn(move || {
            match rx.receive() {
                TaskData::OneTask(task) => {
                    task.call_box();
                },
                _ => { assert!(false); },
            }
        });

        handle.join().unwrap();

        assert_eq!(var.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_send_threaded_receive() {
        let (tx, rx) = channel();

        let var = Arc::new(AtomicUsize::new(0));
        let var2 = var.clone();
        
        let handle = thread::spawn(move || {
            tx.send(TaskData::OneTask(Box::new(move || {
                var2.fetch_add(1, Ordering::SeqCst);
            }))).unwrap();
        });

        match rx.receive() {
            TaskData::OneTask(task) => {
                task.call_box();
            },
            _ => { assert!(false); },
        }

        handle.join().unwrap();

        assert_eq!(var.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_clone_sender() {
        
    }
}
