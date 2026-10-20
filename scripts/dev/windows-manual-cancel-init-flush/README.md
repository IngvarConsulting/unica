# #1759 frozen Windows fixed first observation

CASE `b471028e270826bc03ee163efe91a876d65bbf96` pauses the actual initialize-response flush, observes original execution and completed initialization, then arms the response gate before releasing original work. The selected case is:
`interfaces::mcp::tests::canonical_manual_cancellation_reused_id_keeps_old_control_and_targets_new_receipt`.

Run the retained ordinary default-feature libtest once on Windows MSVC after exact queue inventory and source/binary checks. The fixed case must pass its initialization, exact receipt reuse, cancellation-target and retained-control drain assertions on that first attempt. A native failure remains a failure in JSON, JUnit and the Actions job.

All14 pins come from immutable CASE bytes. Accepted owned-command helpers and selection are unchanged. Raw logs, command receipts, retained executable, source pins and actual control files are uploaded for result review. The causal RED uses its separate immutable CASE/control/results. Payload ready=true binds these source bytes; root separately authorizes materialization and dispatch after normal prepush checks.
