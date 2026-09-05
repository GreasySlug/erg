//! Turns the per-request-type channels into [`Job`]s for the worker pool.
//!
//! The channels stay typed: `Sendable` hands each request kind's params to a
//! channel of its own. But nothing waits on them one thread each any more. A
//! single dispatcher owns every receiver, sleeps until `SendChannels` rings
//! its `wake` channel, then drains the receivers into one job per request and
//! hands the jobs to the pool, whose queue orders them by priority.

use std::marker::PhantomData;
use std::sync::mpsc::{Receiver, TryRecvError};

use erg_compiler::artifact::BuildRunnable;
use erg_compiler::erg_parser::parse::Parsable;
use lsp_types::request::Request;
use serde::Serialize;

use crate::channels::WorkerMessage;
use crate::scheduler::RequestKind;
use crate::server::{Handler, Server};
use crate::thread_pool::Job;

/// A channel has been closed (a `Kill`, or its sender dropped): the services
/// are shutting down, and no request will follow.
#[derive(Debug)]
pub struct Closed;

/// One request type's receiver, able to turn what it holds into jobs.
trait Source<C: BuildRunnable, P: Parsable>: Send {
    /// Moves every request waiting in the channel into `out`.
    fn drain(&mut self, out: &mut Vec<Job<Server<C, P>>>) -> Result<(), Closed>;
}

struct Typed<R: Request, C: BuildRunnable, P: Parsable> {
    receiver: Receiver<WorkerMessage<R::Params>>,
    handler: Handler<Server<C, P>, R::Params, R::Result>,
    priority: u8,
    _marker: PhantomData<fn() -> (C, P)>,
}

impl<R, C, P> Source<C, P> for Typed<R, C, P>
where
    R: Request + 'static,
    R::Params: Send + 'static,
    R::Result: Serialize,
    C: BuildRunnable,
    P: Parsable,
{
    fn drain(&mut self, out: &mut Vec<Job<Server<C, P>>>) -> Result<(), Closed> {
        loop {
            match self.receiver.try_recv() {
                Ok(WorkerMessage::Request(id, params)) => {
                    let handler = self.handler;
                    out.push(Job::new(
                        id,
                        self.priority,
                        move |server: &mut Server<C, P>| {
                            server.execute_request::<R>(id, params, handler)
                        },
                    ));
                }
                Ok(WorkerMessage::Kill) | Err(TryRecvError::Disconnected) => return Err(Closed),
                Err(TryRecvError::Empty) => return Ok(()),
            }
        }
    }
}

/// The receiving end of every request channel, read by the dispatcher.
pub struct ReceiverMultiplexer<C: BuildRunnable, P: Parsable> {
    sources: Vec<Box<dyn Source<C, P>>>,
    /// Rung by `SendChannels` after every send, and on `close`.
    wake: Receiver<()>,
}

impl<C: BuildRunnable, P: Parsable> ReceiverMultiplexer<C, P> {
    pub fn new(wake: Receiver<()>) -> Self {
        Self {
            sources: vec![],
            wake,
        }
    }

    /// Serves `R`'s channel with `handler`, at the priority the scheduler gives
    /// `R`.
    pub fn add<R>(
        &mut self,
        receiver: Receiver<WorkerMessage<R::Params>>,
        handler: Handler<Server<C, P>, R::Params, R::Result>,
    ) where
        R: Request + 'static,
        R::Params: Send + 'static,
        R::Result: Serialize,
    {
        self.sources.push(Box::new(Typed::<R, C, P> {
            receiver,
            handler,
            priority: RequestKind::from(R::METHOD).priority(),
            _marker: PhantomData,
        }));
    }

    /// Blocks until at least one request is waiting, then moves every waiting
    /// request into `out` as a job. `Err(Closed)` once a channel has closed:
    /// `out` then holds the requests that were queued ahead of the close, and
    /// no more will come.
    pub fn recv_jobs(&mut self, out: &mut Vec<Job<Server<C, P>>>) -> Result<(), Closed> {
        loop {
            let mut closed = false;
            for source in &mut self.sources {
                if source.drain(out).is_err() {
                    closed = true;
                }
            }
            if closed {
                return Err(Closed);
            }
            if !out.is_empty() {
                return Ok(());
            }
            self.wake.recv().map_err(|_| Closed)?;
        }
    }
}
