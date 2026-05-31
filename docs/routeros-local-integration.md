# Local RouterOS integration loop

This repo can run its read-only RouterOS integration harness locally against a
disposable RouterOS CHR container, instead of only via the manual
`RouterOS Integration` GitHub Actions workflow.

## What you get

- `.devcontainer/` — a pinned Rust dev container with an **optional** RouterOS
  CHR sidecar service.
- `make integration-local` — one command that builds `roswire`, boots a CHR
  container, waits for it, and runs `scripts/routeros-ci-integration.sh`
  (the same read-only harness CI uses).

## Scope (read this first)

- **Read-only smoke only.** The harness exercises config init/inspect, remote
  `doctor`, and a handful of read-only `print`/`raw` commands plus the degraded
  `schema discover --remote` envelope. It performs **no** writes.
- **Out of scope here (left for follow-up issues):** the v6/v7 dual-version
  matrix, REST protocol coverage, and live SSH/file workflows (`import`,
  `export download`, `backup download`). Do not treat this loop as a full live
  acceptance matrix.
- Existing CI (`.github/workflows/routeros-integration.yml`) is unchanged; this
  only adds a local entry point that reuses the same harness.

## Image and architecture notes

- **There is no official RouterOS Docker image.** MikroTik publishes a CHR disk
  image only. We use the community `evilfreelancer/docker-routeros` image, which
  wraps CHR in QEMU. This is the same image the CI workflow already uses.
- The image manifest is **multi-arch**. Docker selects the host architecture
  automatically, so on Apple Silicon you get the **arm64** CHR and it runs
  **same-architecture** — no slow cross-arch emulation. Avoid forcing a
  mismatched `platform:`; x86 emulation on an arm64 Mac is impractically slow.
- **KVM acceleration:** the CHR boots faster with `/dev/kvm`. `make
  integration-local` injects `/dev/kvm` into the container only when the host
  exposes it; otherwise QEMU falls back to software TCG (slower but functional).
  Under `colima` on Apple Silicon, KVM passthrough is typically unavailable, so
  expect the TCG fallback. To improve odds of KVM on Linux hosts, ensure
  `/dev/kvm` exists and is accessible before running.
- `/dev/net/tun` and `NET_ADMIN` are required for the CHR's QEMU networking.

## Prerequisites

- Docker with the Compose plugin. On macOS, `colima start` (an arm64 Linux VM)
  works well; Docker Desktop also works.
- A Rust toolchain for the host path (`make integration-local` builds on the
  host). The dev container path provides a pinned toolchain instead.

## Usage

### Host path (recommended on macOS)

```bash
colima start            # or start Docker Desktop
make integration-local
```

This builds `roswire`, renders a disposable compose file under
`target/routeros-ci/`, boots the CHR with ports published to `127.0.0.1`
(API `8728`, API-SSL `8729`, SSH `2222`), runs the harness, writes evidence and
`docker-routeros.log` to `target/routeros-ci/`, then tears the container down.

Useful overrides:

```bash
ROSWIRE_KEEP_ROUTEROS=1 make integration-local   # leave the CHR running
ROSWIRE_SKIP_BUILD=1     make integration-local   # reuse an existing debug build
ROUTEROS_API_PORT=18728  make integration-local   # avoid a host port clash
```

### Dev container path

Open the repo in the dev container (VS Code: "Reopen in Container"). Only the
`dev` service starts by default. Start the CHR sidecar on demand and run the
harness against it by service name:

```bash
docker compose -f .devcontainer/docker-compose.yml up -d routeros
cargo build --locked
ROUTEROS_HOST=routeros scripts/routeros-ci-integration.sh
```

The sidecar in the dev container uses the TCG fallback (no `/dev/kvm`), so first
boot can take a few minutes.

## Troubleshooting

- **Hangs on "Waiting for RouterOS API":** the CHR is still booting (TCG is
  slow). Increase patience or run on a host with KVM. Inspect
  `target/routeros-ci/docker-routeros.log`.
- **Port already in use:** override `ROUTEROS_API_PORT` / `ROUTEROS_SSH_PORT` /
  `ROUTEROS_API_SSL_PORT`.
- **`/dev/net/tun` missing:** ensure your Docker VM loads the `tun` module
  (`colima` images do by default).
