//! wasmtime Store creation.

use crate::limits::new_memory_limits;
use std::ptr::NonNull;
use wasmtime::{Engine, ResourceLimiter, Store, StoreLimits};

/// Create a wasmtime Store.
///
/// Memory limits are enforced via wasmtime's `ResourceLimiter` mechanism.
pub fn create_store<T>(engine: &Engine, max_memory_mb: u64, data: T) -> Store<T> {
    let mut store = Store::new(engine, data);
    if max_memory_mb == 0 {
        return store;
    }

    let limits = new_memory_limits(max_memory_mb);
    let limiter = StaticLimiter::new(limits);
    store.limiter(move |_data| limiter.limiter());

    store
}

/// Holds a `StoreLimits` with a `'static` lifetime by leaking it,
/// then hands out `&'static mut dyn ResourceLimiter` to wasmtime.
struct StaticLimiter {
    ptr: NonNull<StoreLimits>,
}

impl StaticLimiter {
    fn new(limits: StoreLimits) -> Self {
        let leaked = Box::leak(Box::new(limits));
        Self {
            ptr: NonNull::from(leaked),
        }
    }

    fn limiter(&self) -> &'static mut dyn ResourceLimiter {
        // SAFETY: Box::leak gives us ownership with 'static lifetime.
        // NonNull is aliasing-free. StoreLimits implements ResourceLimiter.
        // wasmtime calls this for the lifetime of the store, which is fine
        // since the leaked box is never freed.
        unsafe { &mut *self.ptr.as_ptr() }
    }
}

// SAFETY: StaticLimiter owns a 'static leaked Box<StoreLimits>. wasmtime calls
// the limiter from its synchronized internal context, so &self is safe.
unsafe impl Send for StaticLimiter {}
unsafe impl Sync for StaticLimiter {}
