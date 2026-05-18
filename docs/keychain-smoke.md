# 多平台 Keychain Smoke 测试

> 最后更新：2026-05-18
> 关联 issue：[#62](https://github.com/AS153929/roswire/issues/62)

本文记录 `roswire` keychain secret 后端的可复现 smoke 测试策略。目标不是替代单元测试，而是在 CI/发布流程中确认 OS 凭据库路径可用，或在平台不可用时确认有明确 fallback 说明。

## 平台矩阵

| 平台 | 后端 | CI 策略 | 生产级要求 |
| --- | --- | --- | --- |
| macOS | Keychain | PR CI 运行 documented fallback；`workflow_dispatch` 提供 `keychain-native-macos` 原生 round-trip smoke | 发布前必须运行原生 smoke：写入专用 test credential、读回、`config inspect` 不泄漏、清理 credential |
| Linux | Secret Service | 在 `ubuntu-latest` 上运行 documented fallback 校验 | GitHub-hosted runner 默认通常没有用户会话 Secret Service；发布前如启用真实桌面/session runner，应运行原生 smoke |
| Windows | Credential Manager | 在 `windows-latest` 上运行 documented fallback 校验 | 发布前应在 Windows Credential Manager 可用环境运行原生 smoke；若不可用，必须返回结构化错误 |

## 自动化测试入口

`tests/keychain_smoke.rs` 提供两个 ignored 测试；PR CI 自动运行 documented fallback，macOS 原生 smoke 通过手动 `workflow_dispatch` job 或本地命令运行：

```bash
ROSWIRE_KEYCHAIN_SMOKE=native cargo test --test keychain_smoke native_keychain_roundtrip_redacts_inspect_output -- --ignored --exact
ROSWIRE_KEYCHAIN_SMOKE=documented-fallback cargo test --test keychain_smoke documented_fallbacks_cover_linux_and_windows -- --ignored --exact
```

### 原生 smoke 做什么

`native_keychain_roundtrip_redacts_inspect_output` 会：

1. 使用临时 `ROSWIRE_HOME` 初始化配置。
2. 创建 `studio` profile。
3. 通过 `config secret set ... type=keychain value=<generated>` 写入专用测试凭据。
4. 直接用 `keyring` crate 读回同一个 `service/account`，确认 OS 凭据库确实保存了值。
5. 运行 `config inspect --json`，确认只暴露 `type=keychain` 与 `redacted=true`，不会泄漏 secret 值。
6. 尝试删除测试 credential。

测试专用前缀：

- service：`roswire-ci-smoke`
- account：`profiles/ci/keychain-smoke/<pid>/<nanos>`

不得复用真实用户 profile、service 或 account。

### Documented fallback 做什么

`documented_fallbacks_cover_linux_and_windows` 不访问真实 keychain。它验证本文档包含 Linux Secret Service、Windows Credential Manager、`SECRET_BACKEND_UNAVAILABLE` 与 fallback 说明，用于保证 CI/发布流程中不会把未验证平台误标为已验证。

## Keychain 不可用时的期望行为

当 OS 凭据库不可用、被锁定、缺少会话总线或缺少平台依赖时：

- `roswire config secret set ... type=keychain ... value=<...>` 必须失败。
- 错误 JSON 必须写入 `stderr`。
- `error_code` 必须是 `SECRET_BACKEND_UNAVAILABLE`。
- `stdout` 必须为空。
- secret 值不得出现在 `stdout`、`stderr`、日志、`config.toml` 或 `config inspect` 输出中。

## 发布前人工/半自动步骤

在 production-stable 发布前，维护者必须补齐以下记录：

- [ ] macOS：记录 `keychain-native-macos` workflow_dispatch 或本地 CI runner smoke 的通过链接。
- [ ] Linux：若使用 Secret Service，记录 `dbus`/`gnome-keyring`/`libsecret` 依赖与通过链接；若不支持，记录 `SECRET_BACKEND_UNAVAILABLE` fallback。
- [ ] Windows：记录 Credential Manager smoke 的通过链接；若不支持，记录 `SECRET_BACKEND_UNAVAILABLE` fallback。

这些记录关闭后，`docs/feature-checklist.md` 中的 keychain 多平台 smoke 项才能标记为完成。
