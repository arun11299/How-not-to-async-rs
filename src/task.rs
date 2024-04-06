#![allow(dead_code)]
use std::cell::UnsafeCell;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::{Release, Acquire, AcqRel};
use std::sync::Arc;
use std::mem;

/// RawWaker allows the implementor of the task executor to create
/// cutomized versions of Waker by implementing the function pointers in
/// the vtable.
/// Waker is passed down to the future instances by the executor, so it makes
/// sense that executor providers will have their own implementations of the Waker.
/// That means wakers are tightly coupled with the executor and a custom waker provided
/// by the user might not work well with another executor.
/// 
/// Why is this not a trait ? Because, if we implement this as a trait, then we need
/// to define a clone method. The close method has return of `Self` which cannot be used in an
/// object safe manner.
#[derive(PartialEq, Debug)]
pub struct RawWaker {
    /// A data pointer like void* in C, to store arbitrary data as
    /// required by the _executor_.This could be a type erased pointer
    /// to an Arc that is associated to a task or a task identifier which
    /// is associatively mapped to the task instance by the _executor_.
    data: *const (),
    vtable: &'static RawWakerVTable,
}

impl RawWaker {
    #[inline]
    pub const fn new(data: *const (), vtable: &'static RawWakerVTable) -> RawWaker {
        RawWaker{data, vtable}
    }
    #[inline]
    pub const fn data(&self) -> *const() {
        self.data
    }
    #[inline]
    pub const fn vtable(&self) -> &'static RawWakerVTable {
        self.vtable
    }
}

#[derive(PartialEq, Copy, Clone, Debug)]
pub struct RawWakerVTable {
    /// This function will be get cloned when the RawWaker gets cloned eg:
    /// when the Waker holding the RawWaker gets cloned.
    /// 
    /// The implementation for this must ensure that all resources that are
    /// required for this additional instance and associated task are retained.
    /// Calling wake onm the resulting RawWaker should result in waking
    /// of the same task that would have been awoken by the original RawWaker.
    clone: unsafe fn (*const ()) -> RawWaker,
    /// This function will be called when wake in called on the Waker.
    /// It must wake up the task associated with the RawWaker.
    /// 
    /// The implementation must make sure to release any resources that are associated
    /// with this instance of the RawWaker and the associated task.
    wake: unsafe fn (*const ()),
    /// This function will be called when wake_by_ref is called on the Waker.
    /// It must wake up the task associated with the RawWaker.
    /// 
    /// This function is similar to wake but must not consume the provided data pointer.
    wake_by_ref: unsafe fn(*const ()),
    /// This function gets called when the Waker gets dropped.
    /// 
    /// The implementation must release the resources associated with this instance
    /// of RawWaker and the associated task.
    drop: unsafe fn(*const ()),
}

impl RawWakerVTable {
    pub const fn new(
        clone: unsafe fn(*const ()) -> RawWaker,
        wake: unsafe fn(*const ()),
        wake_by_ref: unsafe fn(*const ()),
        drop: unsafe fn(*const ())
    ) -> Self {
        RawWakerVTable{clone, wake, wake_by_ref, drop}
    }
}

/// Passed down to future with poll call.
pub struct Context<'a> {
    waker: &'a Waker
}

impl<'a> Context<'a> {
    #[inline]
    pub const fn from_waker(waker: &'a Waker) -> Self {
        Context{waker}
    }
    #[inline]
    pub const fn waker(&self) -> &'a Waker {
        self.waker
    }
}

/// Waker serves as a notification machanism.
/// As we will see that the complete asynchronous execution world is divided into 3 parts:
///     a. The async computation code. User provided.
///     b. The async task/future driver scheduler. It polls the future to drive it to completetion.
///     c. The IO event loop. It notifies the scheduler(#b) to poll the task/future for which it got readiness event.
/// 
/// Tokio for #c uses mio event loop for example. We will be using mio for our example as well. The job of the Waker
/// is the notification part from event loop to the scheduler.
/// 
/// A waker potentially can get created for each and every call to Future::poll. So, a custom implementation of a waker
/// must be very cheap to create.
pub struct Waker {
    waker: RawWaker
}

impl Unpin for Waker {}

impl Waker {
    #[inline]
    pub fn wake(self) {
        let wake = self.waker.vtable.wake;
        let data = self.waker.data;

        std::mem::forget(self);

        unsafe { (wake)(data) };
    }
    #[inline]
    pub fn wake_by_ref(&self) {
        unsafe { (self.waker.vtable.wake_by_ref)(self.waker.data) }
    }
    #[inline]
    pub const unsafe fn from_raw(waker: RawWaker) -> Waker {
        Waker{waker}
    }
    #[inline]
    pub fn will_wake(&self, other: &Waker) -> bool {
        self.waker == other.waker
    }
}

impl Clone for Waker {
    fn clone(&self) -> Self {
        Waker {
            waker: unsafe { (self.waker.vtable.clone)(self.waker.data) } 
        }
    }
}

impl Drop for Waker {
    fn drop(&mut self) {
        unsafe { (self.waker.vtable.drop)(self.waker.data) }
    }
}

/// AtomicWaker
/// This is mostly as it is from the futures-task library.
/// This is a very handly implementation so that a waker can be used from multiple threads.
/// A very common use case for this is when you want to notifiy the scheduler to proceed with the
/// task/future from another thread. That means you need a thread safe way to set the waker and call its
/// wake function in a thread safe manner.
/// Another immportnt thing relates to the fact that we mentioned above i.e a new waker instance can be 
/// constructed for each poll, so how do we correctly access the waker from the notifying thread when the waker
/// which was registered with is going to get replaced by another? AtomicWaker
/// takes care of that synchronization and making sure that the old waker is not left without calling `wake` on them.
/// 
/// We are redoing the implementation here using atomics. Using Mutex would have been a lot easier for our understanding, 
/// bnut this is a lot more fun.
pub struct AtomicWaker {
    state: AtomicUsize,
    waker: UnsafeCell<Option<Waker>>,
}

const WAITING: usize = 0;
const REGISTERING: usize = 0b01;
const WAKING: usize = 0b10;

impl AtomicWaker {
    #[inline]
    pub const fn new() -> Self {
        AtomicWaker {
            state: AtomicUsize::new(WAITING),
            waker: UnsafeCell::new(None),
        }
    }

    ///  Sets the clone of the passed waker into AtomicWaker.
    /// 
    /// A successful state trasition for registering comprises of 2 steps:
    ///     1. Locking: WAITING -> REGISTERING
    ///     2. Unlocking: REGISTERING -> WAITING
    /// 
    /// A call to register can race with:
    ///     1. Call to race from other threads calling register.
    ///         In this case the successful thread gets to set their waker.
    pub fn register(&self, waker: &Waker) {
        // Try atomically setting the state from WAITING to REGISTERING. If it cannot be done, return the current state.
        match self
            .state
            .compare_exchange( WAITING, REGISTERING, Acquire, Acquire)
            .unwrap_or_else(|x| x)
        {
            WAITING => {
                unsafe {
                    // We have successfully acquired the lock.

                    // Avoid cloning the waker if both the existing waker and the
                    // new waker points to the same task.
                    match &*self.waker.get() {
                        Some(old_waker) if old_waker.will_wake(waker) => (),
                        _ => {
                            // Either the waker is not set or they do point to the same task.
                            *self.waker.get() = Some(waker.clone());
                        }
                    }
                    // We have now a valid or latest waker set. Now unlock.
                    let unlock_state = self.state.compare_exchange(REGISTERING, WAITING, AcqRel, Acquire);
                    match unlock_state {
                        Ok(_) => {
                            // Unlocking has been successful
                        },
                        Err(actual) => {
                            // This can only happen if there was another thread which called a wake
                            // and failed to do so. So it has left its state in WAKING atomically or-ed.
                            debug_assert_eq!(actual, REGISTERING | WAKING);

                            let waker = (*self.waker.get()).take().unwrap();
                            self.state.swap(WAITING, AcqRel);
                            waker.wake();
                        }
                    }

                }
            }
            WAKING => {
                // There was another thread which was successful in getting the lock
                // for waking. So, it has taken ownership of the previously set waker.
                //
                // We will now just call the wake_by_ref on the provided waker
                waker.wake_by_ref();
            }
            _ => {}
        }
    }

    pub fn wake(&self) {
        if let Some(waker) = self.take() {
            waker.wake();
        }
    }

    /// Moves out the waker from the AtomicWaker instance on a successfull
    /// call to wake.
    /// 
    /// This can be called from multiple threads at the same time safely,
    /// but only one thread can actually get the owned waker instance to call
    /// wake on it.
    /// The expected transition is from WAITING -> WAKING. This is done ny atmoically
    /// or-ing the state with 'WAKING' state. If the previous state was not 'WAITING' then
    /// it either means that some other thread was successful in getting the owned instance
    /// OR there is another thread doing a registration in which case the "register" method will
    /// call wake on the existing instance. That makes this function relatively easy to implement
    /// and guarantee multithreading soundness.
    pub fn take(&self) -> Option<Waker> {
        match self.state.fetch_or(WAKING, AcqRel) {
            WAITING => {
                // It is safe to get the waker instance now since we have obtained
                // the lock to safely take the waker
                let waker = unsafe { (*self.waker.get()).take() };
                // Release the lock
                self.state.fetch_and(!WAKING, Release);
                waker
            },
            state => {
                // There is concurrent thread currently updating the associated task.
                // We are not going to set any state here. The state is set to WAKING.
                debug_assert!(
                    state == REGISTERING || state == REGISTERING | WAKING || state == WAKING
                );
                None
            },
        }
    }
}

impl Default for AtomicWaker {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl Send for AtomicWaker {}
unsafe impl Sync for AtomicWaker {}


/// ArcWake
/// In the wake implementation we have seen how it has a space for storing a pointer data.
/// Some implementation of executors can take advantage of this by storing the Task reference in it.
/// The "Task" structure itself is not standadized, different implementations of the executor can
/// implement different structure for that and place it inside an Arc.
/// 
/// ArcWake is a trait to reduce boilerplate code to implent such Arc<T> structures into a wake.
/// i.e. Given a arc of task, how to create a waker by not having to implement the boiler plate for RawWakerVTable.
pub trait ArcWake: Send + Sync {
    /// Indicates that the associated task is ready to make progress and should be `poll`ed.
    /// 
    /// This function can be called from arbitrary thread, including threads which
    /// did not create ArcWake based Waker.
    fn wake(self: Arc<Self>) {
        Self::wake_by_ref(&self)
    }

    /// Executors usually maintain a queue of "ready" tasks; wake_by_ref should
    /// place the associated task onto the queue.
    fn wake_by_ref(arc_self: &Arc<Self>);
}

fn arc_raw_vtable<T: ArcWake>(_data: *const ()) -> &'static RawWakerVTable {
    &RawWakerVTable {
        clone: clone_arc_raw::<T>,
        wake: wake_arc_raw::<T>,
        wake_by_ref: wake_by_ref_arc_raw::<T>,
        drop: drop_arc_raw::<T>,
    }
}

/// Create a Waker from Arc<impl ArcWake>
pub fn arc_waker<T>(arc_task: Arc<T>) -> Waker
where
    T: ArcWake + 'static
{
    let ptr = Arc::into_raw(arc_task).cast::<()>();
    unsafe { Waker::from_raw(RawWaker::new(ptr, arc_raw_vtable::<T>(ptr))) }
}

/// Waker vtable functions
/// 
unsafe fn increase_refcount<T: ArcWake>(data: *const ()) {
    // Arc is based on RAII or drop trait in Rust. It automatically
    // decreases the reference count when it goes out of scope.
    // Since we want to keep up the reference count even after returning,
    // we need to turn off the automatic drop using ManuallyDrop.
    let arc = mem::ManuallyDrop::new(Arc::<T>::from_raw(data.cast::<T>()));
    let _arc_clone = arc.clone();
}

unsafe fn clone_arc_raw<T: ArcWake>(data: *const ()) -> RawWaker {
    increase_refcount::<T>(data);
    RawWaker::new(data, arc_raw_vtable::<T>(data))
}

unsafe fn wake_arc_raw<T: ArcWake>(data: *const ()) {
    ArcWake::wake(Arc::<T>::from_raw(data.cast::<T>()))
}

unsafe fn wake_by_ref_arc_raw<T: ArcWake>(data: *const ()) {
    let arc = mem::ManuallyDrop::new(Arc::<T>::from_raw(data.cast::<T>()));
    ArcWake::wake_by_ref(&arc)
}

unsafe fn drop_arc_raw<T: ArcWake>(data: *const ()) {
    // cound we have just created an arc and left it without calling drop ?
    drop(Arc::<T>::from_raw(data.cast::<T>()))
}