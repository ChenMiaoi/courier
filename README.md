# CRIEW

[![build](https://github.com/ChenMiaoi/CRIEW/actions/workflows/ci.yml/badge.svg)](https://github.com/ChenMiaoi/CRIEW/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/criew?label=latest)](https://crates.io/crates/criew)
[![docs](https://docs.rs/criew/badge.svg)](https://docs.rs/criew/)
[![codecov](https://codecov.io/github/ChenMiaoi/CRIEW/graph/badge.svg?token=AH99YLKKPD)](https://codecov.io/github/ChenMiaoi/CRIEW)

CRIEW is a Rust TUI for Linux kernel patch mail workflows.
`CRIEW` stands for `Code Review in Efficient Workflow`.
The repository name stays uppercase as `CRIEW`, while the crate and CLI use lowercase `criew`.

It is built for developers who work on mailing-list-driven review, especially in Linux kernel style flows, and want a terminal-first tool that keeps subscription, sync, review, patch application, and reply in one local workflow.

![CRIEW TUI demo](docs/media/criew-tui-demo.gif)

Chinese usage guide: [README-zh.md](README-zh.md)

## Guide Map

- [Project Overview](#project-overview)
- [Setup](#setup)
- [Usage](#usage)
- [Reference](#reference)

## Project Overview

### Status

CRIEW is under active development. The current `develop` branch already covers the core workflow:

- sync mail from `lore.kernel.org`
- sync a real IMAP `INBOX` through the built-in `My Inbox` subscription
- browse threads and detect patch series
- apply or export patches through `b4`
- compose and send replies from the TUI through `git send-email`

### Release Baseline

`v0.0.1` is the first supported public baseline for CRIEW.
Starting from `v0.0.1`, the project uses only the CRIEW naming set:
`criew`, `~/.criew/`, `criew-config.toml`, `criew.db`, `CRIEW_B4_PATH`, and `CRIEW_IMAP_PROXY`.

Earlier Courier-era names are not treated as a supported upgrade path.
If you tested an older pre-release snapshot or an earlier `v0.0.1` tag before this rename settled,
refresh your checkout or reinstall the binary and bootstrap a new CRIEW runtime directory.

### Features

- Rust CLI with `criew tui`, `criew sync`, `criew doctor`, and `criew version`
- local SQLite storage with automatic runtime bootstrap
- incremental lore sync with checkpoint-based updates
- real IMAP `INBOX` sync with patch-oriented filtering
- background startup sync for enabled subscriptions
- periodic auto-sync for `My Inbox`
- patch series detection for subjects like `[PATCH vN M/N]`
- patch apply/export workflow powered by `b4`
- undo for the most recent successful apply in the current session
- kernel tree browser with source preview
- inline Vim-like editing and external Vim editing
- reply panel that fills `From`, `To`, `Cc`, `Subject`, `In-Reply-To`, and `References`
- real reply delivery through `git send-email`
- visual config editor, command palette completion, and structured operation logs

## Setup

### Requirements

- Rust stable
- Git
- Python 3
  - needed when using the repo-local `vendor/b4/b4.sh` or the embedded runtime fallback
- `b4`
  - CRIEW resolves it in this order: `[b4].path` -> `CRIEW_B4_PATH` -> `./vendor/b4/b4.sh` -> embedded runtime vendor under `~/.criew/vendor/b4/b4.sh` -> `b4` in `PATH`
- `git send-email`
  - only required if you want to send replies

`criew doctor` checks `b4`, `git send-email`, git mail identity, and IMAP connectivity.

### Installation

`crates.io` installation is the recommended path.

#### Install from crates.io

```bash
cargo install criew
```

This build keeps a minimal vendored `b4` runtime embedded in the binary.
If `[b4].path`, `CRIEW_B4_PATH`, and `./vendor/b4/b4.sh` are all unavailable,
CRIEW can materialize that fallback under `~/.criew/vendor/b4/` on first use.
Python 3 is still required for that fallback.

#### Install from a clone

If you want to use the repo-local `./vendor/b4/b4.sh` fallback from a checkout,
clone the repository with submodules:

```bash
git clone --recurse-submodules https://github.com/ChenMiaoi/CRIEW.git
cd CRIEW
cargo install --path . --locked
```

If you already cloned the repository without submodules:

```bash
git submodule update --init --recursive
```

#### Install directly from GitHub

```bash
cargo install --git https://github.com/ChenMiaoi/CRIEW.git --locked criew
```

In this mode, you should provide `b4` through `b4.path`, `CRIEW_B4_PATH`, or your system `PATH`.
If the checkout also includes `vendor/b4`, CRIEW can use it the same way as a source clone.

#### Run from source

```bash
cargo run -- doctor
cargo run -- tui
```

## Usage

### Quick Start

#### 1. Check your environment

```bash
criew doctor
```

#### 2. Prepare configuration

##### Default locations

The default config file is `~/.criew/criew-config.toml`, and the default runtime directory is `~/.criew/`. CRIEW creates a minimal config file automatically on first run.

##### Typical configuration

See [docs/reference/config.example.toml](docs/reference/config.example.toml) for a complete example.

Typical configuration:

```toml
[source]
mailbox = "io-uring"

[imap]
email = "you@example.com"
user = "you@example.com"
pass = "app-password"
server = "imap.example.com"
serverport = 993
encryption = "ssl"

[kernel]
tree = "/path/to/linux"
```

Notes:

- relative paths are resolved from the config file directory
- `[imap]` is optional if you only use lore sync
- when IMAP config is complete, `My Inbox` is enabled by default on first use
- `imap.proxy` supports `http://`, `socks5://`, and `socks5h://`
- reply identity prefers `git config sendemail.from`, then falls back to `git config user.name` and `git config user.email`
- `ui.startup_sync` defaults to `true`
- `ui.inbox_auto_sync_interval_secs` defaults to `30`
- Courier-era names such as `~/.courier`, `courier-config.toml`, `courier.db`, `COURIER_B4_PATH`, and `COURIER_IMAP_PROXY` are intentionally unsupported from `v0.0.1` onward

#### 3. Sync mail

##### Sync a lore mailbox

Sync a lore mailbox:

```bash
criew sync --mailbox io-uring
```

##### Sync a real IMAP inbox

Sync a real IMAP inbox:

```bash
criew sync --mailbox INBOX
```

##### Use local `.eml` fixtures for debugging

Use local `.eml` fixtures for debugging:

```bash
criew sync --mailbox test --fixture-dir ./fixtures
```

#### 4. Start the TUI

##### Inside the TUI

```bash
criew tui
```

Inside the TUI:

- `:` opens the command palette
- the header shows the active keymap scheme (`default`, `vim`, or `custom`)
- `y` / `n` enable or disable the selected subscription
- `Enter` opens the selected mailbox or thread
- `a` applies the current patch series
- `d` exports the current patch series
- `u` undoes the most recent successful apply from the current session
- `r` or `e` opens the reply panel
- `Tab` switches between the mail page and the code browser
- with `ui.keymap = "vim"`, `gg` jumps to the first line in the active pane, `G` jumps to the last line, and `qq` exits quickly

##### Background sync

When IMAP is configured, `My Inbox` joins startup sync and continues periodic background sync while the TUI remains open.
Enabled mailing-list subscriptions also keep doing periodic background sync while the TUI remains open so Linux lore and QEMU archive mailboxes keep pulling new mail.

## Reference

### Documentation

- [README-zh.md](README-zh.md): Chinese usage guide
- [docs/reference/config.example.toml](docs/reference/config.example.toml): configuration example
- [docs/architecture/design.md](docs/architecture/design.md): design notes
- [docs/specs/reply-format-spec.md](docs/specs/reply-format-spec.md): reply panel and sending format
- [docs/milestones/mvp-milestones.md](docs/milestones/mvp-milestones.md): historical milestone record
- [docs/milestones/reply-mvp-milestones.md](docs/milestones/reply-mvp-milestones.md): reply workflow evolution

### Development

Common development commands:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
./scripts/check-coverage.sh
```

The repository includes GitHub Actions CI for `push` and `pull_request` with the same formatting, lint, and test checks.

### Contributing

Issues and pull requests are welcome.

Before sending changes, run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

If you change user-visible behavior, commands, config keys, or workflows, update the relevant documentation in the same change.

### License

CRIEW's Rust code is licensed under [LGPL-2.1](LICENSE).
Bundled vendored components keep their upstream licenses, including `vendor/b4` (GPL-2.0)
and `vendor/b4/patatt` (MIT-0).
