# SkillRoster 当前轮一手资料研究：确定性治理边界、覆盖证据与默认暴露风险

日期：2026-08-25
范围：SkillRoster 作为 Agent-first 本地治理 CLI；重点核对 Codex/Agent Skills 的发现与渐进披露、上下文成本、候选选择，以及可由 CLI 确定性证明的事实边界。
方法：只采用 OpenAI/Anthropic 官方文档或工程文章、论文原文（arXiv）；把“来源事实”“产品推论”和“尚未证明”分开。本文只新增研究记录，不代表已修改产品实现。

## 结论先行

SkillRoster 不应把自己定义成“替模型决定用户意图的总路由器”。更可靠的 Agent-first 分工是：

1. **CLI 负责事实、边界和可复现证据。** 它应确定库存、规范化身份、路径与来源、解析结果、作用域、启用/显式调用策略、当前 harness 可见性、重复项、内容指纹、候选列表及其计算规则，并生成可重放的 plan/receipt。
2. **模型负责语义和意图。** 它可以判断用户究竟要做什么、是否真的需要 Skill、多个候选中哪个最适合当前语境、是否要组合多个 Skill，以及读取后如何把程序性知识适配到任务；这些判断应带着候选证据返回，而不应伪装成 CLI 已经证明的事实。
3. **“覆盖率”不能只有一个数字。** 库中存在、当前 harness 可见、检索候选命中、模型实际加载、任务通过是五个不同门。报告必须给出 gold 集、分母、候选深度、作用域、快照、harness/模型、方法和时间；没有这些字段的“100% coverage”会误导。
4. **已证实的风险针对“默认可见候选面”，不是针对磁盘库存本身。** Codex 会对初始 Skill 元数据设预算；Skill 研究显示候选选择、干扰和检索会降低表现；过时或不匹配的 Skill 还可能产生上下文干扰。没有一篇来源给出适用于所有模型和 Skill 的固定数量上限。大库存可以保留，但默认暴露应按作用域和需求收敛，并用本地 paired eval 决定边界。

## 一、来源事实

### 1. Codex 的发现和渐进披露

来源：[OpenAI Build skills](https://developers.openai.com/codex/skills/)（当前页面重定向至 ChatGPT Learn 的同一官方文档）。

**来源事实：**

- Codex/ChatGPT 先把每个 Skill 的 `name` 和 `description` 放入初始上下文，决定使用后再读取完整 `SKILL.md`；Codex 初始条目还包括文件路径。这是 metadata → 正文的渐进披露，不是把所有正文一次注入。
- Codex 的初始 Skill 列表最多占模型上下文的 2%；上下文大小未知时上限是 8,000 字符。Skill 很多时，系统会先缩短描述，大集合还可能省略部分 Skill 并显示 warning。
- 隐式调用依赖 `description` 的匹配，因此官方要求描述简洁、边界清楚、把主要触发词放在前面。显式调用和隐式调用是两种不同路径；`agents/openai.yaml` 可以设置 `allow_implicit_invocation: false`，保留显式使用。
- Codex 从仓库、用户、admin 和 system 位置发现本地 Skill；仓库层从当前目录向仓库根目录扫描 `.agents/skills`。相同 `name` 的 Skill 不会自动合并，两者都可能出现在选择器中。

**不能从文档推出：** 初始列表的具体排序/省略算法、桌面任务刷新时刻、一个外部冷库会被 Codex 原生自动检索，或某个固定的“最佳 Skill 数量”。

### 2. 哪些工作适合确定性代码

来源：[OpenAI Model guidance](https://developers.openai.com/api/docs/guides/latest-model)；对照：[Anthropic Agent Skills](https://www.anthropic.com/engineering/equipping-agents-for-the-real-world-with-agent-skills)。

**来源事实：**

- OpenAI 将 Programmatic Tool Calling 定位在有边界、可预测的处理：过滤、连接、排序、去重、聚合、验证；当每个结果都会改变模型下一步判断、需要审批，或最终输出必须保留原生引用/工件时，应使用直接调用让模型判断。
- Anthropic 的 Agent Skills 文章同样明确区分：排序等操作由传统代码执行更便宜，而且代码能提供确定性、可重复结果；Skill 仍可携带脚本，由模型按任务需要选择运行。
- OpenAI 的模型指导还要求对工作流明确输入、输出 schema、证据、重试/停止限制，并提醒资源用量下降只有在最终质量门仍通过时才算改进。

**产品推论：** 这为 SkillRoster 提供了清晰的 seam：把“是什么、在哪里、当前是否可见、发生了什么”留在 CLI；把“这句话想要什么、候选如何适配”交给模型。CLI 可以给模型排序候选，但不应把词法/向量相似度直接命名为“意图真值”。

### 3. 供应商对大工具面的经验只能作为类比

来源：[OpenAI Function calling](https://developers.openai.com/api/docs/guides/function-calling)；[Anthropic advanced tool use](https://www.anthropic.com/engineering/advanced-tool-use)。

**来源事实：**

- OpenAI 文档建议回合开始时尽量少于 20 个可用函数，但明确说这是 soft suggestion，要求在自己的 workload 上比较不同数量；函数定义会进入 system message，计入上下文和输入 token。对大型或低频函数面，官方建议用 tool search 延迟加载。
- Anthropic 的示例中，58 个 MCP tools 约占 55K tokens；相似名称会导致错误工具选择和参数错误。Tool Search 只预载搜索工具，再按需加载约 3–5 个工具；该内部 MCP eval 报告 Opus 4 从 49% 到 74%、Opus 4.5 从 79.5% 到 88.1%。

**边界：** 这些数字测的是 function/MCP schema，不是 Codex `SKILL.md` 的数量阈值。Codex 初始 Skill 主要是 name/description/path，正文按需读取，不能把“少于 20 functions”改写成“少于 20 Skills”。它们支持“少量默认暴露＋按需发现”的方向，但不证明某个本地 Core 数字。

## 二、与 Skill 数量和路由直接相关的论文证据

论文均为原始论文/预印本；数字只适用于论文中的模型、harness、数据集和协议。

### 1. SkillsBench：Skill 有效性依赖组合和 harness

来源：[SkillsBench v4](https://arxiv.org/abs/2602.12670)（全文：[HTML](https://arxiv.org/html/2602.12670)）。

**来源事实：** 当前 v4 摘要报告 87 个任务、8 个领域、18 个 model–harness 配置；配对的 no-Skills 到 curated-Skills 平均通过率从 33.9% 到 50.5%，配置级增益范围为 +4.1 到 +25.7 个百分点；最多三个模块的 focused Skills 优于更大或穷尽式 bundle。论文还明确指出同一模型在不同 harness 下的 Skill 使用方式不同。

**产品推论：** SkillRoster 的质量门应是固定任务集上的“有/无 Skill”配对结果，而不是库存数或单次路由命中。一个 Skill/组合的收益必须带上模型、harness 和任务集版本；不能把跨 Codex、Claude Code、Pi 的使用统计合并成一个普适分数。

### 2. How Well Do Agentic Skills Work in the Wild：候选选择和检索会损失收益

来源：[arXiv 2604.04323](https://arxiv.org/abs/2604.04323)（全文：[HTML](https://arxiv.org/html/2604.04323v1)）。

**来源事实：** 该研究从开源仓库整理、许可过滤、质量过滤和去重后构成约 34k 个真实 Skill。Claude Opus 4.6 在同一组 curated Skills 下，强制加载为 55.4%，让 Agent 自主选择降为 51.2%，加入干扰项进一步降为 43.5%；从 34k 库检索且 curated Skill 仍在库中为 40.1%，无 Skill 基线为 35.4%。其最佳 agentic hybrid 检索在该实验上 Recall@5 为 65.5%。

**产品推论：** “目标 Skill 在库里”不等于“模型能找到并加载”；“候选进入列表”也不等于“模型选对”。默认暴露的相似/无关候选会增加选择压力，而大库检索还引入召回失败。该研究没有证明完全不暴露的归档文件会伤害表现，也没有证明必须删除大库。

### 3. SWE-Skills-Bench：内容质量、版本和上下文匹配同样是风险

来源：[SWE-Skills-Bench](https://arxiv.org/abs/2603.15401)（全文：[HTML](https://arxiv.org/html/2603.15401)）。这是预印本，论文自己标注为 preliminary/work in progress。

**来源事实：** 在 49 个公开 SWE Skill、约 565 个固定提交的真实项目任务上，39/49 个 Skill 的通过率增益为零，平均增益为 +1.2%；某些 Skill 的 token overhead 在通过率不变时最高增加 451%；3 个 Skill 因版本特定指导与项目上下文冲突使性能最多下降 10%。该研究用执行测试做确定性验证，而不是 LLM-as-judge。

**产品推论：** `last_used=0` 不是删除证据；相反，版本漂移、依赖失效、任务/项目不匹配和重复内容应成为治理状态。CLI 可以确定文件是否存在、版本/依赖/哈希是否变化并把候选标为 stale/unknown，但“这份指导会不会与当前项目冲突”仍要由任务评测或模型结合上下文判断。

### 4. 工具 shortlist 研究：必须拆开“呈现、选择、执行”

来源：[How Many Tools Should an LLM Agent See?](https://arxiv.org/abs/2605.24660)（全文：[HTML](https://arxiv.org/html/2605.24660)）。

**来源事实：** 该论文在 20–3,251 个工具的 registry 上研究候选深度。BFCL（370 tools）上，自适应策略平均展示约 7 个工具时，覆盖率 90.3%，接近展示 50 个的 90.8%；在下游 Claude Sonnet 4.6 验证中，正确工具已呈现时，短的自适应列表选择准确率 93.1%，固定展示 5 个为 87.1%。论文明确区分“正确工具是否呈现”“模型是否选对”“执行是否成功”，并说明执行正确性不在该论文范围内。

**边界：** 这是工具检索，不是 Skill 研究，不能拿 7、50 或 5 当成 Skill 数量规则。它支持的是评测分层：SkillRoster 报告必须分别记录 candidate recall、model choice 和 task outcome，不能用一个混合 coverage 数字掩盖失败位置。

### 5. 默认暴露还有供应链/执行面风险，但不能夸大为“库存即执行”

来源：[Agent Skills in the Wild: An Empirical Study of Security Vulnerabilities at Scale](https://arxiv.org/abs/2601.10338)（全文：[HTML](https://arxiv.org/html/2601.10338)）。

**来源事实：** 研究从两个 marketplace 收集 42,447 个 Skill，并对 31,132 个运行检测流程；26.1%（8,126 个）被检测为至少含一种潜在危险模式，覆盖 prompt injection、数据外泄、权限提升和供应链类别；5.2% 含高严重性模式。带 executable scripts 的 Skill 被检测出潜在危险模式的 odds 是 instruction-only Skill 的 2.12 倍。论文的检测器报告 precision 86.7%、recall 82.5%。

**边界和产品推论：** 这些是特定公开生态和检测器的 flagged prevalence，不是“所有 Skill 有 26.1% 漏洞”的普适事实；被扫描出来也不等于已经成功利用。它足以支持默认暴露的安全审查：来源、信任、脚本、网络/凭据权限和审核状态应由 CLI 确定性列出；是否加载、是否执行和执行后影响则需要 harness 日志与安全策略，不应靠“Skill 名称看起来安全”推断。

## 三、SkillRoster 的确定性/语义分工

以下是基于上述来源和当前 Agent-first CLI 目标的**产品推论**，不是供应商对 SkillRoster 的规范。

| 领域 | CLI 应确定性提供 | 模型可以判断 | CLI 不应声称已经证明 |
| --- | --- | --- | --- |
| 身份 | 规范化 `skill_id`/name、解析后的 frontmatter、绝对路径、真实路径、来源、内容 hash、读取时间 | 哪个候选概念上更贴近用户目标 | “名字相似所以就是同一个 Skill” |
| 库与作用域 | canonical inventory、repo/user/admin/system scope、enabled/disabled、explicit-only、重复名、软链接、缺失依赖 | 当前请求需要哪个作用域的能力 | “库存数 = 当前模型可见数” |
| 元数据健康 | `SKILL.md` 是否存在、frontmatter 是否可解析、description 长度、脚本/引用文件是否存在、版本/依赖是否新鲜 | 描述是否足以支持当前任务、旧指导是否与项目冲突 | “metadata 解析通过 = Skill 质量可靠” |
| 候选生成 | 可复现的过滤条件、排序规则、top-k、每项得分特征、未命中和 tie 状态 | 是否需要扩大搜索、是否要组合/改用普通能力 | “词法/向量 score = 意图真值” |
| 用户意图 | 回显原请求、候选证据和不确定性；必要时输出 `ambiguous`/`no_match` | 任务目标、隐含约束、是否需要 Skill、候选适配、加载顺序 | “CLI 自动猜中了用户真正想做什么” |
| 变更 | plan、diff、目标、前置条件、审批点、执行日志、receipt、undo 所需快照 | 是否接受计划、哪些语义冲突需要询问 | “模型说 apply 了 = 文件已变更” |
| 评测 | gold 集版本、可见列表快照、候选列表、top-k 指标、模型/harness/任务版本、原始事件和确定性 verifier 结果 | 选择/加载是否适合任务、自然语言答案是否满足语义要求 | “route 命中 = 任务成功” |

推荐的最小 CLI 输出对象（字段名可调整，但语义应保留）：

```json
{
  "skill_id": "dms-mysql",
  "name": "dms-mysql",
  "path": "/resolved/path/SKILL.md",
  "source": "user",
  "scope": "user",
  "content_sha256": "...",
  "metadata": {"parse": "ok", "allow_implicit_invocation": true},
  "visibility": {"discovered": true, "initially_visible": false},
  "candidate": {"rank": 1, "score": 12, "score_method": "lexical-v1"},
  "status": "candidate",
  "evidence_snapshot": "snapshot-2026-08-25T..."
}
```

这里的 `score` 只描述 CLI 的候选生成过程；它不应被渲染成 `intent_confidence=0.92`，除非另有经过验证的模型/评测定义。

## 四、覆盖证据如何表达才不误导

### 1. 五个必须拆开的门

设 `G` 为固定版本的 gold Skill 集，`V` 为某个 harness/作用域快照中的初始可见 Skill 集，`C_k` 为 CLI 检索的 top-k 候选集，`L` 为模型实际读取的 Skill 集，`P` 为通过确定性任务 verifier 的试验集：

| 指标 | 公式/含义 | 能回答什么 | 不能回答什么 |
| --- | --- | --- | --- |
| `inventory_presence` | `|G ∩ inventory| / |G|` | 目标是否存在于扫描库存 | 当前任务是否可见、模型是否找到 |
| `initial_visibility` | `|G ∩ V| / |G|` | 目标是否在本次初始候选面 | 模型是否选择/读取 |
| `candidate_recall@k` | `|G ∩ C_k| / |G|` | CLI top-k 是否把目标召回 | 模型是否会选它、任务是否成功 |
| `load_rate` | `|G ∩ L| / |G ∩ V|`（分母需明示） | 目标被呈现后是否实际读取 | Skill 内容是否正确、任务是否通过 |
| `task_success` | `passed verifier / valid trials` | 组合对真实任务是否有效 | 失败是召回、选择、执行还是环境问题 |

对单请求的多目标任务，还要明确是“命中任一 gold”还是“全部 gold”；不能把两种聚合放在同一列。

### 2. 每个数字都必须带的证据字段

至少包括：

- `metric`、`numerator`、`denominator`、`value`、`candidate_k`；
- `gold_set_id` / gold 规则（单目标、任一目标或全目标）；
- `inventory_snapshot`、`visible_scope`、`repo`/`cwd`、`enabled_policy`；
- `router_method`（例如 `lexical-v1`、`embedding-v2`、model-assisted）、排序和 tie 规则；
- `harness`、模型、版本、任务/评测集版本、时间；
- 排除项、超时、缺失日志、stale/incomplete 状态；未测量写 `unknown`，不要写成 0。

示例：

```json
{
  "metric": "candidate_recall@3",
  "value": 0.95,
  "numerator": 19,
  "denominator": 20,
  "gold_set_id": "route-cases-2026-08-25",
  "candidate_k": 3,
  "scope": "user+repo:/workspace/example",
  "inventory_snapshot": "sha256:...",
  "router_method": "lexical-v1",
  "harness": "codex-cli:<version>",
  "status": "measured"
}
```

### 3. 应避免的表述

- “本机有 127 个 Skill，因此 Agent 有 127 个 Skill 可用”：混淆 inventory、enabled、visible 和 loaded。
- “覆盖率 100%”：没有 gold 集和分母，无法知道是库存存在、初始可见还是任务成功。
- “router 找到了”：最多证明候选召回；没有证明模型加载、执行或任务通过。
- “90 天未使用，所以无价值”：使用日志可能漏掉隐式收益、未统计的读取或一次性关键能力；应改成“在给定观察窗口内未观察到事件”。
- “上下文窗口很大，所以暴露全部 Skill 没问题”：忽略 Codex 初始 metadata 预算、描述截断/省略和选择干扰。

## 五、默认暴露过多的已证实风险与未证实部分

| 风险 | 证据强度 | 可写成什么 | 不可写成什么 |
| --- | --- | --- | --- |
| 初始上下文预算竞争 | Codex 官方直接事实 | Skill metadata 受 2%/8,000 字符预算；过多时会缩短/省略并警告 | “超过 N 个 Skill 必然失败” |
| 候选选择干扰 | Agentic Skills 研究 + Anthropic 工程观察 | 同一 curated Skill 在自主选择/加入干扰时通过率下降；相似工具名容易错选 | “任何额外 Skill 都会降低效果” |
| 检索漏召回 | Agentic Skills 研究 | 34k 库实验最佳 Recall@5 为 65.5%，库内存在不等于候选出现 | “本地 router 的 recall 就是 65.5%” |
| 内容/版本干扰 | SWE-Skills-Bench 预印本 | 版本错配可能使性能下降，Skill token 成本与正确性不必同向 | “某个低频 Skill 一定有害” |
| 默认执行/供应链暴露 | 安全研究预印本 + Skill 可含脚本的官方事实 | 第三方 Skill 的 metadata、指令、脚本需要来源/信任/权限审查 | “被发现就已经执行”或“26.1% 是所有本地 Skill 的漏洞率” |
| 工具 schema token 成本 | OpenAI/Anthropic 官方工具文档 | function/MCP 定义会占上下文，大面应考虑延迟加载 | “工具数字阈值就是 Skill 数字阈值” |

因此，SkillRoster 可以保留完整 canonical inventory，把默认暴露面按 user/repo scope、explicit-only、on-demand 和 archive 分层；但任何具体 Core 数量（例如 30、40 或 50）只能是本地治理假设，必须通过固定 gold 集的 paired eval 验证，不是论文或官方文档给出的普适常数。

## 六、建议的下一道产品质量门

这是基于来源的**产品推论**，不是新的实现要求：

1. 固定一套带 gold 目标的真实请求集，至少覆盖：明确需要 Skill、明确不需要 Skill、相似 Skill 二选一、冷 Skill、跨 repo scope、过时/有依赖问题的 Skill，以及多 Skill 组合。记录版本化输入，不只记录最终文本。
2. 对 baseline（完整默认暴露）、candidate（Core + repo scope）和 fallback（candidate + 可审计冷库检索）做配对比较；至少分开 `initial_visibility`、`candidate_recall@k`、model `top1/topk choice`、`load_rate` 和任务 verifier 结果。
3. 让 CLI 先输出确定性事实和候选证据；模型输出结构化的意图/选择/不确定性。低 margin、重复名、依赖 stale、来源未审计或可能执行脚本时，返回 `ambiguous`/`needs_review`，不要静默 apply。
4. 将 `scan → report → plan → one confirmation → apply → receipt → undo` 视为状态链：每个状态都记录输入快照和输出，不用自然语言“已完成”替代文件 hash、变更清单和回滚证据。
5. 结论只在同一 snapshot、harness、模型、任务集和方法下比较；跨供应商数字仅作方向性背景，最终阈值由 SkillRoster 自己的 paired eval 决定。

## 七、自然语言治理 dogfood 的新增验证

Issue #209 的四轮 Luna dogfood 把上述边界落到一个真实用户请求。无
Bootstrap 的 Agent 能找到相同结构事实，但执行了 9 次 CLI 调用；修订后的
Bootstrap 最终只用 Scan 和 bounded Report，仍保留四项核心指标、Top 3、
完整 rollups 和覆盖限制。详细证据见
[Natural-language Agent governance dogfood v1](../acceptance/agent-governance-natural-language-v1.md)。

本轮同时验证了一个反例：Agent 曾把 9 个 placements 和 3 个物理来源推导为
“可减少 6 个 placements”。这不是工具事实，也没有 Plan 支持。ReAct 的原始
工作支持 observation-grounded 的行动循环，但不把观察后的自由推断变成确定性
事实；Anthropic 的可信 Agent 原则同样强调 transparency、human control 和在
高风险动作前请求介入。因此 Finding 只陈述当前影响规模，Plan 才能陈述拟议的
before/after，Receipt 才能陈述实际变更结果。该结果支持收紧 Bootstrap，不支持
增加一个新的 Rust 语义分析子系统。

## 来源索引

- OpenAI：[Build skills](https://developers.openai.com/codex/skills/)、[Function calling](https://developers.openai.com/api/docs/guides/function-calling)、[Model guidance](https://developers.openai.com/api/docs/guides/latest-model)。
- Anthropic：[Equipping agents for the real world with Agent Skills](https://www.anthropic.com/engineering/equipping-agents-for-the-real-world-with-agent-skills)、[Introducing advanced tool use](https://www.anthropic.com/engineering/advanced-tool-use)。
- Skills 论文：[SkillsBench v4](https://arxiv.org/abs/2602.12670)、[How Well Do Agentic Skills Work in the Wild](https://arxiv.org/abs/2604.04323)、[SWE-Skills-Bench](https://arxiv.org/abs/2603.15401)。
- 候选深度论文：[How Many Tools Should an LLM Agent See?](https://arxiv.org/abs/2605.24660)。
- 安全论文：[Agent Skills in the Wild](https://arxiv.org/abs/2601.10338)。
- Agent 决策边界：[ReAct](https://arxiv.org/abs/2210.03629)、[Anthropic Trustworthy agents](https://www.anthropic.com/research/trustworthy-agents)。

上述论文是特定实验条件下的原始研究或预印本；除官方文档描述的 Codex 机制外，所有性能数字都应被视为方向性证据，不能直接当作 SkillRoster 的验收结果。
