# AGENT 开发规范

本文定义 `roswire` 仓库内 AI Agent 与人类开发者共同遵循的开发规范。所有实现工作必须优先遵循 [`docs/develop-plan.md`](docs/develop-plan.md) 中的产品与架构约束。

## 1. 基本原则

`roswire` 是给 Agent 使用的 RouterOS CLI 桥接工具，因此开发时必须优先保证：

- **机器可解析**：成功结果写入 `stdout`，错误和诊断写入 `stderr`，默认使用稳定 JSON。
- **非交互**：禁止引入需要人工输入的流程，包括密码提示、确认提示和分页器。
- **确定性**：默认输出不得包含时间戳、随机 ID、非稳定 map 顺序或未脱敏路径。
- **安全默认值**：secret、SSH 私钥、RouterOS 密码、backup 内容不得进入日志、错误、测试快照或示例命令。
- **小步提交**：每个变更应聚焦一个可验证目标，避免“顺手重构半个世界”。

## 2. 测试驱动开发（TDD）是默认工作流

所有新功能、修复和重构默认采用 TDD：

1. **Red**：先写一个失败测试，准确描述期望行为或已知 bug。
1. **Green**：只写足够让测试通过的最小实现。
1. **Refactor**：在测试保护下清理结构、命名和重复代码。
1. **Verify**：运行格式化、lint、测试和覆盖率检查。

没有测试就直接实现功能，视为例外而不是常态。例外必须在 PR、提交说明或相关文档中解释原因。

### 2.1 不可测试功能必须先反省

如果一个功能声称“无法被测试覆盖”，必须先停下来反省设计，而不是继续堆实现。

反省顺序：

1. 是否把副作用和纯逻辑混在一起了？应先抽出纯函数或 trait 边界。
1. 是否缺少 mock/fake transport？协议、文件系统、钥匙链、SSH、HTTP 都必须有可替换边界。
1. 是否依赖真实时间、随机数、环境变量或全局状态？应注入 clock、rng、env provider。
1. 是否把外部系统当成唯一验证方式？应拆分为单元测试、契约测试和 gated integration test。
1. 是否功能范围过大？应拆成可测试的小步骤。

只有完成以上反省后，仍无法自动化覆盖的内容，才允许记录为人工验证项。人工验证项必须包含：

- 无法自动化的原因。
- 风险影响。
- 已经覆盖的替代测试。
- 人工验证步骤。
- 后续可自动化的计划。

## 3. 覆盖率要求

实现代码出现后，仓库应引入覆盖率工具，Rust 项目推荐使用 `cargo llvm-cov`。

最低要求：

- 全仓库行覆盖率不得低于 **85%**。
- 核心纯逻辑模块覆盖率不得低于 **90%**，包括 CLI 解析、配置合并、错误序列化、协议路由、schema 合并和路径映射。
- 涉及安全边界的模块必须覆盖成功、失败和脱敏路径，包括 secret、日志、SSH host key、白名单合并、错误上下文。
- 新增代码不得显著降低覆盖率；如果覆盖率下降，必须说明原因并补测试。

建议命令（项目初始化后启用）：

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo llvm-cov --workspace --all-features --fail-under-lines 85
```

## 4. 测试分层

### 4.1 单元测试

必须优先覆盖纯逻辑：

- CLI path/action 解析。
- 配置优先级：CLI > 环境变量 > profile > 默认值。
- `~/.roswire` / `ROSWIRE_HOME` 路径解析与权限检查。
- secret 引用、`same-as` 循环检测、明文 secret 安全限制。
- JSON 错误结构、字段顺序、脱敏规则。
- RouterOS path 映射和参数转换。
- v6/v7 方言选择逻辑。
- REST/API/SSH 后端路由决策。
- 静态 schema、远端 overlay、cache key 和失效规则。

### 4.2 快照测试

稳定 JSON 是 `roswire` 的公共契约，以下输出应使用快照测试或等价断言：

- `help --json`
- `commands --json`
- `schema command ... --json`
- `config inspect --json`
- 标准错误 JSON
- Agent 自愈提示

快照中不得包含真实密码、token、私钥路径、真实公网 IP、完整本地绝对路径或不可稳定字段。

### 4.3 集成测试

外部依赖测试必须显式隔离：

- RouterOS CHR / 真机测试默认 `ignore` 或通过环境变量开启。
- keychain 测试不得污染真实用户凭据；需要测试专用 service/account。
- SSH 文件传输测试必须使用测试设备、临时路径和清理策略。
- REST/API 测试必须覆盖认证失败、网络失败、协议不可用和 RouterOS trap/error。

### 4.4 契约测试

协议层需要用 fake transport 固定输入输出：

- classic API sentence 编解码。
- `!re` / `!done` / `!trap` / `!fatal` 解析。
- REST 状态码到标准错误的映射。
- RouterOS v6/v7 登录流程分支。
- `auto` 协议探测顺序和认证失败短路。

## 5. 设计可测试性要求

实现前应主动设计测试缝隙：

- 文件系统访问通过抽象或临时目录测试。
- 时间通过 clock provider 注入。
- 环境变量通过 env provider 注入。
- 网络通过 transport trait 注入。
- keychain 通过 secret backend trait 注入。
- SSH 通过 transfer backend trait 注入。
- 日志写入通过 sink 或临时目录验证。

禁止把网络、文件系统、环境变量、真实时间和随机数直接散落在业务逻辑中。

## 6. 错误与日志规范

- 所有错误必须有稳定 `error_code`。
- 错误 payload 默认不得包含时间戳、随机 trace id 或未脱敏绝对路径。
- `stdout` 不得输出错误。
- `stderr` 不得混入成功数据。
- debug 日志也必须脱敏。
- 测试必须覆盖敏感字段不会泄漏。

## 7. Agent 自描述规范

Agent 依赖 `roswire` 自描述能力生成安全命令，因此：

- 每个公开命令必须登记到命令目录。
- 每个公开命令必须有参数结构、示例、输出 schema、错误码和自愈提示。
- 会修改设备状态的命令必须声明 `side_effects` 和 `idempotency`。
- 文件传输命令必须声明 SSH 前置条件、临时文件、清理策略和安全限制。
- 动态 schema 只能作为远端能力 overlay；不能覆盖静态安全策略。

## 8. 开发前检查清单

开始实现任何功能前，先确认：

- 这个功能在 `docs/develop-plan.md` 中有对应设计，或已经补充设计。
- 已写出至少一个失败测试。
- 已明确是否需要 mock、fake、fixture 或 gated integration test。
- 已确认不会破坏 stdout/stderr 契约。
- 已确认不会泄漏 secret。

## 9. 完成前检查清单

提交或结束任务前，必须确认：

- 新增/修改行为有自动化测试覆盖。
- 覆盖率满足阈值，或已记录合理例外。
- `cargo fmt --check` 通过。
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` 通过。
- `cargo test --workspace --all-features` 通过。
- 文档与示例同步更新。
- 错误输出和日志仍然脱敏。

## 10. 例外处理

允许短期例外，但必须显式记录。

例外记录至少包含：

```text
测试例外：<功能名>
原因：<为什么暂时无法自动化覆盖>
风险：<可能坏在哪里>
替代验证：<已有哪些单元/契约/人工验证>
后续计划：<如何把它变成自动化测试>
负责人：<谁需要跟进>
```

没有记录的测试缺口视为开发质量问题。
