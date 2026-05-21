package config

import (
	"fmt"
	"os"

	"gopkg.in/yaml.v3"
)

// MachineConfig represents a single Azure VM configuration
type MachineConfig struct {
	Name                 string `yaml:"name"`
	ResourceGroup        string `yaml:"resource_group"`
	TargetResourceID     string `yaml:"target_resource_id"`
	BastionName          string `yaml:"bastion_name"`
	BastionResourceGroup string `yaml:"bastion_resource_group"`
	BastionSubscription  string `yaml:"bastion_subscription"`
	// Optional SSH configuration for certificate management
	SSHConfigPath string `yaml:"ssh_config_path,omitempty"`
}

// Config represents the root configuration structure
type Config struct {
	Machines []MachineConfig `yaml:"machines"`
}

// Load reads and parses the YAML configuration file
func Load(path string) (*Config, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("failed to read config file: %w", err)
	}

	var config Config
	if err := yaml.Unmarshal(data, &config); err != nil {
		return nil, fmt.Errorf("failed to parse config file: %w", err)
	}

	return &config, nil
}

// LoadOrPrompt attempts to load the config file, and returns an error with
// a helpful message if it fails
func LoadOrPrompt(path string) (*Config, error) {
	config, err := Load(path)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, fmt.Errorf("config file not found at %s\n\nPlease create a burrow.config.yaml file with your Azure VM configurations.\nSee the example in the repository for the required format", path)
		}
		return nil, err
	}

	if len(config.Machines) == 0 {
		return nil, fmt.Errorf("no machines defined in config file %s", path)
	}

	return config, nil
}
