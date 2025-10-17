{
  description = "Azure tunnel management CLI tool";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable"; # Standard nixpkgs for packages
    systems.url = "github:nix-systems/default";
    devenv.url = "github:cachix/devenv"; # Only for dev shell
  };

  # Remove nixConfig entirely - only devShell users need devenv cache

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
          az-burrow = pkgs.buildGoModule {
            pname = "az-burrow";
            version = "0.1.0";
            src = ./.;

            vendorHash = "sha256-7xqgs4xleK2xvcG7waM0UkqpIQPL0Yz9LwWYIAT4YP8=";

            subPackages = [ "cmd/az-burrow" ];

            ldflags = [
              "-s"
              "-w"
            ];

            meta = {
              description = "Azure tunnel management tool";
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
                  pkgs.gofumpt
                  pkgs.golangci-lint
                  pkgs.upx
                ];

                languages.go.enable = true;

                scripts = {
                  dev.exec = "go run ./cmd/az-burrow";
                  build.exec = "go build -o bin/az-burrow ./cmd/az-burrow";
                  build-windows.exec = "GOOS=windows GOARCH=amd64 go build -o bin/az-burrow.exe ./cmd/az-burrow";
                  build-linux.exec = "GOOS=linux GOARCH=amd64 go build -o bin/az-burrow ./cmd/az-burrow";
                  build-all.exec = ''
                    echo "🔨 Building for all platforms..."
                    mkdir -p bin
                    GOOS=windows GOARCH=amd64 go build -o bin/az-burrow.exe ./cmd/az-burrow
                    echo "✓ Windows: bin/az-burrow.exe"
                    GOOS=linux GOARCH=amd64 go build -o bin/az-burrow ./cmd/az-burrow
                    echo "✓ Linux: bin/az-burrow"
                    echo "✨ All builds complete!"
                  '';
                  clean.exec = "rm -rf bin/";
                  test.exec = "go test ./...";
                  fmt.exec = "gofumpt -l -w .";
                  lint.exec = "golangci-lint run";
                };

                git-hooks.hooks = {
                  gofumpt = {
                    enable = true;
                    name = "gofumpt";
                    entry = "${pkgs.gofumpt}/bin/gofumpt -l -w";
                    types = [ "go" ];
                  };
                  golangci-lint = {
                    enable = true;
                    name = "golangci-lint";
                    entry = "${pkgs.golangci-lint}/bin/golangci-lint run --fix";
                    types = [ "go" ];
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
                    entry = "${pkgs.go}/bin/go test ./...";
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
                    dev              Run the application
                    build            Build for current platform
                    build-windows    Build Windows .exe (from Linux)
                    build-linux      Build Linux binary
                    build-all        Build for all platforms
                    clean            Remove build artifacts
                    test             Run tests
                    fmt              Format code with gofumpt
                    lint             Run golangci-lint

                  🔧 Usage:
                    devenv run dev              # Run the TUI
                    devenv run build-windows    # Cross-compile for Windows
                    devenv run lint             # Check code quality

                  📝 Git hooks are active:
                    ✓ Code formatting on commit
                    ✓ Linting on commit
                    ✓ Conventional Commits enforced
                    ✓ Tests run before push

                  💡 Or use nix directly:
                    nix run                     # Build and run the app

                  EOF
                '';

                enterTest = ''
                  echo "🧪 Running az-burrow tests..."
                  go test ./...
                '';
              }
            ];
          };
        }
      );

      formatter = forEachSystem (system: nixpkgs.legacyPackages.${system}.nixfmt-rfc-style);
    };
}
