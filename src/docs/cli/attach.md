---
title: dekit attach
cli: dekit attach
order: 1
related: [cli, config/user, cli/screen]
---

Attaches your terminal to a task's screen. With no target that is the
console, `@dekit/console`: the task list next to the selected task's
terminal. With a task path you get that task's screen alone.

The task list is in the order of `dekit.yaml`, as |cli/ls| prints it. A
task like `web/dev` sits under a `web/` group, placed where the first of
its tasks comes and shown as `web/…` while collapsed. A group's row shows how
many of its tasks are up; `<h>` and `<l>` collapse and expand it, as does
a click, and starting, stopping, or restarting it acts on every task in
it.

`<C-f>`, or `/` in the task list, finds text in the selected task's
output and scrollback. Copy mode holds the screen still while you type;
every match is highlighted, the cursor goes to the newest, and the copy
badge says which match it is on, as `2/7`. `<Enter>` or `<Up>` goes to
the next match up, in older output, and `<S-Enter>` or `<Down>` the next
one down; both wrap around. A query without capital letters ignores
case. `<Esc>` closes the bar and leaves the view at the match, where `n`
and `N` go on, `v` starts a selection, and `<Esc>` again leaves copy
mode. The bar opens again with the last query, selected, so typing
replaces it.

The runner is started if it is not running, with no tasks (|cli/up|
starts them); `--no-start` fails instead. In the console, `q` asks how
to quit: `q` again stops the runner like |cli/down|, `x` stops it
without saving like |cli/runner/stop|, and `d` detaches and leaves
everything running. `Q` stops the runner like |cli/down| without
asking. Other keys are in |config/user|.

:::usage

```sh
dekit attach
dekit attach web
dekit attach host::@dekit/console
```
