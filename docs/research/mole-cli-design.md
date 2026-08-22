# Mole CLI 交互设计调研

> 调研时间：2026-08-22。只使用 Mole 官方仓库、源码和发布材料。源码基线为 `tw93/Mole` commit `064998cf2858703269a0101278e65b7c0e048c06`，对应当前 `V1.51.0`。官方定位是“在终端中清理、卸载、分析、优化和监控 Mac”，命令入口为 `mo`。[仓库 README](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/README.md#L1-L27) · [V1.51.0 Release](https://github.com/tw93/Mole/releases/tag/V1.51.0)

## 结论

Mole 值得 SkillRoster 学习的核心不是全屏 TUI，而是**同一套能力面向两类消费者提供两种输出**：人类在真实终端里得到清楚、紧凑、有节奏的界面；Agent 使用明确的 JSON、历史记录和预览结果，绝不解析 TUI。Mole 自带的 Agent Skill 甚至明确规定“Never parse a TUI frame”，并要求破坏性操作先 `--dry-run`、让用户看到候选项再执行。[官方 Agent Skill](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/.claude/skills/mole/SKILL.md#L6-L30)

SkillRoster 应采用这个“双界面”思想，但保持既定架构：**仍用 Rust，不引入 Go/Bubble Tea，不生成 HTML，也不把全屏 TUI 作为 Agent 工作流的一部分。**稳定 JSON 是产品接口；好看的终端输出是人类直接运行、调试和建立第一印象的界面。

## 1. 命令流与信息架构

Mole 的顶层命令直接使用用户任务语言：`clean`、`uninstall`、`optimize`、`analyze`、`status`。无参数运行进入五项主菜单；传入命令则直接分发，未知命令给出下一步帮助，而不是进入猜测式交互。[命令清单](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/README.md#L58-L95) · [菜单与分发源码](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/mole#L86-L131) · [命令路由](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/mole#L230-L315)

其人类路径大致是：

```text
mo
  → 选择任务
  → 扫描/加载
  → 浏览和选择候选项
  → 展示数量、体积和目标
  → 明确确认
  → 执行
  → 结果摘要与历史
```

SkillRoster 的对应路径应是：

```text
Agent 调用 scan --json
  → report/find --json
  → Agent 基于证据向用户解释
  → plan --stdin --json
  → Agent 展示完整变更计划
  → 用户一次确认
  → apply <plan-id> --json
  → Agent 汇报 Receipt 与 undo 方法
```

这里不需要照搬 Mole 的无参数全屏菜单。`skillroster` 无参数时，显示紧凑品牌头、当前状态和三条最常用命令即可；真实治理仍由 Agent 驱动。

## 2. Mole 为什么“看起来好用”

### 2.1 视觉层级

Mole 将每一屏稳定地分成五层：

1. **一句话标题**：如 `Clean Your Mac`、`Analyze Disk`。
2. **关键上下文**：扫描路径、剩余空间、总量或系统概况。
3. **可比较的主体行**：同类数据对齐，名称居左，数字居右；占比用固定宽度条形表示。
4. **弱化的操作提示**：快捷键固定在底部并使用灰色，不与结果争抢注意力。
5. **强结果摘要**：用分隔线框住完成状态、核心数字、警告和下一步。

官方示例中，清理结果按类别列出并对齐体积，磁盘分析用进度条、百分比、名称和大小构成单行扫描模式，状态页则把指标组织成成对卡片。[清理输出](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/README.md#L118-L135) · [磁盘分析输出](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/README.md#L186-L205) · [状态面板](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/README.md#L207-L238)

### 2.2 颜色与图标

其 Shell UI 的语义色较克制：紫色用于标题/动作，青色用于当前选择，绿色用于成功，黄色用于警告或预览，灰色用于次要信息；`✓`、`○/●`、`➤`、`→`、`⊙` 等符号同时承担状态含义。[颜色与图标定义](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/lib/core/base.sh#L20-L45) · [图标定义](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/lib/core/base.sh#L77-L94)

可迁移原则是“颜色增强含义，但不独占含义”。SkillRoster 的 Finding 必须同时显示文本严重级别与符号，例如 `! High · 断开的软链接`，不能只靠红黄绿区分。

### 2.3 响应式布局

Mole 会读取终端宽度，缩短 spinner 文案、截断长路径；状态页在宽屏使用双列，80 列及以下切为单列，并在高度不足时减少次要 CPU 行。磁盘分析还按可用宽度决定路径与扫描状态是否同处一行。[Spinner 宽度处理](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/lib/core/ui.sh#L330-L356) · [状态页响应式切换](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/cmd/status/main.go#L182-L240) · [扫描路径布局](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/cmd/analyze/view.go#L80-L125)

SkillRoster 不需要卡片式 TUI，但应保证 60、80、120 列三档可读：窄屏把路径/证据放到下一行；宽屏才对齐 Agent、Layer、数量与比例。

## 3. 交互组件、进度与动画

Mole 同时使用两种呈现：

- `clean` 等线性任务使用普通滚动输出、单行 spinner、分类行和最终摘要。
- `analyze`、`status` 等需要持续导航或刷新状态的任务使用 alternate-screen TUI。它们基于 Go 的 Bubble Tea/Lip Gloss，但这只是实现选择，不是视觉体验成立的前提。[Analyze TUI 入口](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/cmd/analyze/main.go#L19-L37) · [Status TUI/JSON 分流](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/cmd/status/main.go#L301-L365)

细节尤其值得借鉴：

- spinner 只在 TTY 中动态刷新，并写入 `stderr`，避免污染 `stdout`；非 TTY 时退化为静态文本。[Spinner 源码](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/lib/core/ui.sh#L359-L425)
- 动画文案可原位更新，完成后清除整行，避免闪烁和残影。[Spinner 更新与清理](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/lib/core/ui.sh#L428-L487)
- 扫描百分比在尚未真正完成时最多显示 99%，避免“100% 但还在工作”的虚假反馈。[Analyze 进度源码](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/cmd/analyze/view.go#L80-L104)
- 底部快捷键提示随当前模式变化，只展示此刻有效的动作。[Analyze 操作提示](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/cmd/analyze/view.go#L360-L400)

SkillRoster 的扫描可能持续数秒，适合使用一条 TTY spinner，例如 `⠋ Scanning Codex · 3/8 agents`；不应为了“好看”加入全屏动画。JSON 模式必须完全无 spinner；如需进度集成，应以后单独设计 NDJSON event stream，不能混入最终 JSON 文档。

## 4. 确认、安全与可恢复性

Mole 的安全面包括 `--dry-run`、白名单、候选项选择、执行前汇总、历史日志和部分 Trash 恢复。安装器在删除前列出文件、总数量和体积，再用 `Enter` 确认、`Esc/Q` 取消；Analyze 对删除也显示目标数量/体积并明确确认。[安装器确认流程](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/bin/installer.sh#L660-L706) · [Analyze 删除确认](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/cmd/analyze/view.go#L401-L438)

官方 Agent Skill 进一步把安全约束提升为工作流：Agent 必须先 dry-run、展示候选项，只有用户明确请求时才执行，并通过 `history --json` 回答“删了什么”。它也说明 `clean` 默认永久删除，而卸载默认进入 Trash。[Agent 安全工作流](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/.claude/skills/mole/SKILL.md#L32-L73) · [恢复边界](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/.claude/skills/mole/SKILL.md#L94-L106)

但 Mole 有一处**不适合照搬**：`mo clean` 在非交互模式下会自动继续用户级清理；dry-run 是强烈提示，而不是命令本身强制的前置门槛。[非交互 clean 行为](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/bin/clean.sh#L1456-L1506)

SkillRoster 应更严格：

- `scan/report/find/status` 永远只读。
- `plan` 是唯一预览，不产生副作用。
- `apply <plan-id>` 只能执行已持久化、不可变且仍通过漂移校验的 Plan。
- JSON 模式不弹交互提示；调用 Agent 在对话里取得用户确认后，再显式执行 Plan。
- 每次写操作生成 Receipt，结果中始终给出 `undo <receipt-id>`；遇到漂移或不确定性则停止，而不是扩大范围。

## 5. JSON、非交互模式与 Agent 使用

Mole 已经把 Agent 与人类界面分开：`analyze --json` 和 `status --json` 输出机器数据，`status` 在 stdout 被管道接走时还会自动切到 JSON；持续监控使用 NDJSON `--watch`。[README 机器输出](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/README.md#L240-L278) · [Status 检测与分流](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/cmd/status/main.go#L22-L46)

其官方 Agent Skill 把真正可消费的接口限定为磁盘 JSON、历史 JSON、dry-run 路径清单和状态 JSON，并要求 Agent 显式传 `--json`，即使工具能自动检测管道。[Agent-facing surfaces](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/.claude/skills/mole/SKILL.md#L46-L73)

这与 SkillRoster 高度一致，但 SkillRoster 应做得更统一：

- 每条命令都支持同一版本化 JSON envelope。
- `stdout` 恰好一个 JSON 文档；日志、进度和诊断不混入其中。
- JSON 里的数字、ID、Evidence 和 suggested actions 是事实来源；Agent 不抓取彩色文本。
- 人类终端输出可以自由优化，但不得改变 JSON 契约。
- 不依赖“管道自动转 JSON”作为 Agent 契约；bootstrap Skill 始终显式使用 `--json`，意图更清楚，也避免 shell 管线中的惊讶行为。

## 6. 可访问性与 TTY 降级

Mole 的 Shell 层支持标准 `NO_COLOR`，并测试非空 `NO_COLOR` 会清除全部 ANSI 色值；它也检测 stdout 是否为 TTY、`TERM` 类型和 terminfo 能力。[NO_COLOR 实现](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/lib/core/base.sh#L20-L46) · [NO_COLOR 测试](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/tests/no_color.bats#L1-L37) · [ANSI 能力检测](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/lib/core/base.sh#L1315-L1362)

不过它并非完全一致：Analyze 的 Go 代码直接定义 ANSI 颜色，而且只有显式 `--json` 才绕开 TUI；这说明不能因为部分命令支持 `NO_COLOR`，就假定整个工具都已无障碍降级。[Analyze 颜色常量](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/cmd/analyze/constants.go#L295-L307) · [Analyze 模式选择](https://github.com/tw93/Mole/blob/064998cf2858703269a0101278e65b7c0e048c06/cmd/analyze/main.go#L19-L37)

SkillRoster 应从第一天统一保证：

- 遵循 `NO_COLOR`；`TERM=dumb`、非 TTY、CI 日志使用纯文本。
- 每个状态除颜色外还有文字或符号；表格不是唯一表达。
- Unicode 图标显示异常时可退化为 ASCII：`✓/!/×` → `OK/WARN/ERR`。
- 长路径中间截断，但 JSON 与 Finding 下钻保留完整值。
- 动画尊重非 TTY，并为未来的 reduced-motion 配置留出简单开关；首版不需要复杂动画系统。

## 7. 对 SkillRoster 的取舍清单

| Mole 设计 | SkillRoster 决策 | 理由 |
|---|---|---|
| 任务动词作为命令 | 采用 | `scan/report/find/plan/apply/undo/status/setup` 已经直观 |
| 人类 TTY 与 Agent JSON 分离 | 强化采用 | 这是 Agent-first 产品最重要的界面边界 |
| 对齐数字、分类行、弱化提示、强摘要 | 采用 | 不需要全屏 TUI 也能产生明显品质感 |
| `--dry-run` + 确认 + history | 改造成 Plan + Apply + Receipt + Undo | Skill 治理涉及持久配置，必须可验证、可撤销 |
| TTY-only spinner 写 stderr | 采用 | 不污染 JSON；低成本且跨平台 |
| 全屏 Analyze/Status TUI | 首版不采用 | 用户最终在 Agent 对话中看总结，TUI 会重复呈现层 |
| Go + Bubble Tea/Lip Gloss | 不采用 | 保持单一 Rust binary；只借鉴交互，不借鉴技术栈 |
| 无参数进入五项菜单 | 简化采用 | 无参数显示状态和下一步，不建立第二套完整导航 |
| 管道时自动 JSON | 不作为契约 | Agent 明确传 `--json`，行为更稳定可审计 |
| 非交互 clean 自动继续 | 拒绝 | SkillRoster 的写操作必须来自明确 Plan 和用户确认 |
| 单一“健康分” | 拒绝 | Skill 治理结论必须可追溯到 Finding 和 Evidence，不能用不可解释总分代替事实 |

## 8. 推荐的首版终端基线

普通终端输出保持短、漂亮、可截图，但定位为“人类界面”，不是 Agent API：

```text
SkillRoster · Scan

  ✓ Agents checked       8 / 8
  ✓ Skills found           137
  ✓ Placements             212
  ! Findings                31

  High     4   broken or unsafe exposure
  Medium  18   duplicate or divergent copies
  Low      9   stale or unknown lifecycle

────────────────────────────────────────────────────────────
Scan complete · snap_01K...
Next: skillroster report
```

执行计划则突出确认边界，而不是品牌装饰：

```text
SkillRoster · Apply plan_01K...

  12 links create   7 links replace   9 skills archive
  8 agents affected · reversible · drift check passed

  ! No Skill content will be deleted.
  Undo after apply: skillroster undo <receipt-id>

Apply this plan?  Enter confirm · Esc cancel
```

最终原则可以概括为：**学习 Mole 的节奏、层级、状态语义和 Agent/TTY 分流；不复制它的全屏形态、Go 技术栈或非交互破坏性默认值。**
