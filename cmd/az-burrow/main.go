package main

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/hegde-atri/az-burrow/internal/tui"
)

const version = "0.1.2"

func printHelp() {
	fmt.Printf(`az-burrow v%s - A cosy TUI for managing Azure Bastion SSH tunnels

Usage:
  az-burrow [config-file]
  az-burrow -h | --help
  az-burrow --version

Arguments:
  config-file    Path to YAML configuration file (default: burrow.config.yaml)

Options:
  -h, --help     Show this help message
  --version      Show version information

Configuration:
  az-burrow requires a YAML configuration file that defines your Azure VMs.
	How config file is chosen:
		Check if a custom config-file path was provided
		Else, look for 'burrow.config.yaml' in the current directory
		Else, look for '~/.config/burrow.config.yaml'

  Example config file (burrow.config.yaml):

    machines:
      - name: my-vm
        resource_group: MY-RG
        target_resource_id: /subscriptions/.../virtualMachines/my-vm
        bastion_name: my-bastion
        bastion_resource_group: BASTION-RG

  Each machine requires:
    - name:                   Display name for the VM
    - resource_group:         Azure resource group containing the VM
    - target_resource_id:     Full Azure resource ID of the VM
    - bastion_name:           Name of the Azure Bastion host
    - bastion_resource_group: Resource group containing the Bastion

Examples:
  az-burrow                              # Use default config file
  az-burrow ./my-vms.yaml                # Use specific config file
  az-burrow /etc/burrow/production.yaml  # Use absolute path

For more information and examples:
  https://github.com/hegde-atri/az-burrow

`, version)
}

// main is the entry point for Az-Burrow, a cosy TUI for managing Azure Bastion tunnels.
// It initializes the TUI application and handles any startup errors gracefully.
func main() {
	// Handle help and version flags
	if len(os.Args) > 1 {
		switch os.Args[1] {
		case "-h", "--help":
			printHelp()
			os.Exit(0)
		case "--version":
			fmt.Printf("Az-Burrow v%s\n", version)
			os.Exit(0)
		}
	}

	// Determine config file path
	var configPath string
	if len(os.Args) > 1 {
		configPath = os.Args[1]
	} else {
		candidates := []string{"burrow.config.yaml"}
		if home, err := os.UserHomeDir(); err == nil {
			candidates = append(candidates, filepath.Join(home, ".config", "burrow.config.yaml"))
		}

		// pick the first candidate that exists, otherwise default to the first candidate
		found := ""
		for _, c := range candidates {
			if _, err := os.Stat(c); err == nil {
				found = c
				break
			}
		}
		if found != "" {
			configPath = found
		} else {
			configPath = candidates[0]
		}
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
