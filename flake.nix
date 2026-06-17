{
  description = "A cosy terminal UI for managing Azure Bastion SSH tunnels";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable"; # Standard nixpkgs for packages
    systems.url = "github:nix-systems/default";
    devenv.url = "github:cachix/devenv"; # Only for dev shell
  };

  outputs =
    {
      self,
      nixpkgs,
      devenv,
      systems,
      ...
    }@inputs:
    let
      forEachSystem = nixpkgs.lib.genAttrs (import systems);
    in
    {
      # =============================================================================
      # PACKAGES: For nix run and nix build (uses standard nixpkgs)
      # =============================================================================
      packages = forEachSystem (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          az-burrow = pkgs.rustPlatform.buildRustPackage {
            pname = "az-burrow";
            version = "0.2.0";
            src = ./.;

            cargoLock.lockFile = ./Cargo.lock;

            # The TUI shells out to `az` and `ssh-keygen` at runtime; they are not
            # build dependencies, so nothing extra is needed here.

            meta = {
              description = "A cosy terminal UI for managing Azure Bastion SSH tunnels";
              mainProgram = "az-burrow";
            };
          };

          default = self.packages.${system}.az-burrow;
        }
      );

      # =============================================================================
      # DEV SHELLS: For development environment (uses devenv)
      # =============================================================================
      devShells = forEachSystem (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = devenv.lib.mkShell {
            inherit inputs pkgs;
            modules = [
              {
                packages = [
                  pkgs.git
                  pkgs.cargo
                  pkgs.rustc
                  pkgs.clippy
                  pkgs.rustfmt
                  pkgs.rust-analyzer
                ];

                scripts = {
                  dev.exec = "cargo run";
                  build.exec = "cargo build --release";
                  test.exec = "cargo test";
                  fmt.exec = "cargo fmt";
                  lint.exec = "cargo clippy --all-targets -- -D warnings";
                  clean.exec = "cargo clean";
                };

                git-hooks.hooks = {
                  rustfmt = {
                    enable = true;
                    name = "rustfmt";
                    entry = "${pkgs.rustfmt}/bin/rustfmt --edition 2021";
                    types = [ "rust" ];
                  };
                  clippy = {
                    enable = true;
                    name = "clippy";
                    entry = "${pkgs.cargo}/bin/cargo clippy --all-targets -- -D warnings";
                    types = [ "rust" ];
                    pass_filenames = false;
                  };
                  commit-msg = {
                    enable = true;
                    name = "conventional-commits";
                    entry = "${pkgs.writeShellScript "check-commit-msg" ''
                      commit_msg=$(cat "$1")
                      if ! echo "$commit_msg" | grep -qE "^(feat|fix|docs|style|refactor|perf|test|chore|revert|ci|bump)(\(.+\))?: .+"; then
                        echo "❌ Commit message must follow Conventional Commits format:"
                        echo "   type(scope): description"
                        echo ""
                        echo "   Types: feat, fix, docs, style, refactor, perf, test, chore, revert, ci"
                        echo "   Example: feat(tui): add quit confirmation dialog"
                        exit 1
                      fi
                    ''}";
                    stages = [ "commit-msg" ];
                  };
                  pre-push = {
                    enable = true;
                    name = "tests";
                    entry = "${pkgs.cargo}/bin/cargo test";
                    pass_filenames = false;
                  };
                };

                enterShell = ''
                  cat << 'EOF'
                    ___
                   (o o)
                   (. .)
                    \-/
                  🦫 Welcome to az-burrow development environment!

                  📦 Available commands:
                    dev              Run the application (cargo run)
                    build            Build a release binary
                    test             Run tests
                    fmt              Format code with rustfmt
                    lint             Run clippy (warnings as errors)
                    clean            Remove build artifacts

                  📝 Git hooks are active:
                    ✓ rustfmt on commit
                    ✓ clippy on commit
                    ✓ Conventional Commits enforced
                    ✓ Tests run before push

                  💡 Or use nix directly:
                    nix run                     # Build and run the app

                  EOF
                '';

                enterTest = ''
                  echo "🧪 Running az-burrow tests..."
                  cargo test
                '';
              }
            ];
          };
        }
      );

      formatter = forEachSystem (system: nixpkgs.legacyPackages.${system}.nixfmt-rfc-style);
    };
}
