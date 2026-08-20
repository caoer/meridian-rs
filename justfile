# meridian-rs — mrd CLI (verbs mirror meridian-go's justfile)

default:
    @just --list

# Build mrd (release)
build:
    cargo build --release -p mrd

# Build and install to ~/.local/bin/mrd (cargo-tracked, lockfile-pinned).
# Whoever installs a build restarts the resident daemon (0025 pipeline duty —
# the engine refuses across builds and never restarts anything itself): TERM
# the pidfile's daemon; the next call auto-starts the new build. Pidfiles are
# hash-keyed beside the short socks (`<12hex>.pid` under $XDG_RUNTIME_DIR/mrd
# on Linux, else $HOME/.cache/mrd-run); the legacy in-root pidfile line stays
# so an install over a pre-short-sock resident still restarts it.
install:
    cargo install --path crates/mrd --root ~/.local --locked --force
    -pkill -TERM -F "${XDG_CACHE_HOME:-$HOME/.cache}/meridian/registry/daemon.pid" mrd 2>/dev/null
    -for f in "$HOME/.cache/mrd-run"/*.pid "${XDG_RUNTIME_DIR:-/nonexistent}/mrd"/*.pid; do [ -e "$f" ] && pkill -TERM -F "$f" mrd; done 2>/dev/null; true

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
