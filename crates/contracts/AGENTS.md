# Contracts Rules

- Keep this crate free of Farcaster crate dependencies.
- Add only values shared by peer domains with no single owner.
- Keep owned models in their domain crate.
- Do not add I/O, state, workflows, or adapter wire types.
- Keep modules narrow; do not add a catch-all prelude.
