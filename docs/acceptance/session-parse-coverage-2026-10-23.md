# 会话解析与使用证据覆盖验收

日期：2026-10-23。目标是修复“读取成功但解析失败仍被报告为覆盖完整”的偏差，
同时保留合法记录中的正面使用证据。本轮没有运行模型或修改真实 Agent 会话。

## 已复现的问题

基线为 `a131e2aab0a2251c8615eca6128e59bccce8f305`，本地二进制 SHA-256：
`2b376a4e7f761027779a4f12f0cfedf119e12417c144e36801b943b1019476c8`。
七组隔离 HOME/state 的输入、Scan、Report 与比较收据保存在：

`/var/folders/25/mpff0p7j7slgl5fj961tl7480000gp/T/skillroster-session-parse-hwkym73s`

每组只有一个 Codex Skill 和一个合成 session 文件。`before.json` 记录基线输出
和各输入的 SHA-256；`after-final.json` 记录最终修复后输出。输入文件未改动。
最终验证二进制 SHA-256 为
`5fd337f7fdc88cae711da422a6ae0ad5bd5834d9a8206ff0e35c9331c5dd94c0`。

| 输入 | 修复前 | 修复后 |
| --- | --- | --- |
| 坏 JSONL，包含 Skill 名称 | 分母可靠；产生一条 inferred/exposed | 分母不可靠；无 usage |
| 合法 loaded 记录与坏行混合 | 分母可靠；额外生成 inferred/exposed | 分母不可靠；仅保留合法 loaded |
| JSONL 字符串内有非法 UTF-8 | 分母可靠；替换坏字节后生成 observed/loaded | 分母不可靠；无 usage |
| 完整 JSON 文档后有非法尾部 | 分母可靠；抽取子对象生成 observed/loaded | 分母不可靠；无 usage |
| 合法普通消息，无 Skill 事件 | 分母可靠；无 usage | 保持原结果 |
| 合法未知事件形状 | 分母可靠；无 usage | 保持原结果 |
| 只有合法 JSON 空白的 JSONL | 分母可靠；无 usage | 保持原结果 |

这里的“分母可靠”只描述既定读取与语法验证范围，不证明能识别任意 Agent 事件，
也不证明某个 Skill 没有使用或应该移出 Roster。

## 修复合同

- JSONL 与结构化 `.log` 在原始字节上验证 UTF-8 和 JSON。坏记录不进入
  timestamp、结构化事件或按名称推断路径；合法邻行继续贡献证据。
- 完整 `.json` 必须作为一个文档通过解析。空内容、纯空白或损坏文档不能通过
  子对象提取伪装成完整读取。大文件的 bounded tail 仍可提供验证过的完整对象，
  同时保留原有采样与提取边界。
- 空白行只豁免 JSON 允许的空格、Tab、LF、CR；FF/VT 等非法字符会被拒绝。
- `file_parse_failure` 以文件为单位记录已观察到的受损文件数量，同一文件多个
  坏记录只计一次。原始坏内容和解析器错误片段不进入状态库。
- 新 Coverage 冻结 `parse_contract_version=1`。缺失或不支持该版本的历史事实
  保持 `legacy_unknown`，不能凭旧的 `limitations=[]` 声称可靠。
- Report 的 `session_coverage_presentation_version` 仅用于缓存展示兼容，
  不表示旧 Snapshot 已通过新解析检查。历史 Finding 的数字与文字同时保守更新；
  失去相关可靠覆盖的历史归档候选变为只读，并提示重扫。原 Snapshot 不被重写。
- 新 Scan 的 Report 只使用其自身冻结的 usage，不把 SQLite 历史 usage 拼回去；
  重新扫描正常数据可以恢复当前覆盖合同。

## 回归与边界

核心回归覆盖坏语法、UTF-8、合法邻行、无末尾换行、空白与空文件、完整 JSON、
bounded tail 中的合法对象、失败文件计数和合法文本 inference 的保留。
公共 CLI 回归覆盖 Scan、Report 各视图、历史 Finding、Coverage Evidence、
lifecycle export 与重新扫描恢复，并验证源字节不变、私有标记不进入状态库。

独立审查复现了 FF 空白漏口，以及历史 Finding 的旧完整摘要/归档依据与新
Coverage 相矛盾的问题。对应修复保留历史记录身份与存储内容，只调整公开的
当前解释和可行动性。

最终 `bash scripts/ci-full-check.sh` 通过：617 项 Rust 测试、152 项 Node
harness 测试，保留 1 项既有 ignored Rust 测试。格式、strict Clippy、README
证据、安装面与 release archive README 检查均通过；另执行
`cargo build --locked` 和 `cargo run --locked -- --help`。
日志为 `/tmp/skillroster-session-parse-final-ci-20261023.log` 和
`/tmp/skillroster-session-parse-final-help-20261023.log`。

独立 reviewer 使用原临时旧库复核两项发现均已闭合：FF 产生解析失败；历史
Five-stage 的 compact/full 摘要与 `complete 0/8` 一致；历史 archive candidate
均变为 `unknown / read_only / not_governable`。查询前后 Snapshot 存储文本一致。
最终局部复核没有未解决 finding。

本轮在 macOS 上运行。有效未知事件形状仍不等于已支持该事件语义；其他平台、
正在并发写入的真实会话和模型任务质量不在这份验收的证明范围内。
