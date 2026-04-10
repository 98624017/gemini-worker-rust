#[test]
fn jemalloc_default_decay_matches_approved_value() {
    assert_eq!(
        rust_sync_proxy::allocator::DEFAULT_JEMALLOC_MALLOC_CONF,
        "background_thread:true,dirty_decay_ms:500,muzzy_decay_ms:500"
    );
}

#[test]
fn compiled_allocator_matches_platform_policy() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    assert_eq!(
        rust_sync_proxy::allocator::compiled_allocator_name(),
        "jemalloc"
    );

    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    assert_eq!(
        rust_sync_proxy::allocator::compiled_allocator_name(),
        "system"
    );
}
