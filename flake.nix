{
  description = "meridian-rs sidecar — the thin NDJSON stdin/stdout wire↔model binary, packaged for the ccc delivery pipeline (CI → binary cache → osfiles flake-input pin → CCC_SIDECAR_BIN)";

  # Self-contained binary delivery, mirroring the ccc-mdformat precedent:
  # deliberately NO `inputs.nixpkgs.follows` on the toolchain consumer side —
  # the prebuilt `sidecar` is served from the ccc binary cache, so this flake
  # keeps its OWN pinned nixpkgs; the store hash matches the cache and
  # substitutes instead of rebuilding in prod.
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # Rust toolchain independent of nixpkgs' rustc lag — meridian-rs needs
    # edition 2024 (≥1.85) / rust-version 1.96. Same discipline as osfiles.
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { self, nixpkgs, fenix }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems f;

      mkSidecar =
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          # Pinned stable MINIMAL toolchain — cargo + rustc + rust-std only
          # (satisfies rust-version 1.96 / edition 2024). No rust-docs / rust-src
          # / rust-analyzer / clippy / rustfmt: a binary build needs none of them,
          # and dropping them keeps the CI build + the cache closure lean.
          toolchain = fenix.packages.${system}.stable.minimalToolchain;
          rustPlatform = pkgs.makeRustPlatform {
            cargo = toolchain;
            rustc = toolchain;
          };
        in
        rustPlatform.buildRustPackage {
          pname = "sidecar";
          # The one real product version in the workspace (crates/sidecar).
          version = "0.1.0";
          src = self;

          cargoLock = {
            lockFile = ./Cargo.lock;
            # The pulldown-cmark obsidian fork ([patch.crates-io]); both member
            # crates ride the same git tree/rev.
            outputHashes = {
              "pulldown-cmark-0.13.1" = "sha256-TDd0Mzm1zgAiSXMAHbsfbTLuq7+//C2ypQqnvbqdm1U=";
              "pulldown-cmark-escape-0.11.0" = "sha256-TDd0Mzm1zgAiSXMAHbsfbTLuq7+//C2ypQqnvbqdm1U=";
            };
          };

          # Build ONLY the sidecar bin (its dep graph excludes transport-proto,
          # so no protoc is needed). Everything else in the workspace is off the
          # delivery path.
          cargoBuildFlags = [ "-p" "sidecar" ];

          # Tests need on-disk workspace fixtures + the testsuite data helpers;
          # the delivery build stays lean (the GH Actions `checks` lane runs the
          # full `cargo test --workspace`). Same stance as osfiles rtk.nix.
          doCheck = false;

          meta = {
            description = "meridian-rs sidecar: NDJSON stdin/stdout wire↔model engine (read/put/CAS/subscribe over a WorkspaceRoot)";
            mainProgram = "sidecar";
            license = with nixpkgs.lib.licenses; [ mit asl20 ];
          };
        };

      # Fully-STATIC musl sidecar for linux (x86_64 + aarch64) — the
      # FLEET-portable binaries. The default `sidecar` above is nix-DYNAMIC
      # (PT_INTERP=/nix/store/…-glibc/ld-linux + /nix/store lib refs) so it only
      # runs on nix hosts / with nix-ld; foreign-Debian prod (glibc, no nix-ld)
      # can't exec it. A musl static build has NO interpreter and links libc
      # statically → runs on any linux (glibc OR musl). The sidecar dep tree is 0
      # blocking C-FFI (blake3's cc-asm builds via the musl cc below), so it links
      # fully static clean. decisions/0006 (ZT).
      mkSidecarStatic =
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          # musl target triple per arch; drives the fenix rust-std selection. The
          # actual --target comes from the pkgsStatic stdenv (see below), so this
          # MUST match pkgsStatic.stdenv.hostPlatform.rust.rustcTarget for `system`.
          rustTarget =
            {
              "x86_64-linux" = "x86_64-unknown-linux-musl";
              "aarch64-linux" = "aarch64-unknown-linux-musl";
            }
            .${system};
          # Fenix host toolchain (rust-version 1.96 — nixpkgs' own rustc is 1.95,
          # too old) PLUS the musl target's rust-std.
          toolchain = fenix.packages.${system}.combine [
            fenix.packages.${system}.stable.minimalToolchain
            fenix.packages.${system}.targets.${rustTarget}.stable.rust-std
          ];
          # Build from the pkgsStatic (musl-static) stdenv: its
          # hostPlatform.rust.rustcTarget = x86_64-unknown-linux-musl, so
          # buildRustPackage targets musl AND links static via the stdenv (the
          # musl cc/linker for blake3's cc-asm comes from pkgsStatic too). Passing
          # a target only via env was IGNORED — buildRustPackage's --target comes
          # from the stdenv, so the stdenv MUST be the musl-static one.
          rustPlatform = pkgs.pkgsStatic.makeRustPlatform {
            cargo = toolchain;
            rustc = toolchain;
          };
        in
        rustPlatform.buildRustPackage {
          pname = "sidecar-static";
          version = "0.1.0";
          src = self;

          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes = {
              "pulldown-cmark-0.13.1" = "sha256-TDd0Mzm1zgAiSXMAHbsfbTLuq7+//C2ypQqnvbqdm1U=";
              "pulldown-cmark-escape-0.11.0" = "sha256-TDd0Mzm1zgAiSXMAHbsfbTLuq7+//C2ypQqnvbqdm1U=";
            };
          };

          cargoBuildFlags = [ "-p" "sidecar" ];
          doCheck = false;

          meta = {
            description = "meridian-rs sidecar — fully-static musl build (fleet-portable, runs on non-nix glibc hosts)";
            mainProgram = "sidecar";
            license = with nixpkgs.lib.licenses; [ mit asl20 ];
            platforms = [ "x86_64-linux" "aarch64-linux" ];
          };
        };
    in
    {
      packages = forAllSystems (
        system:
        rec {
          sidecar = mkSidecar system;
          default = sidecar;
        }
        # The static musl build targets linux only (x86_64 + aarch64) — the
        # fleet-portable binaries the installer/R2 delivery ships to foreign
        # Debian/glibc AND musl arm hosts alike.
        // nixpkgs.lib.optionalAttrs (builtins.elem system [ "x86_64-linux" "aarch64-linux" ]) {
          sidecar-static = mkSidecarStatic system;
        }
      );

      # `nix flake check` builds the delivered binary on the host system.
      checks = forAllSystems (system: {
        sidecar = self.packages.${system}.sidecar;
      });
    };
}
