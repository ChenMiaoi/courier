# Courier

Courier 是一个基于 Rust 的终端内核 patch 邮件工作流工具，面向 Linux kernel 邮件列表协作场景。

当前实现（M2）支持：
- CLI: `tui` / `sync` / `doctor` / `version`
- SQLite 初始化与迁移
- 从 `lore.kernel.org` 同步邮件并构建线程
- 空库首次同步仅拉取最近 20 个 threads
- 后续按 checkpoint 做增量更新
- TUI 订阅启用状态与分组展开状态持久化
- 启动 TUI 时自动同步已启用订阅

详细设计与里程碑见：
- [docs/design.md](docs/design.md)
- [docs/mvp-milestones.md](docs/mvp-milestones.md)

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
- 打开 TUI 前会自动同步“已启用订阅”。
- 命令栏（`:`）可用命令：`help`、`sync`、`sync <mailbox>`、`config`、`quit`。
- 命令栏支持 `Tab` 补全命令与参数；同一位置再按一次 `Tab` 会在下方列出可选参数。

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
