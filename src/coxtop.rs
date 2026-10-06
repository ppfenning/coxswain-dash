// The `coxtop` alias binary. It is the towpath program compiled under the old name: `main` reads
// its own name and prints the alias notice. `include!` keeps one module tree at this crate root,
// which the `crate::` paths need, and keeps cargo from seeing src/main.rs in two build targets.
include!("main.rs");
