# CRIEW 中文使用说明

[![build](https://github.com/ChenMiaoi/CRIEW/actions/workflows/ci.yml/badge.svg)](https://github.com/ChenMiaoi/CRIEW/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/criew?label=latest)](https://crates.io/crates/criew)
[![docs](https://docs.rs/criew/badge.svg)](https://docs.rs/criew/)
[![codecov](https://codecov.io/github/ChenMiaoi/CRIEW/graph/badge.svg?token=AH99YLKKPD)](https://codecov.io/github/ChenMiaoi/CRIEW)

CRIEW 是一个面向 Linux kernel patch 邮件工作流的 Rust TUI 工具，用来把“订阅 -> 同步 -> 阅读 -> 应用 patch -> 回复邮件”放进同一条终端内、本地优先的工作流中。
`CRIEW` 的含义是 `Code Review in Efficient Workflow`。
仓库名保持大写 `CRIEW`，crate 和 CLI 命令使用小写 `criew`。

![CRIEW TUI demo](docs/media/criew-tui-demo.gif)

English README: [README.md](README.md)

## 导航

- [项目概览](#项目概览)
- [安装与配置](#安装与配置)
- [使用](#使用)
- [延伸阅读](#延伸阅读)

## 项目概览

### 当前能力

- 同步 `lore.kernel.org` 邮件列表，并按 checkpoint 做增量更新
- 同步真实 IMAP `INBOX`，内置 `My Inbox` 订阅
- 浏览 thread，识别 `[PATCH vN M/N]` patch series
- 通过 `b4` apply 或导出 patch
- 在 TUI 中撰写回信，并通过 `git send-email` 发送
- 回信面板会明确区分可编辑字段（`From/To/Cc/Subject`）和只读线程字段
- `Send Preview` 会在草稿没有实际回复内容时给出警告，并高亮用户自己写的回复正文
- 浏览本地 kernel tree，支持内联 Vim-like 编辑和外部 Vim 编辑

### 发布基线

`v0.0.1` 是 CRIEW 第一版对外支持的发布基线。
从 `v0.0.1` 开始，项目只使用 CRIEW 这一套命名：
`criew`、`~/.criew/`、`criew-config.toml`、`criew.db`、`CRIEW_B4_PATH`、`CRIEW_IMAP_PROXY`。

更早的 Courier 命名不再视为受支持的升级路径。
如果你之前测试过 rename 落定前的预发布快照，或者更早打出的 `v0.0.1` tag，
请重新拉取代码或重新安装，并按当前命名重新初始化 CRIEW 运行目录。

### 依赖

- Rust stable
- Git
- Python 3
  - 当你使用仓库内 `vendor/b4/b4.sh`，或内置的 runtime fallback 时需要
- `b4`
  - CRIEW 的查找顺序是：`[b4].path` -> `CRIEW_B4_PATH` -> `./vendor/b4/b4.sh` -> `~/.criew/vendor/b4/b4.sh` 内置展开副本 -> `b4` in `PATH`
- `git send-email`
  - 仅在发送回信时需要

建议先运行：

```bash
criew doctor
```

它会检查 `b4`、`git send-email`、git 邮件身份和 IMAP 连接状态。

## 安装与配置

### 安装

#### 从 crates.io 安装

```bash
cargo install criew
```

这个安装包会把一个最小可运行的 vendored `b4` fallback 内嵌进二进制。
当 `[b4].path`、`CRIEW_B4_PATH` 和 `./vendor/b4/b4.sh` 都不可用时，
CRIEW 会在首次需要时把它展开到 `~/.criew/vendor/b4/`。
这个 fallback 仍然需要系统可用的 Python 3。

#### 从源码仓库安装

如果你希望直接使用工作区里的 `./vendor/b4/b4.sh`，建议递归拉取子模块：

```bash
git clone --recurse-submodules https://github.com/ChenMiaoi/CRIEW.git
cd CRIEW
cargo install --path . --locked
```

如果仓库已经 clone 下来但没有初始化子模块：

```bash
git submodule update --init --recursive
```

#### 直接从 GitHub 安装

```bash
cargo install --git https://github.com/ChenMiaoi/CRIEW.git --locked criew
```

这种方式下，建议你自行通过 `b4.path`、`CRIEW_B4_PATH` 或系统 `PATH` 提供 `b4`。
如果 checkout 里也带着 `vendor/b4`，CRIEW 也能像源码模式一样直接使用它。

#### 开发时直接运行

```bash
cargo run -- doctor
cargo run -- tui
```

### 配置

#### 默认路径

默认配置文件路径：

```text
~/.criew/criew-config.toml
```

默认运行数据目录：

```text
~/.criew/
```

#### 常见配置

首次运行时，CRIEW 会自动生成一个最小配置文件。完整示例见 [docs/reference/config.example.toml](docs/reference/config.example.toml)。

一个常见配置如下：

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

#### 配置说明

配置说明：

- 相对路径会相对于配置文件所在目录解析
- 只使用 lore 同步时，可以完全不配置 `[imap]`
- IMAP 配置完整后，左栏会出现默认启用的 `My Inbox`
- `imap.proxy` 支持 `http://`、`socks5://`、`socks5h://`
- 回信身份优先读取 `git config sendemail.from`，否则回退到 `git config user.name` / `git config user.email`
- `ui.startup_sync` 默认是 `true`
- `ui.inbox_auto_sync_interval_secs` 默认是 `30`
- `~/.courier`、`courier-config.toml`、`courier.db`、`COURIER_B4_PATH`、`COURIER_IMAP_PROXY` 从 `v0.0.1` 起都不再受支持

## 使用

### 基本使用

#### 1. 环境自检

```bash
criew doctor
```

#### 2. 同步 lore 邮箱

```bash
criew sync --mailbox io-uring
```

#### 3. 同步真实 IMAP 收件箱

```bash
criew sync --mailbox INBOX
```

#### 4. 使用本地 `.eml` fixture 调试

```bash
criew sync --mailbox test --fixture-dir ./fixtures
```

#### 5. 启动 TUI

```bash
criew tui
```

### TUI 常用操作

#### 键位

- `:` 打开命令栏
- 顶部状态栏会显示当前 keymap 方案：`default`、`vim` 或 `custom`
- 数字前缀可重复垂直移动：`default/custom` 使用 `数字 + i/k`，`vim` 使用 `数字 + j/k`
- `y` / `n` 启用或禁用当前订阅
- `Enter` 打开当前 mailbox 或 thread，并自动切到 threads 或 preview pane
- `[` / `]` 按当前聚焦 pane 向左或向右扩充 mail 三栏宽度；`{` / `}` 做对应方向缩小，并持久化到 `ui-state.toml`
- `-` / `=` 在保持 preview pane 焦点时切换上一封或下一封邮件
- `a` apply 当前 patch series
- `d` 导出当前 patch series
- `u` 撤销本次会话中最近一次成功 apply
- `r` 或 `e` 打开回信面板
- `Tab` 在 Mail 页面和 Code Browser 页面之间切换
- 当 `ui.keymap = "vim"` 时，`gg` 跳到当前 pane 行首，`G` 跳到行尾，`qq` 快速退出

#### 命令栏命令

命令栏常见命令：

- `sync`
- `sync <mailbox>`
- `config`
- `vim`
- `restart`
- `quit`
- `!<shell command>`

#### 后台同步

如果 IMAP 配置完整，`My Inbox` 会参与启动自动同步，并在 TUI 保持打开时按配置周期持续做后台增量同步。
启用的邮件列表订阅也会在 TUI 保持打开时按同一周期做后台增量同步，以持续拉取 Linux lore 和 QEMU GNU archive 上的新邮件。

### 回复邮件

CRIEW 的 Reply Panel 会自动填充：

- `From`
- `To`
- `Cc`
- `Subject`
- `In-Reply-To`
- `References`

同时会生成符合 kernel 邮件习惯的引用正文模板。发送时，底层走 `git send-email`。
`Send Preview` 会把用户自己写的未引用正文高亮出来；如果预览里只有引用内容和生成的回复骨架，也会明确给出警告。

## 延伸阅读

### 相关文档

- [README.md](README.md): 英文项目说明
- [docs/reference/config.example.toml](docs/reference/config.example.toml): 配置示例
- [docs/architecture/design.md](docs/architecture/design.md): 设计文档
- [docs/specs/reply-format-spec.md](docs/specs/reply-format-spec.md): 回信格式规范
- [docs/milestones/mvp-milestones.md](docs/milestones/mvp-milestones.md): 历史里程碑
- [docs/milestones/reply-mvp-milestones.md](docs/milestones/reply-mvp-milestones.md): 回信功能演进记录

### 开发

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
./scripts/check-coverage.sh
```

### License

CRIEW 自身的 Rust 代码使用 [LGPL-2.1](LICENSE) 许可证发布。
打包进来的 vendored 组件保留各自上游许可证，包括 `vendor/b4`（GPL-2.0）
和 `vendor/b4/patatt`（MIT-0）。
