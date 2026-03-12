# Git Hooks

本目录保存 CRIEW 仓库版本化管理的 Git hooks。

这些 hooks 基于 [docs/code-guildline-cn.md](../docs/code-guildline-cn.md) 中的
提交规范实现，目标是让提交信息和 `Signed-off-by:` trailer
在本地就能得到提示和校验。

GitHub Actions CI 也会通过 [scripts/check-commit-messages.sh](../scripts/check-commit-messages.sh)
复用这里的 `commit-msg` 规则，避免本地与 CI 漂移。

## 启用方式

在仓库根目录执行：

```bash
git config core.hooksPath .githooks
```

如果想确认当前仓库是否已启用：

```bash
git config --get core.hooksPath
```

预期输出：

```text
.githooks
```

## 当前 hooks

### `prepare-commit-msg`

用途：

- 当你执行普通 `git commit` 且提交信息还是空白时，自动插入注释模板
- 提示允许的格式：`feat:`、`feat(scope):`、`fix:`、`docs:`、`refactor:`、`test:`、`chore:`
- 提醒主题句使用祈使语气，并尽量控制在 72 字符内
- 提醒作者自己产生的 commit 使用 `git commit -s`
- 提醒较大提交需要补 body，并用分点条目说明主要改动

不会改写的场景：

- `git commit -m "..."`
- merge commit
- squash commit
- 复用已有提交信息的场景

### `commit-msg`

用途：

- 校验提交信息第一条真实主题行
- 要求主题格式为：`<type>: <summary>` 或 `<type>(<scope>): <summary>`
- 要求作者自己产生的 commit 带有合法的 `Signed-off-by:` trailer
- 当提交改动较大时，要求 body 中存在分点条目
- 允许的 `type`：
  - `feat`
  - `fix`
  - `docs`
  - `refactor`
  - `test`
  - `chore`
- 主题超过 72 字符时给出 warning，但不阻断提交

主题特判场景：

- `Merge ...`
- `Revert ...`
- `fixup! ...`
- `squash! ...`

说明：

- `Merge ...` 直接放行
- `Revert ...` 允许不使用 Conventional Commit 前缀，但仍需 `Signed-off-by:`
- `fixup! ...` / `squash! ...` 会先剥离前缀再做主题校验，仍需 `Signed-off-by:`
- `Merge ...`、`Revert ...`、`fixup! ...`、`squash! ...` 不参与“大提交必须写 body”的额外校验

当前“大提交”阈值：

- 单个 commit 改动至少 6 个文件
- 或新增/删除总行数至少 150 行

推荐提交流程：

```bash
git commit -s
git commit -s -m "fix(sync): handle empty IMAP checkpoint"
git commit -s

# Then add a body for a larger change:
# - split sync state handling from UI refresh
# - add regression coverage for startup inbox sync
```

## 推荐写法

推荐：

```text
feat: add IMAP inbox background sync
feat(ci): validate commit messages in workflow
fix: handle empty lore response safely
docs: rewrite README for open source users
refactor: split sync source resolution
test: cover fixup commit message handling
chore: enable repository git hooks
```

不推荐：

```text
update code
fix bug
WIP
misc changes
```

## 对应规范

hooks 当前落地的是 `docs/code-guildline-cn.md` 中与提交信息直接相关的部分：

- `atomic-commits`：一个 commit 一个逻辑变化
- `refactor-then-feature`：重构与功能改动尽量拆开
- `signed-commits`：作者自己产生的 commit 必须带 `Signed-off-by:` trailer
- `large-commit-body`：较大提交必须带分点 body
- Conventional Commit 前缀：
  - `feat:`
  - `feat(scope):`
  - `fix:`
  - `docs:`
  - `refactor:`
  - `test:`
  - `chore:`
- 前缀后的主题句使用祈使语气
- 主题行尽量不超过 72 字符

## 调整规则

如果要修改 hook 行为：

1. 先更新 [docs/code-guildline-cn.md](../docs/code-guildline-cn.md) 中的规范
2. 再同步修改本目录下对应 hook
3. 最后补充或更新本说明文档
