# CRIEW 中文使用说明

[![build](https://github.com/ChenMiaoi/CRIEW/actions/workflows/ci.yml/badge.svg)](https://github.com/ChenMiaoi/CRIEW/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/criew?label=latest)](https://crates.io/crates/criew)
[![docs](https://docs.rs/criew/badge.svg)](https://docs.rs/criew/)
[![codecov](https://codecov.io/github/ChenMiaoi/CRIEW/graph/badge.svg?token=AH99YLKKPD)](https://codecov.io/github/ChenMiaoi/CRIEW)

CRIEW 是一个面向 Linux kernel patch 邮件工作流的 Rust TUI 工具，
把“订阅 -> 同步 -> 阅读 -> 应用 patch -> 回复邮件”
放进同一条终端内、本地优先的工作流中。
仓库名保持大写 `CRIEW`，
crate 和 CLI 命令使用小写 `criew`。

完整文档现在以 wiki 为主：
[CRIEW wiki](https://github.com/ChenMiaoi/CRIEW/wiki)

![CRIEW TUI demo](docs/media/criew-tui-demo.gif)

English README: [README.md](README.md)

## 快速开始

```bash
cargo install criew
criew doctor
criew sync --mailbox io-uring
criew tui
```

启用 IMAP、
patch apply、
或回信发送前，
请先阅读 wiki 中对应的页面。

## 文档

- [CRIEW wiki](https://github.com/ChenMiaoi/CRIEW/wiki)
- [安装与初始化](https://github.com/ChenMiaoi/CRIEW/wiki/Install-and-Setup)
- [配置说明](https://github.com/ChenMiaoi/CRIEW/wiki/Configuration)
- [同步与 TUI](https://github.com/ChenMiaoi/CRIEW/wiki/Sync-and-TUI)
- [Patch 与回信](https://github.com/ChenMiaoi/CRIEW/wiki/Patch-and-Reply)
- [开发与本地 wiki 构建](https://github.com/ChenMiaoi/CRIEW/wiki/Development)
- [贡献流程](https://github.com/ChenMiaoi/CRIEW/wiki/Contribution)
- [docs.rs API 文档](https://docs.rs/criew/)

## 发布基线

`v0.0.1` 是 CRIEW 第一版对外支持的发布基线。
从 `v0.0.1` 开始，
项目只支持 CRIEW 这一套命名：
`criew`、
`~/.criew/`,
`criew-config.toml`,
`criew.db`,
`CRIEW_B4_PATH`,
和 `CRIEW_IMAP_PROXY`。
Courier 时代的命名不再受支持。

## License

CRIEW 自身的 Rust 代码使用 [LGPL-2.1](LICENSE) 许可证发布。
打包进来的 vendored 组件保留各自上游许可证，
包括 `vendor/b4`（GPL-2.0）
和 `vendor/b4/patatt`（MIT-0）。
