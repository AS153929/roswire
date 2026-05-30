# RosWire 发布流程

> 最后更新：2026-05-19
> 关联 issue：[#61](https://github.com/AS153929/roswire/issues/61)

本文面向维护者，说明如何准备、验证并发布 GitHub Release 与 crates.io crate。

## Release workflow

`.github/workflows/release.yml` 支持两种触发方式：

- 推送 `v*` tag：构建并发布 GitHub Release。
- `workflow_dispatch`：手动构建 release artifacts，用于发布前演练。

构建矩阵：

| Asset | Runner | Target | Archive |
| --- | --- | --- | --- |
| `roswire-linux-amd64` | `ubuntu-latest` | `x86_64-unknown-linux-musl` | `.tar.gz` |
| `roswire-linux-arm64` | `ubuntu-latest` | `aarch64-unknown-linux-musl` | `.tar.gz` |
| `roswire-windows-amd64` | `windows-latest` | `x86_64-pc-windows-msvc` | `.zip` |

Linux release 产物使用 musl 静态目标，不依赖目标机器的 glibc。Linux 构建通过 `cross` 运行，因为 `ssh2` / OpenSSL / zlib / ring 包含 C 依赖，普通 Rust target 安装不足以稳定完成 musl 交叉编译。Linux 二进制在 smoke test 和打包前使用 UPX 压缩；如果后续发现某个安全环境误报或禁止 UPX，可在 release workflow 中移除 `Compress Linux binary with UPX` 步骤后重新发布。

每个可执行平台构建后都会执行：

```text
roswire doctor --json
```

该 smoke test 只做本地诊断，不访问 RouterOS。

## 发布前检查

发布前必须在 `main` 上确认：

- [ ] `cargo fmt --check`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo test --workspace --all-features`
- [ ] `make cargo-publish-dry-run`
- [ ] `cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 85`
- [ ] `ROSWIRE_KEYCHAIN_SMOKE=documented-fallback cargo test --test keychain_smoke documented_fallbacks_cover_linux_and_windows -- --ignored --exact`
- [ ] macOS native keychain smoke 已通过本地 runner 或 `workflow_dispatch` 的 `keychain-native-macos` job。
- [ ] README 示例与 [`installation.md`](installation.md) 的命令仍与当前实现一致。
- [ ] 若声明 production-stable，则 [`production-readiness.md`](production-readiness.md) 的 P0 blocker 已关闭。

## crates.io 发布

首次发布前需要：

1. 在 crates.io 登录 GitHub 账号，创建 API token。
1. 在本机执行 `cargo login`，按提示粘贴 token。
1. 在 GitHub repository secrets 中创建 `CARGO_REGISTRY_TOKEN`，值为 crates.io API token。
1. 可选：如需 crates.io 发布前人工审批，可先在 GitHub Settings 创建 environment，再给 `publish-crate` job 增加 `environment` 字段。
1. 确认 `Cargo.toml` 的 `version`、`license`、`description`、`homepage`、`repository`、`readme`、`keywords` 和 `categories` 与本次发布一致。
1. 确认 Git tag 与 `Cargo.toml` 的 `package.version` 一致，例如 `version = "0.1.0"` 对应 tag `v0.1.0`。
1. 确认 crate 名称仍可用：

```bash
cargo search roswire
```

发布前必须先做 dry-run：

```bash
make cargo-publish-dry-run
```

dry-run 通过后发布到 crates.io：

```bash
make cargo-publish
```

发布后验证安装：

```bash
cargo install roswire --locked
roswire doctor --json
```

Release workflow 在 tag 触发时会自动执行同样的 crates.io 发布步骤：

1. 等待 Linux / Windows artifact 构建通过。
1. 校验 `GITHUB_REF_NAME` 与 `Cargo.toml` 版本一致。
1. 执行 `cargo publish --dry-run --locked`。
1. 使用 `secrets.CARGO_REGISTRY_TOKEN` 执行 `cargo publish --locked`。
1. crates.io 发布成功后再发布 GitHub Release。

## 创建发布

1. 更新版本号和 release note 草稿。
1. 确保 `main` 与 `origin/main` 同步。
1. 创建并推送与 `Cargo.toml` 版本一致的 tag：

```bash
make tag
```

1. 等待 Release workflow 完成。
1. 下载产物并校验 `checksums.txt`。
1. 验证至少一个平台上的独立安装：

```bash
roswire doctor --json
```

## Checksum 要求

Release 必须包含 `checksums.txt`，覆盖所有 `.tar.gz` 与 `.zip` artifact。用户安装文档必须要求先校验 checksum，再解压或安装。

Linux 示例：

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
- macOS artifact 暂未提供；等待 macOS runner 策略或交叉编译方案确定后恢复。
- 真机/CHR 端到端矩阵仍由 #60 跟踪；未完成前只能声明 MVP/Beta。
