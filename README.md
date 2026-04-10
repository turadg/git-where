# git-where

A Git extension for navigating repos and worktrees. Built for developers juggling many worktrees across multiple repos.

Copy a branch name from a PR, run `jbr feat/my-branch`, and you're `cd`'d into the right worktree — no matter which repo it lives in or which terminal you're in.

`git where` makes it easy to jump to:

1. **Branch checkout** — `git where checkout <branch>` finds which repo owns a branch and jumps to its worktree (or creates one with `--create`).
2. **File in a repo** — `git where path <query>` resolves a tracked filename to an absolute path, ranked by frecency.
3. **Directory in a repo** — `git where dir <query>` does the same for directories.

When a query is ambiguous, `fzf` opens for interactive selection. When it's unique, the answer prints immediately.

## Requirements

- `git` on `PATH`
- `fzf` on `PATH` (for interactive selection when multiple matches exist)

## Installation

### 1. Install

**Homebrew (macOS):**

```sh
brew install turadg/tap/git-where
```

**Cargo:**

```sh
cargo install git-where
```

**From source:**

```sh
cargo install --path .
```

Git automatically discovers `git-where` as `git where` (no registration needed — git searches `PATH` for `git-<subcommand>` executables).

> **Note:** Use `git where help` for help (not `--help`). Git intercepts `--help` on extensions and tries to open a man page, which we don't ship. `help` is a subcommand that git passes through.

### 2. Set up repos and shell integration

Run the setup command for step-by-step instructions:

```sh
git where setup
```

This shows your tracked repos, shell functions to add, and worktree configuration — everything you need to get started.

### Quick start (manual)

If you prefer to set things up by hand:

```sh
# Track the repos you work in
cd ~/Code/some-monorepo && git where --add-repo
git where --add-repo /path/to/another-repo

# Add shell functions to ~/.zshrc or ~/.bashrc
jp() { local p; p="$(git where path "$1")" || return; cd "$(dirname "$p")"; }
jd() { local d; d="$(git where dir "$1")" || return; cd "$d"; }
jbr() { local d; d="$(git where checkout --create "$1")" || return; cd "$d"; }
```

`git where` prints paths to stdout. The shell functions capture the path and `cd` into it.

## Usage

### `git where checkout <branch> [--create]`

Copy a branch name from a PR page, paste it here, and you're in the right worktree.

1. Searches your tracked repos for one that has the branch (locally or in remote-tracking refs).
2. If exactly one repo has it: that's the answer.
3. If multiple have it: `fzf` asks which.
4. If none have it: `fzf` asks which repo to fetch in, then runs `git fetch origin <branch>`. Errors if the branch doesn't exist on `origin`.
5. Checks `git worktree list` for an existing worktree on that branch — if found, prints its path.
6. With `--create`: creates a worktree at the path determined by `where.worktree-path` using the command from `where.worktree-command`. Without `--create`: prints a hint and exits.

### `git where path [query]`

Searches files tracked by `git ls-files` in the current repo. Matches against the filename only (not the full path). Results are ranked by frecency.

### `git where dir [query]`

Same as `path`, but searches unique parent directories of tracked files.

### `git where --add-repo [path]`

Adds a repo to the tracked list. Defaults to the current repo if no path is given. Idempotent.

### `git where --remove-repo [path]`

Removes a repo from the tracked list. Defaults to the current repo.

### `git where --list-repos`

Prints tracked repo paths, one per line.

## Configuration

All configuration lives in `git config --global` under the `where.*` section.

### `where.repo` (multi-value)

List of absolute repo paths to search when running `checkout`.

```sh
git config --global --add where.repo /Users/you/Code/monorepo
git config --global --get-all where.repo        # list
git config --global --unset where.repo /path     # remove
```

### `where.worktree-path`

Template for computing the worktree directory. Available variables:

| Variable | Value |
|---|---|
| `{repo_path}` | Absolute path to the repo root |
| `{repo}` | Repo directory name |
| `{branch}` | Raw branch name |
| `{branch_sanitized}` | Branch with `/` and `\` replaced by `-` |

Default: `{repo_path}/../{repo}.{branch_sanitized}` (worktrunk sibling convention)

Example — nested worktrees directory:

```sh
git config --global where.worktree-path '{repo_path}.worktrees/{branch_sanitized}'
# Produces: ~/Code/myrepo.worktrees/feat-login
```

### `where.worktree-command`

Template for the command that creates the worktree. Split on whitespace into argv (no shell layer — safe from injection). Available variables:

| Variable | Value |
|---|---|
| `{path}` | Rendered worktree path (from `where.worktree-path`) |
| `{branch}` | Raw branch name |
| `{repo}` | Absolute repo root path |

Default: `git worktree add {path} {branch}`

Example — use [worktrunk](https://worktrunk.dev):

```sh
git config --global where.worktree-command 'wt add {branch}'
```

## Files

| Purpose | Path |
|---|---|
| Frecency state | `$XDG_STATE_HOME/git-where/history.json` (default `~/.local/state/git-where/history.json`) |
| Config | Standard `git config --global` (`~/.gitconfig`) |
