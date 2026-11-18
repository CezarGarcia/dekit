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

When the project is in a git repository, the header has a tab for each
of the console's two views, `Tasks` and `Git`, and names the checked-out
branch with how it stands with its upstream: `↑2 ↓3` is two commits to
push and three to pull, `✓` is level. `<2>` or a click on the tab or the
branch shows the git view, and `<1>` goes back to the tasks.

The git view has the branch and the changed files on the left, staged
and not, each with the lines it adds and removes, and the selected
file's diff on the right. `<j>` and `<k>` select a file; `<C-a>` or
`<Tab>` moves the focus to the diff, where the same keys scroll it, as
do `<C-d>`, `<C-u>`, and the mouse wheel; `<H>` and `<L>` scroll it
sideways. It keeps up with the repository by itself: a commit, a
checkout, or a `git add` from anywhere shows at once, and an edit to a
file within two seconds. It only reads: nothing in it changes the
working tree or the index.

The console also fetches from the remote now and then while someone is
attached, so that what there is to pull is true; `<f>` fetches now, and
`tui.git.fetch_every` sets how often, or `off` (|config/user|). A fetch
never asks for a password: the runner has no terminal to ask on, so one
that needs it fails, and the branch panel says why.

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
