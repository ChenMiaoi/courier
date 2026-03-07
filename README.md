# Courier

Courier 是一个基于 Rust 的终端内核 patch 邮件工作流工具，面向 Linux kernel 邮件列表协作场景。

当前实现（M8）支持：
- CLI: `tui` / `sync` / `doctor` / `version`
- SQLite 初始化与迁移
- 从 `lore.kernel.org` 同步邮件并构建线程
- 空库首次同步仅拉取最近 20 个 threads
- 后续按 checkpoint 做增量更新
- 真实 IMAP `INBOX` 同步，支持 `ssl`/`tls`、`starttls`、`none`
- IMAP 代理支持：`http://`、`socks5://`、`socks5h://`
- `My Inbox` 内置订阅：IMAP 配置完整时首次默认启用，并参与启动自动同步
- `My Inbox` 在 TUI 打开期间会按可配置周期做后台增量同步（默认 30 秒）
- `My Inbox` 首次同步优化：先抓 header，再只下载最新 20 个 patch 相关 threads 的完整 raw
- `INBOX` 只保留 patch 相关邮件，历史非 patch 邮件会在后续同步时 prune
- TUI 订阅启用状态与分组展开状态持久化
- 启动 TUI 后后台自动同步已启用订阅（不阻塞首屏），并显示同步进度与完成摘要
- 启动自动同步支持配置项 `ui.startup_sync`（默认开启）
- 同步失败与 panic 会被隔离为状态栏/日志错误，不再直接打断 TUI
- patch series 识别（`[PATCH vN M/N]`）、完整性校验（缺片/重复/乱序）
- TUI `a`（apply）/`d`（download）/`u`（undo 上次 apply）封装调用 b4 + git
- patch 执行状态与日志回写（`new/reviewing/applied/failed/conflict`）
- 代码浏览页（Kernel Tree + Source Preview）
- Code Preview VM1 内联 Vim-like 编辑（`Browse/Normal/Insert/Command`）
- VM1 最小命令集：`s`、`:w`、`:q`、`:q!`、`:wq`（含 dirty 规则）
- Code Preview VM2 外部 Vim 编辑（`E`/`:vim`），支持 `VISUAL -> EDITOR -> vim` 选择顺序
- 外部 Vim 退出后自动恢复终端状态并刷新预览内容
- 命令栏 `config` 已升级为可视化配置面板，并保留 `config get/set` 文本入口
- 预览区会对非纯文本 / MIME / HTML / 编码邮件显示显眼警告
- 关键用户操作日志统一为 `op/status` 结构（apply/undo/sync/config/vim/local command 等）
- 命令栏支持本地命令（`!<shell command>`）与路径补全
- Mail Preview 回信面板（`e` / `r`）：自动填充 `From/To/Cc/Subject/In-Reply-To/References`
- 回信草稿自动生成 kernel 风格引用正文，并在 `To/Cc` 构造与预览阶段去重、移除自己地址
- 回信编辑沿用最小 Vim-like 状态机（`NORMAL / INSERT / COMMAND`）
- `Send Preview -> Confirm -> Send` 已打通，底层通过 `git send-email` 发送真实回信
- 回信发送结果会持久化记录 `Message-ID`、状态、退出码、stderr 摘要与确认时间
- `doctor` 会检查 `git send-email` 可用性和默认回信身份（git email）

详细设计与里程碑见：
- [docs/design.md](docs/design.md)
- [docs/mvp-milestones.md](docs/mvp-milestones.md)
- [docs/reply-format-spec.md](docs/reply-format-spec.md)
- [docs/reply-mvp-milestones.md](docs/reply-mvp-milestones.md)

## 快速开始

### 依赖
- Rust (stable)
- Python 3（用于构建阶段处理 `vendor/b4`，无则会降级为 warning）

### 安装

从源码仓库安装（clone 后在仓库根目录执行）：

```bash
cargo install --path . --locked
```

如果需要覆盖已安装版本：

```bash
cargo install --path . --locked --force
```

不 clone 直接从 GitHub 安装：

```bash
cargo install --git https://github.com/ChenMiaoi/courier.git --locked courier
```

### 构建与自检

```bash
cargo build
cargo run -- doctor
```

## 配置

默认配置文件路径为 `~/.courier/courier-config.toml`，默认数据目录为 `~/.courier/`。

最小配置示例见 [docs/config.example.toml](docs/config.example.toml)。

可选项：
- `[ui].startup_sync = true|false`（默认 `true`）
- `[ui].inbox_auto_sync_interval_secs = <seconds>`（默认 `30`，必须大于 `0`）

IMAP 说明：
- `imap.email` 用于识别“自己”的邮件；当 `imap.user` 省略时，也会默认作为 IMAP 登录账号。
- Gmail 一般应使用完整邮箱地址作为 `imap.user`。
- `imap.proxy` 支持 `http://`、`socks5://`、`socks5h://`。

回信身份说明：
- `From` 优先读取 `git config sendemail.from`；若未设置，则回退到 `git config user.name` / `user.email`。
- `imap.email` 会参与“自己地址”识别，用于从回信预览中的 `To/Cc` 自动移除自己。
- `courier doctor` 会同时检查 `git send-email` 和默认回信身份是否可用。

## 同步与 TUI

### 1. 手动同步（在线）

```bash
cargo run -- sync --mailbox io-uring
```

### 2. 启动 TUI

```bash
cargo run -- tui
```

说明：
- 打开 TUI 后会在后台自动同步“已启用订阅”，状态栏显示正在同步的 mailbox 与完成汇总。
- IMAP 配置完整时，左栏会出现默认启用的 `My Inbox`；它走真实 IMAP `INBOX`，其余子系统订阅继续走 lore。
- `My Inbox` 在 TUI 保持打开且订阅启用时，会按 `ui.inbox_auto_sync_interval_secs` 做后台增量同步；默认 30 秒。手动 `sync INBOX` 或首次启动同步会顺延下一次定时触发。
- `My Inbox` 同步失败时只会显示错误，不会把整个 TUI 带退出；状态栏和命令栏中的错误也会被压平成单行显示，避免界面紊乱。
- 命令栏（`:`）可用命令：`help`、`sync`、`sync <mailbox>`、`config`、`vim`、`restart`、`quit`、`exit`、`!<shell command>`。
- 命令栏支持 `Tab` 补全命令与参数；同一位置再按一次 `Tab` 会在下方列出可选参数；`!` 本地命令同样支持路径补全。
- `config` 默认打开可视化配置面板；仍支持 `config get <key>` / `config set <key> <value>`。
- `help` 会显示命令与常用键位（`j/l`、`i/k`、`y/n`、`a/d/u`）。
- 在线程列表选中 patch series 后：
  - 按 `a`：执行 apply（封装 `b4 am`）
  - 按 `d`：执行 download（封装 `b4 am -o` 导出到 `patch_dir`）
  - 按 `u`：撤销本次会话中最近一次成功 apply（reset 到 apply 前 HEAD）
- 在 Mail Preview 上：
  - 按 `e`：从 Preview 焦点进入回信面板
  - 按 `r`：直接打开当前线程的回信面板
  - 回信面板会自动填充 `From/To/Cc/Subject/In-Reply-To/References`，并生成引用正文模板
  - 回信面板键位：`Esc` 返回 normal/关闭，`i` 进入插入，`h/j/k/l` 移动，`x` 删除，`Enter/o` 在正文打开下方新行并进入插入
  - 回信命令：`p` 或 `:preview` 打开发送预览，`Enter/c` 确认预览，`S` 或 `:send` 尝试发送，`:q` / `:q!` 关闭草稿
  - 发送成功后会关闭 Reply Panel；发送失败或超时会保留草稿，允许直接重试
- `Tab` 可以在 Mail 页和 Code Browser 页之间切换。
- 在 Code Browser 页，焦点位于 Source 且选中文件时：
  - 按 `e` 进入 VM1 编辑态
  - 按 `E` 进入 VM2 外部 Vim 编辑
  - `h/j/k/l` 移动，`i` 进入插入，`x` 删除当前字符，`s` 保存
  - `:w` 保存，`:q` 退出（dirty 时拒绝），`:q!` 强制丢弃退出，`:wq` 保存并退出，`:vim` 打开外部 Vim

## 本地 fixture 测试（可选）

如果你要用本地 `.eml` 测试同步逻辑：

```bash
cargo run -- sync --mailbox inbox --fixture-dir ./fixtures
```

## 开发命令

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

## CI

GitHub Actions 工作流位于 `.github/workflows/ci.yml`，会在 push / pull_request 时执行：
- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets --all-features`

## License

[GPL-2.0](LICENSE)
