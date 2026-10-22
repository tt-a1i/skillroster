# Hermes Skills：原生发现与默认可见性边界

日期：2026-10-22。状态：官方源码与文档的只读研究；本文没有安装 Hermes、运行模型、修改用户配置或实现 adapter。源码观察与本机原生隔离实验分别记录，不能互相代替。

## 版本与证据范围

- 查询官方 `commits/main` 时，上游 HEAD 为 **`92663ae8054ae81f207f46d83512d3f879dc698d`**，commit 时间 **2026-10-22T15:28:08Z**。下文源码链接均固定到这个 revision；它的 `pyproject.toml` 声明 **0.0.0**，所以不能把 HEAD 叫作某个发布版本。[commit](https://github.com/NousResearch/hermes-agent/commit/92663ae8054ae81f207f46d83512d3f879dc698d)、[包定义](https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/pyproject.toml#L5-L15)。
- 同次官方 Latest release 查询返回 **v0.22.4**，发布时间 **2026-10-21T18:49:24Z**，target `4d04303818b61bf3aa09278af48356315515965b`。该 release 不等于上述 main HEAD，也不等于用户安装版本；本文未另外验证 release tag 的 loader。[官方 release](https://github.com/NousResearch/hermes-agent/releases/tag/v0.22.4)。
- 本机版本由主调查单独核对：`/Users/tushaokun/.hermes/hermes-agent`，**0.20.0**，clean checkout **`76d832d3857551a029c4b39c23945eb47c16fe5b`**（2026-08-12）。本机实验必须使用这个 pin，不能把当前上游细节直接当作已安装行为。[对应官方 revision](https://github.com/NousResearch/hermes-agent/commit/76d832d3857551a029c4b39c23945eb47c16fe5b)。

## 发现规则：源码确认的差异

| 边界 | 当前上游事实 | 对本地事实投影的意义 |
| --- | --- | --- |
| 目录 symlink | `iter_skill_index_files` 使用 `os.walk(..., followlinks=True)`，最后按路径排序；该函数没有 visited-directory 集合、显式深度上限或 cycle guard | 不能继续沿用“所有 Hermes 必须 real copy，因为不跟随链接”的旧假设；发现可跟随链接也不构成 SkillRoster 的内容信任或治理授权 |
| 隐藏目录 | 只裁剪有限名称集合，并非排除所有 `.` 前缀 | `.backup`、`.disabled` 等任意名字不能自动当作不曝光；`.archive` 明确被排除 |
| 已有 package 内继续搜索 | 目录含 `SKILL.md` 时，仅停止进入它的 `references`、`templates`、`assets`、`scripts` 四个直接子目录 | `parent/examples/deep/SKILL.md` 或任意非四种 support 名称的深层入口仍可能发现；不能用“已有 package 后所有后代都是 support”概括 Hermes |
| 同名 category | 四种 support 名称仅在父目录声明 `SKILL.md` 时裁剪 | 根下 `scripts/tool/SKILL.md` 仍可发现；不能全局排除目录名 `scripts` |

以上均来自 [`iter_skill_index_files`](https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/agent/skill_utils.py#L859-L869) 与 [support 判定](https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/agent/skill_utils.py#L24-L52)，属于静态实现证据，未运行当前上游 walker。

完整排除集合为 `.git`、`.github`、`.hub`、`.archive`、`.curator_backups`、`.locks`、`.venv`、`venv`、`node_modules`、`site-packages`、`__pycache__`、`.tox`、`.nox`、`.pytest_cache`、`.mypy_cache`、`.ruff_cache`。`target` 不在此集合。catalog 还对发现路径的全部 lexical components 检查该集合，因此显式根本身位于被排除名称之下也可能被过滤。[常量](https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/agent/skill_utils.py#L24-L32)、[catalog 检查](https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/tools/skills_tool.py#L247-L268)。

## Roots、配置与 catalog

原生路径基于 profile-scoped Hermes home：`get_hermes_home()` 优先 context-local override，再环境 `HERMES_HOME`，再平台默认值；Skill 根为 `<home>/skills`，配置为 `<home>/config.yaml`。默认用户目录文档写作 `~/.hermes/skills`，不能据此忽略 profile 或进程覆盖。[home resolver](https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/hermes_constants.py#L188-L196)、[根与配置路径](https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/hermes_constants.py#L1273-L1280)。

共享搜索顺序为 **trusted project → local profile → `skills.create_dir` → `skills.external_dirs`**。external dirs 只纳入现存目录，展开 `~` 与环境变量，相对路径锚定 Hermes home，经 resolve 去重并排除 local root；create_dir 也参与读取，但目录必须存在。external 配置不等于只读权限，用户指示的 `skill_manage` 可以原地修改外部 Skills。[根解析源码](https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/agent/skill_utils.py#L337-L426)、[官方说明](https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/website/docs/user-guide/features/skills.md#L404-L464)。

Project roots 是最近 `.git` 项目根下 `.hermes/skills` 与 `.agents/skills`。只有 `skills.trusted_project_dirs` 中受信任的项目参与 catalog；`skills.project_discovery: false` 关闭此路径。project skill 还经过 quarantine scanner；`dangerous` 或扫描失败会被排除。根目录就是 OS home 的 dotfiles repo 被特殊拒绝，不应泛化为任意祖先目录累计发现。[project resolver](https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/agent/skill_utils.py#L513-L610)、[quarantine](https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/agent/skill_utils.py#L627-L661)。

文件存在不等于默认可见：`_skill_catalog` 读取 frontmatter，先解析名称冲突与优先级，再应用 `platforms`、`environments`、`requires_apps` 和 disabled 过滤。`skills.disabled` 与当前 session platform 的 `skills.platform_disabled` 取并集；essential `hermes-agent` 不在返回的 disabled 集合内。disabled 匹配 declared name 或解析后的 `load_name`，后者可为消歧相对路径。`skip_disabled=True` 是忽略禁用状态的管理视图，不等于普通模型 catalog。[catalog](https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/tools/skills_tool.py#L229-L282)、[disabled 解析](https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/agent/skill_utils.py#L275-L290)、[消歧与禁用匹配](https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/agent/skill_utils.py#L448-L505)。

同名 lower tier 被 shadow；同 tier 的不同内容不擅自选一个，使用可唯一解析的相对路径，不能解析时不提供 load name。`provably_same_skill` 也考虑同一 resolved entrypoint 或相同入口字节，因此它是 Hermes 自己的 catalog 消歧机制，不能冒充 SkillRoster 的完整 package identity。[身份与排名](https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/agent/skill_utils.py#L429-L505)。

`environments` 与 `requires_apps` 是 offer-time relevance gate；源码明确允许显式加载绕过这些 gate，而 `skill_view` 单独拒绝 disabled Skill。这与 SkillRoster 的 On-demand、Explicit-only、Archived 不是同一套状态。[relevance](https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/agent/skill_utils.py#L109-L192)、[显式加载检查](https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/tools/skills_tool.py#L623-L631)。

## 不能据此宣称的内容

1. 没有测当前上游、release 或所有 OS 上的 broken/cyclic symlink 行为；不得把 followlinks 当作 cycle safety 或对逃逸链接的授权。主调查复核的已安装 iterator 同样没有 visited-directory guard，因此不运行循环 fixture；其余差异仍应保持版本证据分离。
2. 本研究未复刻所有启动参数、gateway/session、插件 namespace、toolset 条件或运行时 overrides。`skills_list` 还会叠加 plugin 提供的 Skills；只扫描目录不是完整的活跃模型 catalog。[plugin 叠加](https://github.com/NousResearch/hermes-agent/blob/92663ae8054ae81f207f46d83512d3f879dc698d/tools/skills_tool.py#L289-L305)。
3. 本文没有把 upstream 解析规则引入 SkillRoster；内容路径信任、完整 fingerprints、不可变 Snapshot、Plan 与 Receipt 保持 SkillRoster 自己的边界。原生可见性结论应标识具体版本、观察范围与尚未核验的运行状态。
