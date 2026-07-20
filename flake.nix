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
    in
    {
      packages = forAllSystems (system: rec {
        sidecar = mkSidecar system;
        default = sidecar;
      });

      # `nix flake check` builds the delivered binary on the host system.
      checks = forAllSystems (system: {
        sidecar = self.packages.${system}.sidecar;
      });
    };
}
