# #1759 frozen Windows causal RED

CASE `37e639fd8d2ce70b41d02b8581621ac6200c1ce9` preserves the old response-gate arm order while the test fixture
pauses completion of the actual initialize-response flush. The selected case is:
`interfaces::mcp::tests::canonical_manual_cancellation_reused_id_keeps_old_control_and_targets_new_receipt`.

This control executes the retained ordinary default-feature libtest once on Windows
MSVC, after exact queue inventory and source/binary checks. There is no retry.
A native failure remains a failure in JSON, JUnit and the Actions job. Root checks
the complete diagnostic tuple in the raw output: initialization pause entered and
released, initialize flush incomplete, response gate entered, zero receipts and
zero executions. Setup, capture and cleanup failures are distinct outcomes.

The accepted owned-command helpers remain byte-for-byte unchanged. Raw logs,
command receipts, retained executable, source pins and actual control files are
uploaded for independent result verification. The later fixed Windows CASE and
control use their own immutable commits and results.
