# Unica dependency patch

The published ureq 2.12.1 source is pinned in `UPSTREAM.json`, including the
archive checksum, upstream commit and all original file hashes. Local modified
and added files are listed separately. The upstream MIT and Apache notices,
including the separate chunk decoder notices, are retained.

`middleware.rs` removes redundant type parentheses reported by Clippy.

`response.rs` preserves a fully received Content-Length response when preparing
its connection for reuse fails. The consumed stream is dropped before pool
insertion. Read, framing and deadline errors before completion keep their prior
handling. Chunked pool-return handling is unchanged.

`stream.rs` and `lib.rs` add only dependency-local test support under `cfg(test)`.
The fault controls run at each timeout reset with a real held socket. The tests
exercise the real parser, readers and pool; they cover empty/buffered/streaming
completion, incomplete bodies, deadlines, no replay, a fresh connection after
failure and healthy reuse.

The manifest has a standalone workspace for these tests. Its lock preserves the
production dependency versions selected by Unica and adds the upstream test and
optional dependency closure. It is distinct from the published upstream lock.
The Unica workspace lock changes only ureq's source from registry to this path.

The CI runner includes this medium dependency-local suite in queue, main,
release and large runs. Its nextest profile keeps its own JUnit path, zero
retries and a nonempty selection; results retain the original gate and runner.

To run the dependency-local checks directly:

```sh
cargo test --manifest-path vendor/ureq/Cargo.toml --locked --features json,serde_json/preserve_order,serde_json/float_roundtrip --lib completed_response_recycling
```

The vendor source and tests are build inputs outside `plugins/unica`; the plugin
carries the required license files and attribution.

## Diagnostic credential redaction (#1792)

A shared diagnostic URL renderer removes URL userinfo without changing the
stored URL or authentication data. Unparseable or hostless diagnostic URLs
use a neutral marker, matching the existing request URL validation. Request, RequestUrl, Response, status/transport errors and the active
request/redirect/retry log paths use this rendering. Proxy Debug hides both
credential fields, including when nested in AgentBuilder.

Header Debug, invalid-header errors and the wire prelude logger share the
case-insensitive Authorization, Proxy-Authorization and Cookie classification.
Invalid header names or values use a neutral diagnostic instead of raw input.
The real header bytes remain unchanged. The optional cookie-store feature and
its cookie-value logging are outside this patch.

The separate diagnostic_redaction integration target owns its logger. Real
loopback tests compare sent authentication data and redirect policies while
asserting that diagnostic output omits distinct synthetic credentials. A
loopback proxy checks CONNECT authentication before an intentional 407, without
claiming an end-to-end TLS connection. The reused-connection test observes the
real read-failure retry; the early write-failure retry shares the same safe URL
value but is not induced by this socket scenario.

The existing ureq-recycling CI profile now includes this integration target
alongside all ten completed-response fault tests. Its historical report path
and zero-retry policy are retained. Run the same selected tests locally with:

```sh
cargo nextest run --manifest-path vendor/ureq/Cargo.toml --locked --lib --test diagnostic_redaction --features json,serde_json/preserve_order,serde_json/float_roundtrip --config-file vendor/ureq/.config/nextest.toml --target-dir target --profile ureq-recycling
```
