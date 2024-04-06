#![allow(dead_code)]
use std::time::Duration;
use std::{io, thread};
use std::sync::{Arc, Mutex};
use std::sync::atomic::AtomicUsize;
use std::cell::UnsafeCell;
use mio::{event, Interest, Poll};
use mio::Events;
use std::collections::HashMap;

use crate::task::AtomicWaker;
use crate::thread_pool::ThreadPool;
use crate::task::Waker;

/// IoService
/// I copied the C++ io_service name here.
/// This clubs together the scheduler and the event loop under a single struct.
/// Ideally a multithreaded async runtime would create a single thread to run both scheduler
/// and the event loop. But in this example we are running the event-loop in its own thread. Hence,
/// a lot of additional synchronization is required.
/// A more efficient way would be for each thread to have its own scheduler and evet loop running together.
pub struct IoService {
    // This is going to cause a lot of pain as it is an inefficient way to do this.
    // For the time being, using a Mutex so that we can call Poll::register from different threads.
    // It can be from different threads because, as per our threading model, the future scheduler
    // runs on its own thread pool. Hence the Futures can be called from any of those threads concurrently.
    // The only time we will not be holding a lock is when Poll::run is executing the events. The lock cannot
    // be taken to register new sources if the poll is blocked in an underlying epoll_wait call.
    state: Arc<Mutex<State>>,
    pub tp: ThreadPool,
}

impl Clone for IoService {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            tp: self.tp.clone(),
        }
    }
}

struct State {
    poll: UnsafeCell<Poll>,
    token: AtomicUsize,
    waker_map: HashMap<usize, AtomicWaker>,
}

unsafe impl Send for IoService {}

impl IoService {
    pub fn new(scheduler_tp_size: usize) -> Self {
        IoService {
            state: Arc::new(
                Mutex::new(
                    State {
                        poll: UnsafeCell::new(Poll::new().unwrap()),
                        token: AtomicUsize::new(0),
                        waker_map: HashMap::new(),
                    }
                )
            ),
            tp: ThreadPool::new(scheduler_tp_size).unwrap(),
        }
    }

    /// register the IoSource along with the waker which knows which task
    /// to wake up.
    pub fn register<S>(&self, source: &mut S, waker: &'_ Waker)
    where
        S: event::Source
    {
        let aw = AtomicWaker::new();
        aw.register(waker);

        let mut guard = self.state.lock().unwrap();
        let token = guard.token.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        guard.waker_map.insert(token, aw);
        guard.poll.get_mut().registry().register(source, mio::Token(token), Interest::READABLE).unwrap();
    }

    pub fn run(&self) {
        let state = self.state.clone();
        let th = thread::spawn(
            move || {
                let mut events = Events::with_capacity(128);

                loop {
                    // Get mutable access to inner poll
                    let mut _guard = state.lock().unwrap();
                    let poll = _guard.poll.get_mut();

                    // We are adding a 2ms timeout for giving any pending register calls enough time to get access
                    // to the lock for adding new sources (or removing...)
                    if let Err(err) = poll.poll(&mut events, Some(Duration::from_millis(2))) {
                        if err.kind() == io::ErrorKind::Interrupted {
                            continue
                        }
                    }
                    // Free up the lock for others
                    drop(_guard);

                    for event in events.iter() {
                        let token = event.token();
                        println!("Got token");
                        let waker = state.lock().unwrap();
                        let waker = waker.waker_map.get(&token.0).unwrap();
                        // Wake up the future which may call "register" method on the poll instance
                        // by taking the lock.
                        waker.wake();
                    }
                }
            }
        );
        th.join().unwrap();
    }
}