# Agent Skill 治理、渐进式披露与路由：证据、边界与实验路线

> 更新日期：2026-08-24
> 目的：为 SkillRoster 的产品边界和实验优先级提供决策依据。本文不是“Skill 越少越好”的论证，也不把工具调用论文直接等同于 Skill 路由证据。

## 结论先行

目前没有高质量证据支持一个跨宿主、跨模型通用的“最佳 Skill 数量”。真正可观测的问题是多段链路的组合：宿主是否发现并列出 Skill、元数据是否因预算被截短或省略、候选是否被正确检索、完整说明是否被加载、模型是否遵循说明、工具接口是否容易正确调用，以及最终任务是否成功。

因此 SkillRoster 最有价值的定位不是替模型做语义理解，而是提供本地、确定性、可审计的治理与事实层：统一盘点、保留身份和来源、计算各宿主的有效暴露面、发现重复/漂移/失效/信任风险、记录使用证据、生成可确认的分层方案，并提供 Receipt 与 Undo。语义意图、候选取舍和最终回答仍由 Agent 完成。

对已合并 PR #146 的正确解读也应保守：两个 On-demand 臂最终都找到了正确 Skill；Humanizer 臂出现 malformed `Find` 且未完成 full load，更像调用接口契约或提示遵循问题；Architecture 臂才形成干净的 Find→load 链。两个 Core 控制臂都没有通过任务 oracle，且历史 run pair 的 `formal_gate_eligible=false`，所以该实验不能证明 On-demand 优于 Core、Core 优于 On-demand，也不能证明需要新的排序或语义检索子系统。

下一步最小且能改变产品决策的动作，是先做一个“协议隔离实验”，验证 Core 控制、oracle、Find 调用和 full-load 链本身是否稳定。只有这层稳定后，才值得研究目录规模、描述重叠和 shortlist 深度。

## 证据标签与术语

本文用三类标签避免把事实、研究结果和产品判断混为一谈：

- **官方事实**：标准、宿主文档或官方源码明确描述的行为。
- **论文结论**：只在论文的模型、任务、工具集合和评测协议内成立。
- **SkillRoster 推论**：基于前两类证据提出、仍需本产品实验验证的判断。

Skill 生命周期至少应拆为下列状态，不能只用“已安装/已使用”概括：

1. **present**：文件系统中存在。
2. **discovered**：某宿主按自己的扫描规则发现。
3. **enabled**：没有被宿主或用户禁用。
4. **listed**：元数据实际进入模型可见目录；大目录下可能只列出子集。
5. **retrieved/selected**：进入当前任务候选或被模型选中。
6. **loaded**：完整 `SKILL.md` 已进入上下文。
7. **invoked**：说明或脚本被实际执行。
8. **task-successful**：最终任务满足 oracle 和安全约束。

另一个正交维度是调用方式：**implicit** 表示宿主/模型根据描述自动选择，**explicit** 表示用户或上层 Agent 明确点名。一个 Skill 可以 present、discovered、enabled，却不允许 implicit；这不代表 explicit 不可用。

治理记录也不能只保存 `name`。建议身份至少保留：

```text
(provider, host, scope, namespace, logical_path, canonical_path, declared_name)
```

其中 `logical_path` 反映宿主看到的位置，`canonical_path` 用于识别多个软链接是否指向同一实体；两者不可相互替代。

## 标准只统一包格式，宿主决定发现与暴露

### Agent Skills 开放标准

**官方事实。** [Agent Skills specification](https://agentskills.io/specification) 定义了以 `SKILL.md` 为核心的目录包、frontmatter 字段、可选脚本/引用/资源，以及渐进式加载的基本结构。[标准首页](https://agentskills.io/home) 将运行过程概括为发现元数据、激活完整说明、按需执行资源。[客户端实现指南](https://agentskills.io/client-implementation/adding-skills-support) 建议启动时把名称和描述放入目录，并估算每项约 50–100 tokens；完整说明和资源应按需加载。

标准没有统一规定宿主必须扫描哪些目录、同名 Skill 如何覆盖、是否跟随软链接、如何禁用、如何分配初始目录预算或如何判定 implicit invocation。把这些宿主策略称为“Agent Skills 标准行为”会制造虚假的可移植性。

### Codex

**官方事实。** [Codex Skills 文档](https://developers.openai.com/codex/skills) 说明：

- Codex 从仓库层级、用户目录、管理员目录和系统范围发现 Skills；同名 Skill 不会自动合并，可能同时出现。
- Skill 目录可以是软链接。
- 初始 Skill 列表最多占模型上下文的约 2%，上下文未知时上限为 8,000 字符；目录过大时 Codex 会先缩短描述，仍超限则可能省略部分 Skill 并给出警告。
- 完整 `SKILL.md` 在选择后才加载。
- `[[skills.config]]` 可按 `SKILL.md` 路径禁用 Skill。
- `agents/openai.yaml` 中 `policy.allow_implicit_invocation: false` 只禁止隐式选择，显式调用仍可用。

Codex 的 [app-server 协议文档](https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md) 还区分由调用方直接提供完整 Skill instructions，和只给 `$name` 再由模型解析的路径；后者可能增加一次解析/加载步骤。这进一步说明“被点名”不等于“完整说明已经加载”。

### Claude Code

**官方事实。** [Claude Code Skills 文档](https://code.claude.com/docs/en/skills) 定义了自己的优先级、限定命名空间、软链接去重和调用开关：项目/用户/同步来源可能有不同 precedence；嵌套同名项可通过限定名称保留；指向同一 canonical target 的多个软链接只加载一次；`disable-model-invocation: true` 可阻止自动调用但保留用户调用。它还规定 description 截断、调用后的上下文驻留与 compaction 行为。

这些是 Claude Code 的宿主策略，不应当被 SkillRoster 抹平为一个跨宿主“唯一真相”。产品应报告“某宿主的有效状态”和“跨宿主的规范化实体”两个视图。

## 核心发现

### 1. 渐进式披露降低正文成本，但没有消除目录选择问题

**官方事实。** Agent Skills 的设计把轻量元数据与完整说明/资源分开；Codex 也只在选择后加载全文。因此安装 100 个 Skill 并不等于 100 份完整说明同时占据上下文。

但第一阶段目录仍有成本。Codex 明确存在 2%/8,000 字符预算和省略行为；名称/描述又是 implicit selection 的主要依据。渐进式披露解决的是“全文何时加载”，没有自动解决：

- 目录是否完整；
- 描述是否互相重叠；
- 正确 Skill 是否进入 shortlist；
- 已选择的 Skill 是否真的 full-load；
- 模型是否按说明行动。

**SkillRoster 推论。** 产品不应只报告磁盘数量，而应优先显示每个宿主的 `discovered → enabled → listed` 有效暴露面、预算风险、描述重叠和被省略风险。这里的“风险”必须标注为宿主规则推导，不应伪装成一次真实模型失败。

### 2. “Skill 太多”不是单一变量，数量本身也不是充分诊断

**官方事实/反例。** OpenAI 在 [Codex app 发布文章](https://openai.com/index/introducing-the-codex-app/) 中描述内部使用大量 Skills。这至少反驳了“数量大必然不可用”的简单命题，但不是“数量永远无害”的受控实验。

**论文结论（间接）。** [Lost in the Middle](https://aclanthology.org/2024.tacl-1.9/) 在长上下文问答与键值检索任务中发现，相关信息的位置会影响模型使用效果。它提醒我们“信息存在于上下文”不等于“稳定可用”，但其输入是长文上下文，不是 Codex Skill 元数据目录，不能直接推出目录数量与失败率的因果关系。

**SkillRoster 推论。** 更可操作的变量是：目录字符预算、同义/近义描述密度、同名冲突、无效路径、版本漂移、来源信任、候选深度和宿主省略。数量只应作为暴露面的一个可解释指标，不宜生成一个脱离宿主和任务分布的“健康总分”。

### 3. 检索、选择、调用形状、完整加载和任务成功必须分层评测

多项工具研究支持分层，而不是用最终成功率反推所有前置环节：

- [API-Bank（EMNLP 2023）](https://aclanthology.org/2023.emnlp-main.187/) 明确拆分规划、API 检索和调用，在可运行工具环境中评测多轮使用。
- [T-Eval（ACL 2024）](https://aclanthology.org/2024.acl-long.515/) 把工具利用拆为 instruction following、planning、reasoning、retrieval、understanding 和 review。
- [ToolSandbox](https://machinelearning.apple.com/research/toolsandbox-stateful-conversational-llm-benchmark) 使用有状态执行、隐式依赖、用户模拟和动态里程碑，说明静态单轮或仅比较函数名不足以代表任务完成。
- [τ-bench（ICLR 2025）](https://openreview.net/forum?id=roNSXZpUDN) 在模拟用户、程序化 API 和领域政策中评测多轮端到端行为。
- [AgentBench（ICLR 2024）](https://proceedings.iclr.cc/paper_files/paper/2024/hash/e9df36b21ff4ee211a8b71ee8b7e9f57-Abstract-Conference.html) 覆盖八类交互环境，并将长期推理、决策和指令遵循列为主要障碍。

**论文结论。** 这些研究的共同启示是：最后一步失败可能来自检索，也可能来自调用参数、状态处理、政策遵循或任务执行。它们并未评测 SkillRoster 的目录或 Bootstrap Skill。

**SkillRoster 推论。** 评测记录应至少保留以下独立字段：

| 层 | 要回答的问题 | 推荐证据 |
|---|---|---|
| Discovery/listing | 正确 Skill 是否真的对模型可见？ | 宿主目录快照、截断/省略警告 |
| Retrieval | 正确 Skill 是否进入候选，排名多少？ | Top-k、返回 path、候选集 |
| Call-shape | Find/CLI 参数是否符合契约？ | 原始 tool call、typed error |
| Full-load | 是否读取了精确返回的 `SKILL.md`？ | load event、canonical path |
| Instruction adherence | 是否按顺序和约束执行？ | transcript、结构化事件 |
| Task oracle | 结果是否解决真实任务？ | 客观 assertion + 必要的人审 |
| Safety | 是否越界读写或绕过确认？ | filesystem diff、Receipt、policy checks |
| Core validity | 控制臂是否完成其预期协议？ | arm-specific invariants |
| Formal eligibility | 配对、指纹、时序是否允许归因？ | frozen digest、run ledger |

只有前置层可用且控制臂有效，最终差异才可能用于讨论 routing 机制。

### 4. shortlist 深度是独立变量，正确呈现不等于正确执行

**论文结论。** [ToolRet（Findings of ACL 2025）](https://aclanthology.org/2025.findings-acl.1258/) 构建了约 7,600 个多样工具检索任务和约 43,000 个工具的语料，发现传统信息检索方法在一般检索上的强弱不能直接迁移到工具检索；在其实验中，较差检索质量会降低下游任务通过率。这证明“工具检索值得单独测”，但并不证明 SkillRoster 当前的 FTS 或 Find 已经是瓶颈。

**预印本结论。** 2026 年预印本 [arXiv:2605.24660](https://arxiv.org/abs/2605.24660) 把候选列表深度 K 当作独立评测对象：K 太小可能不呈现正确工具，K 太大可能增加下游选择难度。该研究区分“正确工具是否被呈现”和“模型是否选择正确工具”；其范围不覆盖工具执行正确性，且尚是预印本，不能作为生产机制的定论。

**SkillRoster 推论。** Find 应返回紧凑、可解释、有身份和路径的候选事实，同时让上层 Agent决定语义取舍。若要改变默认 Top-k，必须同时观察 correct-in-K、selected-correct、full-load 和 task-success，不能只优化检索指标。

### 5. 接口设计可能比新排名算法更先影响结果

**论文结论。** [SWE-agent（NeurIPS 2024）](https://proceedings.neurips.cc/paper_files/paper/2024/hash/5a7c947568c1b1328ccc5230172e1e7c-Abstract-Conference.html) 研究 Agent-Computer Interface，表明面向 Agent 设计的简洁命令和反馈能显著改变软件工程任务行为。Anthropic 的 [Writing tools for agents](https://www.anthropic.com/engineering/writing-tools-for-agents) 将工具描述为确定性系统与非确定性 Agent 之间的契约，强调清晰范围、可操作返回和基于评测迭代。

**SkillRoster 推论。** 当模型调用 `Find` 参数不合法，优先检查参数 schema、错误消息、示例、完整 TASK 的保真传递和 retry 合同，而不是立刻加入 embedding、reranker 或额外路由层。一个窄而稳定的 Agent-facing CLI 往往比多个互相重叠的治理 Skills 更符合产品方向。

### 6. oracle 过严会把可接受结果误判为系统失败

**官方事实。** [Agent Skills 评测指南](https://agentskills.io/skill-creation/evaluating-skills) 建议使用客观 assertions，但明确反对依赖精确措辞等脆弱判断；主观质量应由人工复核。它还要求同一 prompt 在隔离上下文中做 with/without 对照，并运行多次观察波动。Anthropic 的 [Agent evals 工程指南](https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents) 同样建议组合代码 grader、模型 grader 和人审，并检查 transcript，而不是只看一个最终布尔值。

**SkillRoster 推论。** PR #146 Humanizer Core 只差一个字符的最低长度门槛，不足以单独说明 Skill 或路由失败。应检查这个阈值是否对应真实用户价值；机械约束可以严格，但文风、覆盖度等应使用结构化 rubric 和盲审。失败若在 Core/On-demand 两臂共同出现，更可能是 task/oracle/Skill 内容问题，而非路由差异。

### 7. 安装器解决分发，治理还需要证据与可逆变更

**官方开源事实。** Vercel 的 [skills CLI](https://github.com/vercel-labs/skills/blob/main/README.md) 支持面向多个 Agent 的项目级/全局安装、列出、移除和更新，并优先用 canonical copy + symlink、必要时复制。其 [skill-lock 实现](https://github.com/vercel-labs/skills/blob/main/src/skill-lock.ts) 记录来源、ref、路径、hash 和更新时间。

这类工具很好地解决“从哪里获取、安装到哪里、如何更新”，但不自动回答哪些 Skill 实际被各宿主发现/列出、是否互相重叠、是否长期不用、是否安全降级到 On-demand，或治理变更如何按 Agent 回滚。

**SkillRoster 推论。** SkillRoster 应与安装器互补：尊重现有来源与 lock metadata，通过 logical/canonical identity 识别共享副本；任何统一 Library、软链接迁移、分层或归档都先生成 Plan，用户一次确认后 Apply，并用 Receipt/Undo 可逆。不要通过静默接管来源来制造“整洁”。

## 研究基准能证明什么、不能证明什么

| 来源 | 直接评测对象 | 可用于本项目的启示 | 不可直接外推 |
|---|---|---|---|
| [ToolLLM / ToolBench（ICLR 2024）](https://proceedings.iclr.cc/paper_files/paper/2024/hash/28e50ee5b72e90b50e7196fde8ea260e-Abstract-Conference.html) | 16,000+ API、指令生成、检索和工具调用 | 大工具目录需要候选检索；检索可独立训练/测量 | API 工具不是 Skill；不能推出某个 Skill 数量阈值 |
| [API-Bank](https://aclanthology.org/2023.emnlp-main.187/) | 73 个可执行工具、314 段对话/753 次调用；另有大规模训练集 | 规划、检索、调用要分层 | 不代表文件系统发现、full-load 或治理 |
| [ToolRet](https://aclanthology.org/2025.findings-acl.1258/) | 7.6k 检索任务、43k 工具 | 工具检索不同于普通 IR；检索质量可能影响下游 | 不能证明 SkillRoster Find 当前失败或需要神经检索 |
| [ToolSandbox](https://machinelearning.apple.com/research/toolsandbox-stateful-conversational-llm-benchmark) | 有状态、多轮、隐式依赖和动态里程碑 | 端到端任务不能用静态函数名准确率替代 | 不评测 Skill 分层或共享 Library |
| [τ-bench](https://openreview.net/forum?id=roNSXZpUDN) | 用户交互、领域政策、程序化 API | task-success 和政策遵循需放在真实交互中 | 不提供本地 Skill usage 的因果证据 |
| [AgentBench](https://proceedings.iclr.cc/paper_files/paper/2024/hash/e9df36b21ff4ee211a8b71ee8b7e9f57-Abstract-Conference.html) | 八种交互环境 | 指令遵循/长期决策可独立成为瓶颈 | 不能把所有失败归咎于目录过载 |
| [SWE-agent](https://proceedings.neurips.cc/paper_files/paper/2024/hash/5a7c947568c1b1328ccc5230172e1e7c-Abstract-Conference.html) | 软件工程 Agent 的 ACI | CLI 命令、反馈和观察预算会改变结果 | 不证明某个检索算法优越 |
| [T-Eval](https://aclanthology.org/2024.acl-long.515/) | 工具利用能力分解 | 支持分层 ledger，而非单一 pass/fail | 不覆盖 Skill 来源、软链接和生命周期 |
| [Lost in the Middle](https://aclanthology.org/2024.tacl-1.9/) | 长上下文信息使用 | “存在”不等于稳定使用 | 长文位置效应不能直接等同于 Skill 列表效应 |
| [SkillsBench（2026 预印本）](https://arxiv.org/abs/2602.12670) | 87 个任务、18 个模型-宿主配置、curated Skills | 报告 Skills 平均增益，且聚焦模块优于更大说明集 | 预印本；任务/宿主有限，不能给出通用数量上限 |
| [Skills in the Wild（2026 预印本）](https://arxiv.org/abs/2604.04323) | 34k 真实世界 Skills 的评测 | 相关性、质量和现实上下文会改变收益 | 预印本；不能把相关关系写成安装量导致失败 |

总体上，工具论文为“分层测量”和“接口/候选集是独立变量”提供了强依据；对 SkillRoster 的具体机制选择仍必须用本产品、真实宿主和冻结协议验证。

## PR #146：事实映射与禁止归因

本节依据仓库内的 [验收说明](../acceptance/codex-luna-transfer-gate-v1.md) 和 [结构化 artifact](../acceptance/artifacts/codex-luna-transfer-gate-v1.json)。它是一次失败实验的审计，不是对模型能力的总评。

| 证据层 | Humanizer Core | Humanizer On-demand | Architecture Core | Architecture On-demand | 能得出的结论 |
|---|---|---|---|---|---|
| Retrieval | 不适用 | 最终 Top1/path 正确；3 次 Find | 不适用 | 首次 Find 即 Top1/path 正确 | 两个 On-demand 最终都找对；不能说检索召回失败 |
| Call-shape | 不适用 | 出现参数契约错误后恢复 | 不适用 | 干净调用 | Humanizer 更像接口/遵循摩擦；样本不足以归因模型或机制 |
| Full-load/order | 未达到有效控制要求 | 未完成 | 满足 | 满足 | 只有 Architecture On-demand 形成干净 Find→load 链 |
| Task oracle | 未通过；包含仅差 1 字符的门槛 | 通过 | 未通过 | 未通过相同内容约束 | 共享失败优先检查 Skill/oracle；1 字符门槛需审查价值 |
| Safety/workspace | 通过 | 通过 | 通过 | 通过 | 本次未见安全越界；不能证明所有路径安全 |
| Core validity | 无效 | — | 无效 | — | 两个对照臂都不能支持有效成对归因 |
| Formal eligibility | 配对历史 digest 在 invocation 后冻结，false | 同左 | 同左 | 同左 | 旧 run 不满足正式门槛；future driver 修复不追溯改变历史证据 |

明确不能从 #146 得出：

- On-demand 比 Core 更好或更差；
- Skill 数量造成任务失败；
- FTS/Find 排名需要替换；
- 需要内置 embedding、模型路由器或更复杂 Skill 图；
- 两次最终成功足以证明协议稳定。

它真正暴露的是评测基础设施和 Agent 接口的优先问题：控制臂有效性、调用契约、full-load 可观测性、非脆弱 oracle、配对时序与不可变 ledger。

## 产品边界

### SkillRoster 应该负责

1. **盘点与规范化**：扫描多个宿主的 present/discovered/enabled/listed 状态，保留 provider、scope、namespace、logical/canonical path。
2. **事实型诊断**：同名/同源/内容重复、破损软链接、版本漂移、来源不明、宿主预算与潜在省略、默认暴露面和跨 Agent 复制。
3. **保守 usage evidence**：区分明确调用、读取、命令痕迹和仅被发现；证据不足时标 `unknown`，不能把“没看到”写成“从未使用”。
4. **Agent-facing Find**：输出有界、结构化、可解释候选，包括名称、description、provider、路径、scope 和匹配原因；让模型完成语义判断。
5. **治理建议**：Core / On-demand / Explicit-only / Archived 分层，按宿主生成 roster；建议必须附证据和不确定性。
6. **安全执行**：不可变 Plan、一次显式确认、Apply、Receipt、Undo、漂移检测和恢复；不读未信任目标，不静默覆盖用户来源。
7. **评测 ledger**：分开记录 listing、retrieval、call-shape、load、oracle、safety、Core validity 和 formal eligibility。

### SkillRoster 不应该负责

- 不内置一个替 Agent 理解用户意图的通用语义模型；上层模型已经更适合综合当前对话。
- 不因为“数量大”自动归档或删除；数量不是充分证据。
- 不把不同宿主的 precedence、namespace、symlink 和 enablement 伪装成统一标准。
- 不用单一健康分数遮蔽证据，也不把弱 usage 证据升级成确定结论。
- 当前不需要 MCP、daemon、云同步、TUI、HTML 仪表盘、插件 SDK 或自动联网 telemetry。
- 不在没有稳定控制臂和任务 oracle 时，用昂贵 benchmark 推动新检索子系统。

一个 Bootstrap Skill 可以作为“如何调用 SkillRoster”的窄入口，但应保持单一职责：精确保留 TASK、调用 Find、读取返回的精确路径、在 typed error 下有限重试。治理功能本身继续收敛在 CLI，而不是拆成许多互相竞争的 Skills。

## 下一步实验：按是否能改变决策排序

### 实验 1：协议隔离与控制臂有效性（现在做）

**问题。** 失败究竟来自路由、Find 调用、full-load、Skill 内容还是 oracle？

**设计。** 选择一个内容稳定、oracle 可机械验证且不会受文风主观性影响的任务族；冻结 Codex 版本、Luna 模型、system prompt、Bootstrap Skill、目标 Skill、工作区 fixture、超时、温度/推理配置和评分器。Core 与 On-demand 每臂至少 3 个全新隔离 trial；不重跑失败样本来挑结果。保留完整 transcript 和分层 ledger。

评分器应只对真实产品不变量严格：文件是否创建、字段是否存在、是否越界、返回路径是否被读取。表达质量用 rubric + 盲审，不用精确句子或任意字符阈值。

**停止条件。**

- Core 少于 3/3 协议有效：停止比较，先修 task/oracle/harness；不得做产品归因。
- Core 有效，但 On-demand 至少 2/3 malformed Find 或未 load：只改 Bootstrap/CLI schema、错误返回或示例；不改排名器。
- 两臂 route/load 都稳定但共同 fail oracle：修目标 Skill 或 oracle；不归因路由。
- 两臂均稳定且任务通过：停止增加机制，保留现状。
- 只有在控制有效且差异能跨 trial 重现时，才进入实验 2。

### 实验 2：目录规模 × 描述重叠 × shortlist K（条件触发）

**问题。** 是“数量”、目录预算、省略、描述重叠还是候选深度影响选择？

**设计。** 用同一组任务做因子实验：目录规模（如 5/20/50/100）、描述重叠（低/高）、K（小/中/大）；控制正确 Skill 的描述长度、位置、provider 和任务难度。每次记录实际 `listed` 集合和宿主截短/省略，而不是假定所有 present Skills 都可见。

指标依次为：correct-listed、correct-in-K、selected-correct、full-load、task-success、tokens、latency。报告每个 trial，不只给均值。

**停止条件。** 若影响完全由 Codex 明示的目录省略解释，优先做暴露治理而非语义检索；若不同规模下无稳定单调效应，不设全局数量阈值；若 correct-in-K 高但选择/执行低，停止优化召回，转向描述/接口或 Skill 内容。

### 实验 3：真实任务的 with/without 治理闭环（发布前门槛）

**问题。** 分层后是否保持或提高真实任务成功，同时降低默认暴露和维护成本？

按照 [Agent Skills 评测指南](https://agentskills.io/skill-creation/evaluating-skills) 使用真实 prompt、隔离上下文、相同任务的治理前/后对照、多次 trial、客观 assertions 与人工复核。覆盖小目录、大目录、跨 Agent、同名冲突、软链接共享、损坏路径和 Undo。

治理价值需要同时报告：默认暴露减少、重复/损坏修复、任务成功差异、错误选择、Find/load 成功率、tokens/latency、Apply/Undo 完整性。不能用“暴露减少”替代任务不回归，也不能用一次任务通过替代安全可逆性。

**停止条件。** 任一宿主出现不可逆修改、来源丢失或 Undo 不完整即停止发布；任务结果没有稳定差异时，只主张可审计治理价值，不宣传性能或智能提升。

## 当前决策表

| 决策 | 现在 | 触发重新评估的证据 |
|---|---|---|
| 保持 Agent-first CLI + 一个 Bootstrap Skill | 是 | 协议隔离实验显示该入口反复且不可修复地失败 |
| 继续 Core / On-demand / Explicit-only / Archived 分层 | 是，但建议需附证据和确认 | 真实任务显示分层导致稳定回归 |
| 增加 embedding/reranker | 否 | 控制有效、correct-listed 高、词法 Find 的 correct-in-K 稳定不足，且语义方法提升端到端 |
| 设置统一 Core 数量上限 | 否 | 多宿主因子实验给出稳定阈值并能跨任务复现 |
| 自动归档“未使用”Skill | 否 | 有覆盖足够时间和宿主的高置信 invocation 证据，并仍需用户确认 |
| 做云端同步/MCP/daemon/TUI | 否 | 出现本地 CLI 无法解决且被多用户重复验证的需求 |
| 扩大昂贵端到端 benchmark | 暂缓 | 实验 1 的控制、oracle、formal eligibility 全部稳定 |

## 限制与反例

1. 工具、API 和 Skill 都涉及候选选择，但 Skill 还包含长篇程序性说明、脚本、文件系统发现和宿主 precedence；工具论文只能提供评测结构，不能直接决定产品机制。
2. Codex、Claude Code 的文档会迭代。SkillRoster 应把宿主规则版本化，并通过 fixture 验证，不能永久硬编码一份观察。
3. 本地会话日志只能证明被观察到的行为；日志缺失、保留期或其他 Agent 的不可见历史都会造成假“未使用”。
4. 大量高质量、低重叠 Skill 可以通过渐进加载良好运作；少量重叠、错误或恶意 Skill 也可能造成严重问题。计数不是风险的充分统计量。
5. [Anthropic Advanced Tool Use](https://www.anthropic.com/engineering/advanced-tool-use) 展示了在大工具集合中按需检索工具定义的可行性，但这是特定工具搜索实现和模型的结果，不是 SkillRoster 必须复制其架构的证据。
6. OpenAI 的 [第三方评测可信度原则](https://openai.com/index/trustworthy-third-party-evaluations-foundations/) 强调系统版本、harness、预算、重试和评分范围都会影响结论。PR #146 将 formal eligibility 单列是正确方向；未来修复 driver 不能追溯“洗绿”历史 run。

## 最终建议

把当前路线概括为一句话：**SkillRoster 管理事实、暴露面和可逆变更；Agent 管理语义与意图；实验负责证明两者的接口是否稳定。**

下一步不要扩展功能面。先完成 3×2 个全新 trial 的协议隔离实验，并严格执行停止条件。若问题落在 malformed Find/full-load，就做一次窄接口修复；若落在共同 oracle，就修评测；若两臂稳定，就停止优化。只有当这些基础门槛全部成立，目录规模和检索机制的实验才有能力改变产品决策。
