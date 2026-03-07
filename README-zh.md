# Courier 中文使用说明

Courier 是一个面向 Linux kernel patch 邮件工作流的 Rust TUI 工具，用来把“订阅 -> 同步 -> 阅读 -> 应用 patch -> 回复邮件”放进同一条终端内、本地优先的工作流中。

English README: [README.md](README.md)

## 当前能力

- 同步 `lore.kernel.org` 邮件列表，并按 checkpoint 做增量更新
- 同步真实 IMAP `INBOX`，内置 `My Inbox` 订阅
- 浏览 thread，识别 `[PATCH vN M/N]` patch series
- 通过 `b4` apply 或导出 patch
- 在 TUI 中撰写回信，并通过 `git send-email` 发送
- 浏览本地 kernel tree，支持内联 Vim-like 编辑和外部 Vim 编辑

## 依赖

- Rust stable
- Git
- Python 3
  - 当你使用仓库内 vendored 的 `vendor/b4/b4.sh` 时需要
- `b4`
  - Courier 的查找顺序是：`[b4].path` -> `COURIER_B4_PATH` -> `./vendor/b4/b4.sh` -> `b4` in `PATH`
- `git send-email`
  - 仅在发送回信时需要

建议先运行：

```bash
courier doctor
```

它会检查 `b4`、`git send-email`、git 邮件身份和 IMAP 连接状态。

## 安装

### 从源码仓库安装

如果你希望直接使用仓库内 vendored 的 `b4`，建议递归拉取子模块：

```bash
git clone --recurse-submodules https://github.com/ChenMiaoi/courier.git
cd courier
cargo install --path . --locked
```

如果仓库已经 clone 下来但没有初始化子模块：

```bash
git submodule update --init --recursive
```

### 直接从 GitHub 安装

```bash
cargo install --git https://github.com/ChenMiaoi/courier.git --locked courier
```

这种方式下，建议你自行通过 `b4.path`、`COURIER_B4_PATH` 或系统 `PATH` 提供 `b4`。

### 开发时直接运行

```bash
cargo run -- doctor
cargo run -- tui
```

## 配置

默认配置文件路径：

```text
~/.courier/courier-config.toml
```

默认运行数据目录：

```text
~/.courier/
```

首次运行时，Courier 会自动生成一个最小配置文件。完整示例见 [docs/config.example.toml](docs/config.example.toml)。

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

配置说明：

- 相对路径会相对于配置文件所在目录解析
- 只使用 lore 同步时，可以完全不配置 `[imap]`
- IMAP 配置完整后，左栏会出现默认启用的 `My Inbox`
- `imap.proxy` 支持 `http://`、`socks5://`、`socks5h://`
- 回信身份优先读取 `git config sendemail.from`，否则回退到 `git config user.name` / `git config user.email`
- `ui.startup_sync` 默认是 `true`
- `ui.inbox_auto_sync_interval_secs` 默认是 `30`

## 基本使用

### 1. 环境自检

```bash
courier doctor
```

### 2. 同步 lore 邮箱

```bash
courier sync --mailbox io-uring
```

### 3. 同步真实 IMAP 收件箱

```bash
courier sync --mailbox INBOX
```

### 4. 使用本地 `.eml` fixture 调试

```bash
courier sync --mailbox test --fixture-dir ./fixtures
```

### 5. 启动 TUI

```bash
courier tui
```

## TUI 常用操作

- `:` 打开命令栏
- `y` / `n` 启用或禁用当前订阅
- `Enter` 打开当前 mailbox 或 thread
- `a` apply 当前 patch series
- `d` 导出当前 patch series
- `u` 撤销本次会话中最近一次成功 apply
- `r` 或 `e` 打开回信面板
- `Tab` 在 Mail 页面和 Code Browser 页面之间切换

命令栏常见命令：

- `sync`
- `sync <mailbox>`
- `config`
- `vim`
- `restart`
- `quit`
- `!<shell command>`

如果 IMAP 配置完整，`My Inbox` 会参与启动自动同步，并在 TUI 保持打开时按配置周期持续做后台增量同步。

## 回复邮件

Courier 的 Reply Panel 会自动填充：

- `From`
- `To`
- `Cc`
- `Subject`
- `In-Reply-To`
- `References`

同时会生成符合 kernel 邮件习惯的引用正文模板。发送时，底层走 `git send-email`。

## 相关文档

- [README.md](README.md): 英文项目说明
- [docs/config.example.toml](docs/config.example.toml): 配置示例
- [docs/design.md](docs/design.md): 设计文档
- [docs/reply-format-spec.md](docs/reply-format-spec.md): 回信格式规范
- [docs/mvp-milestones.md](docs/mvp-milestones.md): 历史里程碑
- [docs/reply-mvp-milestones.md](docs/reply-mvp-milestones.md): 回信功能演进记录

## 开发

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

## License

Courier 使用 [LGPL-2.1](LICENSE) 许可证发布。仓库内 vendored 第三方组件保留各自上游许可证。
