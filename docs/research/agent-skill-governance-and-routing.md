# Agent Skill 治理、渐进式披露与路由：证据、边界与实验路线

> 更新日期：2026-08-24
> 目的：为 SkillRoster 的产品边界和实验优先级提供决策依据。本文不是“Skill 越少越好”的论证，也不把工具调用论文直接等同于 Skill 路由证据。

## 结论先行

目前没有高质量证据支持一个跨宿主、跨模型通用的“最佳 Skill 数量”。真正可观测的问题是多段链路的组合：宿主是否发现并列出 Skill、元数据是否因预算被截短或省略、候选是否被正确检索、完整说明是否被加载、模型是否遵循说明、工具接口是否容易正确调用，以及最终任务是否成功。

因此 SkillRoster 最有价值的定位不是替模型做语义理解，而是提供本地、确定性、可审计的治理与事实层：统一盘点、保留身份和来源、计算各宿主的有效暴露面、发现重复/漂移/失效/信任风险、记录使用证据、生成可确认的分层方案，并提供 Receipt 与 Undo。语义意图、候选取舍和最终回答仍由 Agent 完成。

对已合并 PR #146 的正确解读也应保守：两个 On-demand 臂最终都找到了正确 Skill；Humanizer 臂出现 malformed `Find` 且未完成 full load，更像调用接口契约或提示遵循问题；Architecture 臂才形成干净的 Find→load 链。两个 Core 控制臂都没有通过任务 oracle，且历史 run pair 的 `formal_gate_eligible=false`，所以该实验不能证明 On-demand 优于 Core、Core 优于 On-demand，也不能证明需要新的排序或语义检索子系统。后续 PR #148 已完成正式协议隔离：Core 3/3，On-demand 1/3；但三个 On-demand trial 的 retrieval、Top-1、精确路径、完整加载、任务 oracle 和安全性均为 3/3，两个拒绝仅来自 Find→load 之间的额外动作和不安全 compound shell shape。

Issue #149 已通过 PR #150 落地并在冻结协议中达到 Core 3/3、On-demand 3/3。后续真实本机 dogfood 暴露了更窄的剩余缺口：同名异内容 Top-1 正确地 fail-closed，但 Agent 只能看到身份和路径，不能经同一可信接口加载它明确选择的变体。当前动作因此是 Issue #151：允许 Agent 只从已排名 Top-1 组中精确加载一个身份用于语义比较，同时继续禁止任意目录跳转、隐式 canonicalization 和治理变更。

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

## Issue #149：一次 Find + 可信完整加载契约

### 是否符合 Agent harness

**官方事实。** Agent Skills 的[客户端实现指南](https://agentskills.io/client-implementation/adding-skills-support) 把专用 activation tool 列为正式实现模式：模型选择 Skill 后，工具可在一次调用中返回完整 instructions、结构化身份标签、Skill 目录和资源清单，并在返回前执行权限检查。指南还明确说，大多数实现把相关性判断留给模型，而不是在 harness 中做关键词触发；专用工具的价值是控制返回内容、施加权限、记录 activation 和避免模型虚构 Skill 名称。

**官方事实。** OpenAI [Agents SDK Function tools](https://openai.github.io/openai-agents-python/tools/) 支持由 JSON Schema 约束输入，并从本地 runtime 返回文本或结构化 tool output。其工具生命周期保留 schema validation、guardrails、timeout、failure handling 和 tracing。Codex 当前也能从本地 shell/exec 工具接收进程输出。因此“一次 Agent tool call 得到完整 Skill bytes”符合常见 harness 的 request→tool result 模型，不要求 MCP、daemon 或内置模型。

**SkillRoster 推论。** #149 应是现有 `find` 的窄扩展，例如显式 `load` mode，而不是第二个路由子系统。这里的“一次”定义为：Agent 发出一个 SkillRoster 调用，并在该调用的单个成功结果中同时取得选中事实和完整、已验证的 `SKILL.md`；不需要再调用文件读取、打开 workspace、使用 MCP 或创建临时 TASK 文件。

“一次调用”不是跨 SQLite 和文件系统的数学事务。准确承诺应是**观察原子性**：一个 CLI 进程要么返回一份内部一致的成功 envelope，要么返回 typed failure；不得返回截断 instructions、部分成功或“路径可用，请自行再读”。

### 一次操作内的确定性边界

推荐顺序如下，全部在同一进程、同一结果边界内完成：

1. 接收完整自然语言 TASK 和可选 HINT；对输入做类型、长度和编码检查，但不改写、翻译或总结。
2. 在一致的 roster/snapshot 视图上运行现有确定性 ranking，只考虑当前 Agent 可用且允许路由的 placement。
3. 固定 Top-1 的 identity、logical path、canonical path、expected digest、roster state 和 source baseline。
4. 在读取前验证 placement 未 archived/disabled，路径可读、是 regular file、没有越出受信 roots，来源状态满足现有信任策略。
5. 从已验证目标读取**完整原始 bytes**，同时执行硬 byte cap；按真实返回 bytes 计算 digest。读取后复核路径/元数据/expected digest，任何不一致都判 drift。
6. 解析 UTF-8 和必要的 Agent Skills frontmatter；返回完整 `SKILL.md`，不提前读取 `references/`、`scripts/` 或 `assets/`。
7. 只在上述全部成立时返回 `ok: true`；成功 envelope 将 retrieval evidence、loaded-content identity 和 governance facts 分开。

这是一种产品设计推论，不是 Agent Skills 标准强制的算法。标准只规定 `SKILL.md` 格式，并明确 activation 时会加载整个文件；它没有规定 path trust、digest、drift、roster 或 byte cap。

### CLI 必须验证的事实

| 事实 | CLI 可验证内容 | 成功结果应提供的证据 |
|---|---|---|
| 请求完整性 | CLI 实际收到的 TASK/HINT bytes、编码、长度 | request digest、byte count；不得声称等同于原始用户消息 |
| 检索 | ranking strategy、query/hint、候选数、Top-1、score/match basis | retrieval block，与内容加载分离 |
| 治理资格 | Agent、placement、roster state、implicit/explicit eligibility、snapshot | stable IDs、state、snapshot/evidence ID |
| 路径 | logical path、canonical path、symlink resolution、allowed-root containment、regular file | 两种路径及 containment decision |
| 来源信任 | source/manager/provider、已确认 root、first-observed baseline、当前 policy state | trust **evidence/state**，而不是笼统 `trusted: true` |
| 漂移 | expected digest/identity 与本次读取 bytes、metadata 的一致性 | expected/observed content digest、size、checked-at |
| 内容完整性 | 全量读取、byte cap、UTF-8、frontmatter、name/path consistency | `content_complete: true`、raw-byte digest、byte/line count |
| 副作用 | 操作是只读的，未创建 Plan/Receipt、未改 roster/Skill/workspace | operation/read-only marker；协议 suite 再用文件 diff 验证 |

两个边界尤其重要：

- CLI 只能证明“它收到的 TASK 是什么”，不能证明上层 shell harness 没有在 argv 构造时丢字符、展开变量或改变引号。若 harness 只有 shell-string 而没有原生 argv/stdin 参数，任意自然语言的安全传输仍是 harness 契约问题。Bootstrap 应给出一个 canonical shape，正式 suite 必须验证 TASK digest；不能让 CLI 宣称自己证明了调用前历史。
- digest 证明本次 bytes 的完整性与某 baseline 是否一致，不证明内容善意。Agent Skills frontmatter 的 `author`、`version` 属于自声明 metadata；“来源已确认”也不等于“说明无恶意”。执行安全仍由 sandbox、审批和 Agent policy 控制。

### 留给模型的语义

模型继续负责：

- 从完整用户任务产生可选、忠实的 capability HINT；
- 判断返回的 Top-1 是否与当前任务语义相关；
- 阅读 instructions，并决定何时按需读取其引用资源；
- 综合对话、用户偏好和任务状态执行 Skill；
- 判断最终答案是否解决用户问题。

CLI 不应负责翻译/总结 TASK、生成语义 hint、用 LLM 重排、判断最终任务成功，或把一个低词法分数包装成“语义正确”。如果现有确定性策略返回 empty、wrong-domain evidence 或现有 ambiguity signal，保持 typed branch，让 Agent 安全重试一次；不要用隐藏模型填空。

### fail-closed 的精确定义

`fail-closed` 应满足四个可测试条件：

1. 任何资格、路径、信任、漂移、读取、编码、格式或大小检查失败，整体 `ok: false`。
2. error envelope 不包含部分 `SKILL.md`、文件前缀或可被误当作 instructions 的正文。
3. error 使用稳定 code 和机器可读 details，保留 candidate identity 与非敏感诊断；`suggested_actions` 只是下一步选项，不是授权，也不能在同一失败调用中自动执行。
4. 安全 retry 修复失败原因，而非绕过检查：oversized → 将细节拆到按需 references；drifted → rescan/reconcile；untrusted → 走显式 source confirmation/adopt Plan；archived/disabled → 由用户明确改变 roster 或显式调用策略；path escape/unreadable → 修路径或权限。不要提供通用 `--force`。

建议 typed codes 至少覆盖现有语义中的：`no_match`、`ambiguous`、`placement_ineligible`、`archived`、`source_untrusted`、`path_escape`、`broken_link`、`not_regular_file`、`unreadable`、`content_drifted`、`content_too_large`、`invalid_utf8`、`invalid_skill_format`。最终命名应复用仓库现有 error taxonomy，避免为 #149 建第二套错误系统。

### 内容上限与响应形状

**官方事实。** [Agent Skills specification](https://agentskills.io/specification) 对 `name`（64 chars）、`description`（1,024 chars）等 frontmatter 有限制，但没有给 `SKILL.md` body 设硬上限；它只提醒 activation 会加载整个文件，较长内容应拆到 references。客户端指南也建议 resources 只列出、不要 eager-load。

**SkillRoster 推论。** 因此 byte cap 是 SkillRoster/harness 的安全运输约束，不应伪装成标准限制，也不应随模型 tokenizer 变化。实现应使用版本化、文档化的原始 byte 上限：在分配/读取前检查 metadata size，并在 streaming read 时再次硬限制，以防 metadata 漂移或特殊文件；超过上限整单失败。上限数值应由当前 CLI JSON envelope、Codex tool-output 截断行为和真实 Skill 分布的 fixture 测试决定，不应从论文或猜测得出。

**本机测量（2026-08-24）。** 对最新 SkillRoster Snapshot 的 882 个可读 placement 统计原始 `SKILL.md` 大小：中位数 6,187 bytes，P90 15,311，P95 23,717，P99 56,848，最大 87,253；placement 包含跨 Agent 软链接重复，因此这不是生态总体分布。首版 load transport cap 取 128 KiB，可覆盖当前最大样本并保留约 50% 余量，同时远小于 inventory parser 的 2 MiB 上限。这个数值是本地运输决策，不是 Agent Skills 标准限制；未来只能由真实宿主截断证据和用户 Skill 分布调整。

成功响应至少需要以下逻辑分区，字段可按现有 schema 命名：

```json
{
  "retrieval": {
    "strategy": "existing-deterministic-strategy",
    "rank": 1,
    "candidate_count": 1,
    "match_basis": []
  },
  "skill": {
    "skill_id": "...",
    "placement_id": "...",
    "name": "...",
    "provider": "...",
    "logical_path": ".../SKILL.md",
    "canonical_path": ".../SKILL.md"
  },
  "governance": {
    "agent": "codex",
    "roster_state": "on_demand",
    "source_trust_state": "confirmed",
    "snapshot_id": "..."
  },
  "loaded_content": {
    "content": "---\n...complete raw UTF-8 SKILL.md...",
    "content_complete": true,
    "content_bytes": 1234,
    "content_digest": "sha256:..."
  }
}
```

digest 必须针对 JSON 解码后的原始 UTF-8 file bytes，而不是转义后的 JSON 文本。不要在成功 envelope 中声称 `task_success`、`skill_safe` 或 `semantic_match_confirmed`。

### 对 #149 的验收推论

已有[协议隔离 v6](../acceptance/codex-skill-protocol-isolation-v6.md) 是正式 eligible 证据：Core 3/3、On-demand 1/3；三个 On-demand 的 ranking、load、task 和 safety 实际均通过，两个失败只在 ordered Agent contract。因此 #149 的验收重点应是缩短协议，而不是改排名：

- 兼容现有 `find`；load mode 是显式 opt-in。
- 一条 canonical invocation 返回 Top-1 + 完整内容；不允许中间 workspace/MCP/read 动作。
- 对 oversized、archived、untrusted、unreadable、escape、drift 分别做 focused core tests，断言无部分 content。
- 使用完整冻结的 3×2 Codex suite；Core 3/3、On-demand 3/3、formal eligibility、任务和 safety 都是独立门槛。
- 若 On-demand 仍因 shell argv shape 失败，停止继续堆 CLI 逻辑，确认是否需要 harness-native argv/stdin 支持；这不是 embedding 或 ranking 问题。
- 若 one-call 全通过，停止扩展 activation 机制，不新增第二个 routing Skill。

## Issue #151：同名内容比较与最新研究校准

**真实产品证据。** PR #150 之后的本机只读 dogfood 中，`humanizer-zh`
与 `agent-session-miner` 都被正确检索为 Top-1，但各自存在两个异内容身份。
普通 `find --load` 的 `same_name_variants_ambiguous` 是正确安全行为；缺口是
Agent 无法通过 CLI 精确读取它已看到的某个身份，只能退回原始文件系统读取。
这会打断“事实由 CLI 提供，语义由模型判断”的边界。

**官方事实。** OpenAI 的 [Codex app 文章](https://openai.com/index/introducing-the-codex-app/)
说明 Skills 可由 Agent 自动选择或由用户显式指定，并披露 OpenAI 内部已构建数百个
Skills。这进一步否定了“数量本身就是故障”的简单结论，同时强化了可发现、可管理、
可显式选择三种能力必须共存。Anthropic 的
[Agent Skills 工程文章](https://www.anthropic.com/engineering/equipping-agents-for-the-real-world-with-agent-skills)
把渐进式披露、从代表性失败开始评测、观察真实轨迹再增量修改列为核心实践；#151
正是一次由真实轨迹触发的窄接口修复，而不是预设新子系统。

**论文结论。** [SkillRouter（2026 预印本）](https://arxiv.org/abs/2603.22455)
在约 8 万个 Skill、75 个专家查询上报告：同名/相近 Skill 的完整正文对重排具有关键
作用，移除正文会显著降低其实验准确率。其查询规模较小、方法包含神经检索，不能证明
SkillRoster 应加入 embedding 或 reranker；但它支持一个更窄的产品判断：当元数据已
暴露真实歧义时，应让上层模型安全取得候选正文再做语义比较，而不是由确定性 CLI
猜测 canonical content。[GoSkills（2026 预印本）](https://arxiv.org/abs/2605.06978)
和 [SkillWeaver（2026 预印本）](https://arxiv.org/abs/2606.18051) 分别探索结构化候选组
与多 Skill 组合。它们提示未来评测可能需要覆盖组合任务，但目前没有证据支持把图、
分解器或规划器加入 SkillRoster。

**SkillRoster 推论。** `--variant-skill-id` 只能选择当前 Top-1 组已暴露的稳定身份，
并复用 #149 的路径、来源、digest、UTF-8、大小和 Archived 检查。成功响应同时说明
“排名组”和“精确加载身份”；CLI 不比较语义、不推荐 canonical、不生成 Plan。

## 产品边界

### SkillRoster 应该负责

1. **盘点与规范化**：扫描多个宿主的 present/discovered/enabled/listed 状态，保留 provider、scope、namespace、logical/canonical path。
2. **事实型诊断**：同名/同源/内容重复、破损软链接、版本漂移、来源不明、宿主预算与潜在省略、默认暴露面和跨 Agent 复制。
3. **保守 usage evidence**：区分明确调用、读取、命令痕迹和仅被发现；证据不足时标 `unknown`，不能把“没看到”写成“从未使用”。
4. **Agent-facing Find/Load**：默认 Find 输出有界、结构化、可解释候选；显式 load mode 可在同一只读调用中验证并返回完整 Top-1 `SKILL.md`，或从当前同名 Top-1 组精确加载一个已暴露身份用于比较。两者都让模型完成语义判断。
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

一个 Bootstrap Skill 可以作为“如何调用 SkillRoster”的窄入口，但应保持单一职责：精确保留 TASK、用 canonical shape 调用 Find/Load、验证 envelope，并在 typed error 下有限重试。治理功能本身继续收敛在 CLI，而不是拆成许多互相竞争的 Skills。

## 下一步实验：按是否能改变决策排序

### 实验 1：协议隔离与控制臂有效性（已完成）

**问题。** 失败究竟来自路由、Find 调用、full-load、Skill 内容还是 oracle？

**设计。** 选择一个内容稳定、oracle 可机械验证且不会受文风主观性影响的任务族；冻结 Codex 版本、Luna 模型、system prompt、Bootstrap Skill、目标 Skill、工作区 fixture、超时、温度/推理配置和评分器。Core 与 On-demand 每臂至少 3 个全新隔离 trial；不重跑失败样本来挑结果。保留完整 transcript 和分层 ledger。

评分器应只对真实产品不变量严格：文件是否创建、字段是否存在、是否越界、返回路径是否被读取。表达质量用 rubric + 盲审，不用精确句子或任意字符阈值。

PR #148 的 v6 suite 已按该设计形成正式 eligible 证据：Core 3/3，On-demand 1/3；后者的 retrieval、full-load、task oracle 和 safety 均为 3/3，失败集中在 ordered contract。因此已经触发下列原定停止条件中的“只修 Bootstrap/CLI seam，不改排名器”。历史设计和停止条件保留如下，作为决策审计：

- Core 少于 3/3 协议有效：停止比较，先修 task/oracle/harness；不得做产品归因。
- Core 有效，但 On-demand 至少 2/3 发生调用/加载协议失败：只改 Bootstrap/CLI schema、错误返回或示例；不改排名器。v6 正是这一分支。
- 两臂 route/load 都稳定但共同 fail oracle：修目标 Skill 或 oracle；不归因路由。
- 两臂均稳定且任务通过：停止增加机制，保留现状。
- 只有在 #149 one-call 验收稳定后，才考虑进入实验 2。

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
| 保持 Agent-first CLI + 一个 Bootstrap Skill | 是；#149 one-call 已稳定，#151 只补同名精确比较 | exact variant load 仍迫使 Agent 绕过 CLI 或削弱安全边界 |
| 继续 Core / On-demand / Explicit-only / Archived 分层 | 是，但建议需附证据和确认 | 真实任务显示分层导致稳定回归 |
| 增加 embedding/reranker | 否 | 控制有效、correct-listed 高、词法 Find 的 correct-in-K 稳定不足，且语义方法提升端到端 |
| 设置统一 Core 数量上限 | 否 | 多宿主因子实验给出稳定阈值并能跨任务复现 |
| 自动归档“未使用”Skill | 否 | 有覆盖足够时间和宿主的高置信 invocation 证据，并仍需用户确认 |
| 做云端同步/MCP/daemon/TUI | 否 | 出现本地 CLI 无法解决且被多用户重复验证的需求 |
| 扩大目录规模 benchmark | 可进入下一独立研究轮，但不阻塞 #151 | 先冻结任务分布、宿主目录事实和可归因协议 |

## 限制与反例

1. 工具、API 和 Skill 都涉及候选选择，但 Skill 还包含长篇程序性说明、脚本、文件系统发现和宿主 precedence；工具论文只能提供评测结构，不能直接决定产品机制。
2. Codex、Claude Code 的文档会迭代。SkillRoster 应把宿主规则版本化，并通过 fixture 验证，不能永久硬编码一份观察。
3. 本地会话日志只能证明被观察到的行为；日志缺失、保留期或其他 Agent 的不可见历史都会造成假“未使用”。
4. 大量高质量、低重叠 Skill 可以通过渐进加载良好运作；少量重叠、错误或恶意 Skill 也可能造成严重问题。计数不是风险的充分统计量。
5. [Anthropic Advanced Tool Use](https://www.anthropic.com/engineering/advanced-tool-use) 展示了在大工具集合中按需检索工具定义的可行性，但这是特定工具搜索实现和模型的结果，不是 SkillRoster 必须复制其架构的证据。
6. OpenAI 的 [第三方评测可信度原则](https://openai.com/index/trustworthy-third-party-evaluations-foundations/) 强调系统版本、harness、预算、重试和评分范围都会影响结论。PR #146 将 formal eligibility 单列是正确方向；未来修复 driver 不能追溯“洗绿”历史 run。

## 最终建议

把当前路线概括为一句话：**SkillRoster 管理事实、暴露面和可逆变更；Agent 管理语义与意图；实验负责证明两者的接口是否稳定。**

下一步仍不扩展功能面。完成 Issue #151 的窄 exact-variant load：Agent 只能从
当前 Top-1 同名组选择一个已暴露身份，CLI 复用现有可信完整加载边界，模型读取完整
入口说明后自行比较。真实 `humanizer-zh` 与 `agent-session-miner` 的双方入口 digest
实际上相同，差异仅来自包内其他文件；因此 #151 能证明入口等价，却不能解决包级
canonical 选择。该观察应另行修复指纹噪声，而不能把两份相同说明说成语义差异。继续用这两个冲突族验证
blocked→inspect→exact load 闭环；若任意 selector 可越过排名组、Archived、source、
path 或 digest 边界，立即停止合并。该闭环稳定后，再把“目录规模 × 描述重叠 × K”
作为独立研究实验，而不是把预印本结果直接实现成 embedding、图或内置模型。
