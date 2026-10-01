pub mod local;
mod remote;
pub use remote::{ProviderRegistry, TransferWriter, probe_host, test_connection};

use crate::domain::{Capabilities, Entry, FsError, Location, ProviderId};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Debug, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ListOptions {
    pub show_hidden: bool,
}

pub type ProviderFuture<T> = Pin<Box<dyn Future<Output = Result<T, FsError>> + Send>>;

/// Owned provider requests must be driven on the GPUI background executor.
/// Local filesystem syscalls can block; never poll these futures on the UI thread.
/// Cancellation is cooperative between syscalls, not an interruption of an OS syscall.
pub trait FileSystem: Send + Sync {
    fn provider_id(&self) -> ProviderId;
    fn capabilities(&self) -> Capabilities;
    fn list(
        &self,
        location: Location,
        options: ListOptions,
        cancel: CancellationToken,
    ) -> ProviderFuture<Vec<Entry>>;
    fn metadata(&self, location: Location, cancel: CancellationToken) -> ProviderFuture<Entry>;
}
