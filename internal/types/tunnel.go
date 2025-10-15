package types

// Machine represents an Azure VM configuration from the YAML file
type Machine struct {
	Name                 string
	ResourceGroup        string
	TargetResourceID     string
	BastionName          string
	BastionResourceGroup string
}

// Tunnel represents an active or configured tunnel with its runtime state
type Tunnel struct {
	// Associated machine
	Machine Machine

	// Port configuration
	LocalPort  string
	RemotePort string

	// Runtime state
	Status        string // "Active", "Inactive", "Connecting...", "Error"
	ReverseTunnel bool
}
