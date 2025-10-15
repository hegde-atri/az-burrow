package main

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/hegde-atri/az-burrow/internal/tui"
)

const version = "0.1.0"

// main is the entry point for Az-Burrow, a cosy TUI for managing Azure Bastion tunnels.
// It initializes the TUI application and handles any startup errors gracefully.
func main() {
	// Determine config file path (default to burrow.config.yaml in current directory)
	configPath := "burrow.config.yaml"
	if len(os.Args) > 1 {
		configPath = os.Args[1]
	}

	// Make path absolute
	absPath, err := filepath.Abs(configPath)
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error resolving config path: %v\n", err)
		os.Exit(1)
	}

	// Initialize and run the TUI application
	app, err := tui.New(version, absPath)
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error initializing Az-Burrow: %v\n", err)
		os.Exit(1)
	}

	if err := app.Run(); err != nil {
		fmt.Fprintf(os.Stderr, "Error running Az-Burrow: %v\n", err)
		os.Exit(1)
	}
}
