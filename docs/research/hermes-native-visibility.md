# Hermes 原生可见性：两个固定版本的本地投影合同

日期：2026-10-23。只读源码研究，没有读取真实用户 `config.yaml`、运行模型、修改安装或实现 Rust。先阅读 [目录机制研究](hermes-skill-discovery.md)；本记录进一步区分目录发现、`skills_list`、system-prompt index 与显式 `skill_view`，不能将它们合并为一个运行时 catalog。

固定比较对象：已安装 **0.20.0，`76d832d3857551a029c4b39c23945eb47c16fe5b`**（下称 I），以及此前记录的 upstream main **`92663ae8054ae81f207f46d83512d3f879dc698d`**（下称 U）。U 是固定源码快照，并非本次重新查询的最新版本。I 直接读取安装源码，U 读取官方指定 revision 的源码；引用均固定 commit。

## 共同规则及明确差异

| 项目 | I：已安装 0.20.0 | U：固定 main | 可否合并为跨版本事实 |
| --- | --- | --- | --- |
| `skills.disabled` | global 名称集合 | global 名称集合 | 普通有效名称可；不是 OS 文件路径规则 |
| global 与 channel-specific | global ∪ 当前 channel 的 `platform_disabled`，空 channel 列表不会解除 global 禁用 | 同左 | 可以 |
| channel 选择 | explicit argument → `HERMES_PLATFORM` → `get_session_env("HERMES_SESSION_PLATFORM")` | 同左 | helper 可；不代表所有调用表面优先级相同 |
| essential 例外 | shared getter 没有例外，`hermes-agent` 可进入 disabled 集合 | shared getter 与配置 UI 去掉 `hermes-agent` | 不可；涉及该名字应保留版本或 unknown |
| disabled 字符串 | scalar string 是一个名字，包括字面量 `'["foo"]'` | scalar string 若可解析为 list literal，拆为多个名字 | list-as-string 不可无版本解释 |
| `skills_list` 名称匹配 | frontmatter `name`，缺失时 dirname；最多 64 字符 | declared name 或 resolved `load_name`；后者可为消歧相对路径 | 普通唯一声明名可；alias/path 不可 |
| prompt index 名称匹配 | declared name **或 dirname** | dirname gate 加共享 catalog 的 declared/load-name gate | 目录别名必须单独考虑，不可只复制 `skills_list` |
| host OS `platforms` | falsy 为全平台；scalar 包为单元素；映射后用 `sys.platform.startswith`；另有 Termux 特例 | 同一基本映射与 prefix 规则，无 Termux 特例 | 正常 macOS/Linux/Windows 值可；Android/Termux 不可 |
| runtime `environments` | `kanban`、`docker`、`s6`，OR；未知 tag 放行 | 同左 | tag 语法可；活跃环境依赖 session/process |
| `requires_apps` | 这些已检查入口不使用此 gate | list/prompt 增加 app availability gate | 不可 |
| `metadata.hermes.session_platforms` | 条件提取与 prompt gate 不支持 | prompt 增加 channel gate；未知 channel 放行 | 不可；也不是 host OS |
| 额外 roots | local + `external_dirs` | project + local + `create_dir` + external | 发现和覆盖语义不可当成跨版本常量 |

disabled、字符串与 essential 的依据：[I helper][i-disabled]、[U helper][u-disabled]、[I UI][i-ui]、[U UI][u-ui]。名称匹配依据：[I list][i-list]、[I prompt][i-prompt]、[U catalog][u-catalog]、[U prompt][u-prompt]。OS/env/apps 依据：[I frontmatter/gates][i-gates]、[U frontmatter/gates][u-gates]；channel gate 依据：[I condition extraction][i-conditions]、[U conditions][u-conditions]、[U prompt conditions][u-prompt-conditions]。Roots 依据：[I external roots][i-roots]、[U roots][u-roots]，详见前述目录研究。

### “platform” 的两种含义

frontmatter 顶层 `platforms: [macos, linux, windows]` 匹配运行 Python 的 **OS**，名称先转字符串、trim、lower，再将 macos→darwin、windows→win32、linux→linux，最后 prefix 匹配。`skills.platform_disabled.telegram` 的 key 则是 **会话渠道**，不是 OS；helper 不对该 key trim/lower，也不会从 `sys.platform` 猜出 `cli`。[I gates][i-gates]、[U gates][u-gates]、[I disabled][i-disabled]、[U disabled][u-disabled]。

`get_session_env` 优先 gateway ContextVar，显式空字符串也不回退；只有该 context 从未设置才读取对应环境变量。另一方面，两版 prompt builder 的 `_current_session_platform_hint` 先读进程 `HERMES_PLATFORM` / `HERMES_SESSION_PLATFORM`，再看已加载的 gateway context，并把结果作为 explicit argument 传给 getter。因此并发 gateway 下不能把一个后台扫描进程的 env 当作目标 Hermes session 的渠道证据。[I session resolver][i-session]、[I prompt hint][i-hint]、[U prompt hint][u-hint]。

### 显式加载不等于默认曝光

两版 `skill_view` 都检查 host OS 与 disabled，并不执行 `environments` offer-time gate；U 的 `requires_apps` 也属于 offer gate。两版 `_is_skill_disabled` 都直接在 `load_config()` 的原始值中做 membership，异常捕获后返回 false，未复用 shared getter 的规范化。由此 scalar 字符串可能发生 substring membership；U 的 shared getter 虽移除 essential 名字，`skill_view` 分支仍没有该例外。不能根据 UI 文案“不能禁用 essential”推导所有加载入口都放行。[I explicit helper][i-explicit-helper]、[I skill_view][i-view]、[U explicit helper][u-explicit-helper]、[U skill_view][u-view]。

同理，`platforms` 不兼容可能阻止显式加载，而环境不相关仅影响 offer；不能将这些原生状态直接重命名成 SkillRoster 的 On-demand、Explicit-only 或 Archived。

## 缺失、无效与类型异常

两版 shared raw-config reader：文件不存在、读取/解析异常、YAML 顶层非 mapping，均返回 `{}`；`skills` 非 mapping 时 disabled getter 返回空集合。原生这种 fail-open 读取行为不证明配置有效。U 还可直接使用 safe-worker 的冻结 config snapshot，绕过磁盘文件；静态扫描必须声明只观察 supplied Home 的本地文件。[I config reader][i-config]、[U config reader][u-config]。

| 输入形态 | 实际代码后果 | 建议的诚实投影 |
| --- | --- | --- |
| 确认文件不存在，或有效 mapping 中未声明 `skills`/disabled | shared getter 无禁用项 | 可记录本地文件规则无禁用；不推断 session catalog |
| 文件不可读、坏 YAML、顶层 scalar/list | shared reader 静默视为空 | unknown，不能把无法读取或校验当作启用证据 |
| `skills` 存在但不是 mapping | getter 视为空 | unknown，披露 malformed section |
| `disabled: foo` | 两版 shared getter 将其视为一个名字；显式 helper 仍可能 substring 匹配 | 有限 list/prompt 投影可支持；不能宣称显式加载完全一致 |
| `disabled: '["foo"]'` | I 视为字面量，U 解析 list | version 未冻结时 unknown |
| disabled 为 list，成员非字符串 | 两版 shared getter 将成员 `str()` 再 trim | 不仿制语言特有转换；相关规则 unknown |
| disabled 为数字/boolean | I shared getter 迭代可能抛 TypeError；U 返回空 | unknown，非兼容的空列表 |
| disabled 为 mapping | I 迭代 keys；U 返回空 | unknown |
| `platform_disabled` 为 truthy 非 mapping | 已解析 channel 时 `.get()` 可能抛异常 | unknown；不依赖“当前没 channel 所以刚好没报错” |
| `platforms` 为有效字符串/list | Python 的 truthiness 与 prefix 规则 | 支持冻结语法范围即可 |
| `platforms: false` / `0` / 空 map，或 list 中空字符串 | falsy 可直接全平台；空元素 `startswith("")` 也可匹配全部 | 类型/值异常应 unknown，不复制意外放行 |
| 坏 frontmatter YAML | 两版都 fallback 成逐行 `key:value` 字符串 | unknown，不尝试在 Rust 猜测 fallback 的生效结果 |

这些类型后果由 [I normalize/config][i-disabled]、[U normalize/config][u-disabled]、[I parser/platform][i-gates]、[U parser/platform][u-gates] 的静态分支直接得出；本轮没有调用真实配置解析。两版 list 只读取入口前 4,000 字符后解析 frontmatter，但 prompt/explicit 表面可读取完整文件；因此控制字段超出该窗口或 frontmatter 未在窗口内完整闭合时，不能仅凭完整文件内容宣称所有表面一致。[I list][i-list]、[U list][u-list]、[I prompt parser][i-prompt-parser]、[U prompt parser][u-prompt-parser]。

## 研究形成的本地投影合同

下述建议已作为后续实现的依据；最终支持范围和原生隔离回执见
[可见性验收记录](../acceptance/hermes-native-visibility-2026-10-23.md)。
研究中的版本差异仍成立，不能因局部实现而扩展为完整 session catalog 声明。

建议名称为 **supplied-Home user Skills local-file rules projection**，只观察 `<supplied-home>/.hermes/config.yaml` 与本轮已验证入口字节。输出须同时标明 `host_config_validity=unverified`、`session_catalog_observed=false`；`on` 仅表示所声明范围的本地规则未禁用此 placement，不能承诺 Hermes 会加载或列出它。该命名和下面的 unknown 策略是 SkillRoster 的证据边界建议，不是声称上游已有此 API。

1. 支持有效 YAML mapping、typed `skills` mapping、普通 disabled 字符串/字符串列表（排除 list-literal string 歧义），以及有效 frontmatter host-OS 字符串/列表。缺失配置与读取失败分别记录，冻结存在性、配置 digest、入口 digest、相关 root identity；任何变化需重新 Scan。
2. 禁用按**验证过的入口自身声明名**精确、区分大小写匹配；缺省 name 可使用已验证 dirname。配置名字 trim，不主动 trim frontmatter name。长度超过 list 表面的 64 字符、非字符串 name、相对 path/目录 alias/namespace/同名消歧可能改变匹配时，对受影响 placement 返回 unknown。不要把旧版不支持的路径匹配伪装成统一能力。[I list][i-list]、[U catalog][u-catalog]。
3. 普通名字的 global disabled 命中可证明此局部规则为 off；无需知道当前 channel。未命中 global 时，若任何有效 channel-specific 列表可能禁用此 placement、而目标 channel 未被明确提供，则为 unknown；不擅自默认 `cli`。若所有渠道规则对该 placement 均无影响，可在所声明的局部范围内记录 on。`hermes-agent` 被禁用、list-as-string 或版本相关 path 匹配必须保留版本或 unknown。
4. host-OS gate 与 channel gate 分开计算。正常 macOS/Linux/Windows 的 OS 不匹配可以给出局部 off；Android/Termux 或异常类型为 unknown。`environments`、apps、toolset/tool 条件、session channel gates 无观测依据时保持未验证，不能推断为满足。若选择做“包含这些条件的默认 offer 投影”，相关 placement 必须 unknown；若明确限定为 global+OS 规则投影，则另列这些未投影条件，`on` 仍只表达局部允许。
5. `external_dirs`/`create_dir`/project trust/HERMES_HOME overrides 的存在与 digest 是本地 scope 事实，不自动加入可信可写根。若它们能影响名称消歧或目标 root，不能据 local winner 推断最终 catalog；保留未验证来源与影响项。明确用户给出的任意 `--root` 也不自动继承 supplied-Home 原生用户配置语义。[I roots][i-roots]、[U roots][u-roots]。
6. disabled、unsupported 或 unknown entries 保留 Library inventory；off 不授权删除，unknown 不授权绕过限制的 Core/迁移/retarget。Scan/Report/Find/Plan/Apply 应共享同一冻结证据，而不是按不同入口猜测另一份可见性。

因此最小兼容目标可以是“普通声明名的 global 禁用 + 正常桌面 OS 平台限制 + 不确定条件显式披露”，但不能称“完整 Hermes native visibility”。若需要覆盖 essential、path alias、channel、root precedence 等差异，应先冻结支持版本与具体表面，再用合成配置的原生隔离回执验收。

[i-disabled]: https://github.com/NousResearch/hermes-agent/blob/76d832d3857551a029c4b39c23945eb47c16fe5b/agent/skill_utils.py#L436-L480
[u-disabled]: https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/agent/skill_utils.py#L275-L314
[i-ui]: https://github.com/NousResearch/hermes-agent/blob/76d832d3857551a029c4b39c23945eb47c16fe5b/hermes_cli/skills_config.py#L28-L83
[u-ui]: https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/hermes_cli/skills_config.py#L12-L53
[i-list]: https://github.com/NousResearch/hermes-agent/blob/76d832d3857551a029c4b39c23945eb47c16fe5b/tools/skills_tool.py#L673-L780
[u-list]: https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/tools/skills_tool.py#L229-L282
[i-prompt]: https://github.com/NousResearch/hermes-agent/blob/76d832d3857551a029c4b39c23945eb47c16fe5b/agent/prompt_builder.py#L1739-L1781
[u-prompt]: https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/agent/prompt_builder.py#L1423-L1460
[u-catalog]: https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/agent/skill_utils.py#L448-L505
[i-gates]: https://github.com/NousResearch/hermes-agent/blob/76d832d3857551a029c4b39c23945eb47c16fe5b/agent/skill_utils.py#L190-L387
[u-gates]: https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/agent/skill_utils.py#L67-L192
[i-conditions]: https://github.com/NousResearch/hermes-agent/blob/76d832d3857551a029c4b39c23945eb47c16fe5b/agent/skill_utils.py#L678-L710
[u-conditions]: https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/agent/skill_utils.py#L731-L740
[u-prompt-conditions]: https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/agent/prompt_builder.py#L1179-L1197
[i-session]: https://github.com/NousResearch/hermes-agent/blob/76d832d3857551a029c4b39c23945eb47c16fe5b/gateway/session_context.py#L374-L397
[i-hint]: https://github.com/NousResearch/hermes-agent/blob/76d832d3857551a029c4b39c23945eb47c16fe5b/agent/prompt_builder.py#L1666-L1678
[u-hint]: https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/agent/prompt_builder.py#L1232-L1242
[i-explicit-helper]: https://github.com/NousResearch/hermes-agent/blob/76d832d3857551a029c4b39c23945eb47c16fe5b/tools/skills_tool.py#L647-L670
[u-explicit-helper]: https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/tools/skills_tool.py#L198-L216
[i-view]: https://github.com/NousResearch/hermes-agent/blob/76d832d3857551a029c4b39c23945eb47c16fe5b/tools/skills_tool.py#L1365-L1386
[u-view]: https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/tools/skills_tool.py#L623-L631
[i-config]: https://github.com/NousResearch/hermes-agent/blob/76d832d3857551a029c4b39c23945eb47c16fe5b/agent/skill_utils.py#L401-L433
[u-config]: https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/agent/skill_utils.py#L220-L260
[i-roots]: https://github.com/NousResearch/hermes-agent/blob/76d832d3857551a029c4b39c23945eb47c16fe5b/agent/skill_utils.py#L499-L590
[u-roots]: https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/agent/skill_utils.py#L337-L426
[i-prompt-parser]: https://github.com/NousResearch/hermes-agent/blob/76d832d3857551a029c4b39c23945eb47c16fe5b/agent/prompt_builder.py#L1606-L1630
[u-prompt-parser]: https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/agent/prompt_builder.py#L1165-L1176
