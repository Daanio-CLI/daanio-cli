# Lifecycle Hooks

daanio can run external commands at well-defined lifecycle points so other
programs can observe or gate agent behavior without forking daanio. Hooks
complement the [spawn hook](SPAWN_HOOK.md) (which controls *where headed
sessions appear*); lifecycle hooks tell you *what is happening inside them*.

## Configuration

```toml
# ~/.daanio/config.toml
[hooks]
turn_end      = "~/bin/daanio-turn-notify"     # observer
session_start = ""                            # observer
session_end   = ""                            # observer
pre_tool      = "~/bin/daanio-tool-policy"     # gate
post_tool     = ""                            # observer
pre_tool_timeout_ms = 5000
```

Env overrides (always win; empty value disables a config hook):
`DAANIO_HOOK_TURN_END`, `DAANIO_HOOK_SESSION_START`, `DAANIO_HOOK_SESSION_END`,
`DAANIO_HOOK_PRE_TOOL`, `DAANIO_HOOK_POST_TOOL`, `DAANIO_HOOK_PRE_TOOL_TIMEOUT_MS`.

## Common contract

- The hook command line is parsed shell-style (quotes and backslash escapes
  work) but executed **directly**, not through a shell. A leading `~/` in the
  program path is expanded.
- The hook runs in the session working directory when known.
- Every hook receives:

| Variable | Meaning |
| --- | --- |
| `DAANIO_HOOK_EVENT` | `turn_end`, `session_start`, `session_end`, `pre_tool`, `post_tool` |
| `DAANIO_HOOK_SESSION_ID` | Session the event belongs to |
| `DAANIO_HOOK_CWD` | Session working directory |
| `DAANIO_HOOK_PAYLOAD` | JSON object mirroring all fields (capped at 16 KB) |
| `DAANIO_HOOKS_DISABLED` | Always `1`; suppresses hooks in nested daanio calls (recursion guard) |

## Observer hooks

`turn_end`, `session_start`, `session_end`, and `post_tool` are
**observers**: spawned detached, fire-and-forget. They can never block or slow
the agent; failures are only logged.

### `turn_end`

Fires when an agent turn completes (streaming turn path, which covers TUI,
desktop, swarm workers, and headless sessions).

Extra fields: `DAANIO_HOOK_STATUS` (`ok`/`error`), `DAANIO_HOOK_DURATION_MS`,
`DAANIO_HOOK_MODEL`, `DAANIO_HOOK_LAST_ASSISTANT_TEXT` (first 4000 chars),
`DAANIO_HOOK_ERROR` (on failure).

### `session_start` / `session_end`

`session_start` fires when an agent session becomes active, with
`DAANIO_HOOK_SOURCE` = `create` (brand new), `attach` (existing session object
attached), or `resume` (restored by id). `session_end` fires on normal close
(`DAANIO_HOOK_SOURCE=close`).

### `post_tool`

Fires after every tool call. Extra fields: `DAANIO_HOOK_TOOL_NAME`,
`DAANIO_HOOK_STATUS`, `DAANIO_HOOK_DURATION_MS`, `DAANIO_HOOK_OUTPUT_BYTES` (on
success), `DAANIO_HOOK_ERROR` (on failure).

## Gate hook: `pre_tool`

`pre_tool` runs **synchronously before every tool call** and can block it:

- The hook receives `DAANIO_HOOK_TOOL_NAME` plus the full tool input JSON on
  **stdin** (and a 16 KB-truncated copy in `DAANIO_HOOK_TOOL_INPUT`).
- **Exit 0**: allow the call.
- **Exit 2**: block the call. The hook's stderr (trimmed, capped at 2000
  chars) is returned to the model as the tool error, so the model can adapt.
- **Anything else fails open** with a logged warning: other exit codes,
  timeout (`pre_tool_timeout_ms`, default 5s), missing binary, spawn errors.

Fail-open is deliberate: a broken policy script should degrade to "no policy"
rather than brick every session. If you need fail-closed semantics, make the
hook itself robust (it is your trust boundary, not daanio).

### Example policy script

```bash
#!/usr/bin/env bash
# ~/bin/daanio-tool-policy
# stdin: tool input JSON. Env: DAANIO_HOOK_TOOL_NAME, DAANIO_HOOK_SESSION_ID...
input=$(cat)

case "$DAANIO_HOOK_TOOL_NAME" in
  bash)
    if grep -qE 'rm -rf /([^a-zA-Z]|$)|mkfs|dd if=' <<<"$input"; then
      echo "blocked: destructive shell command" >&2
      exit 2
    fi
    ;;
  write|edit)
    if grep -q '"file_path":"/etc/' <<<"$input"; then
      echo "blocked: writes to /etc are not allowed" >&2
      exit 2
    fi
    ;;
esac
exit 0
```

## Example: tmux status + desktop notification on turn end

```bash
#!/usr/bin/env bash
# ~/bin/daanio-turn-notify
if [ "$DAANIO_HOOK_STATUS" = ok ]; then icon=✅; else icon=❌; fi
tmux display-message "daanio $icon ${DAANIO_HOOK_SESSION_ID:0:12}" 2>/dev/null
notify-send "daanio turn $DAANIO_HOOK_STATUS" \
  "${DAANIO_HOOK_LAST_ASSISTANT_TEXT:0:120}" 2>/dev/null
exit 0
```

## Example: JSON event log of all hook activity

Point several hooks at one script and fan out on `DAANIO_HOOK_EVENT`:

```bash
#!/usr/bin/env bash
# ~/bin/daanio-event-log
echo "$DAANIO_HOOK_PAYLOAD" >> ~/.local/state/daanio-events.jsonl
```

```toml
[hooks]
turn_end      = "~/bin/daanio-event-log"
session_start = "~/bin/daanio-event-log"
session_end   = "~/bin/daanio-event-log"
post_tool     = "~/bin/daanio-event-log"
```

## Design notes

- Hook lookups are config-driven and re-read on config reload; you can add or
  change hooks without restarting daanio.
- Hot paths (`pre_tool`/`post_tool`) check whether a hook is configured before
  building any payload, so unconfigured hooks cost ~nothing.
- The recursion guard (`DAANIO_HOOKS_DISABLED=1`) means a hook may safely call
  `daanio` CLI commands without re-triggering hooks in that nested process.
