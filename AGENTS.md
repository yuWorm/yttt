## Rules

- Execute Superpowers plans inline; do not delegate plan execution to subagents.
- After completing the Superpowers design and planning workflow, ask whether to launch a reviewer subagent; never launch one automatically.
- Ask for confirmation before removing any entry from `.gitignore`.
- For every task that changes repository files, you MUST create a dedicated Git worktree under `./.worktrees` before the first edit.
- After creating the worktree, you MUST perform all file edits and task commands from that worktree; NEVER edit the primary checkout.
- Treat this as a hard precondition: if the current working directory is not inside the task worktree, stop before using any file-mutating tool or command.
- Before installing test-only hooks, record existing hook registrations and configuration. On success, failure, or cancellation, unregister the test hooks and revert only test-owned configuration changes, preserving pre-existing hooks and unrelated user changes.
- Before deleting test scripts, temporary directories, or worktrees, verify that no hook registration or configuration still references their commands or paths. Delete artifacts only after hook cleanup; deleting a hook script alone is insufficient.