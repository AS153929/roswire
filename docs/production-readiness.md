# RosWire 生产级稳定版验收门槛

> 最后更新：2026-05-18
> 当前结论：RosWire 已进入 **MVP / Beta 候选**，但在本页列出的 blocker 完成前，**不得声明生产级稳定版**。

本文定义从 MVP/Beta 进入生产级稳定版（production-stable）的硬性门槛。目标是让维护者、Agent 和使用者对“可以试用”和“可以生产托管”之间的边界没有歧义。

## 状态边界

| 状态 | 可以做什么 | 不应做什么 | 判定依据 |
| --- | --- | --- | --- |
| MVP | 验证 JSON-first CLI、配置、协议路由、文件工作流与自描述能力 | 不承诺跨平台安装体验，不承诺真实设备矩阵覆盖 | 本地门禁通过，功能 checklist 主要能力闭环 |
| Beta | 在实验室、非关键设备或人工监督下试用；收集 RouterOS v6/v7 与平台差异 | 不建议无人值守生产变更，不建议作为唯一备份/恢复路径 | MVP 完成，真实设备验收仍在进行 |
| Production-stable | 可发布正式版本，文档可面向生产自动化用户推荐 | 不能跳过 release、keychain、真机矩阵和安全门槛 | 本页所有 P0 blocker 关闭，发布物可复现且有校验 |

## P0 blocker 清单

以下 issue 是生产级声明的阻塞项；任何一个未关闭，都只能称为 MVP/Beta。

| Blocker | 必须产出 | 关闭条件 |
| --- | --- | --- |
| [#60 建立 RouterOS 真机/CHR 验收矩阵](https://github.com/AS153929/roswire/issues/60) | RouterOS v6/v7、API/API-SSL/REST、SSH/SFTP/SCP、导入/导出/备份工作流的验收记录 | 至少覆盖一套 v6 与一套 v7/CHR；失败项有结构化 issue 或明确降级说明 |
| [#61 发布工程与安装文档](https://github.com/AS153929/roswire/issues/61) | GitHub Releases 流程、平台二进制、校验和、安装/升级/卸载文档 | release artifact 可下载、可校验、可在目标平台 smoke test 通过 |
| [#62 多平台 keychain smoke 测试](https://github.com/AS153929/roswire/issues/62) | macOS Keychain、Linux Secret Service、Windows Credential Manager 的 smoke/fallback 记录 | PR documented fallback 通过；发布前 macOS `workflow_dispatch` 原生 smoke 或本地 CI runner smoke 有记录 |

## 发布前必须满足的质量门

每个 release candidate 都必须在干净工作区通过：

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- `cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 85`
- `cargo build --release`

CI 与本地结果必须一致。若本地环境无法运行覆盖率或平台 smoke，PR/Release 说明必须明确使用的替代验证来源。

## 真机/CHR 兼容性门槛

生产级前，必须有可追溯记录覆盖以下维度：

### RouterOS 与协议

- RouterOS v6 原生 API：登录、只读 print、典型写操作、错误 trap 归一化。
- RouterOS v7 原生 API：登录、只读 print、典型写操作、与 REST 的语义差异。
- RouterOS v7 REST：GET/PUT/PATCH/DELETE/POST JSON、HTTP 错误映射。
- `--protocol auto`：REST 优先、API-SSL/API fallback、认证失败不静默切换。
- API-SSL：TLS 证书异常、端口关闭、认证失败的错误分类。

### 文件与 SSH

- SSH host key 必填与 mismatch 路径。
- Password auth、key auth、加密 key passphrase（`ROS_SSH_KEY_PASSPHRASE` / profile secret）。
- SFTP 可用时使用 SFTP；SFTP subsystem 不可用时 SCP fallback；两者都不可用时返回明确 `FILE_TRANSFER_FAILED`。
- `file upload` / `file download`、`import`、`export download`、`backup download` 的端到端路径。
- 覆盖策略、超时、有限 retry、`.part` finalize、cleanup 失败路径。
- `--ensure-ssh` / `--restore-ssh` 对 `/ip service ssh` 的快照、白名单合并与恢复。

## 安全门槛

生产级发布不得放宽以下安全规则：

- 默认非交互；不得新增密码、passphrase 或确认提示。
- 成功输出只写 `stdout`；错误、debug 与诊断只写 `stderr`。
- 密码、token、secret、private key、passphrase、本地绝对敏感路径必须脱敏。
- SSH host key 必须非交互校验；未知 host key 不允许自动信任。
- SSH 白名单不得默认写入 `0.0.0.0/0` 或 `::/0`。
- `--ensure-ssh` 只能在用户显式请求时修改 RouterOS SSH 服务。
- 配置目录与 `config.toml` 权限过宽时必须失败。
- keychain 不可用时必须返回结构化错误，不自动退回明文 secret。
- Release artifact 必须提供校验和；安装文档必须要求用户校验下载内容。

## 发布物要求

生产级 release 至少包含：

- GitHub Release tag 与 changelog。
- macOS、Linux、Windows 目标平台的二进制或明确不支持说明。
- SHA256 checksums 文件。
- 安装、升级、卸载文档。
- 最小权限配置示例：env、profile、keychain、SSH host key、allow-from。
- 已知限制：未覆盖 RouterOS 版本、未验证平台、需要用户手动确认的设备侧配置。

## Go / No-Go 决策

发布前必须逐项回答：

- [ ] #60 已关闭，真实设备/CHR 矩阵可追溯。
- [ ] #61 已关闭，release artifact 可安装且可校验。
- [x] #62 已关闭，keychain 多平台 smoke/fallback 可追溯。
- [ ] README 不再包含过期“仅规划”措辞，示例与实现一致。
- [ ] `docs/feature-checklist.md` 中生产级相关未完成项均有 issue 链接或已关闭。
- [ ] 最近一次完整本地门禁与 CI 门禁均通过。
- [ ] 安全门槛没有例外；若有例外，必须列为 release blocker 而不是 release note。

## 当前结论

截至 2026-05-18：

- MVP 功能闭环已经完成，可进入 Beta/实验室试用。
- Production-stable 仍被 #60、#61 阻塞；#62 的 keychain smoke/fallback 方案见 [`keychain-smoke.md`](keychain-smoke.md)。
- 在这些 blocker 关闭前，README、Release note 和安装文档必须使用 “MVP/Beta” 表述，不得使用 “production-ready” 或 “stable” 表述。
