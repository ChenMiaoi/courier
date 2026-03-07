# Courier Reply MVP 里程碑

本文档定义 Courier 回信能力的 MVP 路线，强调两条前置约束：

1. `Patch Preview` 的 Vim 模式先完成（VM1）。
2. SMTP 链路先打通（MVP 可直接走 `git send-email`）。

详细格式规范见 `docs/reply-format-spec.md`。

## RM0：前置依赖就绪（必须）

### 阶段目标

在进入回信开发前，完成最小依赖闭环，避免“编辑已做完但无法发出”的半成品状态。

### 任务拆分

1. 确认 VM1 已可在 `Patch Preview` 进入 Vim 编辑态。
2. `doctor` 增加 `git send-email` 可执行检查。
3. 增加 git email 身份检查（`sendemail.from` 或 `user.name/user.email`）。
4. 回信配置最小字段确认（发件人别名、是否启用自动去自己）。
5. 定义统一发送接口（用户不直接接触底层发送命令）。

### 交付物

- 依赖检查通过的 `doctor` 输出项。
- 回信能力的前置条件检查清单。

### 验收标准

1. 无 Vim 模式时，回信入口不开放。
2. `git send-email` 缺失时，状态栏明确阻止发送并给出修复建议。
3. 缺失 git email 身份时，明确提示配置命令。
4. `Reply Panel` 已具备 `Send Preview` 与确认发送交互入口。

### 退出条件

- VM1 + `git send-email` + git email 身份检查均可演示。

## RM1：Patch Preview 一步回信（MVP，必须，已完成）

### 阶段目标

用户在 `Patch Preview` 进入 Vim 模式后，系统自动弹出回信面板，完成“填充 -> 编辑 -> 发送”最短闭环。

### 已完成项

1. 回信入口与状态机
   - 在 `Patch Preview + Vim` 激活时自动打开 `Reply Panel`。
   - 面板关闭/发送/取消后，稳定回到预览上下文。
   - 发送动作固定为 `Send Preview -> Confirm Send -> Send`。
2. 头部自动填充
   - `From/To/Cc/Subject` 打开面板时会自动填充默认值，但都允许用户修改。
   - `Subject` 自动规范为单一 `Re: ...`。
   - `To/Cc` 默认继承原邮件并去重。
   - 若出现自己地址，自动从 `To/Cc` 移除。
   - `From` 默认读取 git email 身份。
   - 自动构造 `In-Reply-To` 与 `References`。
3. 正文模板与引用输入
   - 自动生成 `On ..., ... wrote:` + `>` 引用模板。
   - 在 Vim `INSERT` 中保留普通换行；用户回复写在不带 `>` 的空白行中。
   - 保持纯文本与内核常用 inline reply 格式。
4. 发送执行（MVP）
   - 实现 `SendService`（或等价抽象），由 Reply Panel 调用统一接口。
   - MVP 发送器由 `git send-email` 适配实现，不直接暴露给用户。
   - 必须先执行 `Send Preview`，用户确认后才允许正式发送。
   - 记录命令、退出码、stdout/stderr 摘要。
   - 失败可重试，重试前保留编辑内容。
5. 结果持久化与可追踪
   - 记录发送时间、状态、错误信息、关联 `mail_id/thread_id`。
   - 状态栏与日志页可查看最近发送结果。
6. 测试补齐
   - 覆盖 `Re:` 规范化、去自己、`From` 解析、引用模板生成。
   - 覆盖发送成功/失败/重试与异常路径。

### 交付物

- `Patch Preview + Vim` 自动回信面板。
- 标准回信头部构造器与正文模板器。
- 统一发送抽象层（MVP 后端接 `git send-email`）。
- `Send Preview` 预览与确认发送流程。
- 对应单元测试与集成测试。

### 验收标准

1. 在 patch 预览进入 Vim 后自动看到回信面板。
2. 标题自动为规范 `Re: ...`，且不重复前缀。
3. `From/To/Cc/Subject` 进入面板时自动填充，但用户可直接修改。
4. `To/Cc` 在预览时仍会自动去自己并去重。
5. `From` 默认取自 git email 身份，修改后仍需保持有效邮箱地址。
6. 用户回复默认写在不带 `>` 的空白行中，历史引用层级以保留的 `>` / `>>` 表示。
7. 用户必须先通过 `Send Preview` 确认，才允许发送。
8. 发送路径对用户仅暴露 `Send`，底层实现细节不外露。
9. MVP 底层 `git send-email` 可完成发送，失败可追踪并可重试。

### 退出条件

- 回信链路可端到端演示：`Patch Preview -> Vim -> Auto Reply Panel -> Edit -> Send`。
- `cargo test` 与关键手工路径验证通过。

## RM2：自实现 SMTP（增强）

### 阶段目标

在不改变回信格式与交互前提下，用 Courier SMTP 发送器替代 `git send-email`。

### 任务拆分

1. SMTP 连接与认证适配（TLS/STARTTLS、认证方式）。
2. 发送队列、退避重试与错误分类。
3. 与 RM1 头部/正文构造器对接，保证输出一致。
4. 增加 SMTP 观测指标（连接失败、认证失败、投递失败）。

### 交付物

- Courier 自实现 SMTP 发送器。
- 与 RM1 兼容的发送抽象层。

### 验收标准

1. 相同输入在 RM1/RM2 路径下头部与正文一致。
2. SMTP 异常可追踪、可重试，不丢失编辑内容。

### 退出条件

- 默认发送路径从 `git send-email` 切换到自实现 SMTP，且回归测试通过。

## 非目标（Reply MVP 不做）

- 不实现 HTML 回信编辑。
- 不实现完整 Vim 命令集（依赖 VM1 已有能力）。
- 不实现复杂邮件模板系统（MVP 固定内核回信模板）。
