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

The git view has the branch, the changed files, and the commit on the
left, and the selected file's diff on the right. `<j>` and `<k>` select
a file; `<C-a>` or `<Tab>` moves the focus to the diff, where the same
keys scroll it, as do `<C-d>`, `<C-u>`, and the mouse wheel; `<H>` and
`<L>` scroll it sideways. The view keeps up with the repository by
itself: a commit, a checkout, or a `git add` from anywhere shows at
once, and an edit to a file within two seconds.

`<Space>`, or a click on a file's checkbox, stages the file or takes it
back out of the index, and `<a>` does so for all of them. `<d>` throws
away a change that is not staged, after asking: it can't be brought
back. A right click, or `<m>`, opens the file's menu.

`<c>` goes to the commit message, where every key is typed; `<Enter>`
is a new line, `<M-Enter>` commits what is staged, and `<Esc>` goes
back to the files. The message is committed as typed, a line that
starts with `#` included. `<A>` makes the commit amend the last one,
and brings its message back to change; amending a commit the remote has
asks first. When a hook refuses the commit, what it said is shown.

`<p>` pulls and `<P>` pushes, or publishes a branch that has no
upstream; the one there is something to do with stands out. Each runs
as a task, `git pull` or `git push`, with a terminal of its own, so a
hook's output is there to read and a password can be typed: `<o>` shows
that terminal while it runs. The task is removed once it ends well, and
shown with what it said when it fails. A pull is `git pull`, as your
git config has it; when the branch and its upstream have each commits
the other does not and the config does not say how to bring them
together, the console asks: rebase or merge. A push is not started
while there is something to pull.

The console also fetches from the remote now and then while someone is
attached, so that what there is to pull is true; `<f>` fetches now, and
`tui.git.fetch_every` sets how often, or `off` (|config/user|). A fetch
never asks for a password: it runs without a terminal, so one that
needs it fails, and the branch panel says why.

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
