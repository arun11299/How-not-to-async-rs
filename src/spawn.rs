#![allow(dead_code)]
use crate::future::LocalFutureObj;
use std::rc::Rc;
use std::sync::Arc;

/// The Spawn trait allows pushing futures into the executor
/// that will run them to completion.
/// 
/// Not really needed for what we are trying to acheive but serves as an example
/// of how a Future can be used in trait APIs using LocalFutureObj.
pub trait Spawn {
    /// We are hardcoding the type of the LocalFutureObj to () here.
    /// This is because the type we have defined for LocalFutureObject is itself incorrect.
    /// The type must have been an associated type in the UnsafeFutureObj trait, but it is part of the type.
    /// This means that, if we use `T` instead of (), the type signature will propagate all the way upto
    /// IoService, thus making it useful for executing only a single type of future.
    fn spawn_obj(&self, future: LocalFutureObj<'static, ()>) -> Result<(), SpawnError>;
    fn status(&self) -> Result<(), SpawnError> {
        Ok(())
    }
}

pub struct SpawnError {
    _priv: (),
}

impl<Sp: ?Sized + Spawn> Spawn for Box<Sp> {
    fn spawn_obj(&self, future: LocalFutureObj<'static, ()>) -> Result<(), SpawnError> {
        (**self).spawn_obj(future)
    }

    fn status(&self) -> Result<(), SpawnError> {
        (**self).status()
    }
}

impl<Sp: ?Sized + Spawn> Spawn for Rc<Sp> {
    fn spawn_obj(&self, future: LocalFutureObj<'static, ()>) -> Result<(), SpawnError> {
        (**self).spawn_obj(future)
    }

    fn status(&self) -> Result<(), SpawnError> {
        (**self).status()
    }
}

impl<Sp: ?Sized + Spawn> Spawn for Arc<Sp> {
    fn spawn_obj(&self, future: LocalFutureObj<'static, ()>) -> Result<(), SpawnError> {
        (**self).spawn_obj(future)
    }

    fn status(&self) -> Result<(), SpawnError> {
        (**self).status()
    }
}