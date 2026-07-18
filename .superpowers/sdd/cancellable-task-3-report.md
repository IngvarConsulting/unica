# Cancellable Workspace Service — Task 3 Report

## Scope

Implemented internal operation IDs and cancellation-aware workspace-service connector behavior in `workspace_services.rs` only. The concurrent service runtime remains deferred to Task 4.

## RED evidence

Command:

```text
cargo test -p unica-coder cancellable_connector -- --nocapture
```

The new connector test failed to compile for the intended missing contract:

```text
ServiceRequestKind::BslMcp has no field named operation_id
ServiceConnector::send takes 2 arguments but 3 were supplied
no variant named Cancel found for ServiceRequestKind
```

After the production contract was added, the first test-harness version exposed an EOF race because the fixture dropped the work connection. The fixture was corrected to retain that connection while accepting the independent cancel connection.

## Implementation decisions

- `BslMcp` and `RlmReady` carry UUID v4 operation IDs generated at the manager boundary.
- `ServiceConnector::send` receives the caller's `CancellationToken`.
- Response reads poll with a 100 ms socket timeout and preserve the 120-second overall request deadline.
- `WouldBlock` and `TimedOut` continue polling; EOF is reported as a service disconnect; a complete JSON line is deserialized as the response.
- On cancellation, the connector writes `Cancel { operation_id }` over a separate TCP connection and returns a stable `cancelled:` error without waiting for a control response.
- Best-effort cancel connection/write time is bounded to 500 ms. Other control requests use fresh uncancelled tokens through the connector.
- The current sequential server recognizes `Cancel` as a wire-compatible acknowledgement. Actual operation lookup and responsive concurrent control handling remain Task 4.

## GREEN evidence

Focused connector test:

```text
cargo test -p unica-coder cancellable_connector -- --nocapture
1 passed; 0 failed
```

Workspace service suite:

```text
cargo test -p unica-coder workspace_services::tests -- --nocapture
10 passed; 0 failed
```

Full verification:

```text
cargo fmt --all -- --check                         PASS
cargo clippy -p unica-coder --all-targets -- -D warnings  PASS
cargo test -p unica-coder                         289 passed; 0 failed
git diff --check                                  PASS
```

## Risks / follow-up

- Until Task 4 makes the service listener concurrent, a cancel message can queue on the listener but cannot interrupt server-side work immediately. The client nevertheless returns promptly and the operation ID/control message contract is now available for Task 4.
- The cancel send is intentionally best-effort and fire-and-forget so cancellation cannot become blocked waiting for a control response.

## Reviewer follow-up

All Task 3 review findings were addressed in a second TDD cycle.

RED evidence:

```text
cargo test -p unica-coder cancellable_connector_deadline_is_aggregate -- --nocapture
FAIL: ManualClock and Deadline were not defined

cargo test -p unica-coder cancel_response_disconnect_is_non_fatal -- --nocapture
FAIL: write_service_response was not defined
```

Changes:

- Added injected connector I/O and monotonic-clock seams. Cancellation is checked before and after connect, write, flush, and read operations, and before transport errors win a race. Deterministic connect/write/read/EOF race tests require the exact `cancelled:` prefix.
- A single 120-second deadline now starts before connect. Connect, every write, flush, and read receive only the remaining budget. Ping, Invalidate, Shutdown, and Cancel use the 500 ms control connect cap.
- Cancel control delivery has one aggregate 500 ms deadline across connect, both writes, and flush. The manual-clock test proves that 300 ms spent connecting leaves 200 ms for the first write and 100 ms after another 100 ms is consumed.
- A disconnected fire-and-forget Cancel response is non-fatal. The regression keeps the service record and proves the same state still answers Ping.
- Added manager-level UUID v4 uniqueness coverage for both BSL and RLM paths, plus tagged-serde shape and roundtrip coverage for BslMcp, RlmReady, and Cancel.

Final GREEN evidence:

```text
cargo test -p unica-coder workspace_services::tests -- --nocapture  18 passed
cargo fmt --all -- --check                                    PASS
cargo clippy -p unica-coder --all-targets -- -D warnings      PASS
cargo test -p unica-coder                                     297 passed
git diff --check                                              PASS
```

## Second reviewer follow-up

RED evidence:

```text
cargo test -p unica-coder cancellable_connector_reads_fragmented_response -- --nocapture
FAIL: read_service_response and SERVICE_RESPONSE_LINE_LIMIT were not defined
```

Changes:

- Replaced `BufReader::read_line` polling with one bounded 8 KiB `Read::read` per iteration and an explicitly capped 8 MiB response line. Cancellation and deadline are checked after every read result, including timeout and EOF. Only bytes through the first newline are parsed; trailing bytes are ignored because the protocol permits one response per connection.
- Added deterministic fragmented-response, endless-partial cancellation/deadline, and oversized-line tests. Newline scanning examines only newly appended bytes, so the limit cannot be abused for quadratic rescanning.
- Replaced `write_all` with explicit partial-write loops for work and control requests. Every partial write recalculates the remaining deadline and socket timeout; `WriteZero` is controlled; error priority is cancellation, then exact `timeout:`, then transport error. Flush follows the same priority.
- Tightened wire tests to exact tags `bsl-mcp`, `rlm-ready`, and `cancel`. Manager-generated BSL/RLM IDs are unique UUID v4 values; the separate cancel-connection test preserves the same operation ID.

Final verification after these changes:

```text
cargo test -p unica-coder workspace_services::tests -- --nocapture  25 passed
cargo fmt --all -- --check                                    PASS
cargo clippy -p unica-coder --all-targets -- -D warnings      PASS
cargo test -p unica-coder                                     304 passed
git diff --check                                              PASS
```
