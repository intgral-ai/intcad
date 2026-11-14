# Multi-agent development

This note explains the behaviour rules in [AGENTS.md](../AGENTS.md) with standard
Git, Cargo and CI commands. It is guidance, not a runtime or a tool to install.
IntCAD does not require a particular worktree directory, cache directory, shell,
operating system or filesystem.

## Worktrees and branches

Use one native Git worktree per independent task, so two agents do not switch
branches under each other:

```sh
git fetch origin
git worktree add ../intcad-<task> -b <task-branch> origin/develop
cd ../intcad-<task>
git worktree list
```

Any location Git accepts is fine. Do not assume a directory name, and do not
touch another agent's worktree or branch. A worktree is removed only by its owner
after the checks below:

```sh
git -C ../intcad-<task> status
git -C ../intcad-<task> log --oneline @{u}..HEAD
git worktree remove ../intcad-<task>
git worktree prune
```

`git worktree remove` refuses a dirty worktree and is the preferred command.
Do not use recursive deletion.

## Checks and build output

Run the checks that the change selects, following
[validation-policy.md](validation-policy.md). Prefer the smallest useful test
over a full desktop build. Heavy regression, packaging and cross-platform
validation belong to the existing CI workflows; a local result is evidence for
the worktree state and inputs it ran on, nothing more.

Two worktrees may keep separate Cargo target directories when they build
concurrently. A mutable target directory shared by several worktrees is not a
trusted validation environment: it can hold artifacts from different revisions,
and a Cargo file lock does not prove which source tree produced a binary. Choose
the target location per platform and per developer; no path is mandated. Cargo's
own `target/` is the default.

Before a large local build, check free space with the platform's normal tools:

| Platform | Check |
|---|---|
| Linux / macOS | `df -h .` |
| Windows (PowerShell) | `Get-PSDrive -Name C` |

If space is low, remove only build output that can be regenerated, such as
`cargo clean` in the worktree you own. Do not delete a worktree, source file,
uncommitted change or another agent's cache.

## Checkpoints and recovery

Commit at meaningful stages, not only at the end. Long-running work should leave
recoverable WIP commits on its task branch:

```sh
git status --short
git add -A
git commit -m "WIP: <what is saved>"
git push -u origin HEAD
```

Before switching tasks, stopping, or handing off, report uncommitted and
unpushed work explicitly (`git status`, `git log @{u}..HEAD`). Never discard
uncommitted code to recover disk space. If a push is rejected, preserve the
local commit and resolve the remote state; never force-push `main` or `develop`.

## Cleaning up

When a task is finished or its PR is merged, check your own worktree:

1. The intended work is committed and pushed to the task branch.
2. No other agent or process is using the worktree.
3. No untracked or ignored file is the only copy of something needed.
4. The PR or issue no longer needs the local checkout.

Then remove the worktree with `git worktree remove`. Regenerable build output may
be removed with Cargo's own command. If any condition is uncertain, keep the
files and state the reason instead of forcing a delete.

## CI as the validation boundary

The pull-request CI lane is the authoritative bounded check. The full local gate
is `cargo xtask ci` when the environment can run it; see
[validation-policy.md](validation-policy.md) for when a result may be reused and
what must be rerun. Do not report a check that did not run as passed.

## Resource responsibility

Avoid starting several heavy builds at the same time and avoid leaving idle
caches or worktrees indefinitely. This is coordination between agents, not an
enforced quota: IntCAD provides no scheduler, daemon, disk quota or automatic
cleanup. Developers who need machine-level limits should configure them outside
the repository.
