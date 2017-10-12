`reqchan-rs` defines a channel for requesting and receiving data. 

[Documentation](https://docs.rs/reqchan)

[![Linux Status](https://travis-ci.org/Elzair/reqchan-rs.svg?branch=master)](https://travis-ci.org/Elzair/reqchan-rs)
[![Build status](https://ci.appveyor.com/api/projects/status/mjf748x86ecf0yip?svg=true)](https://ci.appveyor.com/project/Elzair/reqchan-rs)

# Introduction

Each channel has only one requesting end, but it can have multiple responding ends. It is useful for implementing work sharing.

The two ends of the channel are asynchronous with respect to each other, so it is kinda nonblocking. However, if multiple responding ends try to respond to the same request, only one will succeed; the rest will return errors.

# Design

## Overview

`reqchan` is built around the two halves of the channel: `Requester` and `Responder`. Both implement methods, `Requester::try_request()` and `Responder::try_respond()`, that, when succesful, lock their corresponding side of the channel and return contracts. `RequestContract` **requires** the user to either successfully receive a datum or cancel the request. `ResponseContract` requires the user to send a datum. These requirements prevent the system from losing data sent through the channel.

## Locking 

`Responder::try_response()` locks the responding side to prevent other potential responders from responding to the same request. However, `Requester::try_request()` locks the requesting side of the channel to prevent the user from trying to issue multiple outstanding requests. Both locks are dropped when their corresponding contract object is dropped.

## Contracts 

`Requester::try_request()` has to issue a `RequestContract` so the thread of execution does not block waiting for a response. However, that reason does not apply to `Responder::try_response()`. I originally made `Responder::try_response()` send the datum. However, that required the user to have the datum available to send even if it could not be sent, and it required the user to handle the returned datum if it could not be sent. If the datum was, say, half the contents of a `Vec`, this might entail lots of expensive memory allocation. Therefore, I made `Responder::try_response()` return a `ResponseContract` indicating that the responder *could* and *would* respond to the request. This way the user only has to perform the necessary steps to send the datum if the datum must be sent.

# Examples 

## Simple Example

This simple, single-threaded example demonstrates most of the API. The only thing left out is `RequestContract::try_cancel()`.

```rust
extern crate reqchan;

fn main() {
    // Create channel.
    let (requester, responder) = reqchan::channel::<u32>(); 
    
    // Issue request.
    let mut request_contract = requester.try_request().ok().unwrap();
    
    // Respond with number.
    responder.try_respond().ok().unwrap().send(5);
    
    // Receive and print number.
    println!("Number is {}", request_contract.try_receive().ok().unwrap());
}
```

## More Complex Example 

This more complex example demonstrates more "real-world" usage. One thread requests a 'task' (i.e. a closure to run), and the other two threads fight over who gets to respond with their own personal task. Meanwhile, the requesting thread is polling for a task, and if it gets one in time, it runs it. Regardless of whether or not the receiver got a task or timed out, the receiver notifies other threads to stop running, and stops itself.

```rust
extern crate reqchan as chan;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

// Stuff to make it easier to pass around closures.
trait FnBox {
    fn call_box(self: Box<Self>);
}
impl<F: FnOnce()> FnBox for F {
    fn call_box(self: Box<F>) {
        (*self)()
    }
}
type Task = Box<FnBox + Send + 'static>;

// Variable used to test calling a `Task` sent between threads.
let test_var = Arc::new(AtomicUsize::new(0));
let test_var2 = test_var.clone();
let test_var3 = test_var.clone();

// Variable needed to stop `responder` thread if `requester` times out
let should_exit = Arc::new(AtomicBool::new(false));
let should_exit_copy_1 = should_exit.clone();
let should_exit_copy_2 = should_exit.clone();

let (requester, responder) = chan::channel::<Task>();
let responder2 = responder.clone();

// requesting thread
let requester_handle = thread::spawn(move || {
    let start_time = Instant::now();
    let timeout = Duration::new(0, 1000000);
    
    let mut contract = requester.try_request().ok().unwrap();

    loop {
        // Try to cancel request and stop threads if runtime
        // has exceeded `timeout`.
        if start_time.elapsed() >= timeout {
            // Try to cancel request.
            // This should only fail if `responder` has started responding.
            if let Ok(_) = contract.try_cancel() {
                // Notify other threads to stop.
                should_exit.store(true, Ordering::SeqCst);
                break;
            }
        }

        // Try getting `task` from `responder`.
        match contract.try_receive() {
            // `contract` received `task`.
            Ok(task) => {
                task.call_box();
                // Notify other threads to stop.
                should_exit.store(true, Ordering::SeqCst);
                break;
            },
            // Continue looping if `responder` has not yet sent `task`.
            Err(chan::Error::Empty) => {},
            // The only other error is `chan::Error::Done`.
            // This only happens if we call `contract.try_receive()`
            // after either receiving data or cancelling the request.
            _ => unreachable!(),
        }
    }
});

// responding thread 1
let responder_1_handle = thread::spawn(move || {
    let mut tasks = vec![Box::new(move || {
        test_var2.fetch_add(1, Ordering::SeqCst);
    }) as Task];
    
    loop {
        // Exit loop if `receiver` has timed out.
        if should_exit_copy_1.load(Ordering::SeqCst) {
            break;
        }
        
        // Send `task` to `receiver` if it has issued a request.
        match responder2.try_respond() {
            // `responder2` can respond to request.
            Ok(contract) => {
                contract.send(tasks.pop().unwrap());
                break;
            },
            // Either `requester` has not yet made a request,
            // or `responder2` already handled the request.
            Err(chan::Error::NoRequest) => {},
            // `responder2` is processing request..
            Err(chan::Error::AlreadyLocked) => { break; },
            _ => unreachable!(),
        }
    }
});

// responding thread 2
let responder_2_handle = thread::spawn(move || {
    let mut tasks = vec![Box::new(move || {
        test_var3.fetch_add(2, Ordering::SeqCst);
    }) as Task];
    
    loop {
        // Exit loop if `receiver` has timed out.
        if should_exit_copy_2.load(Ordering::SeqCst) {
            break;
        }
        
        // Send `task` to `receiver` if it has issued a request.
        match responder.try_respond() {
            // `responder2` can respond to request.
            Ok(contract) => {
                contract.send(tasks.pop().unwrap());
                break;
            },
            // Either `requester` has not yet made a request,
            // or `responder` already handled the request.
            Err(chan::Error::NoRequest) => {},
            // `responder` is processing request.
            Err(chan::Error::AlreadyLocked) => { break; },
            _ => unreachable!(),
        }
    }
});

requester_handle.join().unwrap();
responder_1_handle.join().unwrap();
responder_2_handle.join().unwrap();

// `num` can be 0, 1 or 2.
let num = test_var.load(Ordering::SeqCst);
println!("Number is {}", num);
```

# Platforms

`reqchan-rs` should Work on Windows and any POSIX compatible system (Linux, Mac OSX, etc.).

`reqchan-rs` is continuously tested on:
  * `x86_64-unknown-linux-gnu` (Linux)
  * `i686-unknown-linux-gnu`
  * `x86_64-unknown-linux-musl` (Linux w/ [MUSL](https://www.musl-libc.org/))
  * `i686-unknown-linux-musl`
  * `x86_64-apple-darwin` (Mac OSX)
  * `i686-apple-darwin`
  * `x86_64-pc-windows-msvc` (Windows)
  * `i686-pc-windows-msvc`
  * `x86_64-pc-windows-gnu`
  * `i686-pc-windows-gnu`

`reqchan-rs` is continuously cross-compiled for:
  * `arm-unknown-linux-gnueabihf`
  * `aarch64-unknown-linux-gnu`
  * `mips-unknown-linux-gnu`
  * `aarch64-unknown-linux-musl`
  * `i686-linux-android`
  * `x86_64-linux-android`
  * `arm-linux-androideabi`
  * `aarch64-linux-android`
  * `i386-apple-ios`
  * `x86_64-apple-ios`
  * `i686-unknown-freebsd`
  * `x86_64-unknown-freebsd`
  * `x86_64-unknown-netbsd`
  * `asmjs-unknown-emscripten`
