# RouterOS / CHR 验收矩阵

> 最后更新：2026-05-18
> 关联 issue：[#60](https://github.com/AS153929/roswire/issues/60)
> 当前状态：已提供可重复执行的验收矩阵与 harness；真实 RouterOS/CHR 运行记录仍需在具备设备后补齐。

本文定义 RosWire 进入 production-stable 前需要完成的 RouterOS v6/v7、协议、SSH/SFTP/SCP 和文件工作流验收矩阵。

## 需要的外部环境

Issue #60 不能只靠本地单元测试完成，必须准备真实 RouterOS/CHR 或真机：

| 目标 | 最低要求 | 说明 |
| --- | --- | --- |
| RouterOS v6 CHR/真机 | API 开启，建议 API-SSL 开启 | 验证 v6 登录、classic API 字段和错误差异 |
| RouterOS v7 CHR/真机 | API、API-SSL、REST 开启 | 验证 auto 协议优先级、REST 映射、classic fallback |
| SSH 文件传输目标 | SSH 服务、host key 指纹、窄 `allow-from` | 验证 SFTP、SCP fallback、host key、文件工作流 |
| 低权限账号 | 只读或受限权限 | 验证 `ROS_API_FAILURE` / 权限不足路径不泄漏 secret |

建议使用可重置的 CHR/lab 设备，避免在生产路由器上直接运行 live 文件工作流。

## 凭据与环境变量

所有 secret 都必须通过环境变量或 profile secret 提供，不要写进脚本参数或 shell history。

最小环境变量示例：

```bash
export ROS_HOST="198.51.100.10"
export ROS_USER="roswire-ci"
export ROS_PASSWORD="replace-with-secret"
export ROS_PROTOCOL="auto"
export ROS_ROUTEROS_VERSION="auto"
export ROS_TRANSFER="ssh"
export ROS_SSH_HOST_KEY="SHA256:replace-with-routeros-host-key"
export ROS_SSH_ALLOW_FROM="203.0.113.10/32"
```

如使用 SSH key：

```bash
export ROS_SSH_USER="roswire-ssh"
export ROS_SSH_KEY="$HOME/.ssh/roswire_acceptance_ed25519"
export ROS_SSH_KEY_PASSPHRASE="replace-with-passphrase-if-needed"
```

## Harness

构建 release binary 后运行：

```bash
cargo build --release --locked
ROSWIRE_ACCEPTANCE_REMOTE=1 \
ROSWIRE_ACCEPTANCE_OUT=target/acceptance/routeros-v7-rest \
./scripts/routeros-acceptance.sh
```

脚本默认只运行本地自描述与只读远端命令。Live 文件工作流默认跳过；只应在 disposable lab 目标上开启：

```bash
ROSWIRE_ACCEPTANCE_REMOTE=1 \
ROSWIRE_ACCEPTANCE_RUN_FILE_WORKFLOWS=1 \
ROSWIRE_ACCEPTANCE_OUT=target/acceptance/routeros-v7-live \
./scripts/routeros-acceptance.sh
```

输出结构：

```text
target/acceptance/<target>/
├── <case>.meta.json      # case 名称、exit_code、命令形态或 skip 原因
├── <case>.stdout.json    # 成功输出；失败时通常为空
└── <case>.stderr.json    # 结构化错误；成功时通常为空
```

脚本不会启用 `set -x`，也不会打印 secret 环境变量值。

## 必跑矩阵

| ID | 目标 | 协议/路径 | 必须验证 |
| --- | --- | --- | --- |
| M60-01 | 本地无设备 | `doctor` / `commands` / `schema command` | 输出稳定 JSON，默认不访问 RouterOS |
| M60-02 | v6 API | `--protocol api` | 登录、`system resource print`、`interface print`、错误上下文 selected protocol |
| M60-03 | v6 API-SSL | `--protocol api-ssl` | TLS 连接、证书异常分类、认证失败不回落 |
| M60-04 | v7 REST | `--protocol rest` | REST GET/PUT/PATCH/DELETE/POST JSON 映射与 HTTP 错误映射 |
| M60-05 | v7 auto | `--protocol auto` | REST 可用时优先 REST；REST 不可用/无映射时回落 classic API |
| M60-06 | SSH password | SSH/SFTP | host key 必填、password auth、上传/下载 checksum |
| M60-07 | SSH key | SSH/SFTP/SCP | key auth、加密 key passphrase、SFTP 不可用时 SCP fallback |
| M60-08 | 文件工作流 | import/export/backup | 临时文件、`.part` finalize、cleanup、覆盖策略、超时/重试 |
| M60-09 | 权限不足 | 低权限账号 | 结构化 `ROS_API_FAILURE` / `AUTH_FAILED`，不泄漏 secret |
| M60-10 | 网络失败 | 关闭端口/错误 host | `NETWORK_ERROR`，stdout 为空，stderr 单一 JSON |

## 结果判定

每个 case 记录：

- 目标类型：CHR/真机、RouterOS 版本、架构、board name。
- 协议：requested 与 selected。
- 命令：只记录脱敏命令形态，不记录 secret 值。
- Exit code。
- stdout/stderr payload 路径。
- 是否稳定复现。
- 如果失败：是否符合预期错误码；是否需要新 issue。

## 失败转 issue 规则

真实设备发现差异时，不应在验收记录里长期备注绕过；应创建独立 follow-up issue。每个 issue 至少包含：

- RouterOS 版本与架构。
- 协议和命令。
- 脱敏 stdout/stderr payload。
- 期望行为与实际行为。
- 是否阻塞 production-stable。

## 当前待补记录

- [ ] v6 CHR / API / API-SSL。
- [ ] v7 CHR / REST / API-SSL / API。
- [ ] SSH password auth + SFTP。
- [ ] SSH key/passphrase + SCP fallback。
- [ ] import/export/backup live 工作流。
- [ ] TLS 证书异常、认证失败、网络失败、权限不足。

这些记录补齐前，#60 仍保持打开，`docs/production-readiness.md` 中的 production-stable 判定仍为阻塞。
