# CRIEW

[![build](https://github.com/ChenMiaoi/CRIEW/actions/workflows/ci.yml/badge.svg)](https://github.com/ChenMiaoi/CRIEW/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/criew?label=latest)](https://crates.io/crates/criew)
[![docs](https://docs.rs/criew/badge.svg)](https://docs.rs/criew/)
[![codecov](https://codecov.io/github/ChenMiaoi/CRIEW/graph/badge.svg?token=AH99YLKKPD)](https://codecov.io/github/ChenMiaoi/CRIEW)

CRIEW is a Rust TUI for Linux kernel patch mail workflows.
It keeps subscription,
sync,
review,
patch application,
and reply in one terminal-first local workflow.
`CRIEW` is the repository name,
while the crate and CLI use lowercase `criew`.

Full documentation lives in the
[CRIEW wiki](https://github.com/ChenMiaoi/CRIEW/wiki).

![CRIEW TUI demo](docs/media/criew-tui-demo.gif)

Chinese quick start: [README-zh.md](README-zh.md)

## Quick Start

```bash
cargo install criew
criew doctor
criew sync --mailbox io-uring
criew tui
```

Use the wiki before enabling IMAP,
patch application,
or reply sending.

## Documentation

- [CRIEW wiki](https://github.com/ChenMiaoi/CRIEW/wiki)
- [Install and Setup](https://github.com/ChenMiaoi/CRIEW/wiki/Install-and-Setup)
- [Configuration](https://github.com/ChenMiaoi/CRIEW/wiki/Configuration)
- [Sync and TUI](https://github.com/ChenMiaoi/CRIEW/wiki/Sync-and-TUI)
- [Patch and Reply](https://github.com/ChenMiaoi/CRIEW/wiki/Patch-and-Reply)
- [Development](https://github.com/ChenMiaoi/CRIEW/wiki/Development)
- [Contribution](https://github.com/ChenMiaoi/CRIEW/wiki/Contribution)
- [API docs on docs.rs](https://docs.rs/criew/)

## Release Baseline

`v0.0.1` is the first supported public baseline for CRIEW.
From `v0.0.1` onward,
CRIEW supports only the CRIEW naming set:
`criew`,
`~/.criew/`,
`criew-config.toml`,
`criew.db`,
`CRIEW_B4_PATH`,
and `CRIEW_IMAP_PROXY`.
Courier-era names are unsupported.

## License

CRIEW's Rust code is licensed under [LGPL-2.1](LICENSE).
Bundled vendored components keep their upstream licenses,
including `vendor/b4` (GPL-2.0)
and `vendor/b4/patatt` (MIT-0).
