# Kimi Code CLI Skill discovery：版本分界与接入边界

日期：2026-10-22。状态：只读资料研究；没有安装、启动或修改 Kimi，没有运行真实环境验收。Issue #383 只提供需求线索，不作为机制证据。

## 结论

不能把 `~/.kimi/skills` 加入一个新 adapter 后就宣称支持当前 Kimi Code CLI。官方已将 Python `kimi-cli` 归档，并迁移至 Node.js `kimi-code`；两代的目录、默认合并设置和加载规则存在差别。应先确定支持代际，再冻结发现、可见性、覆盖与真实环境验收合同。当前 SkillRoster 的八 adapter 与 `SKILL.md` package 模型不足以直接代表 Kimi 的 flat `.md` Skills。

通过只读 GitHub API `repos/MoonshotAI/kimi-code/commits/main` 确认查询当时 HEAD 为 **`79a76ea2da0d4dd6e81501155454e646b909c695`**（commit 时间 `2026-10-22T11:57:24Z`）；随后所有新增源码核验均指定该 `ref`。该 revision 的 `apps/kimi-code/package.json` 为 **2.3.0**。这证明查询时源码 HEAD 与包版本，不证明 Latest release、用户安装版本或真实运行行为。[固定 revision](https://github.com/MoonshotAI/kimi-code/commit/79a76ea2da0d4dd6e81501155454e646b909c695)、[固定包定义](https://github.com/MoonshotAI/kimi-code/blob/79a76ea2da0d4dd6e81501155454e646b909c695/apps/kimi-code/package.json)。旧仓库网页 `pyproject.toml` 为 1.52.0 且明确归档，该历史部分未做 commit pin。[旧包定义](https://github.com/MoonshotAI/kimi-cli/blob/main/pyproject.toml)、[归档声明](https://github.com/MoonshotAI/kimi-cli/blob/main/README.md)。

## 当前代际：官方文档确认的合同

| 方面 | 已确认事实 | 对 SkillRoster 的影响 |
| --- | --- | --- |
| User roots | `$KIMI_CODE_HOME/skills`，默认 `~/.kimi-code/skills`；另有真实 OS home 下的 `~/.agents/skills` | 不把泛用目录随 Kimi home 一起搬迁 |
| Project roots | 最近 `.git` 项目根下 `.kimi-code/skills`、`.agents/skills` | 不能只扫描启动 cwd 或任意向上累积 |
| Scope priority | Project > User > Extra > Built-in | 存在于根内不等于最终胜出或默认可见 |
| 持久附加目录 | `config.toml` 顶层 `extra_skill_dirs: array<string>`，叠加默认发现 | 配置是发现输入，不能自动变成可信可写来源 |
| 临时覆盖目录 | 可重复 `--skills-dir` 替换本次启动的 User/Project 自动发现 | 静态文件观察无法证明某个活跃 session 使用的 overrides |
| 合并开关 | 新文档 `merge_all_available_skills` 默认 `true` | 不沿用旧 Python 默认 `false`；当前每个品牌组只有一个候选，开关不改变这两个默认根的选择 |
| 产品内置 Skills | `builtin_product_skills` 默认 `true`，可关闭产品说明类内置 Skills | 不将包内内置目录当普通可治理 root |

根与作用域见[当前 Skill 文档](https://www.kimi.com/code/docs/en/kimi-code-cli/customization/skills.html)；Kimi home 与真实 home 的分离见[数据目录](https://www.kimi.com/code/docs/en/kimi-code-cli/configuration/data-locations.html)。配置键、类型、默认值见[配置文档](https://github.com/MoonshotAI/kimi-code/blob/main/docs/en/configuration/config-files.md)。临时替换与持久叠加的区别见[CLI 参数](https://www.kimi.com/code/docs/en/kimi-code-cli/reference/kimi-command.html)。

### Package 与 activation

- 目录式要求显式 `name`、`description`；flat 顶层 `.md` 可用文件名和正文首行 fallback。同一 root 内目录式优先于同名 flat；任意子目录中的普通 `.md` 不是独立 Skill。
- 名称大小写归一；`whenToUse` 与 `disableModelInvocation` 有 camelCase、kebab-case、snake_case 别名。`disableModelInvocation=true` 和 `type=flow` 阻止自动调用，不能因此推断内容不存在或无用。
- 用户以 `/skill:<name>` 加载，模型按描述与 when-to-use 选择；调用时展开参数与 `${KIMI_SKILL_DIR}`。目录式该变量指 package 目录，flat 指共享 root。展开是 Kimi 的运行时行为，不应由 SkillRoster 静态扫描执行。

这些格式与行为来自[官方 Skill 源文档](https://github.com/MoonshotAI/kimi-code/blob/main/docs/en/customization/skills.md)。大小写与类型还有[源码 `catalog/types.ts`](https://github.com/MoonshotAI/kimi-code/blob/main/packages/agent-core-v2/src/features/skill/catalog/types.ts) 支持。注意该源码的 `isSupportedSkillType` 还接受 `reference`，而上述文档称其他 type 被跳过；固定 revision 的 `parser.ts` 调用该函数，确认 `reference` 会进入解析与库存，但不会进入 `listInvocableSkills` 或用户 activation；不能把文档表枚举直接当完整 parser 合同。

新源码 [`skillService.ts`](https://github.com/MoonshotAI/kimi-code/blob/main/packages/agent-core-v2/src/features/skill/skillService.ts) 记录 `skill_activation`、`activationId`、`trigger=user-slash`、Skill path/source，并把渲染后的 Skill 加入用户消息；[`prompt.ts`](https://github.com/MoonshotAI/kimi-code/blob/main/packages/agent-core-v2/src/features/skill/prompt.ts) 用 `<skill-loaded>` 标记区分 `user-slash`、`model-tool`、`nested-skill`。这些是未来 usage adapter 的候选证据，尚未核对最终磁盘 session schema，因此本轮不实现日志解析，也不把 activation 当 Applied 或 Outcome。

## 旧代际：只能用于明确的 legacy 支持

Python 版 `src/kimi_cli/skill/__init__.py` 的机制如下：User 品牌组依序 `~/.kimi/skills`、`~/.claude/skills`、`~/.codex/skills`，默认只取首个现存目录；泛用组独立取 `~/.config/agents/skills`、`~/.agents/skills` 中首个现存目录。两组结果合并，品牌更优先。Project 也有品牌组和独立 `.agents/skills`；先找最近 `.git`，无标记退回工作目录。`merge_brands=true` 才合并全部品牌根。空但存在的品牌根也会截断品牌 fallback。

该源码还说明：local backend 的 root dedupe 先经 `Path.resolve()` 解析 symlink，再 canonicalize；非 local backend 不做本机 symlink 解析。`--skills-dir` 覆盖 User/Project；config Extra 仍叠加。不要把这些旧版细节移植成新版保证。[旧源码](https://github.com/MoonshotAI/kimi-cli/blob/main/src/kimi_cli/skill/__init__.py)。

官方迁移会复制配置、MCP、输入历史与选定 sessions，并保留 `~/.kimi` 原数据；这不等于旧 Skill 根自动成为新版 roots。迁移页面未给出旧 Skills 自动兼容的保证。[官方迁移说明](https://www.kimi.com/code/docs/en/kimi-code-cli/guides/migration.html)。

## 固定 revision 的实际 catalog 函数

下列链接均固定到 `79a76ea2da0d4dd6e81501155454e646b909c695`，来自 `gh api .../contents/...?...ref=` 的源码读取；没有执行上游测试。

- [`skillRoots.ts`](https://github.com/MoonshotAI/kimi-code/blob/79a76ea2da0d4dd6e81501155454e646b909c695/packages/agent-core-v2/src/features/skill/catalog/skillRoots.ts)：`userRoots`、`projectRoots` 都先加入品牌根，再独立加入泛用根；当前每组各一个候选，所以 merge 开关 true/false 不改变默认候选集合。`configuredRoots` 对 `~`、`~/` 用真实 OS home，绝对路径原样、相对路径以项目根解析；不存在或 stat 失败被跳过。`pushExistingRoot` 使用 `fs.stat` + `fs.realpath`，同一次 root 列表按 resolved path 去重，保留先出现者。`findProjectRoot` 经 [`findUpwardRoot`](https://github.com/MoonshotAI/kimi-code/blob/79a76ea2da0d4dd6e81501155454e646b909c695/packages/agent-core-v2/src/_base/utils/paths.ts) 找最近 `.git` marker（目录或文件均可），无标记返回原始 cwd。
- [`fileSkillDiscovery.ts`](https://github.com/MoonshotAI/kimi-code/blob/79a76ea2da0d4dd6e81501155454e646b909c695/packages/agent-core-v2/src/features/skill/catalog/fileSkillDiscovery.ts)：`discoverFileSkills` 按 roots 顺序扫描，同一 case-insensitive 名称首个成功解析者胜出；plugin 用 plugin ID + name 单独去重。最大 walk depth 为 8（depth > 8 才停止）；每层目录名排序。`.hidden`、`node_modules` 不递归，但直接含 SKILL.md 的子目录在 exclusion 前被识别，因此 hidden package 仍可能直接注册。普通 category 可递归；已声明 package 默认停止深入，除非 `has-sub-skill`/`hasSubSkill` 在顶层或 `metadata` 下为 true。子 Skill 名称限定为 `parent.child` 并设置 `isSubSkill=true`。顶层 flat `.md` 才被考虑，同目录 basename 的目录式会先占位并排除 flat，即使该目录式 parse 失败。
- [`runtimeSkillDiscovery.ts`](https://github.com/MoonshotAI/kimi-code/blob/79a76ea2da0d4dd6e81501155454e646b909c695/packages/agent-core-v2/src/features/skill/workspace/runtimeSkillDiscovery.ts)：runtime 路径也设置 depth=8、category/package/sub-skill 分界。目录 symlink 通过 stat 后可遍历；flat 分支要求 readdir 的 `isFile`，与 Node-file 实现再次 `stat` 的方式不同，因此不能把某一实现的 flat symlink 行为推广到所有运行后端。
- [`skillSource.ts`](https://github.com/MoonshotAI/kimi-code/blob/79a76ea2da0d4dd6e81501155454e646b909c695/packages/agent-core-v2/src/features/skill/catalog/skillSource.ts) 与 [`workspaceSkillCatalogService.ts`](https://github.com/MoonshotAI/kimi-code/blob/79a76ea2da0d4dd6e81501155454e646b909c695/packages/agent-core-v2/src/features/skill/workspace/workspaceSkillCatalogService.ts)：按 builtin=0、plugin=5、extra=10、user=20、workspace=30 依次合并，后者 `replace=true` 覆盖前者。plugin 名称注册为 `pluginId:name`。因此对普通名称，Project > User > Extra > Built-in；同 scope 根内是首个名称胜出。
- [`userFileSkillSource.ts`](https://github.com/MoonshotAI/kimi-code/blob/79a76ea2da0d4dd6e81501155454e646b909c695/packages/agent-core-v2/src/features/skill/catalog/userFileSkillSource.ts)、[`rootFileSkillSource.ts`](https://github.com/MoonshotAI/kimi-code/blob/79a76ea2da0d4dd6e81501155454e646b909c695/packages/agent-core-v2/src/features/skill/workspace/rootFileSkillSource.ts)、[`explicitFileSkillSource.ts`](https://github.com/MoonshotAI/kimi-code/blob/79a76ea2da0d4dd6e81501155454e646b909c695/packages/agent-core-v2/src/features/skill/workspace/explicitFileSkillSource.ts)、[`extraFileSkillSource.ts`](https://github.com/MoonshotAI/kimi-code/blob/79a76ea2da0d4dd6e81501155454e646b909c695/packages/agent-core-v2/src/features/skill/workspace/extraFileSkillSource.ts)：非空 `skillDirs` 令 User/Project sources 返回空，Explicit source 以 User scope 加载；Extra source 不受此条件影响，继续叠加。User/Project source 有目录 watch 和 200ms debounce；不能断言仅启动加载一次。
- [`parser.ts`](https://github.com/MoonshotAI/kimi-code/blob/79a76ea2da0d4dd6e81501155454e646b909c695/packages/agent-core-v2/src/features/skill/catalog/parser.ts)、[`types.ts`](https://github.com/MoonshotAI/kimi-code/blob/79a76ea2da0d4dd6e81501155454e646b909c695/packages/agent-core-v2/src/features/skill/catalog/types.ts)、[`registry.ts`](https://github.com/MoonshotAI/kimi-code/blob/79a76ea2da0d4dd6e81501155454e646b909c695/packages/agent-core-v2/src/features/skill/catalog/registry.ts)：目录式首行必须是 frontmatter delimiter，name/description 必须非空；`reference` 被支持但不可自动或用户激活。自动调用集合只含 prompt/inline（或未声明 type）且 `disableModelInvocation !== true`；model listing 再排除 `isSubSkill===true`。该条件是严格 boolean，字符串 `"true"` 不等于 true。model listing 描述最多 250 字符预算，另列 when-to-use 与 path。

这些是静态实现事实，不能据此声称上游 loader 对逃逸链接实施 SkillRoster 的信任或 containment 策略。特别是 `fs.stat`/readFile 会跟随链接，发现 walker 没有内容信任验证；SkillRoster 必须保留自己独立的 fail-closed source boundary。

## 尚未验证的边界

1. 实际启动时选择 `FileSkillDiscovery` 或 `RuntimeSkillDiscovery` 的 wiring、远端/backend 对 stat/readdir 的含义、循环/破链/逃逸链接的真实平台回执。
2. plugin 安装与启用标记、scope (`tui`/`web`) 和 session 级额外过滤；本研究不能把 workspace catalog 等同于任意活跃模型完整 catalog。
3. usage 事件的最终磁盘 session schema、retention 与重放；activation 不证明 Applied 或 Outcome。
4. 当前发布版本和用户安装版本；macOS/Linux/Windows 的原生加载与 invocation 验收。固定源代码不是运行验收。

## 可实施的最小下一步

本次不建议增加一个仅硬编码 roots 的完整 Kimi adapter。先用明确的 `--source-root <实际目录>` 观察符合现有 `SKILL.md` 模型的目录包，可获得 Library/Find 事实；它不赋予 Kimi 身份、default exposure 或治理权限，也不覆盖 flat Skills。这是现有能力，不需要冒充 `--root kimi=...` 支持。

若实施新 adapter，应以 **Node.js Kimi Code CLI** 为独立、明确的验收对象：

1. 基于本研究已固定的源码 revision，核对上面剩余未知项；用合成 home/project/config 建 fixtures，再和同 revision 原生 loader 对照。至少覆盖自定义 home、两种 root 共存、合并开关、同名遮蔽、无 `.git`、嵌套 cwd、CLI override 与 Extra 共存。
2. Inventory 与 native exposure 分开；manual-only、shadowed、unknown 的事实不能折算为默认目录曝光。未知可见性须阻止依赖该事实的 Roster 变更。
3. flat `.md` 先明确现有 Skill Identity、package hash、support files、Mutation、Receipt/Undo 模型的兼容成本；不能把整个共享 skills root 视为每个 flat Skill 的独占 package，亦不能扫描时偷偷复制成目录式。
4. symlink 沿用 SkillRoster 的 source trust 与 containment；配置目录仅提供路径，provider/plugin roots 保持 Observed/read-only，原生读取成功不等于治理授权。
5. 先交付有范围声明的 inventory/exposure 支持；usage、治理与正式 Supported 各自通过独立 fixtures 和真实环境验收后再声明完成。不要仅为复用现有 `Conservative` 模式把递归结果说成 Kimi 原生 catalog。

仓库依据：[`src/harness.rs`](../../src/harness.rs) 的八 Agent 枚举与 discovery semantics、[`src/model.rs`](../../src/model.rs) 的独立 Agent 枚举，以及 [`docs/product-spec.md`](../product-spec.md) 的 supported gate、目录包定义、source trust、未知 exposure 与 mutation 边界。新增代际与 flat package 是真实合同变化，应先写入规范再实现，避免只修路径却误报接入完成。
