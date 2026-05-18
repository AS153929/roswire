# RosWire 发布流程

> 最后更新：2026-05-18
> 关联 issue：[#61](https://github.com/AS153929/roswire/issues/61)

本文面向维护者，说明如何准备、验证和发布 GitHub Release。

## Release workflow

`.github/workflows/release.yml` 支持两种触发方式：

- 推送 `v*` tag：构建并发布 GitHub Release。
- `workflow_dispatch`：手动构建 release artifacts，用于发布前演练。

构建矩阵：

| Asset | Runner | Target | Archive |
| --- | --- | --- | --- |
| `roswire-linux-amd64` | `ubuntu-latest` | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| `roswire-linux-arm64` | `ubuntu-latest` | `aarch64-unknown-linux-gnu` | `.tar.gz` |
| `roswire-macos-amd64` | `macos-13` | `x86_64-apple-darwin` | `.tar.gz` |
| `roswire-macos-arm64` | `macos-13` | `aarch64-apple-darwin` | `.tar.gz` |
| `roswire-windows-amd64` | `windows-latest` | `x86_64-pc-windows-msvc` | `.zip` |

每个平台构建后都会执行：

```text
roswire doctor --json
```

该 smoke test 只做本地诊断，不访问 RouterOS。

## 发布前检查

发布前必须在 `main` 上确认：

- [ ] `cargo fmt --check`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo test --workspace --all-features`
- [ ] `cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 85`
- [ ] `ROSWIRE_KEYCHAIN_SMOKE=documented-fallback cargo test --test keychain_smoke documented_fallbacks_cover_linux_and_windows -- --ignored --exact`
- [ ] macOS native keychain smoke 已通过本地 runner 或 `workflow_dispatch` 的 `keychain-native-macos` job。
- [ ] README 示例与 [`installation.md`](installation.md) 的命令仍与当前实现一致。
- [ ] 若声明 production-stable，则 [`production-readiness.md`](production-readiness.md) 的 P0 blocker 已关闭。

## 创建发布

1. 更新版本号和 release note 草稿。
1. 确保 `main` 与 `origin/main` 同步。
1. 创建并推送 tag：

```bash
git tag v0.1.0
git push origin v0.1.0
```

1. 等待 Release workflow 完成。
1. 下载产物并校验 `checksums.txt`。
1. 验证至少一个平台上的独立安装：

```bash
roswire doctor --json
```

## Checksum 要求

Release 必须包含 `checksums.txt`，覆盖所有 `.tar.gz` 与 `.zip` artifact。用户安装文档必须要求先校验 checksum，再解压或安装。

Linux/macOS 示例：

```bash
sha256sum -c checksums.txt --ignore-missing
```

Windows 示例：

```powershell
$hash = (Get-FileHash .\roswire-windows-amd64.zip -Algorithm SHA256).Hash.ToLower()
Select-String -Path .\checksums.txt -Pattern $hash
```

## README 示例验证记录

本次 #61 文档校正覆盖以下示例：

- `roswire doctor --json`：Release workflow 每个平台构建后 smoke。
- `roswire interface print --json`：需要真实 RouterOS 配置；属于 #60 真机/CHR 矩阵。
- `roswire file upload ... --dry-run ... --json`：本地 dry-run 可在无 RouterOS 环境运行，依赖 transfer dry-run 集成测试覆盖。

## 已知限制

- Windows arm64 artifact 暂未提供。
- Linux musl/static artifact 暂未提供；当前 Linux 产物使用 GNU target。
- 真机/CHR 端到端矩阵仍由 #60 跟踪；未完成前只能声明 MVP/Beta。
