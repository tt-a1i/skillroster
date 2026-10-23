# Hermes 本地可见性与治理保护验收

日期：2026-10-23。本轮目标是修复已复现的局部规则偏差：Hermes 原生已过滤的 global-disabled 或 OS 不兼容技能，不应仍被 SkillRoster 宣称为已知默认曝光；治理也不能绕过这些限制。此验收不代表完整 session catalog 或模型调用质量。

## 原生对照

已安装 Hermes 为 0.20.0，源码 commit
`76d832d3857551a029c4b39c23945eb47c16fe5b`。实验仅调用原生
`tools.skills_tool._find_all_skills()` 与
`agent.prompt_builder.build_skills_system_prompt()`；没有运行模型。
每个原生子进程使用最小环境和临时 HOME/HERMES_HOME，未读取或修改真实用户配置。
prompt builder 的缓存只落在临时目录。

Fixture 根为
`/var/folders/25/mpff0p7j7slgl5fj961tl7480000gp/T/skillroster-hermes-visibility-u6u46nj2`。
四个包及规则如下；原始文件、`native-catalog.json`、
`native-prompt-index.txt`、`scan.json` 和 `payload-before.json` 均在该目录保留。

| 包目录 | 声明名 | 规则 | 原生 list | 原生 prompt index |
| --- | --- | --- | --- | --- |
| `ordinary` | `ordinary` | 无 | 有 | 有 |
| `disabled` | `blocked` | `skills.disabled: [blocked, renamed-folder]` | 无 | 无 |
| `wrong-platform` | `wrong-platform` | `platforms: [windows]`，本机 macOS | 无 | 无 |
| `renamed-folder` | `renamed` | 目录别名被列入 disabled | 有 | 无 |

修复前 SkillRoster 将四个 placement 全部计入默认曝光。原生 list
只有两个名字，而 prompt index 只有一个。后者直接证明“列表”和“模型提示索引”
不能混为一个已确认的运行时状态。

修复后的第一轮对照保存于 `state-after-hs4zv0bb`：`ordinary=on`、
`blocked=off`、`wrong-platform=off`、`renamed=unknown`。
两个 Off 不计曝光；别名差异没有被错误归类成确定启用或确定禁用。

## 投影合同

实现保留目录 inventory，增加 `hermes_visibility`：`on` 表示本轮已观察的
本地规则允许，`off` 表示已确认的普通 global 名称禁用或桌面 OS 限制，
`unknown` 保留未确认状态。Off 不计默认曝光；Unknown 仍计可能曝光。
显式 Find 读取仍由 SkillRoster 的内容信任、字节身份和权限合同控制。

投影只使用 supplied Home 的 `.hermes/config.yaml` 和逐 placement 验证过的
入口字节。配置以安全的有界只读方式读取，不跟随配置文件链接；冻结配置摘要、
存在性、根身份和 host OS。没有保存完整配置、无关值或密钥。
YAML 解析使用 `yaml_serde 0.10.7`，另限制 8,192 节点、64 层、1 MiB
累计展开 scalar，避免小输入经 alias 放大；简单 anchor 和包含 `*`/`&`
的普通字符串不因此被错误拒绝。新增依赖为该库、`libyaml-rs 0.3.0` 与
`ryu 1.0.23`，未更新其他依赖；其声明的最低 Rust 版本均不高于本项目 1.85。
依赖的定位与版本声明见 [维护者仓库](https://github.com/yaml/yaml-serde)
和本轮冻结的 Cargo.lock；实际编译测试使用本机 Rust 1.99.1，未执行 1.85 构建。

版本差异、目录别名、潜在渠道禁用、运行时环境/工具条件和其他 root 优先级
保持 Unknown。`session_catalog_observed=false`、
`host_config_validity=unverified` 明确限制了结论范围。
[固定版本研究](../research/hermes-native-visibility.md) 记录了判定依据和来源。

独立审查另外复现了三个解析边界：PyYAML 1.1 把未加引号的日期名解释为
date；额外根列表的非字符串项会被原生 loader 字符串化；多个开头 BOM
可能改变配置键。它们不能成为确定 On/Off 的依据。投影对这些情况返回
Unknown。因为当前解析器不保留 scalar 的引号样式，`YYYY-MM-DD` 形状的
带引号名字也保守返回 Unknown；普通非日期字符串和简单 anchor 仍受支持。
这些输入使用安装版 Hermes 的 PyYAML 6.0.3 `CSafeLoader` 在临时目录核对。

## 治理与恢复

- Core 创建检查目标逻辑路径与迁移前验证过的来源 metadata；不能重新暴露 Off 或 Unknown。
- 来源更新、Library 合并/托管、隐式 retarget 和 Bootstrap 入口替换检查所有受影响的 Hermes consumer，包括只请求调整另一个 Agent 的情况。
- 移动父 Skill 时还检查不同 Skill 身份的嵌套 consumer；没有可信路径保留或 retarget 的计划在准备和 Apply 阶段均拒绝，防止 metadata 不变却断链。多跳链接可能经过待移除的别名，因此即使 canonical 内容仍保留，相关别名移除也会保守拒绝；本轮未实现完整链接链依赖证明。
- 已知 On/Off 必须保持；无法证明的相关改动拒绝。配置文件本身不被修改。
- 首次 Bootstrap 的新入口同样必须被本地规则允许；配置禁用 `skillroster` 或渠道条件不明时，Setup 不产生安装 Plan。普通空目录 Setup/Apply/Undo 仍可完成。
- 缺失合同、配置/host OS/根身份变化要求一个 Scan 恢复动作；Ready Apply 在写入和状态转换前拒绝。只有 sessions、尚无 Skills 根的 Bootstrap 场景也受保护。
- 历史 Plan 可查看；冻结合同过旧时详情给出 Scan continuation。配置漂移检测留在执行入口，查看历史记录不会因当前文件不可用而失败。

## 验证记录

最终构建的对照保存在同一 fixture 根的 `state-final-v3-7ky3fswf`，
包含 Scan、Report、原始 payload 和 `comparison.json`。
合同为 `hermes-user-skill-local-rules-v3`，仍得到 1 On、2 Off、1 Unknown；
该构建 SHA-256 为
`2b376a4e7f761027779a4f12f0cfedf119e12417c144e36801b943b1019476c8`。
原生输入没有变动：

| 文件 | SHA-256 |
| --- | --- |
| `native-catalog.json` | `27140a5bc8de6d3549cc4322d8e80e0081a5a8a49a5cfab1925f396ea3ee5a1a` |
| `native-prompt-index.txt` | `5a0a7e0fca50db16961f358f000644e7316692d8d7f7f8fc1cc919b887edf811` |
| `payload-before.json` | `1b0165393e3f1bffd1678f5525dc8a82c768dba8656548a1aca7c0ed3bc08ec7` |

首次 Bootstrap 的受限 fixture 保存在
`/var/folders/25/mpff0p7j7slgl5fj961tl7480000gp/T/skillroster-hermes-new-bootstrap-cwjeeu3u/state-final-v3-o8055qy6`。
Scan 后 Setup 返回 `native_visibility_blocks_roster_change`、`visibility=off`、
`files_changed=false`，未生成安装 Plan。

首轮完整回归揭示了根身份检查过严的问题：记录所有 Agent/Library 根的
POSIX ctime，会把正常目录内容变化误判为 Hermes 根替换。修复将范围限定为
Hermes 用户根，冻结 canonical path 与稳定对象身份（POSIX dev/inode 和可用
birthtime，Windows volume/file index/creation time）。普通增删文件不触发
此根身份门禁；真实根替换与 alias retarget 仍必须重扫。包内容漂移另由既有
内容指纹和 Plan 校验处理。

最终 `bash scripts/ci-full-check.sh` 通过：611 项 Rust 测试、152 项 Node
harness 测试，保留 1 项既有 ignored Rust 测试。格式、strict Clippy、README
证据、安装面和 release archive README 检查均通过；另执行 `cargo build --locked`
和 `cargo run --locked -- --help`。完整日志为
`/tmp/skillroster-hermes-visibility-final-ci-20261023.log` 与
`/tmp/skillroster-hermes-visibility-final-help-20261023.log`。
紧凑 Scan 摘要保持既有体积预算，完整 Scan/Report 仍保留规则范围信息。

两位独立 reviewer 分别检查 parser/Scan 合同与 app/Roster 治理接线。最终复核
没有未解决 finding。除三个解析边界外，审查复现并关闭了父包迁移使跨 Skill
Hermes consumer 断链的问题，覆盖直接依赖和未扫描中间 alias 的多跳依赖。
修复前真实 Ready Plan 的 Apply 在写前拒绝，Plan 保持 Ready、Receipt 数为 0、
原始字节和链接保持完整。其中 direct fixture 的旧 v2 合同首先要求重扫；
为独立验证移动守卫，仅在其临时数据库把版本标记改为 v3 后再次拒绝。
alias-chain fixture 本身即为 v3，原样重放即拒绝。合法 Library Hosted
移动并恢复原父路径的对照成功，Hermes 仍为 On。

路径依赖拒绝使用已有 `roster_dependent_source_conflict` 分类，给出
`dependent_source_would_break`、相关 consumer/移动路径和
`preserve_or_retarget_dependent_source` 恢复方向；不会引导用户通过修改原生配置
或反复 Scan 解决路径问题。普通原生 Off/Unknown 仍使用原可见性 blocker。

本轮实际运行环境为 macOS；未据此证明 Windows/Linux 原生运行、Rust 1.85
编译或完整 Hermes session catalog。所有 mutation 验收只发生在隔离 fixture，
未更新用户安装的 CLI、真实 Agent 文件或配置，也未发布新版本。
