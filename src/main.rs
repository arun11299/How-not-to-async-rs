use std::{sync::WaitTimeoutResult, time};
use std::pin::Pin;
use future::{Future, LocalFutureObj};
use spawn::Spawn;

mod future;
mod task;
mod spawn;
mod thread_pool;
mod net;

fn say(msg: String) -> impl future::Future<Output=()> {
    println!("Simon says {}", msg);
    future::Ready(Some(()))
}

enum Fstate {
    Start,
    F1Done,
    F2Done,
}

struct SayTwiceFuture<F> {
    f1:  F,
    f2: F,
    state: Fstate,
}

impl<F> Unpin for SayTwiceFuture<F> {}

impl<F> SayTwiceFuture<F> 
where
    F: future::Future
{
    fn new(f1: F, f2: F) -> Self {
        SayTwiceFuture{
            f1,
            f2,
            state: Fstate::Start,
        }
    }
}

impl<F> future::Future for SayTwiceFuture<F>
where
    F: future::Future + Unpin
{
    type Output = ();

    fn poll(mut self: std::pin::Pin<&mut Self>, ctx: &mut task::Context<'_>) -> future::Poll<Self::Output> {
        match self.state {
            Fstate::Start => {
                let res = Pin::new(&mut self.f1).poll(ctx);
                match res {
                    future::Poll::Ready(_) => Fstate::F1Done,
                    future::Poll::Pending => Fstate::Start
                }
            },
            Fstate::F1Done => {
                let res = Pin::new(&mut self.f2).poll(ctx);
                match res {
                    future::Poll::Ready(_) => Fstate::F2Done,
                    future::Poll::Pending => Fstate::F1Done,
                }
            },
            Fstate::F2Done => {
                return future::Poll::Ready(());
            },
        };

        future::Poll::Pending
    }
}

fn say_twice(msg: String) -> impl future::Future<Output=()> {
    // Sadly we cannot call await here since compiler for desugaring requires
    // that we implement the standard Future trait instead of our own.
    // I do not know of any other way, but that gives us an opportunity to
    // write our own state machine!
    let f1 = say(msg.clone());
    let f2 = say(msg.clone());
    SayTwiceFuture::new(f1, f2)
}

fn accept_sock_connection(ios: &net::io_service::IoService) -> impl Future<Output=()> {
    let acceptor = net::socket::AsyncAcceptor::new("127.0.0.1:8010".to_string());
    acceptor.async_accept(ios)
}

fn main() {
    let ios = net::io_service::IoService::new(2);
    let f = Box::pin(say_twice("hi".to_string()));
    let f = future::LocalFutureObj::<'static, ()>::new(f);
    let _ = ios.tp.spawn_obj(f);

    let sock_f = Box::pin(accept_sock_connection(&ios));
    let sock_f = LocalFutureObj::<'static, ()>::new(sock_f);
    let _ = ios.tp.spawn_obj(sock_f);

    ios.run();
}