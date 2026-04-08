const GIB: u64 = 1024 * 1024 * 1024;
const MIB: u64 = 1024 * 1024;

#[test]
fn blob_budget_defaults_match_2gib_profile() {
    let budget = rust_sync_proxy::config::blob_budget_defaults_for_memory(2 * GIB);
    assert_eq!(budget.inline_max_bytes, 8 * MIB);
    assert_eq!(budget.request_hot_budget_bytes, 24 * MIB);
    assert_eq!(budget.global_hot_budget_bytes, 384 * MIB);
}

#[test]
fn blob_budget_defaults_match_4gib_profile() {
    let budget = rust_sync_proxy::config::blob_budget_defaults_for_memory(4 * GIB);
    assert_eq!(budget.inline_max_bytes, 12 * MIB);
    assert_eq!(budget.request_hot_budget_bytes, 40 * MIB);
    assert_eq!(budget.global_hot_budget_bytes, 768 * MIB);
}

#[test]
fn blob_budget_defaults_match_8gib_profile() {
    let budget = rust_sync_proxy::config::blob_budget_defaults_for_memory(8 * GIB);
    assert_eq!(budget.inline_max_bytes, 16 * MIB);
    assert_eq!(budget.request_hot_budget_bytes, 64 * MIB);
    assert_eq!(budget.global_hot_budget_bytes, 1536 * MIB);
}
