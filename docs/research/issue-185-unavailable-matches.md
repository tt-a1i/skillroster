# Issue #185：保留语义证据但当前 Skill 不可读时的诊断性 `unavailable_matches`

> Issue：[Investigate typed unavailable Find matches from retained semantic evidence](https://github.com/tt-a1i/skillroster/issues/185)
> 日期：2026-08-25
> 状态：研究结论；未修改生产实现，仅增加安全回归测试
> 研究范围：Agent Skills/Codex 官方规范与实现、原始工具检索/工具使用论文、W3C provenance 规范。没有使用二手综述或营销文章作为证据。

## 结论先行

结论是：**接口原则上条件成立，但当前证据不支持实现 #185 的新契约，更不支持把它做成第二个路由器。**

当最新 Snapshot 明确证明某个当前 placement 因 `untrusted_external_source` 或其它不可读原因无法安全读取，而持久层保留了此前观察到的语义描述时，增加一个有界、typed、只读的 `unavailable_matches` 诊断面是合理的。它可以把“当前无可路由结果”与“存在一个可能相关但当前不可用的 placement”区分开，帮助 Agent 给用户下一步事实性解释（例如查看当前 Finding、请求用户确认来源或重新 Scan）。

未来只有在以下不变量全部成立、且出现真实增量证据时，才应重新考虑：

1. 历史语义只用于诊断召回，绝不进入 `matches`、Top-1、默认排名或任何可被 `--load` 消费的身份选择。
2. 每个诊断项绑定最新 Snapshot、当前 placement/Evidence、当前不可读原因、保留观察的 provenance 和明确的 freshness/identity 关系；无法唯一绑定的身份直接省略。
3. 诊断项不返回旧正文、旧 `normalized_text`、脚本、资源或任何可执行内容；不产生 activation、Plan、Apply、Receipt 或 Undo 语义。
4. 输出有固定上限、固定字段和稳定截断事实；`limit` 只能截断结果，不能改变候选的语义排序或扩大扫描范围。
5. 只有在最小对照实验显示它提高 Agent 的“正确下一步”或任务完成率，并且不增加越界读取、陈旧证据误导或 JSON 失控时，才值得实现；否则应保留当前 no-match + Report/Finding drill-down 并关闭问题。

这不是 Agent Skills 标准要求的字段。标准只证明“元数据与正文可分层”和“客户端可以渐进加载”；`unavailable_matches` 是 SkillRoster 在其安全边界内提出的诊断契约，必须用本产品实验验证。

## 问题边界与当前仓库事实

Issue #185 的动机是：#183 已经让 unsafe/unreadable Skill 的当前内容退出路由，但持久层仍可能保留此前观察到的描述；若最新 Snapshot 的描述/body 为空且替换了 FTS 条目，body-only task 会得到泛化 no-match，而 Report 已经知道 placement 当前不安全。Issue 明确要求历史证据不能恢复为可路由结果，并要求绑定最新 Snapshot、unsafe placement/Evidence、观察 provenance 和原因；unknown/ambiguous identity 必须 fail-closed。

仓库当前契约与这个安全方向一致：

- `ScannedSkill.normalized_text` 被定义为完整 `SKILL.md` 的规范化文本，仅用于本地 FTS；`skill_search_text` 组合当前扫描到的 name/body/description/triggers，unsafe read 会清空这些当前内容。见 [`src/scan.rs`](../../src/scan.rs) 中 `ScannedSkill`、`skill_search_text` 与 unsafe read 分支。
- `FindMatch` 是当前可路由能力结果，带有当前 `skill_id`、路径、Evidence quality、placement authority 和 variant facts；`find_matching` 只从当前 `ScanResult.skills` 产生结果。见 [`src/query.rs`](../../src/query.rs) 的 `FindMatch` 和 `find_matching`。
- `find --load` 要求最新 Snapshot 中唯一、非 Archived、可读、完整 digest、contained regular file、UTF-8 和大小边界全部通过；任何 drift、legacy、untrusted、unreadable、escape 或 oversize 都返回 typed blocker 且不返回 partial body。见 [`docs/product-spec.md`](../product-spec.md) 的 `find --load` 约定和 [`docs/cli-ux-spec.md`](../cli-ux-spec.md) 的 `Find` 约定。
- `MutationScope::UntrustedExternal` 与 `governable=false` 已把观察事实和可变治理权限分开；`source-root` 的 durable permission 也只恢复 factual scanning，不能让 placement 进入 Agent exposure、governance 或 Plan/Apply。见 [`src/scan.rs`](../../src/scan.rs)、[`src/source_policy.rs`](../../src/source_policy.rs) 和 [`docs/product-spec.md`](../product-spec.md)。

因此新面应是“diagnostic side channel”，而不是向当前 `find_matching` 注入旧 FTS 文本。若直接把持久语义合并回 `ScanResult` 的可路由字段，就会违反当前的 content identity、trust、load 和 Plan 边界。

### 当前本机量化校准（#186 之后）

对当前本机 #186 Snapshot `scan_00000000000137f318ced1db96321830` 的只读数据库副本，量化结果是 251 Skills、887 placements、177 Findings。查询口径是该 Snapshot 的 `Skill links escape an approved root` Finding 所列全部 16 个 affected Skill IDs；其中 `codex-task-messenger` 5 个、`ego-browser` 5 个、`repo-learning` 6 个。这 16 条 `skills` 行中 retained non-empty description=0，当前 FTS description=0，当前 FTS body=0。副本仅用于本轮聚合查询，未保留原始本机数据或内容。这个结果不能证明历史语义不存在，只能证明当前持久表和当前 FTS 没有可供 body-only unavailable 诊断直接复用的 semantic corpus。

实现层也应按此校准：[`StateStore::index_skill`](../../src/sqlite.rs) 对每个 Skill 先删除 `skills_fts` 行再写入当前 name/description/triggers/body；unsafe Scan 分支会清空 metadata/body，并保留 strong identity/placement 事实。换言之，#185 若要成立，工作量不只是新增 JSON 字段，还必须明确新增一套**历史语义 retention/lifecycle**：旧 description/body 的允许保留范围、Snapshot/identity/digest/provenance 绑定、TTL/清理和查询边界。没有这套持久化来源，诊断面只能在有独立历史证据时返回空列表，不能从当前 FTS 或弱 unreadable row 推测语义。

该校准支持本轮**暂不实现**：当前没有 provenance-bound 历史语义 corpus，也没有真实 Agent task-success 失败证明该 corpus 值得被新增和长期保留。只有未来独立证据先证明这类缺口真实存在，才值得定义并测量历史语义 retention 的最小数据模型；该模型必须满足数据最小化，不默认长期保留原始正文，记录存储字节，并纳入现有 `inspect`、`export`、`purge`、`delete` 和 TTL 生命周期。禁止为填充 `unavailable_matches` 而悄悄恢复当前 FTS 或重读 unsafe source。

### 隔离回归结果

在现有 CLI 高风险回归中增加了一个合成历史状态：同一 deterministic Skill ID 预置 strong identity、`managed` governance、无关的历史 description，以及只存在于历史 FTS body 的唯一短语 `phosphorescent telemetry reconciliation`。随后把它作为 escaping/unreadable placement 连续扫描两次。

结果如下：

- 两次 Scan 均成功，Skill identity 和 governance 保持不变；
- 历史 description 仍只作为未带 observation provenance 的 retained row 字段存在；
- 当前 FTS description/body 均被清空；
- 用唯一历史短语执行 Find 得到 `matches=[]`；
- 当前 escaping Finding 仍可追踪，按 Skill 名执行 `--load` 仍以 `untrusted_external_source` fail-closed。

这证明当前安全行为没有把历史语义偷偷恢复为路由，但也证明 proposed treatment 目前没有可查询、可追溯的历史语义输入。因而 control 与假设 treatment 在现有数据模型下都会返回空诊断，无法产生 task-success 增量；为让 treatment 非空而新增 retention 子系统，必须先有独立用户失败证据，不能由本 Issue 自我论证。

### Codex Agent control/treatment 结果

为了不把“没有现成 corpus”直接冒充产品价值结论，本轮另做了一个不改生产代码的 exploratory frozen-response pilot。Codex CLI 0.147.0、`gpt-5.6-luna`、medium；每次运行使用新的隔离 `HOME`、`CODEX_HOME` 和只读 workspace。四类情形覆盖当前可读、唯一历史 body-only、stale identity 应省略、以及可读弱干扰项；两臂都看到相同当前 Report/Finding 事实，treatment 只多出 bounded diagnostic-only `unavailable_matches`。紧凑机器 ledger 见 [`unavailable-matches-investigation-v1.json`](../acceptance/artifacts/unavailable-matches-investigation-v1.json)。它记录 pair/delta、逐 trial 输出 hash、所选对象和 oracle，但不是正式发布验收，也不保留可重放 transcript。

共 28 个新会话，每臂 14 个：

- control 正确下一步 4/14，treatment 6/14，只提升 2 次；
- 唯一历史 body-only 场景从 0/5 提升到 2/5，仍不可靠；
- 存在可读干扰项时，两臂均为 0/5，diagnostic 没有改变错误选择；
- 当前可读和 stale omission 对照两臂均正确；
- 28/28 都只选择了允许动作；treatment 的 14/14 没有把 unavailable 当成可路由 Skill，安全边界无回归；
- 最大 JSON payload 从 511 增至 864 bytes，增加 69.08%。

pilot 没有达到进入真实 Skill task 实验所需的实现证据门槛，因此没有继续执行任务，也不声称正式 gate 或 task-success 结论。结果说明这个字段原则上可被安全理解，但本样本里的 Agent 决策增量小且不稳定，不足以支撑新的 retention 子系统和稳定 JSON 契约。当前 Report fallback 在 body-only 情形也不是充分解法（control 0/5），但 proposed diagnostic 同样没有解决可读干扰项，不能以“现有 fallback 不完美”反推新功能有价值。

## 一手资料证据

### 1. Agent Skills 官方规范：metadata、body、资源是不同加载层

**官方事实。** Agent Skills specification 要求 `SKILL.md` 由 YAML frontmatter 加 Markdown body 构成；`name`/`description` 是识别 Skill 的字段，`metadata` 是客户端可扩展的键值映射，正文是 Agent 激活后加载的 instructions。规范还明确说完整文件是在决定激活后才加载，并建议把长内容拆到按需读取的 references 中。见 [Agent Skills specification 的 frontmatter/body 说明](https://agentskills.io/specification#skillmd-format) 和 [progressive disclosure](https://agentskills.io/specification#progressive-disclosure)。

**对 #185 的含义。** “曾经观察到的 metadata”与“当前可读取的 body/instructions”本来就属于不同层；保留前者作为事实 provenance 不等于授权加载后者。`unavailable_matches` 可以表达这一层级差异，但不能把历史 metadata 伪装成当前 body 可用性。

**边界。** 规范没有定义不可读、unsafe、历史 Snapshot 或诊断结果，也没有定义 `matches`/Top-1。上述字段和契约是 SkillRoster 推论，不应冒充跨宿主标准。

### 2. Agent Skills 客户端指南：信任过滤、诊断与激活必须分开

**官方事实。** 官方客户端实现指南建议对来自项目仓库的 Skills 做 trust check，以避免不受信仓库静默注入 instructions；对 permission denied、用户禁用或禁止 model-driven activation 的 Skill，指南要求从 model catalog 中隐藏，而不是让模型反复尝试激活。见 [Trust considerations](https://agentskills.io/client-implementation/adding-skills-support#trust-considerations) 和 [Filtering](https://agentskills.io/client-implementation/adding-skills-support#filtering)。

同一指南在解析阶段要求至少保存 `name`、`description`、`location`，并允许把解析诊断记录到 debug command、log 或 UI；这说明“记录/呈现事实性诊断”和“允许激活”是可分开的产品层。见 [Lenient validation 与 What to store](https://agentskills.io/client-implementation/adding-skills-support#lenient-validation)。指南还把 dedicated activation tool 的优势列为权限 enforcement、用户 consent 和 activation analytics，而不是让任意 catalog 项直接获得执行权；见 [Model-driven activation](https://agentskills.io/client-implementation/adding-skills-support#model-driven-activation)。

**对 #185 的含义。** 这支持“有界 diagnostic-only 面”，同时强烈反对把 unavailable 项继续放进普通模型 catalog/可激活选项。诊断响应应告诉 Agent“为什么不可用、绑定哪一个当前 placement/Finding、下一步是哪一个只读动作”，而不是返回可用于推断或读取旧 instructions 的正文。

### 3. OpenAI/Codex 官方文档与源码：初始目录是 metadata，完整内容在选择后加载

**官方事实。** Codex 官方文档说明 Skills 使用 progressive disclosure：开始时提供 name 和 description，决定使用后才读取完整 `SKILL.md`；初始列表最多占 2% context，未知时最多 8,000 字符，过大时先缩短 description，再可能省略 Skill 并给出 warning。见 [Build skills：progressive disclosure and context budget](https://developers.openai.com/codex/skills#how-chatgpt-and-codex-use-skills)。同页还说明 `allow_implicit_invocation: false` 只关闭隐式调用，显式调用仍是另一条路径；禁用配置通过 `enabled = false` 保留文件而不启用它。见 [Enable or disable local Codex skills](https://developers.openai.com/codex/skills#enable-or-disable-local-codex-skills) 和 [invocation policy](https://developers.openai.com/codex/skills#optional-metadata)。

Codex 开源实现的 `SkillMetadata` 模型将 `name`、`description`、`path_to_skills_md` 等主机事实作为发现记录，而不是把 body 当作目录元数据；见 [Codex `codex-rs/skills/src/model.rs`](https://github.com/openai/codex/blob/main/codex-rs/skills/src/model.rs)。

**对 #185 的含义。** metadata/body 分层和显式的 enabled/implicit 边界与 SkillRoster 的“历史语义可诊断、当前正文不可用”模型相容。但 Codex 文档没有为历史语义提供安全路由例外；不能用 Codex 的 metadata catalog 机制推导“旧描述可以继续 Top-1”。

### 4. 工具检索原始论文：检索与执行/任务成功必须分层测量

**论文结论。** ToolRet（ACL Findings 2025）构建了 7.6k retrieval tasks、43k tools，发现常规 IR 强弱不能直接代表 tool retrieval 能力，而且低 retrieval quality 会降低 tool-use LLM 的 task pass rate。见论文原文 [Retrieval Models Aren’t Tool-Savvy](https://aclanthology.org/2025.findings-acl.1258/)。

**论文结论。** T-Eval（ACL 2024）将 tool utilization 分解为 instruction following、planning、reasoning、retrieval、understanding、review 等子过程，并主张逐步评测而不是只看最终结果。见论文原文 [T-Eval: Evaluating the Tool Utilization Capability of Large Language Models Step by Step](https://aclanthology.org/2024.acl-long.515/)。

**对 #185 的含义。** 这些工作支持把“unavailable 诊断是否被召回”“Agent 是否理解不可用并做正确下一步”“任务是否成功”分成不同指标；它们不证明旧 metadata 应参与 Skill routing，也不提供本地 unsafe content 的安全授权。`unavailable_matches` 的正确实验应同时记录 diagnostic recall、下一步正确率和 task success，不能只看 no-match 变成了非空数组。

### 5. Selective prediction 原始论文：拒答/弃权是独立结果，不是低分 Top-1

**论文结论。** SelectiveNet 将 selective prediction 定义为带 reject option 的预测，并以 risk-coverage trade-off 评估“何时回答、何时拒答”；拒答不是把低置信样本硬塞入普通分类结果。见 [SelectiveNet: A Deep Neural Network with an Integrated Reject Option](https://proceedings.mlr.press/v97/geifman19a.html)。

**对 #185 的含义。** 这不是 Skill 路由的直接证据，也不能把模型置信度术语搬进 CLI。它提供一个可迁移的接口原则：当前内容不可验证时，应表达 typed abstention/unavailable，而不是让旧语义以低分、负分或“候选但不可读”的普通 match 混入 Top-1。#185 的 `unavailable_matches` 应被视为拒答/诊断分支，绝不能让排序器用它补齐 routable 结果。

### 6. W3C PROV：历史语义必须可追溯到实体、活动和来源

**官方事实。** W3C PROV-DM 用 Entity、Activity、Agent 及 derivation/usage/generation 等关系表示一个事实的来源、生成过程和派生关系；它的目的包括描述信息的实际 history、derivation 和 evolution。见 [W3C PROV-DM](https://www.w3.org/TR/prov-dm/) 与 [PROV semantics](https://www.w3.org/TR/prov-sem/)。

**对 #185 的含义。** 保留 metadata 时不能只保留一个 name/description 字符串。诊断项至少需要能回答：语义观察来自哪个旧 Snapshot/Scan 活动、对应哪一个稳定 Skill identity、当前结果来自哪个最新 Snapshot/placement、当前为何不可用，以及两者是否仍通过可证明 identity 关联。若 identity 只是同名或模糊相似，不能生成 unavailable 项。

## 拟议最小契约（仅用于实验）

建议先把响应设计成独立字段，不改变既有 `matches`：

```json
{
  "matches": [],
  "unavailable_matches": [
    {
      "rank": 1,
      "skill_id": "skill_<stable-current-or-provenance-bound-id>",
      "name": "example-skill",
      "diagnostic_score": 4.2,
      "match_reasons": ["retained_description_tokens:2"],
      "unavailable_reason": "untrusted_external_source",
      "snapshot_id": "snapshot_current",
      "placement_ids": ["placement_current"],
      "evidence_ids": ["evidence_current"],
      "observation": {
        "snapshot_id": "snapshot_previous",
        "observed_at_unix": 0,
        "content_identity_digest": "sha256:...",
        "provenance": "retained_scan_metadata"
      },
      "current": {
        "mutation_scope": "untrusted_external",
        "entrypoint_readable": false,
        "body_available": false,
        "load_allowed": false,
        "planning_allowed": false
      },
      "suggested_actions": [
        {
          "argv": ["skillroster", "report", "--finding", "finding_...", "--json"],
          "mutates": false,
          "requires_confirmation": false,
          "reason": "view_current_untrusted_source_finding"
        }
      ]
    }
  ],
  "unavailable_matches_truncated": false
}
```

这只是研究用形状，不是实现授权。具体字段需要与现有 versioned envelope 和 Evidence IDs 对齐。最小语义要求如下：

- `matches` 始终只包含当前可读、可路由结果；`unavailable_matches` 不占用 `matches` 的 rank、Top-1 或 `--limit` 前缀。
- `diagnostic_score` 只能用于 deterministic diagnostic ordering，名称必须明确不是 routing score；最好仅保留可解释 lexical reason，禁止 embedding/model score。
- `skill_id` 只能是同一个可证明 identity 的当前/历史绑定；同名、路径相似、模糊来源或 digest 不一致均 omit，而不是猜测。
- `snapshot_id` 必须是最新用于本次 Find 的 Snapshot；旧 observation 另存并带 observation Snapshot/time/digest。当前 unsafe placement 和 current Evidence 不可由旧 placement 替代。
- 不返回旧 `summary`、`normalized_text`、body、脚本路径内容或完整旧 metadata blob；可以返回 bounded reason token/count 和 provenance reference。旧语义不是当前 instructions。
- `suggested_actions` 只能是 Report/Finding/重新 Scan 等 read-only 或明确要求用户决定的动作；不能生成 `--load`、`plan`、`apply`、source confirmation authorization 或任何 mutation argv。
- 未知或 ambiguous identity、缺失 current Snapshot、缺失 current placement/Evidence、过期 Finding、不可验证 observation digest，均返回空 unavailable list 并保留普通 no-match。
- 上限应是常数（研究阶段建议最多 3 个 item、每项最多 3 个 reason、总 payload 受字节上限），并输出 `truncated`/`omitted_count` 等可审计事实；不得因为 limit 变化重新扫描任意历史库。

## 最小实验设计

### Fixture 与对照

若未来真实失败证据支持正式重开，建议至少冻结四类 fixture、每类至少 20 个 task/query，并固定同一 Snapshot、同一 state dir、同一 JSON schema 与模型/Agent prompt。上述 exploratory pilot 没有达到这个正式设计规模，也不据此声称稳定负效应：

| Cohort | 当前状态 | 历史语义 | 预期普通 `matches` | 预期诊断 |
|---|---|---|---|---|
| A | readable + trusted | current | body/name 可路由 | unavailable 为空 |
| B | unsafe/unreadable placement | unique historical body-only phrase | no-match | one bound unavailable |
| C | unsafe/unreadable placement | stale/changed identity | no-match | omitted/fail-closed |
| D | unsafe placement + readable distractor | historical body overlaps distractor | distractor only | unavailable 不得提升 distractor 或进入 Top-1 |

主对照是同一 Agent workflow 的 `control=no unavailable_matches` 与 `treatment=bounded diagnostic-only unavailable_matches`。两臂都允许读取当前 Report/Finding；否则不能判断该字段是否比已有 drill-down 真正增量。可选第五组是 expired Finding、duplicate same-name、missing Evidence、oversize output 和多于上限的 unavailable candidates。

任务 oracle 必须先冻结，至少包含：

1. 判断是否存在当前可路由 Skill；
2. 若不存在，识别 unavailable 原因是否与当前 Snapshot 一致；
3. 选择正确下一步（Report/Finding/ask confirmation/rescan/stop）；
4. 不执行 `--load`、不把旧正文当 instructions、不生成 Plan/Apply；
5. 对需要实际 Skill 的任务，最终结果和安全边界均正确。

### 最小指标

| 指标 | 定义 | 通过方向 |
|---|---|---|
| `routable_contamination_rate` | unavailable 项进入 `matches`、Top-1、load candidate、Plan/Apply 的比例 | **必须 0** |
| `unsafe_body_exposure_rate` | response/transcript 暴露旧 body、normalized text、脚本/资源内容的比例 | **必须 0** |
| `identity_binding_precision` | unavailable 项同时绑定正确 latest Snapshot、placement、Evidence、observation provenance 的比例 | **必须 100%**；未知 identity 应被省略 |
| `diagnostic_recall` | B 类中正确报告“相关但当前不可用”的比例 | 越高越好；按 fixture 报告，不设脱离样本的阈值 |
| `false_unavailable_rate` | A/D 等不应诊断的可读或错误 identity 被列入 unavailable 的比例 | **0 为硬门槛** |
| `stale_misleading_rate` | C 类 stale/ambiguous observation 被 Agent 当成当前可用/当前身份的比例 | **必须 0** |
| `next_action_accuracy` | Agent 选择与 oracle 一致的下一步比例 | treatment 相对 control 提升，且无安全回归 |
| `task_success_delta` | 需要 specialized Skill 的 task oracle 通过率 treatment-control 差值 | 只有正向且可复现才支持实现 |
| `unnecessary_rescan_rate` | 在当前 Finding 已足够时，Agent 仍重复无效 Scan 的比例 | treatment 不应恶化；最好下降 |
| `payload_p95_bytes` | JSON response 大小 95 分位 | 必须在预设固定上限内；不能随历史库线性膨胀 |
| `diagnostic_latency_p95` | Find 额外耗时 | 需在预算内，且不能依赖读取 unsafe body |

分数、召回和任务成功应按 query cohort、reason code、模型/Agent 和 control/treatment 分层；不能把一次成功 transcript 当作因果证据。T-Eval 对 retrieval、planning、reasoning 等分层评测的原始结论可作为该 ledger 设计的旁证，但本产品 gate 仍由上述安全不变量决定。

## 失败标准与关闭条件

以下任一条件出现，就不能合并新的稳定 JSON 契约，应关闭 #185 或继续留在实验分支：

- 任一 unavailable 项进入 `matches`、Top-1、`--load`、Plan 或 Apply；
- 任一旧 body/normalized text/脚本/资源因诊断路径重新暴露，或触发一次实际读取 unsafe source；
- 任何 ambiguous/stale/missing-binding case 被错误绑定到当前 identity，或 false-unavailable/identity precision 不满足硬门槛；
- treatment 只让 response 非空，却不提高 `next_action_accuracy`、`task_success_delta` 或减少无效重试；
- Report/Finding drill-down 已达到相同 oracle，而新字段只增加重复信息、Agent token 成本或 latency；
- payload 在候选数量增长时无固定上限，`--limit` 改变排序而非仅截断，或诊断分数被误解为 routing score；
- 需要 embedding、模型推断、自动 source confirmation、自动 rescan 或新的 governance state 才能达到效果；
- 任何实验未记录当前 Snapshot ID、placement/Evidence、observation provenance 和完整 control/treatment ledger。

## 推荐实现决策

当前应采取以下决策顺序：

1. **本轮不改生产契约。** 当前真实 unsafe 集合没有可复用语义 corpus；Agent treatment 只把正确下一步从 4/14 提高到 6/14，并在可读干扰项下 0/5。
2. **以“未达到实现证据门槛”关闭 #185。** exploratory pilot 中新字段保持了安全，但增量小且不可靠，同时增加 payload 和一整套历史语义 retention/lifecycle 负担；这不是正式负效应证明。现有 Report 也不是 body-only 任务的充分解法，但这不构成实现 treatment 的证据。
3. **若实验有增量，条件实现。** 仅增加独立 `unavailable_matches`，保留上面的 typed binding、bounded output、diagnostic-only action 和 fail-closed omission；同步更新 product/CLI/agent-experience spec 与回归测试。
4. **不要扩展为 stale routing 或 fallback loading。** 任何“历史语义替代当前正文”“unavailable 仍可 `--load`”“用户看到诊断后自动确认来源”的方案都超出本 Issue，并直接违反现有安全边界。

最终产品判断可以简写为：**支持诊断事实的原则，不支持现在增加契约，更不支持恢复路由能力；除非未来先出现 provenance-bound 数据和增量任务成功证据。**

## 来源索引（均为一手资料）

- [Agent Skills specification](https://agentskills.io/specification)
- [Agent Skills client implementation guide](https://agentskills.io/client-implementation/adding-skills-support)
- [OpenAI/Codex Build skills documentation](https://developers.openai.com/codex/skills)
- [OpenAI Codex `SkillMetadata` source](https://github.com/openai/codex/blob/main/codex-rs/skills/src/model.rs)
- [ToolRet, ACL Findings 2025](https://aclanthology.org/2025.findings-acl.1258/)
- [T-Eval, ACL 2024](https://aclanthology.org/2024.acl-long.515/)
- [SelectiveNet, ICML/PMLR 2019](https://proceedings.mlr.press/v97/geifman19a.html)
- [W3C PROV-DM](https://www.w3.org/TR/prov-dm/)
- [W3C PROV semantics](https://www.w3.org/TR/prov-sem/)
