# meridian-rs — mrd CLI (verbs mirror meridian-go's justfile)

default:
    @just --list

# Build mrd (release)
build:
    cargo build --release -p mrd

# Build and install to ~/.local/bin/mrd (cargo-tracked, lockfile-pinned)
install:
    cargo install --path crates/mrd --root ~/.local --locked --force

# Build only (no install)
check:
    cargo build --workspace

# Run tests
test:
    cargo test --workspace

# Clean build artifacts
clean:
    cargo clean
