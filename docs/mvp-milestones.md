# Courier MVP 里程碑

本文档承载 Courier 的 MVP 范围、阶段目标、交付物与验收标准。

## 使用方式

- 里程碑按顺序推进，默认前一阶段完成后进入下一阶段。
- 每个阶段都要求可演示、可回归、可记录，不接受“仅代码提交”。
- 阶段完成后，同步更新 `docs/design.md` 的已决策项与风险项。

## 跨阶段约束

- b4 为默认 patch 入口，任何阶段不引入 `git am` 主路径。
- 配置文件仅维持最小可用，不扩展为复杂配置中心。
- IMAP 同步必须保持幂等，任何增量任务应可安全重放。
- 每阶段至少补齐对应单元测试或集成测试，不留长期空白。

## M1：基础骨架（必须）

### 阶段目标

建立可运行的工程骨架，打通 CLI、日志、存储、b4 可执行检查与
TUI 主循环，确保项目具备后续迭代基础。

### 任务拆分

1. 建立目录与模块骨架：`app`、`domain`、`infra`、`ui`。
2. CLI 命令框架：`tui`、`sync`、`doctor`、`version`。
3. 日志与错误处理：`tracing` 初始化、统一错误类型与错误码。
4. 最小可用配置：读取 TOML，支持默认值与路径覆盖。
5. SQLite 初始化：建表迁移入口与 schema 版本表。
6. b4 集成：安装阶段自动编译，运行阶段 `doctor` 做可执行检查。
7. TUI 骨架：三栏布局占位、焦点切换与基础状态栏。

### 交付物

- 可执行程序 `courier`，核心命令可运行。
- 初始数据库 schema 与迁移脚本。
- 配置样例文件（最小字段）。
- `doctor` 输出（包含 b4 路径与版本）。

### 验收标准

- `courier doctor` 可输出配置路径、数据库路径、b4 检查结果。
- `courier tui` 可稳定启动并退出，无 panic。
- 首次运行可自动创建数据库与必要目录。
- 核心模块通过 `cargo check` 与基础单元测试。

### 退出条件

- CLI、日志、DB、b4、TUI 五条基础链路全部可演示。
- 没有阻塞 M2 的结构性缺口（如无状态持久层抽象）。

## M2：邮件读取链路（必须，已完成）

### 阶段目标

完成从同步源（lore / 本地 fixture）增量拉取到本地入库、解析、thread 展示、
订阅驱动浏览与状态持久化的闭环，保证同步一致性与线程结构正确性。

### 已完成项

1. 同步入口：完成 `sync` 命令与 TUI 命令栏 `sync` 命令，支持
   `--mailbox`、`--fixture-dir`、`--uidvalidity`、`--reconnect-attempts`。
2. 同步源：实现本地 fixture 同步与 `lore.kernel.org/<mailbox>/new.atom` 抓取同步，
   统一接入同一入库与建线程链路。
3. checkpoint：落地并维护 `UIDVALIDITY`、`last_seen_uid`、`highest_modseq`，
   支持断点续传与 UIDVALIDITY 变化重建。
4. 初次窗口策略：当订阅 mailbox 数据为空时，仅保留最近 20 个 threads；
   非空时从 checkpoint 增量更新到最新。
5. 入库与建线程：完成 `mail` / `mail_ref` / `thread` / `thread_node` /
   `imap_mailbox_state` 的事务写入与幂等去重。
6. 线程模型：实现 JWZ 风格（`References` 优先，`In-Reply-To` 回退）构建与局部重建。
7. 订阅视图：左栏内置 vger 子系统列表，支持 `y/n` 启停、启用/停用分组、
   各组字典序、分组折叠展开。
8. 交互与状态：`Enter` 在订阅项上打开对应 Threads；
   若本地无数据则自动触发该订阅同步；UI 状态持久化（启用列表、分组展开状态、
   active mailbox）并在下次启动恢复。
9. 启动同步：进入 TUI 时自动同步已启用订阅；首次打开默认全 `n`（无启用项）。
10. 预览链路：右栏预览隐藏 RFC 头部，清理控制字符，线程切换时清屏并重置滚动。

### 交付物

- 同步 worker（fixture + lore）与 checkpoint 机制。
- `.eml` fixture 同步路径（用于离线调试与回归）。
- 线程列表页（层级展示、检索定位）与订阅驱动切换。
- UI 状态持久化文件（`ui-state.toml`）与启动自动同步逻辑。

### 验收标准

- 重复执行同步不会产生重复邮件记录。
- `UIDVALIDITY` 变化时可触发并完成 mailbox 重建。
- 进程中断后可从 checkpoint 继续，不破坏一致性。
- thread 展示可正确反映 `References` / `In-Reply-To` 关系。
- 空库首次同步仅落最近 20 个 threads，后续同步按 checkpoint 增量补齐。
- 启动 TUI 时仅同步已启用订阅；首次启动默认不启用任何订阅。
- 订阅启用与分组折叠状态在重启后可恢复。

### 退出条件

- 同步一致性测试全部通过。
- 能稳定展示真实邮件样本中的线程结构。
- M2 相关单元/集成测试与 `clippy -D warnings` 通过。

## M3：patch 工作流（必须，已完成）

### 阶段目标

完成 patch series 识别、校验、导出与 `b4 am` 应用闭环，支持失败可追踪
与状态回写。

### 任务拆分

1. series 识别：解析 `[PATCH vN M/N]` 标题并归并邮件集合。
2. 完整性校验：检测缺片、乱序、重复 patch。
3. 导出策略：生成 mbox 或临时目录供 b4 消费。
4. 执行器：封装 `b4 am` 调用、超时、退出码映射。
5. 状态流转：`new -> reviewing -> applied|failed|conflict`。
6. 结果回写：记录执行日志、失败原因与关联 series。
7. TUI 操作：支持在 series 视图触发 apply。

### 交付物

- series 聚合服务与状态机。
- `b4 am` 执行适配层与日志持久化。
- patch 应用结果页或状态列。

### 验收标准

- 完整 series 可通过一次操作完成应用并回写状态。
- 缺片 series 明确标注不可应用原因。
- `b4 am` 失败时可查看命令、退出码与关键错误摘要。

### 退出条件

- 主流 patch 邮件样本可稳定完成识别与应用流程。
- 失败路径可复现、可定位、可重试。

## M4：Code Preview Vim（VM1，必须）

### 阶段目标

在 `Code Preview` 内完成 VM1 内联 Vim-like 编辑能力，作为回信流程的基础输入能力。

### 任务拆分

1. 编辑状态机：实现 `Browse / VimNormal / VimInsert / VimCommand` 四态与键位分流。
2. 进入与退出：仅在文件预览场景按 `e` 进入；支持 `Esc`、`:q`、`:w`、`:wq` 退出/保存。
3. 文本缓冲区：支持最小移动、插入、删除、保存能力。
4. 渲染与提示：展示 mode/dirty/command 状态，确保切换无残影。
5. 回归测试：未进入编辑态时，现有全局键位行为保持不变。
6. 文档同步：与 `docs/vim-mvp-milestones.md`、`docs/code-preview-vim-prototype.md` 保持一致。

### 交付物

- `Code Preview` VM1 内联编辑能力。
- VM1 测试与帮助文案更新。
- VM1 设计与里程碑文档同步。

### 验收标准

1. 默认模式下现有键位行为不变。
2. 仅在合法文件场景按 `e` 进入编辑态。
3. 支持最小 Vim-like 编辑 + 保存 + 退出。
4. `:w`、`:q`、`:wq` 路径符合 dirty 规则。

### 退出条件

- “选中文件 -> `e` -> 编辑 -> 保存 -> 退出 -> 预览更新”链路可演示并通过测试。

## M5：Code Preview 外部 Vim（VM2，增强，已完成）

### 阶段目标

在 VM1 的基础上支持从 `Code Preview` 切出到外部 Vim，会话结束后稳定返回 Courier，
并自动刷新预览内容。

### 前置依赖

1. M4 已完成（内联 Vim 编辑能力可用）。

### 任务拆分

1. 触发入口：在 `Code Preview` 文件场景支持 `E`（或命令栏中输入 vim 等价命令）启动外部 Vim。
2. 脏缓冲保护：若内联 buffer 为 dirty，要求先保存后再切出。
3. 编辑器选择：按 `VISUAL -> EDITOR -> vim` 优先级选择可执行编辑器。
4. 终端切换：启动前正确退出 raw mode/alternate screen，退出后完整恢复。
5. 状态同步：外部 Vim 退出后重载文件并刷新 `Code Preview`。
6. 异常处理：启动失败/异常退出可追踪，且不导致 TUI 卡死。
7. 测试补齐：覆盖触发条件、dirty 拦截、恢复流程与失败路径。

### 交付物

- `Code Preview` 外部 Vim 启动与返回能力。
- 外部会话后的文件重载与预览刷新。
- VM2 测试与帮助文案更新。

### 验收标准

1. 在目标文件上触发 VM2 可成功拉起外部 Vim。
2. 外部 Vim 退出后能稳定回到 Courier，终端状态正常。
3. 返回后 `Code Preview` 显示外部编辑后的最新内容。
4. dirty 状态触发 VM2 会被阻止并提示先保存。
5. 启动失败或异常退出时，状态信息可追踪且不影响继续操作。

### 退出条件

- “选中文件 -> 触发 VM2 -> 外部编辑 -> 退出 -> 预览更新”链路可演示并通过测试。

## M6：真实 IMAP 接入与自邮箱订阅（必须，已完成）

### 阶段目标

在 M2 已有 checkpoint / threading / 幂等模型基础上接入真实 IMAP 账号，
并在订阅栏中新增一个默认开启的“自己收件箱”订阅，打通配置 -> 自动同步 -> 展示闭环。

### 前置依赖

1. M2 已完成（同步 checkpoint、JWZ threading、订阅驱动浏览已可用）。
2. M5 已完成（当前 TUI 交互与状态持久化能力稳定）。

### 任务拆分

1. 配置模型：新增 `[imap]` 配置段，首版固定字段 `email`、`user`、
   `pass`、`server`、`serverport`、`encryption`，并兼容 legacy alias
   `imapuser`、`imappass`、`imapserver`、`imapserverport`、`imapencryption`；
   同时补齐字段级校验。
2. 邮箱地址解析：IMAP 相关“自己邮箱”地址按 `[imap].email -> git config user.email`
   优先级解析；若 Courier 配置显式设置，则覆盖 git 结果，并在诊断信息中标明来源。
3. 连接与认证：实现真实 IMAP 会话建立、`SELECT INBOX`、最小 `LOGIN` 认证路径，
   支持 `tls` / `starttls` / `none` 三种加密模式。
4. 默认订阅：左栏新增一个内置 `My Inbox`（命名可实现期微调）订阅，映射当前账号
   的 `INBOX`，在 IMAP 配置完整时首次默认开启。
5. 订阅联动：`My Inbox` 与现有订阅使用同一启停、排序、折叠、状态持久化模型；
   但仅 `My Inbox` 走 IMAP，同步启动时应包含该默认开启项。
6. 子系统订阅保持现状：vger 等子系统订阅继续沿用 M2 的网页/lore 抓取同步路径，
   不切换到 IMAP。
7. 同步复用：真实 IMAP 路径仅负责 `My Inbox`（或未来显式 `imap` 源订阅），
   并复用 M2 的 `mail` / `thread` / `imap_mailbox_state` 写入模型，保持
   `(imap_mailbox, imap_uid)` 幂等约束与 `UIDVALIDITY` 重建逻辑。
8. 诊断与错误：`doctor` 增加 IMAP 配置完整性、邮箱地址来源、连接/认证结果检查；
   对认证失败、TLS 配置错误、邮箱地址缺失给出明确提示。
9. 测试补齐：覆盖邮箱地址优先级、默认订阅开启、配置缺失、认证失败、
   `My Inbox` 的 IMAP 同步幂等，以及子系统订阅仍走 lore/web 路径等场景。

### 交付物

- 真实 IMAP 配置模型与校验逻辑。
- IMAP 会话适配器（连接、认证、选择 mailbox、增量同步）。
- 左栏内置 `My Inbox` 默认订阅及其状态持久化。
- 子系统订阅继续走 lore/web 抓取的设计约束说明。
- `doctor` 的 IMAP 诊断项与错误提示。

### 验收标准

1. 当 `[imap].email` 存在且与 git email 不同时，系统使用 Courier 配置中的值。
2. 当 `[imap].email` 缺失时，系统自动回退到 `git config user.email`。
3. IMAP 配置完整时，左栏出现默认开启的 `My Inbox` 订阅，并在进入 TUI 后自动同步。
4. `user`、`pass`、`server`、`serverport`、`encryption`
   （兼容 legacy alias `imapuser`、`imappass`、`imapserver`、
   `imapserverport`、`imapencryption`）可驱动真实 IMAP 建连并读取 `INBOX`。
5. 子系统订阅继续通过此前的 lore/web 抓取方式同步，不依赖 IMAP。
6. 重复同步不会产生重复邮件记录，`UIDVALIDITY` 变化后仍可完成 `My Inbox` mailbox 重建。

### 退出条件

- “配置 IMAP -> 启动 TUI -> 自动同步 `My Inbox` -> 展示 threads”链路可端到端演示。

## M7：回信编辑与预览（必须，已完成）

### 阶段目标

在 VM1 基础上完成 `Reply Panel` 编辑闭环：自动填充头部、生成引用正文、提供发送前预览。

### 前置依赖

1. M4 已完成（`Patch Preview` 可进入 Vim 编辑态）。
2. git email 身份可读（`sendemail.from` 或 `user.name/user.email`）。

### 已完成项

1. 回信入口：在 `Patch Preview` 进入 Vim 模式时自动弹出 `Reply Panel`。
2. 头部编辑：`From/To/Cc/Subject` 打开时自动填充默认值，但在 `Reply Panel` 中可修改。
3. 标题规范：自动将标题规范为单一 `Re: ...`（不重复前缀，保留 `[PATCH ...]` 标签）。
4. 收件人填充：`To/Cc` 默认继承原邮件；预览时仍会去重，且若包含自己地址则自动移除。
5. 发件人填充：`From` 默认读取 git email 信息，但允许用户在面板中改写。
6. 线程头构造：自动填充 `In-Reply-To` 与 `References`，并保持只读。
7. 回信正文：按内核常见引用格式生成模板；用户回复默认写在不带 `>` 的空白行中，不自动续写引用前缀。
8. Send Preview：发送前展示最终邮件预览（头部 + 正文），并校验必填项。
9. 预览确认门控：未确认预览时禁止发送动作。
10. 测试补齐：覆盖 `Re` 规范化、去自己、引用模板、预览渲染与校验失败路径。

### 交付物

- `Patch Preview + Vim` 自动回信面板。
- 标准回信头部/正文构造器（内核风格）。
- `Send Preview` 预览能力与确认门控。
- 回信相关设计文档：
  - `docs/reply-format-spec.md`
  - `docs/reply-mvp-milestones.md`

### 验收标准

1. 在 patch 预览进入 Vim 后，回信面板自动弹出。
2. 标题自动规范为 `Re: ...`，且不出现重复 `Re: Re:`。
3. `From/To/Cc/Subject` 进入面板时会自动填充，但用户可直接修改。
4. `To/Cc` 在预览时仍会自动去自己并去重。
5. `From` 默认来自 git email 配置，修改后仍需保持有效邮箱地址。
6. 正文默认在不带 `>` 的空白行中回复，历史引用层级以保留的 `>` / `>>` 表示。
7. 可展示完整发送预览，且未确认时不能进入发送流程。

### 退出条件

- “阅读 -> Patch Preview -> Vim -> 回复编辑 -> Send Preview”链路可端到端演示。

## M8：Send Email 发送链路（MVP，必须）

### 阶段目标

在 M7 的预览闭环上接入真实发送能力：对用户只暴露统一 `Send` 体验，
底层先使用 `git send-email` 适配实现。

### 前置依赖

1. M7 已完成（可稳定生成并确认发送预览）。
2. 发送环境可用（MVP 阶段可直接调用 `git send-email`）。

### 任务拆分

1. 环境检查：`doctor` 增加 `git send-email` 与 git email 身份检查。
2. 发送抽象层：封装统一 `SendService`（或等价接口）。
3. 发送流程：固定 `Send Preview -> Confirm -> Send`，由 Reply Panel 驱动。
4. MVP 发送器：实现 `git send-email` 适配器并隐藏底层细节。
5. 发送容错：失败重试、超时、取消与状态提示。
6. 发送结果入库：记录 `Message-ID`、时间、状态、错误信息与关联邮件。
7. 观测与诊断：记录退出码、stderr 摘要与发送器类型。
8. 测试补齐：覆盖成功/失败/重试/超时路径。

### 交付物

- 统一发送抽象层 + `git send-email` MVP 适配器。
- Reply Panel 的确认发送能力（用户只见 `Send` 入口）。
- `doctor` 的 send-email 检查项。
- 发送结果持久化与诊断日志。

### 验收标准

1. 用户必须先完成 `Send Preview` 确认，才允许正式发送。
2. 用户侧发送入口统一为 `Send`，不暴露底层 `git send-email` 细节。
3. `doctor` 能明确报告 `git send-email` 与发件身份可用性。
4. 底层 `git send-email` 路径可发送成功；失败可重试且原因可追踪。
5. 发送结果可查询 `Message-ID`、状态与错误摘要。

### 退出条件

- “阅读 -> Patch Preview -> Vim -> 回复编辑 -> Send Preview -> Send”链路可端到端演示。

## M9：过滤规则（必须）

### 阶段目标

在发送闭环已可用的前提下，补齐过滤系统，降低邮件噪声并提升 review 聚焦效率。

### 任务拆分

1. 过滤规则模型：作者、列表、关键词、标签组合匹配。
2. 规则执行器：同步后自动打标、隐藏或置顶。
3. TUI 交互：规则增删改与即时预览命中结果。
4. 默认规则与订阅模板：提供最小内置模板并可启停。
5. 测试补齐：覆盖规则匹配、优先级与冲突处理。

### 交付物

- 过滤规则持久化与执行模块。
- 规则管理交互界面与命中预览。
- 默认规则/订阅模板集（最小可用）。

### 验收标准

1. 新邮件到达后过滤规则可自动生效，命中结果可见。
2. 规则变更可即时预览命中影响。
3. 规则冲突行为可解释、可追踪、可回归。

### 退出条件

- “收取 -> 过滤 -> 阅读 -> 回复 -> 发送”链路可端到端演示。

## M10：配置体验增强（可选，低优先）

### 阶段目标

在不改变“最小可用配置”策略前提下，提升配置可读性、可校验性与迁移
体验。

### 任务拆分

1. 生成配置模板：`courier config init`。
2. 配置校验：`courier config check`，输出字段级错误。
3. 版本迁移提示：旧字段兼容与弃用提示。
4. 文档补齐：字段说明、示例、覆盖优先级规则。

### 交付物

- 配置模板生成命令。
- 配置校验命令与错误提示规范。
- 配置文档与迁移说明。

### 验收标准

- 新用户可在 5 分钟内完成最小配置并通过校验。
- 配置错误可定位到具体字段与建议修复动作。
- 旧配置升级时有明确兼容提示，不静默破坏行为。

### 退出条件

- 配置体验优化完成且不影响 M1-M9 既有行为。

## 维护规则

- 每个里程碑新增任务时，必须同步验收标准与退出条件。
- 如果阶段范围变更，先改本文档，再改实现任务单。
- 任何里程碑降级或延期，需记录原因与新目标日期。
