# Diff-plan benchmark corpus

These immutable fixtures exercise the renderer-neutral patch planner with real
git diff data. Tests verify semantic invariants rather than matching rendered
snapshots.

| Fixture | Provenance | Bytes | Lines | SHA-256 |
| --- | --- | ---: | ---: | --- |
| `small-pi-ai.patch` | Pi packaged `@earendil-works/pi-ai@0.84.2` patch | 4,783 | 90 | `f657a33e9ca92e3570a10edea4e4aa9acdf40be6f30931110140fcf2f97a46a1` |
| `medium-pi-web.patch` | Pi web-access routing patch | 14,950 | 156 | `b442b7cea45d0c1a47af25ccaad459af26ff42eb5d02acfc1d3d4a0ce2931083` |
| `mixed-pi-coding-agent.patch` | Pi packaged `@earendil-works/pi-coding-agent@0.84.2` patch | 6,896 | 147 | `cde6d6186f87583e05611fe482245d67ee457d0e9907a062be4b7ae21170408a` |
| `stress-pierre.patch` | Pierre `apps/demo/src/mocks/diff.patch` at commit `55a941914056af44c78c4ba607b37130f189fb70` | 402,920 | 9,812 | `ee4ced9e5d3510fd68cb7f1b6395fe294503794f123d2434bbf67d7d4c8ba83b` |

The Pierre fixture is distributed under Apache-2.0. Its license and provenance
are recorded in `apps/farcaster/NOTICE.md` and
`apps/farcaster/THIRD_PARTY_LICENSES/PIERRE_DIFFS_APACHE-2.0.txt`.
