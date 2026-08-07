# Reload Resume Recovery Fix

## Issue

Lighthouse Sauropod (`session_sauropod_1785982488339_03f764b6b56dec2e`) could be explicitly stopped and later resumed, but Daanio immediately entered **Sending** and replayed stale recovery text:

> Reload complete — continuing because a recovery directive was pending.

The desired behavior is for an ordinary manual resume to open idle, while preserving automatic continuation immediately after a genuine server reload.

## Root Cause

Two server resume paths treated every persisted `SessionStatus::Crashed` value as evidence that a server reload interrupted the session:

- Restored live-session classification in `server/client_session.rs`
- Persisted-history fallback classification in `server/client_state.rs`

Sauropod had an ordinary terminal/interruption crash state, not an active reload recovery record. The broad `Crashed { .. }` match therefore synthesized a reload recovery directive during a later manual resume.

## Fix

Commit: `97ef40364` (`fix(reload): ignore ordinary crashes during resume recovery`)

Daanio now recognizes a crashed status as reload-related only when its reason exactly matches the reason written by genuine reload disconnect cleanup:

```text
Server reload interrupted processing
```

Ordinary terminal crashes, other client-disconnect crashes, and crashes without a reason no longer trigger reload continuation.

Legitimate recovery remains supported through:

1. The exact genuine reload crash reason.
2. Active or closed sessions with a pending user turn and a fresh reload marker.
3. Explicit transcript markers showing generation or tool execution was interrupted by reload.
4. A durable pending reload-recovery record, consumed only when the matching continuation is accepted.

## Files Changed

- `crates/daanio-app-core/src/server/client_session.rs`
- `crates/daanio-app-core/src/server/client_session_tests.rs`
- `crates/daanio-app-core/src/server/client_session_tests/reload.rs`
- `crates/daanio-app-core/src/server/client_state.rs`
- `crates/daanio-app-core/src/server/client_state_tests.rs`

## Regression Coverage

Added tests confirming:

- A terminal/window-close crash does not count as reload interruption.
- A crash without a reason does not count as reload interruption.
- The exact genuine reload crash reason still counts.
- Persisted-history recovery does not infer reload recovery for ordinary crashes.
- Persisted-history recovery does not infer reload recovery for crashes without a reason.
- Persisted-history recovery still recognizes the exact genuine reload crash reason.

Existing tests continue to cover active/closed reload-marker recovery, transcript interruption markers, and durable one-shot recovery delivery.

## Validation

Passed:

```text
cargo fmt --all -- --check
git diff --check
cargo test -p daanio-app-core server::client_session_tests::reload --lib
cargo test -p daanio-app-core server::client_state_tests::history_reload_recovery --lib
cargo test -p daanio-app-core server::reload_recovery::tests --lib
cargo check -p daanio-app-core -p daanio-tui
```

Only an existing unrelated dead-code warning for `PROC_PIDFDVNODEPATHINFO` appeared.

## Repository Scope

The source fix was committed separately. Existing untracked files were preserved and not modified:

- `.DS_Store`
- `telemetry-worker.zip`

## Deployment and Live Verification

Pending:

- Build the committed selfdev binary.
- Reload Daanio through the supported selfdev workflow.
- Confirm the shared server and unrelated sessions remain healthy.
- Resume only Lighthouse Sauropod.
- Observe it for at least 10 seconds and confirm it remains idle without automatically sending stale recovery text.
