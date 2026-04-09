# Tasks Document

> Adapt this checklist to the concrete feature before implementation.
> Replace placeholders such as `[feature]`, `[module]`, `[route]`,
> and `[config_key]` with project-specific names, and remove any task
> that does not apply.

- [ ] 1. Confirm feature boundaries and affected Rust modules
  - Files: `src/[feature].rs`, `src/lib.rs`, `src/http/router.rs` (if HTTP-facing), `tests/[feature]_test.rs`
  - Identify which existing modules own the change and whether a new module is required
  - Record public API, configuration, and test impact before implementation
  - Purpose: Keep the task plan aligned with the current Rust crate layout
  - _Leverage: src/lib.rs, Cargo.toml, existing modules under src/_
  - _Requirements: 1.1_
  - _Prompt: Role: Rust architect specializing in backend service design | Task: Map the feature to the correct Rust modules, runtime entry points, and test files following requirement 1.1 | Restrictions: Do not invent non-existent project layers; follow the crate structure that already exists in src/ and tests/ | Success: The affected files are identified up front, module ownership is clear, and the implementation plan matches the repository layout_

- [ ] 2. Implement core feature logic in `src/[feature].rs`
  - File: `src/[feature].rs`
  - Add structs, enums, helper functions, and async logic required by the feature
  - Reuse existing serialization, validation, and error-handling patterns where applicable
  - Purpose: Implement the main Rust business logic in a focused module
  - _Leverage: src/request_rewrite.rs, src/response_rewrite.rs, src/request_materialize.rs, src/response_materialize.rs, src/cache.rs, src/blob_runtime.rs_
  - _Requirements: 2.1, 2.2_
  - _Prompt: Role: Rust developer with expertise in async services and module design | Task: Implement the core feature module in src/[feature].rs following requirements 2.1 and 2.2, reusing established patterns from adjacent Rust modules | Restrictions: Preserve existing behavior unless the spec explicitly changes it, avoid duplicate abstractions, and keep ownership and error propagation idiomatic | Success: The module compiles cleanly, covers the required behavior, and integrates with existing Rust code without introducing stack-incompatible patterns_

- [ ] 3. Wire module exports and internal boundaries
  - File: `src/lib.rs`
  - Add `pub mod [feature];` and selective re-exports only when the public crate API needs them
  - Keep dependency direction consistent with existing module boundaries
  - Purpose: Make the new functionality reachable without leaking internals
  - _Leverage: src/lib.rs, src/http/mod.rs_
  - _Requirements: 2.3_
  - _Prompt: Role: Rust maintainer focused on crate API design | Task: Register the new module and expose only the minimal public surface needed for requirement 2.3 | Restrictions: Do not over-export internals, do not create circular module dependencies, and keep the crate API coherent | Success: Module wiring is explicit, public exports are minimal, and internal boundaries remain clear_

- [ ] 4. Integrate runtime flow, routing, or pipeline hooks when applicable
  - Files: `src/http/router.rs`, `src/http/mod.rs`, or the relevant request/response pipeline module
  - Add Axum routes, extractors, response mapping, or request/response processing hooks as required
  - Keep status codes, headers, and error responses consistent with existing handlers
  - Purpose: Connect the feature logic to the actual runtime entry points
  - _Leverage: src/http/router.rs, src/admin.rs, src/upload.rs, src/request_encode.rs, src/upstream.rs_
  - _Requirements: 3.1, 3.2_
  - _Prompt: Role: Rust backend developer with expertise in Axum and HTTP proxy flows | Task: Integrate the feature into the runtime path required by requirements 3.1 and 3.2, using existing routing and pipeline patterns | Restrictions: Do not bypass current middleware or request processing conventions, maintain response compatibility, and keep handler responsibilities narrow | Success: The feature is reachable through the correct runtime path, behavior is consistent with existing endpoints, and the flow remains testable_

- [ ] 5. Update configuration and startup wiring when applicable
  - Files: `src/config.rs`, `src/main.rs`, and any affected runtime module
  - Add new environment variables, defaults, timeouts, size budgets, or feature flags when needed
  - Validate config parsing, fallback behavior, and startup-time wiring
  - Purpose: Ensure the feature can be configured and started safely
  - _Leverage: src/config.rs, src/main.rs, src/blob_runtime.rs, src/upstream.rs_
  - _Requirements: 4.1_
  - _Prompt: Role: Rust infrastructure engineer specializing in configuration and service startup | Task: Extend configuration and runtime wiring for the feature following requirement 4.1, matching existing Config parsing and startup patterns | Restrictions: Keep environment parsing explicit, preserve existing defaults unless the spec changes them, and avoid hidden runtime side effects | Success: New configuration keys are parsed correctly, startup remains stable, and runtime wiring is consistent with the current service design_

- [ ] 6. Add focused unit tests in `tests/[feature]_test.rs`
  - File: `tests/[feature]_test.rs`
  - Cover success cases, failure cases, and important boundary conditions
  - Reuse existing helpers such as `rust_sync_proxy::test_config()` and `rust_sync_proxy::test_blob_runtime()` when applicable
  - Purpose: Verify the feature logic in isolation and prevent regressions
  - _Leverage: tests/config_test.rs, tests/request_rewrite_test.rs, tests/response_materialize_test.rs, tests/blob_runtime_test.rs_
  - _Requirements: 5.1, 5.2_
  - _Prompt: Role: Rust QA engineer with expertise in unit and async testing | Task: Add focused tests for the new feature module covering requirements 5.1 and 5.2, following the repository's `*_test.rs` conventions | Restrictions: Keep tests deterministic, avoid real network calls unless intentionally covered by fixtures or local test servers, and assert both happy-path and edge behavior | Success: The module has reliable unit coverage, boundary cases are checked, and tests run consistently under cargo test_

- [ ] 7. Add integration or regression coverage for cross-module flows
  - Files: `tests/[feature]_integration_test.rs` or existing `*_test.rs` files that exercise the changed flow
  - Verify routing, proxying, uploads, rewriting, caching, or materialization behavior end-to-end as needed
  - Focus on user-visible behavior and interactions between modules
  - Purpose: Catch regressions that unit tests alone would miss
  - _Leverage: tests/http_forwarding_test.rs, tests/http_smoke.rs, tests/upload_mode_test.rs, tests/request_inline_data_flow_test.rs_
  - _Requirements: 5.3_
  - _Prompt: Role: Integration test engineer for Rust network services | Task: Add regression coverage for the affected end-to-end flow following requirement 5.3, using the existing test style in the repository | Restrictions: Keep the test scope tied to real integration risks, avoid duplicating unit-level assertions, and preserve reasonable execution time | Success: Cross-module behavior is verified, regressions are caught at the system boundary, and the tests reflect how the feature is actually exercised_

- [ ] 8. Run quality gates and update supporting documentation
  - Files: `Cargo.toml`, `README.md`, `docs/`, or comments in touched modules
  - Run `cargo fmt`, `cargo test`, and `cargo clippy` if the repository uses it
  - Document any new config keys, runtime behavior, or operational caveats
  - Purpose: Leave the change reviewable, testable, and maintainable
  - _Leverage: Cargo.toml, README.md, existing module docs and comments_
  - _Requirements: All_
  - _Prompt: Role: Rust maintainer responsible for release quality | Task: Finish the implementation by running the relevant quality gates and updating supporting documentation for all requirements | Restrictions: Do not claim success without running the checks that exist in the repository, keep documentation scoped to the shipped behavior, and avoid unrelated cleanup | Success: Formatting and tests pass, supporting documentation matches the implementation, and the change is ready for review_
