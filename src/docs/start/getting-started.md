---
title: Getting started
summary: Install dekit, write a dekit.yaml, and bring a stack up.
related: [start/targets, cli/up, config]
order: 10
---

This dekit is a fork of github.com/pvolok/dekit, installed from
github.com/CezarGarcia/dekit. Install with the script:

```sh
curl -fsSL https://github.com/CezarGarcia/dekit/releases/latest/download/install.sh | sh
```

On Windows (PowerShell):

```powershell
irm https://github.com/CezarGarcia/dekit/releases/latest/download/install.ps1 | iex
```

The scripts put `dekit` in `~/.local/bin`; `DEKIT_INSTALL_DIR` picks
another directory. The latest release is the newest build of `master`,
also called `canary`; `DEKIT_VERSION` installs a tagged one, such as
`1.2.3`.

`dekit update` brings a dekit from the script and its running runners to
the latest version (|cli/update|). npm (`@dekit/cli`) and PyPI
(`dekit-cli`) have the original dekit, without this fork's changes.

## A first project

Describe the stack in `dekit.yaml` at the project root:

```yaml
tasks:
  db:
    cmd: ["postgres", "-D", ".data/db"]
    ready: {log: "ready to accept connections"}

  api:
    cmd: ["cargo", "run", "-p", "api"]
    deps: [db]
    autostart: true

  web:
    cmd: ["npm", "run", "dev"]
    deps: [api]
    autostart: true
```

Then:

```sh
dekit up        # start the autostart tasks and what they need
dekit ls        # see what is running
dekit why web   # explain why a task is (or is not) running
dekit attach    # watch live output in the TUI
```

`db` has no `autostart`, but `api` needs it, so `up` starts it first and
waits until its output says it is ready. `ready` can also wait for a port,
a URL, a command, or a file (|config/tasks#ready|). Another `dekit up`
starts only what is not running.

On Windows, write `web`'s command as `["cmd", "/c", "npm run dev"]` for
now: dekit does not yet start `.cmd` programs such as `npm` directly
(|config/tasks#commands|).

Close the terminal and the tasks keep running. `dekit down` stops
everything for the day, and the next `dekit up` brings it back
(|cli/down|).

:::callout note
The project root is the nearest directory above you with a `dekit.yaml`,
else a git repository, else a `package.json`. `-C <dir>` names it
explicitly (|start/runners|).
:::

