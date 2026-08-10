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

# Recut the CI image (run ON the fleet runner, workstation-nyc-2), then bump
# &rust_image in .woodpecker.yaml to the new tag — a pipeline only ever runs
# against the recipe it names.
ci-image tag=`date +%F`:
    docker build -f Dockerfile.ci -t meridian-ci:{{tag}} .
