# Courier 回信格式规范（Kernel 风格）

本文档定义 Courier 在 patch review 场景下的回信格式、自动填充规则与发送策略。
目标是让回复行为与内核社区邮件习惯保持一致，并与 `Patch Preview + Vim` 工作流无缝衔接。

## 1. 范围与前置条件

- 本规范仅覆盖“回复已有邮件（reply）”场景，不覆盖“发起全新线程”。
- 前置能力：
  - VM1 已完成（`Patch Preview` 可进入 Vim 模式）。
  - SMTP 发送链路可用（MVP 阶段允许直接调用 `git send-email`）。
- 成熟版本会替换为 Courier 自实现 SMTP，但保持相同的回信格式与字段语义。

## 2. 触发与面板行为

在 `Patch Preview` 视图进入 Vim 模式后，自动弹出 `Reply Panel`。
发送动作必须经过 `Send Preview -> Confirm -> Send` 三步，不允许直接跳过确认。

`Reply Panel` 至少展示以下字段并允许编辑：

- `From`
- `To`
- `Cc`
- `Subject`
- `Body`（Vim 编辑区）

并自动注入线程字段（只读）：

- `In-Reply-To`
- `References`

`Reply Panel` 发送区最小交互：

- `Send Preview`：生成并展示最终待发送邮件（头部 + 正文）
- `Confirm Send`：用户确认无误后触发实际发送
- `Cancel`：取消本次发送，保留当前编辑内容

## 3. 头部自动填充规则

### 3.1 Subject（内核风格 `Re:`）

- 对原邮件 `Subject` 做规范化：
  - 若已是 `Re:`（忽略大小写），不重复添加。
  - 否则自动前置 `Re: `。
- 保留原有 `[PATCH ...]`、`[RESEND ...]` 等标签顺序，不改写主题主体。

示例：

- 原标题：`[PATCH v3 2/7] mm: fix foo`
- 回信标题：`Re: [PATCH v3 2/7] mm: fix foo`

### 3.2 To / Cc（继承并排除自己）

- 默认直接继承被回复邮件中的 `To:` 与 `Cc:`。
- 地址归一化后去重（按邮箱地址大小写不敏感比较）。
- 若 `To` / `Cc` 中包含“自己”的地址，自动移除。

“自己”地址来源（用于过滤）：

1. 当前回信 `From` 地址
2. 配置中声明的别名地址（若存在）

兜底规则：

- 若移除自己后 `To` 为空，且原邮件 `From` 不是自己，则将原邮件作者加入 `To`。
- 若仍无法得到有效收件人，则阻止发送并给出错误提示。

### 3.3 From（来自 git email 身份）

`From` 按以下优先级自动获取：

1. `git config sendemail.from`
2. `git config user.name` + `git config user.email`

若无法解析有效发件身份，阻止发送并提示先配置 git email 信息。

### 3.4 线程头

- `In-Reply-To` = 当前被回复邮件的 `Message-ID`
- `References` = 原 `References` + 当前被回复邮件 `Message-ID`（去重后保序）

## 4. 正文格式规范

### 4.1 模板

回信正文默认使用纯文本模板：

```text
On <date>, <author> wrote:
> <quoted line 1>
> <quoted line 2>
```

- 使用 `>` 进行逐行引用，符合内核邮件常见 inline reply 习惯。
- 保留原文段落结构；空行引用为 `>`。

### 4.2 Enter 自动引用行为

在 `Reply Panel` 的 Vim `INSERT` 模式下：

- 当当前行是引用行（以 `>` 开头）时，按 `Enter` 自动在新行补 `> `。
- 当用户首次进入正文并按 `Enter` 开始回信时，自动插入 `> ` 作为引用前缀。
- 该行为用于降低 inline review 输入成本，保持引用格式稳定。

### 4.3 文本约束

- 纯文本（`text/plain; charset=UTF-8`）。
- 推荐软换行列宽：72（patch/diff 行不强制折行）。
- 不自动插入 HTML 或富文本结构。

## 5. 发送策略

### 5.1 MVP（必须）

MVP 通过 Courier 内部发送适配层完成发送：

- 用户侧仅暴露统一动作：`Send Preview`、`Confirm Send`、`Send`。
- 底层实现默认适配 `git send-email`，但实现细节对用户透明。
- Courier 负责生成待发送内容、拼装参数、捕获退出码与错误输出。

MVP 发送时序：

1. 用户在 `Reply Panel` 点击或触发 `Send Preview`。
2. 系统渲染最终邮件快照（`From/To/Cc/Subject/In-Reply-To/References/Body`）。
3. 用户执行 `Confirm Send` 后，Courier 调用底层发送器（MVP 为 `git send-email`）。
4. 若发送失败，停留在 `Reply Panel` 并保留内容，允许重试。

底层命令形态（示意）：

```bash
git send-email \
  --from "<from>" \
  --to "<to1>" --to "<to2>" \
  --cc "<cc1>" \
  --subject "<subject>" \
  --in-reply-to "<message-id>" \
  --confirm=never \
  <reply-file>
```

### 5.2 成熟版（增强）

- 在保持同一回信格式构造器前提下，替换为 Courier 自实现 SMTP 发送器。
- 支持认证、重试、连接池、错误分类与发送状态追踪增强。
- 继续沿用 `Send Preview -> Confirm -> Send` 的用户交互，不改变前端行为。

## 6. 发送结果与审计

每次发送至少记录：

- `Message-ID`（若可获取）
- `thread_id` / `mail_id`
- 发送时间
- 发送器类型（`git-send-email` / `smtp-native`）
- 发送命令或 SMTP 通道
- 预览确认时间（`preview_confirmed_at`）
- 成功/失败状态
- 错误摘要（退出码、stderr 或 SMTP 错误码）

## 7. 合规检查清单（MVP）

以下条件全部满足才算回信格式合规：

1. `Subject` 为单一规范 `Re: ...`（无重复前缀）。
2. `To/Cc` 继承原邮件并完成去重、去自己。
3. `From` 来自 git email 身份。
4. `In-Reply-To` / `References` 构造正确。
5. 正文为 `>` 引用风格，`Enter` 可连续生成引用前缀。
6. 用户必须先完成 `Send Preview` 确认，才允许正式发送。
7. MVP 路径可通过底层 `git send-email` 完成实际发送并留存结果。
