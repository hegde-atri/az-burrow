package types

// Machine represents an Azure VM configuration from the YAML file
type Machine struct {
	Name                 string
	ResourceGroup        string
	TargetResourceID     string
	BastionName          string
	BastionResourceGroup string
	BastionSubscription  string
	// SSH Configuration (optional)
	SSHConfigPath string // Path to SSH config directory (e.g., ~/.ssh/az_ssh_config/vm-name)
}

// Tunnel represents an active or configured tunnel with its runtime state
type Tunnel struct {
	// Unique identifier for this tunnel instance
	ID int

	// Associated machine
	Machine Machine

	// Port configuration
	LocalPort  string
	RemotePort string

	// Runtime state
	Status string // "Active", "Inactive", "Connecting...", "Error"

	// Certificate status (for SSH tunnels)
	CertStatus    string // "valid", "expiring_soon", "expired", "renewing", "renewal_failed"
	CertExpiresIn string // Human-readable time until expiry (e.g., "45m30s")
}
