package main

import (
	"fmt"
	"os"

	"github.com/hegde-atri/az-burrow/internal/tui"
)

const version = "0.1.0"

// main is the entry point for Az-Burrow, a cosy TUI for managing Azure Bastion tunnels.
// It initializes the TUI application and handles any startup errors gracefully.
func main() {
	// Initialize and run the TUI application
	app := tui.New(version)

	if err := app.Run(); err != nil {
		fmt.Fprintf(os.Stderr, "Error running Az-Burrow: %v\n", err)
		os.Exit(1)
	}
}