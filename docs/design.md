# Courier 设计文档

## 1. 项目定位

Courier 是一个基于 Rust 的现代 TUI 内核 patch 工作流工具，面向 Linux
kernel 邮件列表协作场景，目标是把「订阅 -> 阅读 -> 过滤 -> 提取 patch ->
应用/回复」串成一条高效、可追踪的本地流程。

核心原则：

- 终端优先：以 TUI 为主界面，CLI 为自动化入口。
- 纯文本优先：邮件编辑默认纯文本，自动换行 80 列。
- 标准兼容：遵循 RFC 5322/MIME，兼容常见 patch 邮件格式。
- 渐进实现：先交付 MVP，并以 b4 作为 patch 工作流核心。
- 工具自包含：安装 Courier 时自动编译并安装 b4。

## 2. 目标与非目标

### 2.1 目标

- 收取邮件：支持 IMAP 同步 patch 相关邮件。
- 发送邮件：支持 SMTP 发送 reply / patch cover letter。
- 邮件解析：支持 RFC 5322 头解析、MIME 多 part、附件提取。
- Patch 处理：识别 `[PATCH vN M/N]`、series 分组、导出/应用 patch。
- b4 内建：安装阶段自动编译并安装 b4，运行期直接可用。
- Thread 视图：按 `Message-ID` / `In-Reply-To` / `References` 组织对话树。
- 过滤系统：按列表、子系统、作者、标签、关键词建立规则过滤。
- lore.kernel.org 订阅：内置常见子系统订阅模板。
- 配置文件：支持 TOML 配置文件，覆盖账号、订阅与基础行为参数。

### 2.2 非目标（MVP 阶段）

- 不做 GUI。
- 不做完整邮件客户端替代（如日历、联系人、HTML 渲染）。
- 不在首版封装 b4 全量能力，仅先覆盖最关键工作流。
- 不做复杂配置中心，当前仅维持最小可用配置集。

## 3. 用户画像与关键场景

用户：

- 内核维护者/贡献者
- 需要高频处理 patch 邮件的开发者

关键场景：

1. 订阅多个列表后，快速看到与自己子系统相关的 patch series。
2. 从 thread 中提取完整 patch 序列，并在本地仓库应用验证。
3. 在终端内完成 review 回复并发送，保留标准邮件线程关系。

## 4. 技术选型

### 4.1 语言与运行时

- Rust（edition 2024）
- Tokio（异步任务调度：IMAP 同步、后台索引、网络请求）

### 4.2 交互层

- TUI: `ratatui` + `crossterm`
- CLI: `clap`

### 4.3 协议与解析

- IMAP: `async-imap`（或同类异步实现）
- SMTP: `lettre`
- 邮件解析: `mail-parser`（RFC 5322 + MIME）

### 4.4 数据与配置

- 本地存储: SQLite（`rusqlite` 或 `sqlx + sqlite`）
- 配置格式: TOML（`serde` + `toml`）
- 日志: `tracing` + `tracing-subscriber`
- 当前策略: 仅实现最小可用配置（账号、服务器、订阅、存储路径）。

### 4.5 b4 / patch 工作流

- 安装流程自动编译并安装 b4，与 Courier 一起交付。
- 运行时默认调用内置 b4 入口，无需用户单独安装 b4。
- patch 提取与应用统一走 `b4 am`。
- 后续按需接入 `b4 prep`、`b4 send` 等扩展流程能力。

## 5. 系统架构

分层结构：

1. `ui`：TUI 页面、键位、状态渲染。
2. `app`：用例编排（同步、过滤、应用 patch、发送回复）。
3. `domain`：核心模型（Mail、Thread、PatchSeries、Rule）。
4. `infra`：IMAP/SMTP/SQLite/b4/外部命令适配器。

后台任务：

- `sync_worker`: 周期拉取 IMAP 增量。
- `index_worker`: 更新 thread 索引、过滤命中、series 状态。
- `fetch_worker`: 按需从 lore 补齐 thread 缺失邮件。

## 6. 数据模型（逻辑）

核心实体：

- `mail`
  - `id`, `message_id`, `subject`, `from_addr`, `date`, `raw_path`
  - `in_reply_to`, `list_id`, `flags`, `imap_mailbox`, `imap_uid`, `modseq`
  - `is_expunged`
- `mail_ref`
  - `mail_id`, `ref_message_id`, `ord`（来自 `References` 的有序引用链）
- `thread`
  - `id`, `root_mail_id`, `subject_norm`, `last_activity_at`, `message_count`
- `thread_node`
  - `mail_id`, `thread_id`, `parent_mail_id`, `root_mail_id`, `depth`, `sort_ts`
- `patch_series`
  - `id`, `version`, `total`, `author`, `status`（new/reviewing/applied/rejected）
- `patch_item`
  - `id`, `series_id`, `seq`, `mail_id`, `filename`, `checksum`
- `filter_rule`
  - `id`, `name`, `query`, `action`（tag/star/hide/notify）
- `subscription`
  - `id`, `name`, `source`（imap/lore）, `pattern`
- `imap_mailbox_state`
  - `mailbox`, `uidvalidity`, `last_seen_uid`, `highest_modseq`, `synced_at`

### 6.1 Thread 建模方案（成熟方案）

- 采用 JWZ 风格 threading：优先依据 `References` 构建父子关系，
  `In-Reply-To` 作为回退。
- 对缺失祖先使用内存容器节点占位，后续邮件到达时可重连。
- 对无引用关系邮件，仅在主题归一化后做弱关联分组，避免错误串线。
- 持久化只落真实邮件节点（`thread_node`），占位节点不入库。
- 新邮件到达或补齐父邮件时，按 `root_mail_id` 局部重建线程树。

索引重点：

- `message_id` 唯一索引
- `(imap_mailbox, imap_uid)` 唯一索引
- `thread_node(thread_id, sort_ts)` 索引
- `thread_node(parent_mail_id)` 索引
- `patch_series(status, author)` 组合索引
- `imap_mailbox_state(mailbox)` 唯一索引

## 7. 核心流程

### 7.1 同步与建索引

1. `SELECT` mailbox，读取 `UIDVALIDITY`、`UIDNEXT`、`HIGHESTMODSEQ`。
2. 读取本地 `imap_mailbox_state`，对比同步断点。
3. 若 `UIDVALIDITY` 变化，触发该 mailbox 全量重建。
4. 按 UID 增量拉取新邮件；若服务器支持，按 MODSEQ 拉取 flag 变更。
5. 在单事务中落盘 `.eml`、解析头部、写入 `mail`/`mail_ref`。
6. 基于 JWZ 规则局部更新 `thread`/`thread_node`，并聚合 patch series。
7. 提交事务并更新 `last_seen_uid`/`highest_modseq` 检查点。

### 7.2 Patch 提取与应用

1. 用户在 thread/series 视图选择目标。
2. 系统按序拼装 patch 集，校验序号完整性。
3. 导出为 mbox 或临时 patch 目录。
4. 执行 `b4 am`，回写结果（成功/冲突/失败日志）。

### 7.3 回复与发送

1. 从当前邮件生成 reply 模板（含 `In-Reply-To` / `References`）。
2. 编辑器内纯文本编辑，80 列软换行。
3. SMTP 发送并记录 `Sent` 状态和 `Message-ID`。

### 7.4 IMAP 同步一致性策略

- 幂等写入：依赖 `message_id` 与 `(imap_mailbox, imap_uid)` 双唯一键去重。
- 事务提交：邮件写入、thread 更新、checkpoint 更新在同一事务完成。
- 断点恢复：进程异常后从最近一次 checkpoint 继续，不回退已提交状态。
- 删除一致性：收到 `EXPUNGE` 时标记 `is_expunged`，默认不在主视图展示。
- 全量重建：仅在 `UIDVALIDITY` 变化时触发，避免常态全量扫描。

## 8. TUI 设计（MVP）

主布局三栏：

- 左：订阅可选框（内核子系统邮件列表）
- 中：thread 或 series 列表
- 右：邮件正文/patch diff 预览

左栏订阅可选框：

- 展示形式：以可选框展示订阅项（`[x]` 已启用，`[ ]` 未启用）。
- 订阅内容：默认提供常见内核子系统邮件列表。
- 用户管理：支持用户新增订阅项、删除订阅项。
- 生效规则：仅拉取和展示被勾选订阅对应的邮件流。

命令栏（Command Palette）：

- 呼出方式：`:` 为首选呼出键，`Ctrl + Backtick (\`)` 作为兼容后备方案。
- 呈现方式：命令栏显示在主窗口之上，作为悬浮窗口。
- 交互方式：顶部输入框 + 下拉候选列表。
- 匹配规则：先前缀匹配，再模糊匹配（命令名、别名、说明文本）。
- 排序规则：前缀命中优先，其次按模糊得分和最近使用次数排序。
- 候选展示：每个候选显示命令标识和功能说明（作用）。
- 执行动作：`Enter` 执行当前候选，`Esc` 关闭命令栏。

关键操作：

- `j/l`: 页面焦点移动（在左/中/右面板间切换）
- `i/k`: 当前聚焦页面内上下移动（列表项或正文滚动）
- `:`: 打开/关闭命令栏（首选）
- `Ctrl + Backtick (\`)`: 打开/关闭命令栏（兼容后备）
- `Space`: 在左栏切换当前订阅项的勾选状态
- `n`: 在左栏新增订阅项
- `d`: 在左栏删除订阅项
- `Enter`: 展开 thread / 打开邮件
- `a`: apply 当前 series
- `r`: reply
- `f`: 添加过滤条件
- `/`: 搜索

状态反馈：

- 顶部状态条显示当前 mailbox、同步时间、未读数量
- 底部显示快捷键和后台任务进度

## 9. b4 集成策略

策略：MVP 即采用 b4 主流程，后续逐步补齐高级能力。

MVP 阶段：

- 安装时自动编译并安装 b4，启动时做可执行性检查。
- 支持以 `Message-ID` 抓取并重建 series 的基础能力。
- 以 `b4 am` 为默认导入/应用链路。

后续阶段：

- 扩展对 `b4 prep`、`b4 send`、系列元数据校验等能力的支持。
- 保持输出格式和社区工具链兼容。

## 10. 安全与可靠性

- 凭据不明文存储：优先系统密钥环，回退到本地加密文件。
- 所有原始邮件保留只读副本，便于追溯。
- 解析失败邮件进入隔离区，不阻塞主流程。
- 外部命令（b4 及其调用链）执行记录标准输出和退出码。

## 11. MVP 里程碑文档

MVP 范围与阶段目标已迁移至独立文档：

- `docs/mvp-milestones.md`

## 12. 测试策略

- 单元测试：标题解析、JWZ thread 构建、过滤规则匹配。
- 集成测试：IMAP 拉取 -> 入库 -> thread 展示 -> patch 应用链路。
- 一致性测试：`UIDVALIDITY` 变化、断点恢复、重复拉取去重、EXPUNGE。
- 端到端测试：使用本地测试邮箱和临时仓库验证完整流程。
- 回归样本：维护一组真实 `.eml` 样本（含异常 MIME 和破损 patch）。

## 13. 开发优先级建议

先做可验证闭环，再追求功能覆盖：

1. 本地 `.eml` 导入 + thread 展示（不依赖网络即可开发）。
2. patch series 识别 + `b4 am`。
3. IMAP 同步。
4. SMTP 发送。
5. b4 高级自动化（如 prep/send 流程）。
6. 配置体验增强（可选，低优先，不阻塞主流程）。

## 14. M1 已决策项与风险更新

### 14.1 已决策项

- 工程结构采用四层模块：`app` / `domain` / `infra` / `ui`。
- CLI 命令固定为：`tui`、`sync`、`doctor`、`version`。
- 配置读取采用 TOML，支持 `--config` 路径覆盖和默认目录策略。
- 启动阶段统一执行目录引导与 SQLite 初始化迁移，`schema_version` 作为版本入口。
- b4 检查顺序固定：配置路径 -> `COURIER_B4_PATH` -> `vendor/b4/b4.sh` -> `PATH` 中 `b4`。
- TUI 先交付三栏骨架与状态栏，键位实现 `j/l` 焦点切换和 `i/k` 面板内移动。

### 14.2 风险与后续动作

- b4 已在构建阶段通过 `build.rs` 执行 Python 字节码编译，当前仍依赖仓库内
  `vendor/b4` 资产，尚未引入跨平台“一键安装到用户环境”脚本；M2 前需补齐。
- 配置默认目录在受限沙箱环境可能不可写，当前通过 `--config` 可绕过；
  后续应补充显式 `config init` 与目录可写性提示。

---

该文档作为实现基线，后续应同步更新设计决策、风险项与约束变化。
