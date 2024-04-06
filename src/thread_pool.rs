#![allow(dead_code)]

use crate::future::{Poll, Future, LocalFutureObj};
use crate::task::{arc_waker, ArcWake, Context};
use crate::spawn::{Spawn, SpawnError};
use std::thread;
use std::sync::{Arc, Mutex, mpsc };
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::AtomicUsize;
use std::pin::Pin;
use std::cell::UnsafeCell;

/// ThreadPool
/// A lot of the code is taken from futures-executor library/crate.
/// 
/// Our executor of the tasks backed by threadpool.
pub struct ThreadPool {
    /// state to store the channels. Shared amongst multiple worker threads.
    state: Arc<PoolState>,
    pool_size: usize,
    cnt: AtomicUsize,
}

/// PoolState
/// We will use the MPSC sender and received for communication between
/// other domains (mio event loop, user code) and the scheduler.
/// PoolState basically implements the scheduler functionality. A more rubust scheduler
/// implemetation will probably have more queues like runnable, pending etc.
/// 
/// A good example of a scheduler woould be the Go scheduler. I think Tokio scheduler
/// also have a lot of similarities. For eg: both implement a global queue besides per thread
/// local queues. Also, I think they both implement work stealing so as to avoid starvation of some threads.
/// 
struct PoolState {
    tx: Mutex<mpsc::Sender<Message>>,
    rx: Mutex<mpsc::Receiver<Message>>,
}

unsafe impl Send for PoolState {}
unsafe impl Sync for PoolState {}

impl PoolState {
    pub fn send(&self, msg: Message) {
        self.tx.lock().unwrap().send(msg).unwrap()
    }

    pub fn work(&self, _cnt: usize) {
        loop {
            // Receive a task and execute the run method on it.
            // Task::run will take care about the messy details of calling Future::poll
            let msg = self.rx.lock().unwrap().recv().unwrap();
            match msg {
                Message::Run(task) => task.run(),
                Message::Close => break,
            }
        }
    }
}

enum Message {
    Run(Task),
    Close,
}

/// Task
/// Most important data structure in our scheduler.
/// Task::run will poll the future. Thus it is responsible for
/// creating the waker and the context object.
struct Task {
    future: LocalFutureObj<'static, ()>,
    wake_handle: Arc<WakeHandle>,
    exec: ThreadPool,
}

struct WakeHandle {
    // The reason for this mutex is described below where we use it.
    guard: Mutex<()>,
    // UnsafeCell because we need mutable access to the rewrite this member.
    // But that is not possible when using Arc. Arc will not give a mutable to its inner.
    task: UnsafeCell<Option<Task>>
}

/// Required for ArcWake
unsafe impl Send for WakeHandle {}
unsafe impl Sync for WakeHandle {}

impl ArcWake for WakeHandle {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        // Need a lock sync between here and task::run
        let _g = arc_self.guard.lock().unwrap();
        let t = unsafe { &mut *arc_self.task.get() }.take().unwrap();
        let exec = t.exec.clone();
        exec.state.send(Message::Run(t));
    }
}

impl Task {
    /// Called by the executor/scheduler. The same task thus could be called multiple
    /// times till the inner future runs to completion.
    /// The task in one way or another needs to be rescheduled back on the executor
    /// as the future makes progress. This has to be done by the future by calling the
    /// wake method on the waker.
    /// What is waker here ? 
    /// We could have used Arc<Task> as the waker and the future inside a UnsafeCell
    /// since we need to get a mutable reference to an object inside Arc; which provides no
    /// way to access such a mutable reference. Hence we ned to go unsafe to get that.
    /// Here we just created another abstraction/indirection in the form of `WakerHandle`. This way we can
    /// use the future object inside task more naturally as we will see.
    /// 
    /// The problem comes in the form of creating a waker instance, which when passed along with
    /// the poll call could be used in another thread based on what the poll implementation does. For eg:
    /// it could potentially spin off another thread to call wake on it. This is what the lock guard prevents
    /// by synchronizing access to the WakeHandle's task member.
    fn run(self) {
        // Future is morved out here. Hence we should prevent the waker.wake
        // being called parallely while the task has not future member set.
        // This can happen because WakerHandle also stores the Task.
        let mut future = self.future;
        let wake_handle = self.wake_handle;
        let waker = arc_waker(wake_handle.clone());
        let mut ctx = Context::from_waker(&waker);

        // Need to call this lock before invoking poll
        // Otherwise, if the future is still pending it can
        // invoke waker on another thread and call wake on it pre-maturely without
        // having a task to continue its computation.
        let _g = wake_handle.guard.lock().unwrap();

        let res = Pin::new(&mut future).poll(&mut ctx);
        match res {
            Poll::Ready(()) => { println!("Future completed!"); },
            Poll::Pending => { 
                println!("Future pending..");
                // Set the task which can be schedules on the scheduler when wake
                // method gets called.
                let task = Task{
                    future,
                    wake_handle: wake_handle.clone(),
                    exec: self.exec.clone(),
                };
                unsafe { *wake_handle.task.get() = Some(task); }
            },
        }

    }
}

impl ThreadPool {
    pub fn new(pool_size: usize) -> Result<Self, std::io::Error> {
        let (tx, rx) = mpsc::channel();
        let tp = ThreadPool {
            state: Arc::new(PoolState {
                tx: Mutex::new(tx),
                rx: Mutex::new(rx),
            }),
            pool_size,
            cnt: AtomicUsize::new(0),
        };

        for counter in 0..pool_size {
            let ps = tp.state.clone();
            thread::spawn(move || ps.work(counter));
        }

        Ok(tp)
    }
}

impl Spawn for ThreadPool {
    fn spawn_obj(&self, future: LocalFutureObj<'static, ()>) -> Result<(), SpawnError> {
        let task = Task{future, wake_handle: Arc::new(WakeHandle{guard: Mutex::new(()), task: UnsafeCell::new(None)}), exec: self.clone()};
        self.state.send(Message::Run(task));
        Ok(())        
    }
}

impl Clone for ThreadPool {
    fn clone(&self) -> Self {
        let cnt = self.cnt.fetch_add(1, Relaxed);
        ThreadPool {
            state: self.state.clone(),
            pool_size: self.pool_size,
            cnt: AtomicUsize::new(cnt + 1),
        }
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        if self.cnt.fetch_sub(1, Relaxed) == 1 {
            self.state.send(Message::Close);
        }
    }
}

unsafe impl Send for ThreadPool {}
unsafe impl Sync for ThreadPool {}