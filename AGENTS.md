# Design Principles

- **All side-effects require user opt-in.** Read-only operations (looking up a path, listing repos) run unconditionally. Operations that modify the filesystem or git state (creating worktrees, fetching branches) require an explicit flag (e.g. `--create`). Never surprise the user with mutations.
