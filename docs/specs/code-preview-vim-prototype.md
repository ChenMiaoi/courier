# Code Preview Vim 内联编辑原型设计

## 文档导航

1. 背景与目标
2. 原型范围（P0）
3. 交互原型
4. 技术方案（原型建议）
5. 代码变更点（草案）
6. 测试原型（建议）
7. 后续迭代（P1/P2）
8. 验收标准（P0）
9. VM2 外部 Vim 会话原型（增强）

## 1. 背景与目标

当前 `Code Browser` 页面的 `Source Preview` 是只读预览。
本原型目标是在不离开 `Code Preview` 的前提下提供 Vim 风格编辑体验，并避免和主页面全局键位冲突：

- 默认仍是浏览模式，沿用当前配置选中的主页面键位
- `ui.keymap = "default"` 时为 `j/l` 焦点切换、`i/k` 移动
- `ui.keymap = "vim"` 时为 `h/l` 焦点切换、`j/k` 移动
- 仅在用户按 `e` 后，进入 Vim-like 编辑模式
- 编辑完成后可保存并回到浏览模式，预览立即显示最新内容

## 2. 原型范围（P0）

本阶段只做最小可用的内联 Vim-like 模式：

- 支持在 `UiPage::CodeBrowser` + `CodePaneFocus::Source` + 选中文件时，按 `e` 进入编辑
- 提供 `NORMAL` / `INSERT` / `COMMAND` 三种模式
- 支持基础移动、插入、删除、保存、退出
- 支持最小 `:` 命令：`:w`、`:q`、`:wq`
- 不改变默认模式下当前配置所选择的全局键位语义

不在 P0 范围内：

- 完整 Vim/Ex 命令集（如宏、寄存器、复杂文本对象、复杂命令组合）
- Neovim 嵌入协议、插件体系、LSP
- 多文件 buffer / split 窗口

## 3. 交互原型

### 3.1 模式状态

- `Browse`（默认）
  - CRIEW 接管按键
  - 现有行为不变
- `VimNormal`（按 `e` 进入）
  - 按键由编辑状态机接管
  - 可移动光标、删除字符、进入插入、保存退出
- `VimInsert`
  - 可输入文本
  - `Esc` 返回 `VimNormal`
- `VimCommand`
  - 底部显示 `:` 命令输入行
  - `Enter` 执行命令后返回 `VimNormal`

### 3.2 进入条件

按 `e` 仅在以下条件满足时生效：

1. 当前页面是 `CodeBrowser`
2. 当前焦点是 `Source`
3. 当前选中项是文件（不是目录）

否则仅提示状态信息，例如：

- `select a source file in Source pane, then press e`

### 3.3 P0 键位草案

`Browse` 模式：

- 保持当前键位，不变

`VimNormal` 模式：

- `h/j/k/l`: 光标左/下/上/右
- `i`: 进入 `VimInsert`
- `x`: 删除当前字符
- `s`: 保存到文件
- `:`: 进入 `VimCommand`
- `Esc`: 退出编辑并回到 `Browse`（未保存时给出提示）

`VimInsert` 模式：

- 可打印字符：插入
- `Enter`: 换行
- `Backspace`: 删除前一字符
- `Esc`: 返回 `VimNormal`

`VimCommand` 模式：

- 可打印字符：追加到命令行（前缀 `:` 仅展示，不重复输入）
- `Backspace`: 删除命令字符
- `Enter`: 执行命令（MVP 仅支持 `w`、`q`、`wq`）
- `Esc`: 取消命令并返回 `VimNormal`

说明：

- 编辑模式激活后，`j/l/i/k` 不再触发全局动作，避免冲突。
- `:q` 在有未保存改动时不退出，状态栏提示先 `:w` 或使用 `:wq`。

## 4. 技术方案（原型建议）

### 4.1 核心数据结构

建议在 `AppState` 增加内联编辑状态：

- `code_edit_mode: Browse | VimNormal | VimInsert | VimCommand`
- `code_edit_target: Option<PathBuf>`
- `code_edit_buffer: Vec<String>`（按行存储）
- `code_edit_cursor_row: usize`
- `code_edit_cursor_col: usize`
- `code_edit_dirty: bool`
- `code_edit_command_input: String`（`COMMAND` 模式命令缓存）

### 4.2 运行流程

1. 在 `Browse` 中按 `e`，校验当前选中是文件
2. 读取文件到 `code_edit_buffer`，进入 `VimNormal`
3. 按当前模式路由按键事件并更新 buffer/cursor
4. 在 `VimNormal` 下按 `:` 进入 `VimCommand`
5. 执行 `:w` / `:q` / `:wq`，并更新 dirty 与模式状态
6. 用户按 `s` 或 `:w` 时写回文件并清除 dirty
7. 用户退出编辑模式后恢复 `Browse`
8. 退出后右侧预览基于最新文件内容重新渲染

### 4.3 异常与恢复

- 读取失败：不进入编辑，状态栏显示错误
- 保存失败：保持编辑模式，状态栏显示错误
- 命令不支持：保持 `VimNormal`，提示 `unsupported command`
- `:q` 遇到未保存改动：拒绝退出并提示 `unsaved changes`
- 文件在外部变化：P0 不做自动合并，仅提示可能已过期

## 5. 代码变更点（草案）

建议最小变更点：`src/ui/tui.rs`

- `AppState` 新增内联编辑状态字段
- `handle_key_event` 新增编辑模式分流：
  - 编辑模式开启时，优先走 `handle_code_edit_key_event`
  - 非编辑模式下保留现有分支
- `draw_code_source_preview` 根据模式渲染：
  - 浏览态渲染只读文本
  - 编辑态渲染 buffer + 光标/模式指示
- 新增函数（或方法）：
  - `enter_code_edit_mode()`
  - `save_code_edit_buffer()`
  - `exit_code_edit_mode()`
  - `handle_code_edit_key_event()`
  - `handle_code_command(input: &str)`

## 6. 测试原型（建议）

### 6.1 行为测试

1. 不在 `CodeBrowser` 页面按 `e`：不进入编辑模式
2. 选中目录按 `e`：不进入编辑模式，给出提示
3. 选中文件按 `e`：进入 `VimNormal`
4. `i` 进入 `VimInsert`，输入后 `Esc` 回到 `VimNormal`
5. `s` 保存后文件内容更新，预览同步更新
6. `:w` 保存成功，`:q` 退出，`:wq` 保存并退出
7. `:q` 在 dirty 状态下拒绝退出并提示
8. `Esc` 退出编辑后恢复 `Browse` 键位行为

### 6.2 回归测试

- 未进入编辑模式时，主页面当前配置键位、`Tab`、命令栏、搜索、patch 操作行为不变
- Code Browser 的 tree 展开/收起行为不变

## 7. 后续迭代（P1/P2）

- P1: 增加 `a/o/dd/u` 等常见 Vim 动作
- P1: 增加 `:q!`、`:w!` 等最小扩展命令
- P2: 评估 Neovim 内嵌（msgpack-rpc）可行性，决定是否升级为“真 Vim”

## 8. 验收标准（P0）

满足以下条件即验收通过：

1. 默认状态下，CRIEW 现有键位无冲突
2. 用户按 `e` 才进入 `Code Preview` 内联编辑模式
3. 编辑态支持最小 Vim-like 体验（`NORMAL/INSERT/COMMAND`、保存、退出）
4. 命令模式支持 `:w`、`:q`、`:wq` 且行为符合预期
5. 退出编辑后能回到浏览态并看到最新内容
6. 异常路径（无文件、读写失败、非法命令）有明确状态提示

## 9. VM2 外部 Vim 会话原型（增强）

VM2 目标：在 VM1 内联编辑之外，支持从 `Code Preview` 切出到外部 Vim 进行完整编辑。

### 9.1 入口与触发

- 建议入口：`E`（与 VM1 的 `e` 区分）
- 可选入口：命令模式支持 `:vim`
- 触发条件：
  1. 当前页面 `CodeBrowser`
  2. 当前焦点 `Source`
  3. 当前选中项是文件

### 9.2 与 VM1 协同规则

- 若内联编辑 buffer 为 dirty：
  - 默认拒绝切出外部 Vim
  - 状态栏提示：`unsaved changes, run :w before external vim`
- 若无 dirty：
  - 允许直接切出外部 Vim
  - 外部 Vim 退出后，强制从磁盘重载并刷新预览

### 9.3 时序草图

1. 用户触发 `E`（或 `:vim`）
2. 校验文件与 dirty 状态
3. 执行 `disable_raw_mode()`
4. 执行 `LeaveAlternateScreen`
5. 前台启动外部编辑器（`VISUAL -> EDITOR -> vim`）
6. 编辑器退出后执行 `EnterAlternateScreen`
7. 执行 `enable_raw_mode()`
8. 重新读取文件并刷新 `Code Preview`
9. 状态栏提示执行结果（成功/失败/退出码）

### 9.4 错误处理草图

- 启动失败：回到 TUI，提示 `failed to launch external editor`
- 编辑器异常退出：回到 TUI，提示 exit code/signal
- 恢复失败：记录错误并尽力恢复可操作终端

### 9.5 VM2 验收补充

1. 有效文件场景触发 `E` 可打开外部 Vim
2. 退出后返回 CRIEW，预览显示最新内容
3. dirty 场景触发 `E` 会被阻止并提示先保存
4. 失败路径不会导致 TUI 卡死或终端不可用
