## wherewasi

The user keeps a live "where was I?" overview of this repo in a herdr sidebar. It is how they catch up after switching between tabs and agents, so keep it current as you work. Update it with these commands (run from inside the repo; the first one creates the file):

- `wherewasi now "TEXT"`: what you are working on right now. Set it when you start or change focus.
- `wherewasi did "TEXT"`: log something you just finished. Newest entries go first.
- `wherewasi todo "TEXT"` / `wherewasi check "TEXT"`: add an open task, or tick the first open task containing TEXT.
- `wherewasi decide "TEXT"`: record a decision and its reason.
- `wherewasi show`: print the whole file.

For multi-step work, write your steps down with `wherewasi todo` before you start and `wherewasi check` them as you go; this is your written checklist. If you already keep a plan with a built-in tool (a task list, `update_plan`, plan mode), that plan is mirrored into `## Plan` automatically, and finished steps and commits are logged for you, so don't repeat them.

Write for the user, who reads it at a glance: one plain line per entry, past tense for `did`, no code, diffs or secrets. Log meaningful steps, not every edit. If the user edits the file by hand, their wording wins; build on it rather than rewriting it.
