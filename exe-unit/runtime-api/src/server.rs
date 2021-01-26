use std::future::Future;
use std::pin::Pin;

pub mod proto {
    use std::fmt;

    include!(concat!(env!("OUT_DIR"), "/ya_runtime_api.rs"));

    impl response::Error {
        pub fn msg<D: std::fmt::Display>(msg: D) -> Self {
            let mut e = Self::default();
            e.set_code(response::ErrorCode::Internal);
            e.message = msg.to_string();
            e
        }
    }

    impl fmt::Display for Response {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match &self.command {
                Some(response::Command::Status(status)) => {
                    write!(f, "event: {}, id: {}, status: ", self.event, self.id)?;
                    write!(
                        f,
                        "[pid: {}, running: {}, return_code: {}], ",
                        status.pid, status.running, status.return_code
                    )?;
                    write!(f, "stdout: {:?}, ", status.stdout.as_slice())?;
                    write!(f, "stderr: {:?}", status.stderr.as_slice())
                }
                _ => write!(f, "{:?}", self),
            }
        }
    }
}
mod codec;

#[cfg(feature = "codec")]
pub use codec::Codec;
pub use proto::request::{KillProcess, RunProcess};
pub use proto::response::Error as ErrorResponse;
pub use proto::response::RunProcess as RunProcessResp;
pub use proto::response::{ErrorCode, ProcessStatus};

pub type DynFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;
pub type AsyncResponse<'a, T> = DynFuture<'a, Result<T, ErrorResponse>>;

use futures::future::BoxFuture;
use futures::prelude::*;
use std::process::{ExitStatus, Stdio};
use std::sync::Arc;
use tokio::process;

pub trait RuntimeService {
    fn hello(&self, version: &str) -> AsyncResponse<'_, String>;

    fn run_process(&self, run: RunProcess) -> AsyncResponse<'_, RunProcessResp>;

    fn kill_process(&self, kill: KillProcess) -> AsyncResponse<'_, ()>;

    fn shutdown(&self) -> AsyncResponse<'_, ()>;
}

pub trait RuntimeEvent {
    fn on_process_status(&self, _status: ProcessStatus) {}
}

pub trait RuntimeStatus {
    fn exited<'a>(&self) -> BoxFuture<'a, i32>;
}

pub trait ProcessControl {
    fn id(&self) -> u32;

    fn kill(&self);
}

mod client;
mod service;

pub use client::spawn;
pub use service::{run, run_async};
