<!--
This file IS the swarm config. Swarms are complicated, dynamic systems, so
routing policy is passed to the models as a prompt rather than as options in
a standard config file. Edit freely: override globally at
~/.daanio/swarm-prompt.md or per-project at ./.daanio/swarm-prompt.md.
-->

Model routing guidance for spawned swarm agents:

- By default, omit both `model` and `effort`. Workers then inherit the
  coordinator's selected model and use that model's default reasoning effort.
- Do not automatically switch models based on task type. Implementation,
  investigation, review, summarization, and all other work should stay on the
  coordinator's model by default.
- Pass `model` or `effort` only when the user explicitly requests a different
  worker model or reasoning effort. Run `swarm list_models` first when you need
  to confirm that a requested model or route is available.
- Use `model: "inherit"` to force coordinator inheritance when a configured
  `agents.swarm_model` pin would otherwise override it.

Structure guidance for spawned swarm agents:

- Always pass `label` when spawning (e.g. `label: "api reviewer"`) so the swarm
  UI shows what each agent is for. The explicit `spawn` action rejects missing or
  blank labels.
- In normal and light-swarm mode, only the root session may spawn agents. Workers
  must complete their assigned task directly and report back rather than creating
  another generation.
- Recursive spawning is reserved for a root running in `swarm-deep` mode. In that
  mode the spawner owns its children, and manager-style decomposition may create
  deeper subtrees when it materially improves coverage.
