#![allow(dead_code)]
use std::net::SocketAddr;
use mio::net::TcpListener;
use std::pin::Pin;
use crate::future::{Future, Poll};
use crate::task::Context;
use super::io_service::IoService;
use std::rc::Rc;

/// AsyncAcceptor
/// An async socket acceptor. Provides the required mechanics to await on an Future.
/// NOTE: Actual awaiting may not be possible since we are using our own version of the Future.
/// A call to `accept` will basically create a `Acceptor` future.
pub struct AsyncAcceptor {
    listener: Rc<TcpListener>,
}

pub struct AcceptFuture {
    ios: IoService,
    listener: Rc<TcpListener>,
}

impl AsyncAcceptor {
    pub fn new(addr: String) -> Self {
        let addr = addr.parse::<SocketAddr>().unwrap();
        Self {
            listener: Rc::new(TcpListener::bind(addr).unwrap()),
        }
    }

    pub fn async_accept(&self, ios: &IoService) -> AcceptFuture {
        AcceptFuture{
            ios: ios.clone(),
            listener: self.listener.clone(),
        }
    }
}

impl Future for AcceptFuture {
    // See explanation in spawn.rs on why it is being set to () instead of
    // a proper return type like Result<mio::net::TcpStream, std::io::Error>
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Self::Output> {
        println!("Acceptor polled....");
        let ios = self.ios.clone();
        let listener = Rc::get_mut(&mut self.listener).unwrap();
        
        match listener.accept() {
            Ok((_stream, addr)) => {
                println!("Accepted socket connection: {}", addr);
                return Poll::Ready(());
            },
            Err(e) => {
                if e.kind() != std::io::ErrorKind::WouldBlock {
                    println!("Error in accepting: {}", e);
                    return Poll::Ready(());
                }
            },
        };

        ios.register(listener, ctx.waker());
        println!("Socket: Pending for accept....");
        Poll::Pending
    }
}