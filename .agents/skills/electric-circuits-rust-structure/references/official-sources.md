# Primary Rust and Cargo sources

Use these primary references for language and Cargo behavior; repository architecture and
`AGENTS.md` remain authoritative for Electric Circuits-specific decisions.

| Topic | Primary source |
| --- | --- |
| Workspaces, inherited metadata/dependencies, and feature unification | [Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html) |
| Resolver policy and lockfile updates | [Cargo resolver](https://doc.rust-lang.org/cargo/reference/resolver.html) and [Rust 2024 resolver](https://doc.rust-lang.org/edition-guide/rust-2024/cargo-resolver.html) |
| Package manifests and publication controls | [Cargo manifest reference](https://doc.rust-lang.org/cargo/reference/manifest.html) |
| Public and restricted visibility | [Rust Reference: visibility and privacy](https://doc.rust-lang.org/reference/visibility-and-privacy.html) |
| Modules and file layout | [The Rust Programming Language: modules](https://doc.rust-lang.org/book/ch07-05-separating-modules-into-different-files.html) |
| Public error design and validation | [Rust API Guidelines: interoperability](https://rust-lang.github.io/api-guidelines/interoperability.html) and [dependability](https://rust-lang.github.io/api-guidelines/dependability.html) |
| Feature design | [Cargo features](https://doc.rust-lang.org/cargo/reference/features.html) |
| Platform-specific dependencies | [Cargo dependencies: platform-specific dependencies](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#platform-specific-dependencies) |
| Build scripts | [Cargo build scripts](https://doc.rust-lang.org/cargo/reference/build-scripts.html) |
| Rust-version/MSRV behavior | [Cargo rust-version](https://doc.rust-lang.org/cargo/reference/rust-version.html) |
| Incremental builds, codegen units, and LTO | [Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html) |
| Dependency and feature inspection | [cargo tree](https://doc.rust-lang.org/cargo/commands/cargo-tree.html) |
| Public rustdoc and examples/bench targets | [rustdoc command line](https://doc.rust-lang.org/rustdoc/command-line-arguments.html) and [Cargo targets](https://doc.rust-lang.org/cargo/reference/cargo-targets.html) |
