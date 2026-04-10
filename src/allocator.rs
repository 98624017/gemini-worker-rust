#[cfg(all(target_os = "linux", target_env = "gnu"))]
use tikv_jemallocator::Jemalloc;

pub const DEFAULT_JEMALLOC_MALLOC_CONF: &str =
    "background_thread:true,dirty_decay_ms:500,muzzy_decay_ms:500";

#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[global_allocator]
static GLOBAL_ALLOCATOR: Jemalloc = Jemalloc;

pub const fn compiled_allocator_name() -> &'static str {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        "jemalloc"
    }

    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    {
        "system"
    }
}
