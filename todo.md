# Modularization TODO

## Structure

- Organize the proposed boundaries as in-crate Rust modules rather than extracting additional crates.
- Do not add a top-level or separate `contracts` module/crate.
- Use these product-capability boundaries:
  - `agents`
  - `sessions`
  - `repository`
  - `access`
  - `projects`
  - `workgraph`
- Keep the existing `workgraph` crate as its product-capability boundary.
- Keep application composition, cross-module workflows, GPUI, startup, and delivery adapters such as MCP under `app`.

```text
src/
  modules/
    agents/
    sessions/
    repository/
    access/
    projects/
  app/
  main.rs
workgraph/
```

## Module boundaries

- Make each module self-contained and keep its implementation private.
- Expose only a minimal interface through the module's `mod.rs`.
- Keep boundary DTOs with the module that owns the interface.
- Put boundary types in `contract.rs` when they are substantial; otherwise define them in `mod.rs`.
- Let `app` translate between module APIs instead of introducing shared DTOs.
- Do not allow cyclic dependencies between modules.
- Allow explicit one-way module dependencies where required.
- Do not import another module's internals.

## Internal module layout

Use only the files and directories a module needs:

```text
module/
  mod.rs
  contract.rs
  domain/
  core/
  adapter/
```

- `mod.rs`: minimal public interface and re-exports.
- `contract.rs`: commands, results, and DTOs crossing the module boundary.
- `domain/`: internal domain objects; do not export them.
- `core/`: core logic and the adapter interfaces it requires.
- `adapter/`: external integrations such as databases, processes, connections, Git, Jujutsu, or coding-agent harnesses.
- Core must depend on its own adapter interfaces and never import adapter implementations directly.
- Adapters implement interfaces owned by core.
- Do not require every module to contain every listed layer.

## Crates

- Do not extract additional crates now.
- Extract a module into a crate only after compiler-enforced isolation, reuse, a stable public API, substantially different dependencies, or measured independent compilation/testing value earns the split.
