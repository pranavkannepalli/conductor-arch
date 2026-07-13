# Archductor Agent Instructions

Read the full Codex instructions in `.codex/AGENTS.md`, then read the current
status in `progress.md`.

Before calling behavior done, verify with written tests plus relevant CLI smoke
and GTK smoke. Keep CLI and GTK behavior inline; do not land a user-visible
change in only one surface unless you report the other as incomplete.

Old one-off implementation plans/specs have been pruned from `docs/`; use the
durable docs listed in `.codex/AGENTS.md` instead of dated task artifacts.

## Project Context

Archductor work lives in the Linear `Archductor` project on the `Perceo` team.

When pulling work from Linear in this repository:

- Query the `Perceo` team and `Archductor` project specifically.
- Be specific with Linear queries: team, project, status, assignee, issue key,
  labels, and relevant text.
- Link related Linear issues in branch/PR context when an issue key is present.
- When starting a Linear task, move it to `In Progress`.
- When finishing a Linear task, move it to `In Review` so the user can review and push.
