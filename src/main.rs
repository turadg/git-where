//! # git-where
//!
//! A Git extension for navigating repos and worktrees.
//!
//! ## Usage:
//! - `git where checkout <branch>` : Find or create a worktree for a branch.
//! - `git where path [query]`      : Find a tracked file and print its absolute path.
//! - `git where dir [query]`       : Find a directory and print its absolute path.
//!
//! ## Repo management (via git config):
//! - `git where --add-repo [path]`    : Track a repo (defaults to current).
//! - `git where --remove-repo [path]` : Untrack a repo (defaults to current).
//! - `git where --list-repos`         : List tracked repos.
//!
//! ## Configuration (git config --global):
//! - `where.repo`             : Multi-value list of tracked repo paths.
//! - `where.worktree-path`    : Path template for new worktrees.
//! - `where.worktree-command` : Command template for creating worktrees.
//!
//! ## Requirements:
//! - `git` must be installed and available in `PATH`.
//! - `fzf` must be installed for interactive selection.

use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

// ── CLI ──────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "git-where", bin_name = "git-where")]
#[command(about = "Git extension for navigating repos and worktrees")]
#[command(disable_help_flag = true)]
struct Cli {
    /// Add a repo to the tracked list (defaults to current repo if no path given)
    #[arg(long, value_name = "PATH", num_args = 0..=1, default_missing_value = "")]
    add_repo: Option<String>,

    /// Remove a repo from the tracked list (defaults to current repo if no path given)
    #[arg(long, value_name = "PATH", num_args = 0..=1, default_missing_value = "")]
    remove_repo: Option<String>,

    /// List all tracked repos
    #[arg(long)]
    list_repos: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Find or create a worktree for a branch across tracked repos
    Checkout {
        /// Branch name (as shown on the GitHub PR page)
        branch: String,
        /// Create the worktree if it doesn't already exist
        #[arg(long)]
        create: bool,
    },
    /// Find a tracked file and print its absolute path
    Path {
        /// Optional search query (matches against filename only)
        query: Option<String>,
    },
    /// Find a directory containing tracked files and print its absolute path
    Dir {
        /// Optional search query (matches against directory name only)
        query: Option<String>,
    },
    /// Print shell integration and setup instructions
    Setup,
}

// ── Frecency history ─────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Default, Debug)]
struct History {
    /// Map of absolute path to (count, last_access_timestamp)
    entries: HashMap<String, (u64, u64)>,
}

fn get_history_file_path() -> PathBuf {
    xdg_dir("XDG_STATE_HOME", ".local/state")
        .join("git-where")
        .join("history.json")
}

fn load_history(path: &Path) -> History {
    if let Ok(content) = fs::read_to_string(path) {
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        History::default()
    }
}

fn save_history(path: &Path, history: &History) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(content) = serde_json::to_string(history) {
        let _ = fs::write(path, content);
    }
}

fn update_history(history: &mut History, path: String) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let entry = history.entries.entry(path).or_insert((0, 0));
    entry.0 += 1;
    entry.1 = now;
}

// ── XDG helpers ──────────────────────────────────────────────────────────

fn xdg_dir(var: &str, fallback: &str) -> PathBuf {
    match std::env::var(var) {
        Ok(v) if !v.is_empty() => PathBuf::from(v),
        _ => {
            let home = std::env::var("HOME").expect("HOME environment variable not set");
            PathBuf::from(home).join(fallback)
        }
    }
}

// ── Git helpers ──────────────────────────────────────────────────────────

/// Returns the absolute path to a Git repository root.
/// If `dir` is `None`, uses the current working directory.
fn get_repo_root(dir: Option<&Path>) -> Result<PathBuf, String> {
    let mut cmd = Command::new("git");
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    let output = cmd
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| format!("Failed to execute git: {}", e))?;
    if !output.status.success() {
        return Err(match dir {
            Some(d) => format!("{} is not inside a git repository", d.display()),
            None => "Not inside a Git repository".to_string(),
        });
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}

/// Resolves a CLI path argument (or the cwd if empty/absent) to a git repo root.
fn resolve_repo_path(raw: &str) -> PathBuf {
    let dir = if raw.is_empty() {
        None
    } else {
        Some(Path::new(raw))
    };
    get_repo_root(dir).unwrap_or_else(|e| {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    })
}

/// Returns a list of all tracked files relative to the repo root.
fn list_tracked_files(repo_root: &Path) -> Result<Vec<String>, String> {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(["ls-files"])
        .output()
        .map_err(|e| format!("Failed to execute git ls-files: {}", e))?;
    if !output.status.success() {
        return Err("Failed to list tracked files".to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|s| s.to_string())
        .collect())
}

// ── Git config helpers ───────────────────────────────────────────────────

fn git_config_get(key: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["config", "--global", "--get", key])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

fn git_config_get_all(key: &str) -> Vec<String> {
    let output = Command::new("git")
        .args(["config", "--global", "--get-all", key])
        .output();
    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| !l.is_empty())
            .map(|s| s.to_string())
            .collect(),
        _ => Vec::new(),
    }
}

fn git_config_add(key: &str, value: &str) -> Result<(), String> {
    let status = Command::new("git")
        .args(["config", "--global", "--add", key, value])
        .status()
        .map_err(|e| format!("git config: {}", e))?;
    if !status.success() {
        return Err("git config --add failed".to_string());
    }
    Ok(())
}

fn git_config_unset_value(key: &str, value_regex: &str) -> Result<(), String> {
    let status = Command::new("git")
        .args(["config", "--global", "--unset", key, value_regex])
        .status()
        .map_err(|e| format!("git config: {}", e))?;
    if !status.success() {
        return Err(format!("'{}' not found in {}", value_regex, key));
    }
    Ok(())
}

// ── Repo management ──────────────────────────────────────────────────────

const CONFIG_KEY_REPO: &str = "where.repo";
const CONFIG_KEY_WORKTREE_PATH: &str = "where.worktree-path";
const CONFIG_KEY_WORKTREE_COMMAND: &str = "where.worktree-command";

fn load_tracked_repos() -> Result<Vec<PathBuf>, String> {
    let repos: Vec<PathBuf> = git_config_get_all(CONFIG_KEY_REPO)
        .into_iter()
        .map(PathBuf::from)
        .collect();
    if repos.is_empty() {
        return Err("No repos configured. Run:\n  git where --add-repo [path]".to_string());
    }
    Ok(repos)
}

fn handle_add_repo(raw: &str) {
    let resolved = resolve_repo_path(raw);
    let target = resolved.display().to_string();

    let existing = git_config_get_all(CONFIG_KEY_REPO);
    if existing.iter().any(|r| r == &target) {
        eprintln!("Already tracked: {}", target);
        return;
    }

    if let Err(e) = git_config_add(CONFIG_KEY_REPO, &target) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
    eprintln!("Added: {}", target);
}

fn handle_remove_repo(raw: &str) {
    let resolved = resolve_repo_path(raw);
    let target = resolved.display().to_string();

    // git config --unset uses a POSIX regex to match the value.
    // Anchor with ^ and $ and escape regex-special chars in the path.
    let escaped = regex_escape(&target);
    let pattern = format!("^{}$", escaped);

    if let Err(e) = git_config_unset_value(CONFIG_KEY_REPO, &pattern) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
    eprintln!("Removed: {}", target);
}

fn handle_list_repos() {
    let repos = git_config_get_all(CONFIG_KEY_REPO);
    if repos.is_empty() {
        eprintln!("No repos configured. Run: git where --add-repo [path]");
        std::process::exit(1);
    }
    for r in repos {
        println!("{}", r);
    }
}

/// Escapes characters special to POSIX basic regex so a literal path can be
/// used as a `git config --unset` value pattern.
fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        if ".[\\*^$".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

// ── checkout ─────────────────────────────────────────────────────────────

fn handle_checkout(branch: &str, create: bool) {
    let repos = load_tracked_repos().unwrap_or_else(|e| {
        eprintln!("{}", e);
        std::process::exit(1);
    });

    let mut matches: Vec<PathBuf> = repos
        .iter()
        .filter(|r| repo_has_branch(r, branch))
        .cloned()
        .collect();

    let repo = match matches.len() {
        1 => matches.remove(0),
        0 => {
            let header = format!("Branch '{}' not found. Pick a repo to fetch from:", branch);
            let chosen = prompt_select_repo(&repos, &header)
                .unwrap_or_else(|| std::process::exit(1));
            eprintln!("Fetching origin/{} in {}...", branch, chosen.display());
            let status = Command::new("git")
                .current_dir(&chosen)
                .args(["fetch", "origin", branch])
                .status();
            let ok = std::matches!(status, Ok(s) if s.success());
            if !ok || !repo_has_branch(&chosen, branch) {
                eprintln!(
                    "Error: branch '{}' does not exist on origin in {}",
                    branch,
                    chosen.display()
                );
                std::process::exit(1);
            }
            chosen
        }
        _ => {
            let header = format!("Branch '{}' found in multiple repos:", branch);
            prompt_select_repo(&matches, &header)
                .unwrap_or_else(|| std::process::exit(1))
        }
    };

    // If a worktree already exists for this branch, just print its path.
    if let Some(existing) = find_worktree_for_branch(&repo, branch) {
        println!("{}", existing.display());
        return;
    }

    // Render the worktree path from the template.
    let wt_path = render_worktree_path(&repo, branch).unwrap_or_else(|e| {
        eprintln!("Error in worktree-path template: {}", e);
        std::process::exit(1);
    });

    if !create {
        eprintln!("No worktree for '{}'. To create one, run:", branch);
        eprintln!("  git where checkout --create {}", branch);
        std::process::exit(1);
    }

    if wt_path.exists() {
        eprintln!(
            "Error: {} already exists but is not a worktree for '{}'",
            wt_path.display(),
            branch
        );
        std::process::exit(1);
    }

    // Ensure parent directories exist (needed for nested templates like
    // `{repo_path}.worktrees/{branch_sanitized}`).
    if let Some(parent) = wt_path.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        eprintln!("Error creating {}: {}", parent.display(), e);
        std::process::exit(1);
    }

    // Run the worktree-command template.
    let wt_path_str = wt_path.display().to_string();
    exec_worktree_command(&repo, branch, &wt_path_str).unwrap_or_else(|e| {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    });

    println!("{}", wt_path.display());
}

/// Returns true if `branch` exists as a local branch or under any
/// remote-tracking ref in `repo`.
fn repo_has_branch(repo: &Path, branch: &str) -> bool {
    let output = Command::new("git")
        .current_dir(repo)
        .args([
            "for-each-ref",
            "--format=%(refname)",
            "refs/heads/",
            "refs/remotes/",
        ])
        .output();
    let Ok(output) = output else { return false };
    if !output.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    refs_contain_branch(&stdout, branch)
}

/// Parses `git for-each-ref` output to check if a branch exists.
fn refs_contain_branch(refs_output: &str, branch: &str) -> bool {
    for line in refs_output.lines() {
        if let Some(rest) = line.strip_prefix("refs/heads/") {
            if rest == branch {
                return true;
            }
        } else if let Some(rest) = line.strip_prefix("refs/remotes/")
            && let Some(slash) = rest.find('/')
            && &rest[slash + 1..] == branch
        {
            return true;
        }
    }
    false
}

/// Searches `git worktree list --porcelain` for an existing worktree on `branch`.
fn find_worktree_for_branch(repo: &Path, branch: &str) -> Option<PathBuf> {
    let output = Command::new("git")
        .current_dir(repo)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_worktree_for_branch(&stdout, branch)
}

/// Parses `git worktree list --porcelain` output to find a worktree path for `branch`.
fn parse_worktree_for_branch(porcelain: &str, branch: &str) -> Option<PathBuf> {
    let target = format!("refs/heads/{}", branch);
    let mut current_path: Option<PathBuf> = None;
    for line in porcelain.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            current_path = Some(PathBuf::from(p));
        } else if let Some(b) = line.strip_prefix("branch ") {
            if b == target {
                return current_path;
            }
        } else if line.is_empty() {
            current_path = None;
        }
    }
    None
}

// ── Template engine ──────────────────────────────────────────────────────

const DEFAULT_WORKTREE_PATH: &str =
    "{repo_path}/../{repo}.{branch_sanitized}";
const DEFAULT_WORKTREE_COMMAND: &str =
    "git worktree add {path} {branch}";

/// Sanitizes a branch name for filesystem use: `/` and `\` become `-`.
fn sanitize_branch(branch: &str) -> String {
    branch
        .chars()
        .map(|c| if c == '/' || c == '\\' { '-' } else { c })
        .collect()
}

/// Renders the `where.worktree-path` template into a normalized absolute path.
///
/// Available variables: `{repo_path}`, `{repo}`, `{branch}`, `{branch_sanitized}`.
fn render_worktree_path(repo: &Path, branch: &str) -> Result<PathBuf, String> {
    let template = git_config_get(CONFIG_KEY_WORKTREE_PATH)
        .unwrap_or_else(|| DEFAULT_WORKTREE_PATH.to_string());

    let vars = worktree_path_vars(repo, branch);
    let rendered = render_template(&template, &vars)?;
    Ok(normalize_path(Path::new(&rendered)))
}

/// Builds the variable map for worktree-path templates.
fn worktree_path_vars(repo: &Path, branch: &str) -> HashMap<&'static str, String> {
    let mut vars = HashMap::new();
    vars.insert("repo_path", repo.display().to_string());
    vars.insert(
        "repo",
        repo.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    vars.insert("branch", branch.to_string());
    vars.insert("branch_sanitized", sanitize_branch(branch));
    vars
}

/// Executes the `where.worktree-command` template to create a worktree.
///
/// The template is split on whitespace into argv tokens, then each token has
/// `{var}` placeholders substituted. This avoids a shell layer entirely —
/// no injection risk from branch names or paths containing special characters.
///
/// Available variables: `{path}`, `{branch}`, `{repo}`.
fn exec_worktree_command(repo: &Path, branch: &str, wt_path: &str) -> Result<(), String> {
    let template = git_config_get(CONFIG_KEY_WORKTREE_COMMAND)
        .unwrap_or_else(|| DEFAULT_WORKTREE_COMMAND.to_string());

    let mut vars: HashMap<&str, String> = HashMap::new();
    vars.insert("path", wt_path.to_string());
    vars.insert("branch", branch.to_string());
    vars.insert("repo", repo.display().to_string());

    let tokens: Vec<String> = template
        .split_whitespace()
        .map(|tok| render_template(tok, &vars))
        .collect::<Result<Vec<_>, _>>()?;

    if tokens.is_empty() {
        return Err("worktree-command template is empty".to_string());
    }

    eprintln!("Creating worktree: {}", wt_path);
    // Suppress the child's stdout so only our final println goes to
    // stdout. Git's useful progress output (Preparing worktree,
    // Updating files) goes to stderr and is still visible. Only the
    // "HEAD is now at..." confirmation is suppressed.
    let status = Command::new(&tokens[0])
        .args(&tokens[1..])
        .current_dir(repo)
        .stdout(Stdio::null())
        .status()
        .map_err(|e| format!("failed to run '{}': {}", tokens[0], e))?;

    if !status.success() {
        return Err(format!("command failed: {}", template));
    }
    Ok(())
}

/// Simple `{var}` template renderer. No shell involved.
fn render_template(template: &str, vars: &HashMap<&str, String>) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after
            .find('}')
            .ok_or_else(|| format!("unclosed '{{' in template: {}", template))?;
        let var = &after[..end];
        let value = vars
            .get(var)
            .ok_or_else(|| format!("unknown template variable: '{}'", var))?;
        out.push_str(value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Lexically normalizes a path (collapses `.` and `..` without filesystem access).
fn normalize_path(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

// ── path / dir selection ─────────────────────────────────────────────────

fn derive_directories(files: &[String]) -> Vec<String> {
    let mut dirs = HashSet::new();
    for file in files {
        let path = Path::new(file);
        let mut ancestors = path.ancestors();
        ancestors.next(); // skip the file itself
        for ancestor in ancestors {
            let s = ancestor.to_string_lossy();
            if !s.is_empty() && s != "." {
                dirs.insert(s.to_string());
            }
        }
    }
    let mut result: Vec<_> = dirs.into_iter().collect();
    result.sort();
    result
}

fn handle_selection(
    items: Vec<String>,
    query: Option<String>,
    repo_root: &Path,
    history: &mut History,
    history_path: &Path,
) {
    let query_lower = query.as_ref().map(|q| q.to_lowercase());

    let mut candidates: Vec<_> = items
        .into_iter()
        .filter(|item| {
            if let Some(ref q) = query_lower {
                let path = Path::new(item);
                if let Some(last_comp) = path.file_name() {
                    last_comp.to_string_lossy().to_lowercase().contains(q)
                } else {
                    false
                }
            } else {
                true
            }
        })
        .collect();

    if candidates.is_empty() {
        std::process::exit(1);
    }

    // Pre-compute scores once to avoid allocating inside the sort comparator.
    let scores: Vec<(u64, u64)> = candidates
        .iter()
        .map(|c| {
            let abs = repo_root.join(c).to_string_lossy().into_owned();
            history.entries.get(&abs).copied().unwrap_or((0, 0))
        })
        .collect();
    let mut pairs: Vec<(String, (u64, u64))> = candidates
        .into_iter()
        .zip(scores)
        .collect();
    pairs.sort_by(|(a, sa), (b, sb)| sb.cmp(sa).then_with(|| a.cmp(b)));
    candidates = pairs.into_iter().map(|(c, _)| c).collect();


    let selected = if candidates.len() == 1 {
        candidates[0].clone()
    } else {
        match run_fzf(&candidates, query.as_deref(), None) {
            Ok(sel) => sel,
            Err(e) => {
                eprintln!("{}", e);
                std::process::exit(1);
            }
        }
    };

    let abs_path = repo_root.join(&selected).display().to_string();
    println!("{}", abs_path);

    update_history(history, abs_path);
    save_history(history_path, history);
}

// ── fzf ──────────────────────────────────────────────────────────────────

fn run_fzf(items: &[String], query: Option<&str>, header: Option<&str>) -> Result<String, String> {
    let mut child = Command::new("fzf")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .args(["--tiebreak=index", "--header-first"])
        .args(header.map(|h| vec!["--header".to_string(), h.to_string()]).unwrap_or_default())
        .args(query.map(|q| vec!["--query".to_string(), q.to_string()]).unwrap_or_default())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "fzf is not available in PATH. Please install fzf for interactive selection."
                    .to_string()
            } else {
                format!("Failed to spawn fzf: {}", e)
            }
        })?;

    let mut stdin = child.stdin.take().expect("Failed to open stdin");
    let input = items.join("\n");
    stdin
        .write_all(input.as_bytes())
        .map_err(|e| format!("Failed to write to fzf stdin: {}", e))?;
    drop(stdin);

    let mut stdout = child.stdout.take().expect("Failed to open stdout");
    let mut selected = String::new();
    stdout
        .read_to_string(&mut selected)
        .map_err(|e| format!("Failed to read from fzf stdout: {}", e))?;

    let status = child
        .wait()
        .map_err(|e| format!("Failed to wait for fzf: {}", e))?;
    if !status.success() {
        std::process::exit(1);
    }

    let selected = selected.trim().to_string();
    if selected.is_empty() {
        return Err("No selection made".to_string());
    }
    Ok(selected)
}

fn prompt_select_repo(repos: &[PathBuf], header: &str) -> Option<PathBuf> {
    let items: Vec<String> = repos.iter().map(|p| p.display().to_string()).collect();
    run_fzf(&items, None, Some(header)).ok().map(PathBuf::from)
}

// ── setup ───────────────────────────────────────────────────────────────

fn handle_setup() {
    let repos = git_config_get_all(CONFIG_KEY_REPO);
    let has_repos = !repos.is_empty();

    eprintln!("git-where setup");
    eprintln!("===============");
    eprintln!();

    // 1. Repo tracking status
    eprintln!("1. Track repos");
    eprintln!();
    if has_repos {
        eprintln!("   Tracked repos:");
        for r in &repos {
            eprintln!("     {}", r);
        }
        eprintln!();
        eprintln!("   To add more:  git where --add-repo /path/to/repo");
    } else {
        eprintln!("   No repos tracked yet. Add repos you work in:");
        eprintln!();
        eprintln!("     cd ~/Code/some-repo && git where --add-repo");
        eprintln!("     git where --add-repo /path/to/another-repo");
    }
    eprintln!();

    // 2. Shell integration
    eprintln!("2. Shell integration");
    eprintln!();
    eprintln!("   Add these functions to your shell config (~/.zshrc, ~/.bashrc, etc.):");
    eprintln!();
    eprintln!("     # Jump to a tracked file's directory");
    eprintln!("     jp() {{ local p; p=\"$(git where path \"$1\")\" || return; cd \"$(dirname \"$p\")\"; }}");
    eprintln!();
    eprintln!("     # Jump to a directory in the current repo");
    eprintln!("     jd() {{ local d; d=\"$(git where dir \"$1\")\" || return; cd \"$d\"; }}");
    eprintln!();
    eprintln!("     # Jump to (or create) a worktree for a branch");
    eprintln!("     jbr() {{ local d; d=\"$(git where checkout --create \"$1\")\" || return; cd \"$d\"; }}");
    eprintln!();

    // 3. Optional: worktree path template
    let wt_path = git_config_get(CONFIG_KEY_WORKTREE_PATH);
    eprintln!("3. Worktree path template (optional)");
    eprintln!();
    match wt_path {
        Some(ref t) => eprintln!("   Current: {}", t),
        None => eprintln!("   Using default: {}", DEFAULT_WORKTREE_PATH),
    }
    eprintln!();
    eprintln!("   To customize:");
    eprintln!("     git config --global where.worktree-path '{{repo_path}}.worktrees/{{branch_sanitized}}'");
    eprintln!();
}

// ── main ─────────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();

    // Handle repo-management flags first (they exit after running).
    if let Some(ref raw) = cli.add_repo {
        handle_add_repo(raw);
        return;
    }
    if let Some(ref raw) = cli.remove_repo {
        handle_remove_repo(raw);
        return;
    }
    if cli.list_repos {
        handle_list_repos();
        return;
    }

    // Subcommand dispatch.
    match cli.command {
        Some(Commands::Setup) => {
            handle_setup();
        }
        Some(Commands::Checkout { branch, create }) => {
            handle_checkout(&branch, create);
        }
        Some(Commands::Path { query }) => {
            let (repo_root, tracked_files) = load_repo_and_files();
            let history_path = get_history_file_path();
            let mut history = load_history(&history_path);
            handle_selection(tracked_files, query, &repo_root, &mut history, &history_path);
        }
        Some(Commands::Dir { query }) => {
            let (repo_root, tracked_files) = load_repo_and_files();
            let history_path = get_history_file_path();
            let mut history = load_history(&history_path);
            let directories = derive_directories(&tracked_files);
            handle_selection(directories, query, &repo_root, &mut history, &history_path);
        }
        None => {
            eprintln!("No command given. Run `git where help` for usage.");
            std::process::exit(1);
        }
    }
}

fn load_repo_and_files() -> (PathBuf, Vec<String>) {
    let repo_root = get_repo_root(None).unwrap_or_else(|e| {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    });
    let tracked = list_tracked_files(&repo_root).unwrap_or_else(|e| {
        eprintln!("Error listing tracked files: {}", e);
        std::process::exit(1);
    });
    (repo_root, tracked)
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── render_template ─────────────────────────────────────────────

    fn vars(pairs: &[(&'static str, &str)]) -> HashMap<&'static str, String> {
        pairs.iter().map(|(k, v)| (*k, v.to_string())).collect()
    }

    #[test]
    fn render_template_basic_substitution() {
        let v = vars(&[("name", "world")]);
        assert_eq!(render_template("hello {name}!", &v).unwrap(), "hello world!");
    }

    #[test]
    fn render_template_multiple_vars() {
        let v = vars(&[("a", "1"), ("b", "2")]);
        assert_eq!(render_template("{a}+{b}", &v).unwrap(), "1+2");
    }

    #[test]
    fn render_template_no_vars() {
        let v = vars(&[]);
        assert_eq!(render_template("plain text", &v).unwrap(), "plain text");
    }

    #[test]
    fn render_template_adjacent_vars() {
        let v = vars(&[("x", "a"), ("y", "b")]);
        assert_eq!(render_template("{x}{y}", &v).unwrap(), "ab");
    }

    #[test]
    fn render_template_unclosed_brace() {
        let v = vars(&[]);
        assert!(render_template("bad {template", &v).is_err());
    }

    #[test]
    fn render_template_unknown_var() {
        let v = vars(&[]);
        assert!(render_template("{missing}", &v).is_err());
    }

    #[test]
    fn render_template_real_worktree_path() {
        let v = vars(&[
            ("repo_path", "/home/user/code/myrepo"),
            ("repo", "myrepo"),
            ("branch", "feat/login"),
            ("branch_sanitized", "feat-login"),
        ]);
        let result = render_template("{repo_path}/../{repo}.{branch_sanitized}", &v).unwrap();
        assert_eq!(result, "/home/user/code/myrepo/../myrepo.feat-login");
    }

    // ── normalize_path ──────────────────────────────────────────────

    #[test]
    fn normalize_path_collapses_parent() {
        assert_eq!(
            normalize_path(Path::new("/a/b/../c")),
            PathBuf::from("/a/c")
        );
    }

    #[test]
    fn normalize_path_collapses_dot() {
        assert_eq!(
            normalize_path(Path::new("/a/./b")),
            PathBuf::from("/a/b")
        );
    }

    #[test]
    fn normalize_path_multiple_parents() {
        assert_eq!(
            normalize_path(Path::new("/a/b/c/../../d")),
            PathBuf::from("/a/d")
        );
    }

    #[test]
    fn normalize_path_real_template_output() {
        // The default template produces paths like /home/user/code/myrepo/../myrepo.feat-login
        assert_eq!(
            normalize_path(Path::new("/home/user/code/myrepo/../myrepo.feat-login")),
            PathBuf::from("/home/user/code/myrepo.feat-login")
        );
    }

    // ── sanitize_branch ─────────────────────────────────────────────

    #[test]
    fn sanitize_branch_replaces_slashes() {
        assert_eq!(sanitize_branch("feat/login"), "feat-login");
    }

    #[test]
    fn sanitize_branch_multiple_slashes() {
        assert_eq!(sanitize_branch("feat/scope/thing"), "feat-scope-thing");
    }

    #[test]
    fn sanitize_branch_backslashes() {
        assert_eq!(sanitize_branch("feat\\login"), "feat-login");
    }

    #[test]
    fn sanitize_branch_no_slashes() {
        assert_eq!(sanitize_branch("main"), "main");
    }

    // ── regex_escape ────────────────────────────────────────────────

    #[test]
    fn regex_escape_plain_path() {
        assert_eq!(regex_escape("/home/user/code"), "/home/user/code");
    }

    #[test]
    fn regex_escape_dots_in_path() {
        assert_eq!(
            regex_escape("/home/user/my.project"),
            "/home/user/my\\.project"
        );
    }

    #[test]
    fn regex_escape_all_special_chars() {
        assert_eq!(regex_escape(".[\\*^$"), "\\.\\[\\\\\\*\\^\\$");
    }

    // ── derive_directories ──────────────────────────────────────────

    #[test]
    fn derive_directories_basic() {
        let files = vec!["src/main.rs".to_string(), "src/lib.rs".to_string()];
        assert_eq!(derive_directories(&files), vec!["src"]);
    }

    #[test]
    fn derive_directories_nested() {
        let files = vec!["a/b/c.rs".to_string()];
        let dirs = derive_directories(&files);
        assert!(dirs.contains(&"a".to_string()));
        assert!(dirs.contains(&"a/b".to_string()));
    }

    #[test]
    fn derive_directories_deduplicates() {
        let files = vec![
            "src/foo.rs".to_string(),
            "src/bar.rs".to_string(),
            "src/sub/baz.rs".to_string(),
        ];
        let dirs = derive_directories(&files);
        assert_eq!(dirs.iter().filter(|d| d.as_str() == "src").count(), 1);
    }

    #[test]
    fn derive_directories_excludes_root() {
        let files = vec!["top.rs".to_string()];
        assert!(derive_directories(&files).is_empty());
    }

    // ── update_history ──────────────────────────────────────────────

    #[test]
    fn update_history_new_entry() {
        let mut h = History::default();
        update_history(&mut h, "/some/path".to_string());
        let (count, ts) = h.entries["/some/path"];
        assert_eq!(count, 1);
        assert!(ts > 0);
    }

    #[test]
    fn update_history_increments() {
        let mut h = History::default();
        update_history(&mut h, "/p".to_string());
        update_history(&mut h, "/p".to_string());
        assert_eq!(h.entries["/p"].0, 2);
    }

    // ── refs_contain_branch (extracted from repo_has_branch) ────────

    #[test]
    fn refs_contain_branch_local() {
        let refs = "refs/heads/main\nrefs/heads/feat/login\n";
        assert!(refs_contain_branch(refs, "main"));
        assert!(refs_contain_branch(refs, "feat/login"));
        assert!(!refs_contain_branch(refs, "develop"));
    }

    #[test]
    fn refs_contain_branch_remote() {
        let refs = "refs/remotes/origin/main\nrefs/remotes/origin/feat/login\n";
        assert!(refs_contain_branch(refs, "main"));
        assert!(refs_contain_branch(refs, "feat/login"));
    }

    #[test]
    fn refs_contain_branch_multiple_remotes() {
        let refs = "refs/remotes/origin/main\nrefs/remotes/upstream/main\n";
        assert!(refs_contain_branch(refs, "main"));
    }

    #[test]
    fn refs_contain_branch_empty() {
        assert!(!refs_contain_branch("", "main"));
    }

    // ── parse_worktree_for_branch (extracted from find_worktree_for_branch)

    #[test]
    fn parse_worktree_finds_matching_branch() {
        let porcelain = "\
worktree /home/user/code/myrepo
HEAD abc123
branch refs/heads/main

worktree /home/user/code/myrepo.feat-login
HEAD def456
branch refs/heads/feat/login

";
        assert_eq!(
            parse_worktree_for_branch(porcelain, "feat/login"),
            Some(PathBuf::from("/home/user/code/myrepo.feat-login"))
        );
    }

    #[test]
    fn parse_worktree_returns_none_when_missing() {
        let porcelain = "\
worktree /home/user/code/myrepo
HEAD abc123
branch refs/heads/main

";
        assert_eq!(parse_worktree_for_branch(porcelain, "develop"), None);
    }

    #[test]
    fn parse_worktree_empty_input() {
        assert_eq!(parse_worktree_for_branch("", "main"), None);
    }

    #[test]
    fn parse_worktree_detached_head() {
        // Detached worktrees have no "branch" line — they should not match.
        let porcelain = "\
worktree /home/user/code/myrepo
HEAD abc123
branch refs/heads/main

worktree /tmp/detached
HEAD def456
detached

";
        assert_eq!(parse_worktree_for_branch(porcelain, "main"),
            Some(PathBuf::from("/home/user/code/myrepo")));
        assert_eq!(parse_worktree_for_branch(porcelain, "detached"), None);
    }
}
