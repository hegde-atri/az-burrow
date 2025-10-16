{
  inputs = {
    nixpkgs.url = "github:cachix/devenv-nixpkgs/rolling";
    systems.url = "github:nix-systems/default";
    devenv.url = "github:cachix/devenv";
    devenv.inputs.nixpkgs.follows = "nixpkgs";
  };

  nixConfig = {
    extra-trusted-public-keys = "devenv.cachix.org-1:w1cLUi8dv3hnoSPGAuibQv+f9TZLr6cv/Hm9XgU50cw=";
    extra-substituters = "https://devenv.cachix.org";
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
                # =============================================================================
                # PACKAGES: Development tools and utilities
                # =============================================================================
                packages = [
                  pkgs.git # Version control
                  pkgs.gofumpt # Stricter Go formatter (replacement for gofmt)
                  pkgs.golangci-lint # Meta-linter aggregating multiple Go linters
                  pkgs.upx # Binary compressor for smaller executables
                ];

                # =============================================================================
                # LANGUAGES: Enable Go development environment
                # =============================================================================
                languages.go.enable = true;

                # =============================================================================
                # SCRIPTS: Common development tasks
                # These can be run with: devenv run <script-name>
                # =============================================================================
                scripts = {
                  # Run the application in development mode
                  dev.exec = "go run ./cmd/az-burrow";

                  # Build for current platform
                  build.exec = "go build -o bin/az-burrow ./cmd/az-burrow";

                  # Cross-compile for Windows (from Linux/WSL)
                  build-windows.exec = "GOOS=windows GOARCH=amd64 go build -o bin/az-burrow.exe ./cmd/az-burrow";

                  # Build for Linux
                  build-linux.exec = "GOOS=linux GOARCH=amd64 go build -o bin/az-burrow ./cmd/az-burrow";

                  # Build for all platforms
                  build-all.exec = ''
                    echo "🔨 Building for all platforms..."
                    mkdir -p bin
                    GOOS=windows GOARCH=amd64 go build -o bin/az-burrow.exe ./cmd/az-burrow
                    echo "✓ Windows: bin/az-burrow.exe"
                    GOOS=linux GOARCH=amd64 go build -o bin/az-burrow ./cmd/az-burrow
                    echo "✓ Linux: bin/az-burrow"
                    echo "✨ All builds complete!"
                  '';

                  # Clean build artifacts
                  clean.exec = "rm -rf bin/";

                  # Run tests
                  test.exec = "go test ./...";

                  # Format code with gofumpt (stricter than gofmt)
                  fmt.exec = "gofumpt -l -w .";

                  # Run linter
                  lint.exec = "golangci-lint run";
                };

                # =============================================================================
                # GIT HOOKS: Enforce code quality and standards
                # Runs automatically on git actions (commit, push, etc.)
                # =============================================================================
                pre-commit.hooks = {
                  # Format Go code before committing
                  gofumpt = {
                    enable = true;
                    name = "gofumpt";
                    entry = "${pkgs.gofumpt}/bin/gofumpt -l -w";
                    types = [ "go" ];
                  };

                  # Run linter before committing
                  golangci-lint = {
                    enable = true;
                    name = "golangci-lint";
                    entry = "${pkgs.golangci-lint}/bin/golangci-lint run --fix";
                    types = [ "go" ];
                    pass_filenames = false;
                  };

                  # Enforce Conventional Commits format
                  # Format: type(scope): description
                  # Example: feat(tui): add tunnel creation form
                  commit-msg = {
                    enable = true;
                    name = "conventional-commits";
                    entry = "${pkgs.writeShellScript "check-commit-msg" ''
                      commit_msg=$(cat "$1")
                      if ! echo "$commit_msg" | grep -qE "^(feat|fix|docs|style|refactor|perf|test|chore|revert|ci)(\(.+\))?: .+"; then
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

                  # Run tests before pushing
                  pre-push = {
                    enable = true;
                    name = "tests";
                    entry = "${pkgs.go}/bin/go test ./...";
                    pass_filenames = false;
                  };
                };

                # =============================================================================
                # ENTER SHELL: Welcome message and helpful information
                # Displayed when entering the devenv shell
                # =============================================================================
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

                  Happy coding! 🚀
                  EOF
                '';

                # =============================================================================
                # ENTER TEST: Test environment setup
                # Runs when entering test mode
                # =============================================================================
                enterTest = ''
                  echo "🧪 Running az-burrow tests..."
                  go test ./...
                '';

                # =============================================================================
                # ADDITIONAL CONFIG
                # =============================================================================

                # Uncomment to set environment variables
                # env.AZURE_SUBSCRIPTION_ID = "your-subscription-id";

                # Uncomment to add processes that run in the background
                # processes.watcher.exec = "watchexec -w . -e go -- echo 'File changed'";
              }
            ];
          };
        }
      );
    };
}
