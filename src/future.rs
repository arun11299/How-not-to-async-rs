#![allow(dead_code)]

use std::marker::PhantomData;
use std::pin::Pin;
use std::mem;
use crate::task::Context;

/// Poll
/// The status of the future when polled. The future
/// can only be in one of the 2 states. Ready, either with success or failure
/// OR Pending.
/// A future can get polled multiple times even when in Pending state.
/// A poll which is Ready means that the async computation task has finished.
/// Whether it finished successfully or not does not matter to Poll. The generic type
/// parameter `T` should be able to convey that. For eg: `T` can be Option<T> or Result<T, dyn Error> etc.
/// 
/// The `Poll` type itself has nothing to with the async computation. The magic is performed
/// within the `Future::poll` call.
pub enum Poll<T> {
    Ready(T),
    Pending,
}

/// Some helper functions which is not very interesting or would be even used
/// in our examples.
impl<T> Poll<T> {
    pub fn map<F, U>(self, f: F) -> Poll<U>
    where
        F: FnOnce(T) -> U,
    {
        match self {
            Poll::Ready(t) => Poll::Ready(f(t)),
            Poll::Pending => Poll::Pending
        }
    }

    pub const fn is_ready(&self) -> bool {
        matches!(self, Poll::Ready(_))
    }

    pub const fn is_pending(&self) -> bool {
        matches!(self, Poll::Pending)
    }
}

impl<T, E> Poll<Result<T, E>> {
    pub fn map_ok<F, U>(self, f: F) -> Poll<Result<U, E>>
    where
        F: FnOnce(T) -> U
    {
        match self {
            Poll::Ready(Ok(t)) => Poll::Ready(Ok(f(t))),
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            _ => Poll::Pending,
        }
    }

    pub fn map_err<F, U>(self, f: F) -> Poll<Result<T, U>>
    where
        F: FnOnce(E) -> U,
    {
        match self {
            Poll::Ready(Ok(t)) => Poll::Ready(Ok(t)),
            Poll::Ready(Err(e)) => Poll::Ready(Err(f(e))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<T, E> Poll<Option<Result<T, E>>> {
    pub fn map_ok<F, U>(self, f: F) -> Poll<Option<Result<U, E>>>
    where
        F: FnOnce(T) -> U,
    {
        match self {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Ready(Some(Ok(t))) => Poll::Ready(Some(Ok(f(t)))),
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e)))
        }
    }

    pub fn map_err<F, U>(self, f: F) -> Poll<Option<Result<T, U>>>
    where
        F: FnOnce(E) -> U,
    {
        match self {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Ready(Some(Ok(t))) => Poll::Ready(Some(Ok(t))),
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(f(e))))
        }
    }
}

impl<T> From<T> for Poll<T> {
    fn from(value: T) -> Self {
        Poll::Ready(value)
    }
}

/// `Future` represents the state of any asynchronous operation.
/// The `Future` object will hold the result of the async operation (whether success or failure)
/// when the async operation runs to completion.
/// NOTE: "Hold" is probably not the correct word here since future is not required to "store"
/// the output value. It can just return it when the async operation finishes.
/// 
/// The `Future` model is _poll_ based i.e there needs to be a driver to run the async operation 
/// to completion and this is done by calling the `poll` method on the future.
/// 
/// The `poll` method can either return Poll::Pending or Poll::Ready(T) where T is the output from the
/// async computation.
/// `Poll` method needs to be called atleast once so as to prime it i.e to make sure that the completion
/// of the async operation is notified.
pub trait Future {
    // The type of value returned by the async operation.
    type Output;
    fn poll(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Self::Output>;
}

impl<F: Future + Unpin> Future for &mut F {
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut **self).poll(ctx)
    }
}


/// A future which is always ready.
/// Not used in our examples!
pub struct Ready<T>(pub Option<T>);

impl<T> Unpin for Ready<T> {}

impl<T> Future for Ready<T> {
    type Output = T;

    #[inline]
    fn poll(mut self: Pin<&mut Self>, _ctx: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(self.0.take().expect("Ready polled after completion"))        
    }
}

/// Future is a trait object. There are times when we may need to 
/// create new traits having APIs which takes Future as parameter.
/// As of today, impl Trait is not allowed as a parameter due to object safety
/// reasons, since you cannot dynamically dispatch the function. Similar to why
/// in C++ you cannot have a template function defined as virtual.
/// 
/// Another way is to use Box<dyn Future>, but this cannot be used in performance
/// sensitive applications and also in no_std environment.
/// 
/// Thus we need to expose the Future in its raw pointer form stored inside a
/// struct as an object. And for that, we need to be able to convert the Future like
/// type into its pointer form. Hence this trait. Future like objects can implement this trait
/// and then can be used inside a structure. This is what UnsafeFutureObj trait does.
/// 
/// We really did not have to define this trait and could have directly used unsafe inside
/// LocalFutureObj implementation. We are going with this as it is interesting pattern that have
/// actually come up in different forms (Waker..) and also it is already implemented for us in the futures library.
pub unsafe trait UnsafeFutureObj<'a, T>: 'a {
    /// Convert an owned instance into a (conceptually owned) fat pointer.
    /// 
    /// The trait implementor must guarantee that its safe to convert
    /// *mut (dyn Future<Output=T> + 'a) into Pin<&mut (dyn Future<Output=T> + 'a)>
    /// and call methods on it, non-reentrantly, until drop is called.
    fn into_raw(self) -> *mut (dyn Future<Output = T> + 'a);

    /// Drops the future represented by the given fat pointer.
    /// 
    /// The caller must ensure:
    ///     a. The pointer passed is the one passed to into_raw.
    ///     b. It is not being used by any Pin<&mut (dyn Future<Output=T>+'a)>
    ///     c. The pointer must not be used again after invoking this function.
    unsafe fn drop(ptr: *mut (dyn Future<Output=T> + 'a));
}

/// Implement UnsafeFutureObj for all the common pointer types for Future.
/// This makes it easier to construct a LocalFutureObj which we will see later.
unsafe impl<'a, T, F> UnsafeFutureObj<'a, T> for &'a mut F
where
    F: Future<Output = T> + Unpin + 'a,
{
    fn into_raw(self) -> *mut (dyn Future<Output = T> + 'a) {
        self as *mut dyn Future<Output = T>
    }

    unsafe fn drop(_ptr: *mut (dyn Future<Output = T> + 'a)) {}
}

unsafe impl<'a, T> UnsafeFutureObj<'a, T> for &'a mut (dyn Future<Output = T> + Unpin + 'a) {
    fn into_raw(self) -> *mut (dyn Future<Output = T> + 'a) {
        self as *mut dyn Future<Output = T>
    }

    unsafe fn drop(_ptr: *mut (dyn Future<Output = T> + 'a)) {}
}

unsafe impl<'a, T, F> UnsafeFutureObj<'a, T> for Pin<&'a mut F>
where
    F: Future<Output = T> + 'a,
{
    fn into_raw(self) -> *mut (dyn Future<Output = T> + 'a) {
        unsafe { self.get_unchecked_mut() as *mut dyn Future<Output = T> }
    }

    unsafe fn drop(_ptr: *mut (dyn Future<Output = T> + 'a)) {}
}

unsafe impl<'a, T> UnsafeFutureObj<'a, T> for Pin<&'a mut (dyn Future<Output = T> + 'a)> {
    fn into_raw(self) -> *mut (dyn Future<Output = T> + 'a) {
        unsafe { self.get_unchecked_mut() as *mut dyn Future<Output = T> }
    }

    unsafe fn drop(_ptr: *mut (dyn Future<Output = T> + 'a)) {}
}

unsafe impl<'a, T, F> UnsafeFutureObj<'a, T> for Box<F>
where
    F: Future<Output = T> + 'a,
{
    fn into_raw(self) -> *mut (dyn Future<Output = T> + 'a) {
        Box::into_raw(self)
    }

    unsafe fn drop(ptr: *mut (dyn Future<Output = T> + 'a)) {
        drop(Box::from_raw(ptr.cast::<F>()))
    }
}

unsafe impl<'a, T: 'a> UnsafeFutureObj<'a, T> for Box<dyn Future<Output = T> + 'a> {
    fn into_raw(self) -> *mut (dyn Future<Output = T> + 'a) {
        Box::into_raw(self)
    }

    unsafe fn drop(ptr: *mut (dyn Future<Output = T> + 'a)) {
        drop(Box::from_raw(ptr))
    }
}

unsafe impl<'a, T: 'a> UnsafeFutureObj<'a, T> for Box<dyn Future<Output = T> + Send + 'a> {
    fn into_raw(self) -> *mut (dyn Future<Output = T> + 'a) {
        Box::into_raw(self)
    }

    unsafe fn drop(ptr: *mut (dyn Future<Output = T> + 'a)) {
        drop(Box::from_raw(ptr))
    }
}

unsafe impl<'a, T, F> UnsafeFutureObj<'a, T> for Pin<Box<F>>
where
    F: Future<Output = T> + 'a,
{
    fn into_raw(self) -> *mut (dyn Future<Output = T> + 'a) {
        let mut this = mem::ManuallyDrop::new(self);
        unsafe { this.as_mut().get_unchecked_mut() as *mut _ }
    }

    unsafe fn drop(ptr: *mut (dyn Future<Output = T> + 'a)) {
        drop(Pin::from(Box::from_raw(ptr)))
    }
}

unsafe impl<'a, T: 'a> UnsafeFutureObj<'a, T> for Pin<Box<dyn Future<Output = T> + 'a>> {
    fn into_raw(self) -> *mut (dyn Future<Output = T> + 'a) {
        let mut this = mem::ManuallyDrop::new(self);
        unsafe { this.as_mut().get_unchecked_mut() as *mut _ }
    }

    unsafe fn drop(ptr: *mut (dyn Future<Output = T> + 'a)) {
        drop(Pin::from(Box::from_raw(ptr)))
    }
}

unsafe impl<'a, T: 'a> UnsafeFutureObj<'a, T> for Pin<Box<dyn Future<Output = T> + Send + 'a>> {
    fn into_raw(self) -> *mut (dyn Future<Output = T> + 'a) {
        let mut this = mem::ManuallyDrop::new(self);
        unsafe { this.as_mut().get_unchecked_mut() as *mut _ }
    }

    unsafe fn drop(ptr: *mut (dyn Future<Output = T> + 'a)) {
        drop(Pin::from(Box::from_raw(ptr)))
    }
}

/// LocalFutureObj
/// As mentioned before this is useful for 2 cases:
///     a. Defining traits which takes a future as parameter without comprimising object safety rules.
///     b. For use cases where Box<Future> cannot be used.
/// 
/// For defining any traits which expects Future as parameter, we will use this object instead.
pub struct LocalFutureObj<'a, T> {
    future: *mut (dyn Future<Output=T> + 'static),
    drop_fn: unsafe fn(*mut (dyn Future<Output=T> + 'static)),
    _marker: PhantomData<&'a()>,
 }

impl<T> Unpin for LocalFutureObj<'_, T> {}

unsafe fn remove_future_lifetime<'a, T>(
    ptr: *mut (dyn Future<Output=T> + 'a)) -> *mut (dyn Future<Output=T> +'static)
{
    std::mem::transmute(ptr)
}

unsafe fn remove_drop_lifetime<'a, T>(
    ptr: unsafe fn(*mut (dyn Future<Output=T> + 'a))) -> unsafe fn(*mut (dyn Future<Output=T> + 'static))
{
    std::mem::transmute(ptr)
}

impl<'a, T> LocalFutureObj<'a, T> {
    #[inline]
    pub fn new<F: UnsafeFutureObj<'a, T>>(f: F) -> Self {
        LocalFutureObj {
            future: unsafe {remove_future_lifetime(f.into_raw())},
            drop_fn: unsafe {remove_drop_lifetime(F::drop)},
            _marker: PhantomData
        }
    }
}

impl<T> Future for LocalFutureObj<'_, T> {
    type Output = T;
    fn poll(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Self::Output> {
        let inner_future = unsafe {Pin::new_unchecked(&mut *self.future)};
        inner_future.poll(ctx)
    }
}

impl<T> Drop for LocalFutureObj<'_, T> {
    fn drop(&mut self) {
        unsafe { (self.drop_fn)(self.future) }
    }
}