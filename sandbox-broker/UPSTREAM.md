# Upstream source record

This directory includes `LICENSE-APACHE` and `NOTICE` copied from OpenAI Codex at commit `65ae4c26e088913176a50d6daeb742d00942caee` on 2026-07-27.

The broker now includes a Pi-adapted macOS policy builder and the Codex base Seatbelt policy. The protocol, validation, process runner, and framing are Pi code. Keep this record next to future imports and update it in the same change as each import.

## Approved source groups for the macOS port

| Purpose | Upstream commit | Upstream paths | State |
| --- | --- | --- | --- |
| Seatbelt policy builder | `65ae4c26e088913176a50d6daeb742d00942caee` | `codex-rs/sandboxing/src/seatbelt.rs` | Adapted in `src/seatbelt.rs`; Pi replaced Codex policy and network types, added exact file scope and hard denies, and now permits only one request-provided loopback proxy port in proxy mode |
| Base policy | `65ae4c26e088913176a50d6daeb742d00942caee` | `codex-rs/sandboxing/src/seatbelt_base_policy.sbpl` | Copied as `src/seatbelt_base_policy.sbpl` without policy changes |
| Network service policy | `65ae4c26e088913176a50d6daeb742d00942caee` | `codex-rs/sandboxing/src/seatbelt_network_policy.sbpl` | Studied; not imported yet |
| Minimal read defaults | `65ae4c26e088913176a50d6daeb742d00942caee` | `codex-rs/sandboxing/src/restricted_read_only_platform_defaults.sbpl` | Studied; not imported yet |
| Seatbelt tests | `65ae4c26e088913176a50d6daeb742d00942caee` | `codex-rs/sandboxing/src/seatbelt_tests.rs` | Test source only; not imported yet |
| Improved denial collector and PID tracker | `484518f28433c37d3142c49d7060bd35462ce352` | `codex-rs/sandboxing/src/seatbelt_denials/mod.rs`, `pid_tracker.rs`, `seatbelt_denials_tests.rs` | Adapted in `src/denial_collector.rs` and `src/pid_tracker.rs`. Pi uses a fixed `/usr/bin/log`, one session stream, readiness before broker `ready`, bounded raw lines and records, command sequence windows, synchronous tracker setup, process start-time checks, and observations for denial attribution and best-effort cleanup |
| Exec denial wiring | `f847460584b7f4ee472e6b30700a0754e915ecbf` | Diff under `codex-rs` | Reference only |

The denial collector first came from Codex debug work at commit `0271c20d8f31f42545868076e1d10f4497f18b35`. Its own commit text calls collection best effort. Pi must keep that limit in code, tests, and user-facing text.

## Linux source groups

The Linux backend is adapted from the same inspected Codex commit, `65ae4c26e088913176a50d6daeb742d00942caee`. Pi did not copy Codex's protocol, proxy, bundled-binary, Landlock fallback, or full policy model.

| Purpose | Upstream paths | State |
| --- | --- | --- |
| Bubblewrap mounts and namespace layout | `codex-rs/linux-sandbox/src/bwrap.rs`, `linux_run_main.rs` | Reduced and adapted in `src/linux.rs`; Pi uses its normalized rights, a read-only root, ordered exact write and deny mounts, a bounded secret snapshot, and a private network namespace |
| Launcher and inherited descriptors | `codex-rs/linux-sandbox/src/launcher.rs`, `exec_util.rs` | Adapted in `src/linux.rs`; Pi uses a compile-time fixed path and an inherited seccomp file descriptor, with no `PATH` or bundled-binary lookup |
| `no_new_privs` and restricted seccomp | `codex-rs/linux-sandbox/src/landlock.rs` | Adapted in `src/linux.rs`; Pi directly encodes the reviewed x86_64/aarch64 cBPF rules and lets Bubblewrap apply them after namespace setup |
| Bubblewrap readiness probe | `codex-rs/sandboxing/src/bwrap.rs`, `codex-rs/linux-sandbox/src/linux_run_main.rs` | Adapted in `src/linux.rs`; Pi requires the fixed binary to establish all release namespaces, private `/proc`, seccomp, and `NoNewPrivs` before readiness |
| Bundled Bubblewrap | `codex-rs/linux-sandbox/src/bundled_bwrap.rs` | Studied, not imported; Nix supplies a fixed store path and non-Nix builds require `/usr/bin/bwrap` |
| Network routing | `codex-rs/linux-sandbox/src/proxy_routing.rs` | Design reference only. Pi did not import it. Pi's protocol v4 code mounts its own broker as a fixed in-namespace launcher, enables command-only private loopback when approved, bridges one fixed TCP port to the validated host proxy Unix socket when needed, and denies `AF_UNIX` to the user command |

## macOS ownership finding

No Codex source inspected here provides atomic ownership of descendants that leave a process group. On 2026-07-27, Pi also checked Apple's public XNU coalition definitions and `bsd/kern/sys_coalition.c` from `apple-oss-distributions/xnu`. The kernel requires the caller to be in a privileged coalition before `coalition_create`; a direct normal-user probe returned `EPERM`. No Apple code was copied. This source check supports the explicit daemon-escape limit in `THREAT_MODEL.md`; it does not add an Apple license to the broker.

## Rust dependencies

| Crate | Version | License |
| --- | --- | --- |
| `base64` | 0.22.1 | MIT OR Apache-2.0 |
| `libc` | 0.2.189 | MIT OR Apache-2.0 |
| `nix` | 0.31.3 | MIT |
| `regex-lite` | 0.1.8 | MIT OR Apache-2.0 |
| `serde` | 1.0.229 | MIT OR Apache-2.0 |
| `serde_json` | 1.0.151 | MIT OR Apache-2.0 |

`Cargo.lock` records transitive packages and checksums. Nix builds from that lock file.

## Import rules

- Copy the least code needed; do not pull in the Codex workspace.
- Mark changed source headers or module docs with the source commit and a note that Pi changed the code.
- Keep copied third-party notices that apply to a source group.
- List any new crate dependency and its license before packaging.
- Keep imported policy snapshots and adapted tests in the same review.
- Do not state or imply that OpenAI supports this broker.

`NOTICE` includes the full Codex notice. Ratatui code is not planned for this broker, but retaining the full upstream notice avoids dropping notice text from the copied work.
