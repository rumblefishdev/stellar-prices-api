# Lore Directory

Context persistence for stateless Claude sessions.

## IMPORTANT: Required Skills

**You MUST use the following skills when working in the `lore/` directory. This is not optional.**

| Skill | When to use | Invoke with |
|-------|-------------|-------------|
| `lore-framework` | General lore work, notes, workflows, session management | `/lore-framework` |
| `lore-framework-git` | ALL git commits (Conventional Commits + task references) | `/lore-framework-git` |
| `lore-framework-tasks` | ALL task updates: status changes, completion, spawning follow-ups | `/lore-framework-tasks` |

**Never update task status, complete tasks, or create follow-up tasks without invoking `/lore-framework-tasks` first.**
**Never make git commits without invoking `/lore-framework-git` first.**

## Session Files

| File | Purpose | Committed |
|------|---------|-----------|
| `0-session/current-user.md` | Who is working (generated) | No |
| `0-session/current-task.md` | Symlink to active task | No |
| `0-session/current-task.json` | Task metadata for agents (id + path) | No |
| `0-session/team.yaml` | Team data (source of truth) | Yes |
| `0-session/next-tasks.md` | Available tasks (auto-generated) | No |
| `README.md` | Full index with Mermaid (heavy) | No — generated |
| `BOARD.md` / `board.json` | Board render (generated) | No |

**Every generated index is gitignored** (`.gitignore:77-80`). Running
`lore-framework_generate-index` therefore changes nothing anyone else can see —
the board is built from the **task files themselves**, so what updates it is
pushing those to `develop`.

**Before coding:** Ensure `0-session/current-user.md` and `0-session/current-task.md` exist.

**Creating a task is one atomic action on `develop`: pull → create → push.**
Pull first, because the `NNNN` sequence is shared across every task and ADR and
an ID picked without pulling collides with whatever a teammate created
meanwhile. Push immediately, because a task left unpushed — or parked on a
feature branch until its PR merges — is invisible to the board and to everyone
else. Never create a task inside a feature branch or a PR; the code goes on the
branch, the task document goes straight to `develop`. See also
[1-tasks/CLAUDE.md](1-tasks/CLAUDE.md).

**Setup:** Use MCP tools `lore_set-user` and `lore_set-task`

## Structure

Each subdirectory has `CLAUDE.md` with local context.

```
lore/
├── 0-session/CLAUDE.md    # Session state
├── 1-tasks/CLAUDE.md      # Task system
│   ├── backlog/CLAUDE.md
│   ├── active/CLAUDE.md
│   ├── blocked/CLAUDE.md
│   └── archive/CLAUDE.md
├── 2-adrs/CLAUDE.md       # ADR frontmatter
└── 3-wiki/CLAUDE.md       # Project docs
```

## Quick Reference

**Format:** `NNNN_TYPE_slug.md` (shared ID sequence for tasks + backlog)

**Task lifecycle:** `backlog/` → `active/` → `blocked/` ↔ `active/` → `archive/`

**Note prefixes:** Q- (Question), I- (Idea), R- (Research), S- (Synthesis), G- (Generation)

**Templates:** Use `_template.md` in each directory.

## Full Documentation

All system docs are in the `/lore-framework` skill.
