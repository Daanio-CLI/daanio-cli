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

Completed:

- Built and published selfdev build `b908cdb1f` (`v0.2.18-dev`).
- Gracefully reloaded the actual shared server socket through `daanio server reload --force`; handoff returned `handoff_ready: true`.
- Confirmed unrelated sessions survived the reload.
- Found a Sauropod-only pending recovery record that the old server created during handoff because its stale in-memory entry still said Sauropod was processing. The persisted session itself remained the ordinary crash case:

  ```text
  Crashed("Terminal or window closed (SIGHUP)")
  ```

- Archived the stale Sauropod recovery, pending-soft-interrupt, and streaming marker files under:

  ```text
  ~/.daanio/scratch/sauropod-stale-recovery-20260817T170900Z/
  ```

- The first verification resume did not send reload-recovery text, but an inherited in-memory processing task later continued ordinary todo work. It appended 26 transcript entries and created two read-only child sessions. This was not acceptable as a normal idle resume even though it was distinct from the original recovery-directive bug.
- Closed only the Sauropod client so server disconnect cleanup released that inherited task. Confirmed unrelated shared-server sessions remained available.
- Archived the accidental 1,700-message snapshot and all state for the two child sessions under:

  ```text
  ~/.daanio/scratch/sauropod-accidental-continuation-20260817T172513Z/
  ```

- Restored Sauropod atomically from its untouched pre-resume backup, validating the exact ordinary crash status and 1,674-message transcript before replacement.
- Confirmed the only observed emulator firewall change had already been restored to the intended local-only policy: IPv4 permits only loopback and `10.0.2.2`, and IPv6 rejects the app UID.
- Relaunched only Lighthouse Sauropod through the normal current-build client.
- Final verification after more than 30 seconds:
  - Shared server reports `status: ready` and `is_processing: false`.
  - Persisted session status is `Active`.
  - Transcript remains exactly 1,674 messages.
  - No reload-recovery, pending-soft-interrupt, or streaming marker exists.
  - No recovery continuation text appears anywhere in the transcript.
  - The two accidental child sessions have no remaining live session, PID, todo, or journal state outside the archive.
  - Unrelated sessions remain connected and healthy.

## Final Result

The reported reload-resume loop is fixed. Lighthouse Sauropod now resumes silently into a ready, non-processing state with its original transcript intact. A later manual resume of an ordinary crashed session no longer synthesizes or sends reload recovery text, while genuine reload interruption and durable one-shot recovery paths remain covered by tests.
