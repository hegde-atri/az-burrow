package azure

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"
	"sync"
	"time"
)

// CertificateManager handles Azure AD SSH certificate lifecycle and renewal
type CertificateManager struct {
	// Map of VM name to certificate info
	certs map[string]*CertInfo
	mu    sync.RWMutex

	// Status channel for certificate events
	statusCh chan CertStatus

	// Context for cancellation
	ctx    context.Context
	cancel context.CancelFunc
}

// CertInfo stores information about a certificate
type CertInfo struct {
	VMName         string
	SSHConfigPath  string // Path to SSH config directory (e.g., ~/.ssh/az_ssh_config/vm-name)
	PublicKeyPath  string // Path to public key
	CertPath       string // Path to certificate file
	CreatedAt      time.Time
	ExpiresAt      time.Time
	LastRenewalTry time.Time
	RenewalStatus  string // "valid", "expiring_soon", "expired", "renewing", "renewal_failed"
	mu             sync.RWMutex
}

// CertStatus represents a certificate status update
type CertStatus struct {
	VMName    string
	Status    string // "valid", "renewed", "expiring_soon", "expired", "renewal_failed"
	Message   string
	ExpiresIn time.Duration
}

const (
	CertLifetime      = 1 * time.Hour    // Azure AD SSH certs expire after 1 hour
	RenewalWindow     = 5 * time.Minute  // Start trying to renew 5 minutes before expiry
	RenewalRetryDelay = 30 * time.Second // Retry every 30 seconds if renewal fails
	CheckInterval     = 1 * time.Minute  // Check certificate status every minute
)

// NewCertificateManager creates a new certificate manager
func NewCertificateManager() *CertificateManager {
	ctx, cancel := context.WithCancel(context.Background())
	return &CertificateManager{
		certs:    make(map[string]*CertInfo),
		statusCh: make(chan CertStatus, 10),
		ctx:      ctx,
		cancel:   cancel,
	}
}

// RegisterCertificate registers a certificate for monitoring and auto-renewal
// sshConfigPath should be the base path for SSH config (e.g., ~/.ssh/az_ssh_config/vm-name)
func (cm *CertificateManager) RegisterCertificate(vmName, sshConfigPath string) error {
	cm.mu.Lock()
	defer cm.mu.Unlock()

	// Expand home directory if needed
	if strings.HasPrefix(sshConfigPath, "~") {
		homeDir, err := os.UserHomeDir()
		if err != nil {
			return fmt.Errorf("failed to get home directory: %w", err)
		}
		sshConfigPath = filepath.Join(homeDir, sshConfigPath[2:])
	}

	// Construct paths
	publicKeyPath := filepath.Join(sshConfigPath, "id_rsa.pub")
	certPath := publicKeyPath + "-aadcert.pub"

	// Check if certificate exists
	_, err := os.Stat(certPath)
	if err != nil {
		if os.IsNotExist(err) {
			// Certificate doesn't exist yet - mark as expired so it will be generated
			cm.certs[vmName] = &CertInfo{
				VMName:        vmName,
				SSHConfigPath: sshConfigPath,
				PublicKeyPath: publicKeyPath,
				CertPath:      certPath,
				ExpiresAt:     time.Now(), // Already expired
				RenewalStatus: "expired",
			}
			return nil
		}
		return fmt.Errorf("failed to check certificate: %w", err)
	}

	// Try to read the certificate expiry time using ssh-keygen
	var expiresAt time.Time
	cmd := exec.Command("ssh-keygen", "-L", "-f", certPath)
	output, err := cmd.CombinedOutput()
	if err == nil {
		// Parse the certificate details
		expiresAt, err = parseCertificateExpiry(string(output))
		if err != nil {
			// Fallback to file modification time + 1 hour
			certInfo, _ := os.Stat(certPath)
			expiresAt = certInfo.ModTime().Add(CertLifetime)
		}
	} else {
		// Fallback to file modification time + 1 hour
		certInfo, _ := os.Stat(certPath)
		expiresAt = certInfo.ModTime().Add(CertLifetime)
	}

	ci := &CertInfo{
		VMName:        vmName,
		SSHConfigPath: sshConfigPath,
		PublicKeyPath: publicKeyPath,
		CertPath:      certPath,
		CreatedAt:     time.Now(),
		ExpiresAt:     expiresAt,
		RenewalStatus: getRenewalStatus(expiresAt),
	}

	cm.certs[vmName] = ci

	// Send initial status
	select {
	case cm.statusCh <- CertStatus{
		VMName:    vmName,
		Status:    ci.RenewalStatus,
		Message:   fmt.Sprintf("Certificate tracked: expires at %s", expiresAt.Format("15:04:05")),
		ExpiresIn: time.Until(expiresAt),
	}:
	default:
	}

	return nil
}

// StartMonitoring starts the certificate monitoring and auto-renewal goroutine
func (cm *CertificateManager) StartMonitoring() {
	go cm.monitorLoop()
}

// monitorLoop continuously monitors certificates and handles renewal
func (cm *CertificateManager) monitorLoop() {
	ticker := time.NewTicker(CheckInterval)
	defer ticker.Stop()

	for {
		select {
		case <-cm.ctx.Done():
			return
		case <-ticker.C:
			cm.checkAndRenewCertificates()
		}
	}
}

// checkAndRenewCertificates checks all registered certificates and renews if needed
func (cm *CertificateManager) checkAndRenewCertificates() {
	cm.mu.RLock()
	certs := make([]*CertInfo, 0, len(cm.certs))
	for _, cert := range cm.certs {
		certs = append(certs, cert)
	}
	cm.mu.RUnlock()

	now := time.Now()

	for _, cert := range certs {
		cert.mu.Lock()
		expiresAt := cert.ExpiresAt
		lastTry := cert.LastRenewalTry
		status := cert.RenewalStatus
		cert.mu.Unlock()

		timeUntilExpiry := time.Until(expiresAt)

		// Update status based on time until expiry
		newStatus := getRenewalStatus(expiresAt)
		if newStatus != status {
			cert.mu.Lock()
			cert.RenewalStatus = newStatus
			cert.mu.Unlock()

			select {
			case cm.statusCh <- CertStatus{
				VMName:    cert.VMName,
				Status:    newStatus,
				Message:   fmt.Sprintf("Certificate status changed to: %s", newStatus),
				ExpiresIn: timeUntilExpiry,
			}:
			default:
			}
		}

		// Check if we should attempt renewal
		// Renewal window: last 5 minutes before expiry
		// But also try immediately if expired or in renewal_failed state
		shouldRenew := false

		if timeUntilExpiry <= 0 {
			// Certificate expired - try to renew
			shouldRenew = true
		} else if timeUntilExpiry <= RenewalWindow {
			// Within renewal window - try to renew
			// Only retry if we haven't tried in the last 30 seconds
			if now.Sub(lastTry) >= RenewalRetryDelay {
				shouldRenew = true
			}
		}

		if shouldRenew {
			go cm.renewCertificate(cert)
		}
	}
}

// renewCertificate attempts to renew a certificate
func (cm *CertificateManager) renewCertificate(cert *CertInfo) {
	cert.mu.Lock()
	cert.LastRenewalTry = time.Now()
	cert.RenewalStatus = "renewing"
	vmName := cert.VMName
	publicKeyPath := cert.PublicKeyPath
	certPath := cert.CertPath
	cert.mu.Unlock()

	// Notify that renewal is starting
	select {
	case cm.statusCh <- CertStatus{
		VMName:  vmName,
		Status:  "renewing",
		Message: "Attempting to renew certificate...",
	}:
	default:
	}

	// Run: az ssh cert --file <cert-path> --public-key-file <public-key-path>
	cmd := exec.CommandContext(cm.ctx, "az", "ssh", "cert",
		"--file", certPath,
		"--public-key-file", publicKeyPath,
	)

	output, err := cmd.CombinedOutput()
	if err != nil {
		cert.mu.Lock()
		cert.RenewalStatus = "renewal_failed"
		cert.mu.Unlock()

		errMsg := string(output)
		if errMsg == "" {
			errMsg = err.Error()
		}

		select {
		case cm.statusCh <- CertStatus{
			VMName:  vmName,
			Status:  "renewal_failed",
			Message: fmt.Sprintf("Certificate renewal failed: %s", errMsg),
		}:
		default:
		}
		return
	}

	// Parse the actual expiry time from the output
	expiresAt, err := parseExpiryFromOutput(string(output))
	if err != nil {
		// Fallback to estimated time if parsing fails
		expiresAt = time.Now().Add(CertLifetime)
	}

	cert.mu.Lock()
	cert.CreatedAt = time.Now()
	cert.ExpiresAt = expiresAt
	cert.RenewalStatus = "valid"
	cert.mu.Unlock()

	select {
	case cm.statusCh <- CertStatus{
		VMName:    vmName,
		Status:    "renewed",
		Message:   fmt.Sprintf("Certificate renewed successfully! Expires at: %s", expiresAt.Format("15:04:05")),
		ExpiresIn: time.Until(expiresAt),
	}:
	default:
	}
}

// GenerateCertificate generates a new certificate for the first time
// This should be called before the first connection
func (cm *CertificateManager) GenerateCertificate(vmName, sshConfigPath, publicKeyPath string) error {
	// Expand home directory if needed
	if strings.HasPrefix(sshConfigPath, "~") {
		homeDir, err := os.UserHomeDir()
		if err != nil {
			return fmt.Errorf("failed to get home directory: %w", err)
		}
		sshConfigPath = filepath.Join(homeDir, sshConfigPath[2:])
	}

	if strings.HasPrefix(publicKeyPath, "~") {
		homeDir, err := os.UserHomeDir()
		if err != nil {
			return fmt.Errorf("failed to get home directory: %w", err)
		}
		publicKeyPath = filepath.Join(homeDir, publicKeyPath[2:])
	}

	certPath := publicKeyPath + "-aadcert.pub"

	// Ensure directory exists
	if err := os.MkdirAll(filepath.Dir(publicKeyPath), 0o700); err != nil {
		return fmt.Errorf("failed to create SSH config directory: %w", err)
	}

	// Generate SSH key pair if it doesn't exist
	if _, err := os.Stat(publicKeyPath); os.IsNotExist(err) {
		privateKeyPath := strings.TrimSuffix(publicKeyPath, ".pub")
		cmd := exec.Command("ssh-keygen", "-t", "rsa", "-b", "4096",
			"-f", privateKeyPath, "-N", "")
		if output, err := cmd.CombinedOutput(); err != nil {
			return fmt.Errorf("failed to generate SSH key: %s: %w", string(output), err)
		}
	}

	// Generate the Azure AD certificate
	cmd := exec.CommandContext(cm.ctx, "az", "ssh", "cert",
		"--file", certPath,
		"--public-key-file", publicKeyPath,
	)

	output, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("failed to generate certificate: %s: %w", string(output), err)
	}

	// Parse the actual expiry time from the output
	expiresAt, err := parseExpiryFromOutput(string(output))
	if err != nil {
		// Fallback to estimated time if parsing fails
		expiresAt = time.Now().Add(CertLifetime)
	}

	// Register the certificate for monitoring with the actual expiry time
	cm.mu.Lock()
	cm.certs[vmName] = &CertInfo{
		VMName:        vmName,
		SSHConfigPath: sshConfigPath,
		PublicKeyPath: publicKeyPath,
		CertPath:      certPath,
		CreatedAt:     time.Now(),
		ExpiresAt:     expiresAt,
		RenewalStatus: "valid",
	}
	cm.mu.Unlock()

	// Send status update
	select {
	case cm.statusCh <- CertStatus{
		VMName:    vmName,
		Status:    "valid",
		Message:   fmt.Sprintf("Certificate generated! Expires at: %s", expiresAt.Format("15:04:05")),
		ExpiresIn: time.Until(expiresAt),
	}:
	default:
	}

	return nil
}

// GetStatus returns the current status of a certificate
func (cm *CertificateManager) GetStatus(vmName string) (string, time.Duration, error) {
	cm.mu.RLock()
	cert, exists := cm.certs[vmName]
	cm.mu.RUnlock()

	if !exists {
		return "", 0, fmt.Errorf("certificate not registered")
	}

	cert.mu.RLock()
	defer cert.mu.RUnlock()

	return cert.RenewalStatus, time.Until(cert.ExpiresAt), nil
}

// StatusChannel returns the channel for certificate status updates
func (cm *CertificateManager) StatusChannel() <-chan CertStatus {
	return cm.statusCh
}

// Stop stops the certificate manager
func (cm *CertificateManager) Stop() {
	cm.cancel()
}

// getRenewalStatus determines the renewal status based on time until expiry
func getRenewalStatus(expiresAt time.Time) string {
	timeUntil := time.Until(expiresAt)

	if timeUntil <= 0 {
		return "expired"
	} else if timeUntil <= RenewalWindow {
		return "expiring_soon"
	}
	return "valid"
}

// parseExpiryFromOutput parses the expiry time from Azure CLI output
// Example output: "Generated SSH certificate ... is valid until 2025-10-15 18:06:23 in local time."
func parseExpiryFromOutput(output string) (time.Time, error) {
	// Match pattern: "is valid until YYYY-MM-DD HH:MM:SS in local time"
	re := regexp.MustCompile(`is valid until (\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}) in local time`)
	matches := re.FindStringSubmatch(output)

	if len(matches) < 2 {
		return time.Time{}, fmt.Errorf("could not parse expiry time from output")
	}

	// Parse the timestamp
	expiryStr := matches[1]
	expiresAt, err := time.ParseInLocation("2006-01-02 15:04:05", expiryStr, time.Local)
	if err != nil {
		return time.Time{}, fmt.Errorf("failed to parse expiry time: %w", err)
	}

	return expiresAt, nil
}

// parseCertificateExpiry parses expiry time from ssh-keygen -L output
// Example output contains: "Valid: from 2025-10-15T17:31:23 to 2025-10-15T18:31:23"
func parseCertificateExpiry(output string) (time.Time, error) {
	// Match pattern: "Valid: from ... to YYYY-MM-DDTHH:MM:SS"
	re := regexp.MustCompile(`Valid: from .+ to (\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})`)
	matches := re.FindStringSubmatch(output)

	if len(matches) < 2 {
		return time.Time{}, fmt.Errorf("could not parse certificate expiry from ssh-keygen output")
	}

	// Parse the timestamp (it's in local time)
	expiryStr := matches[1]
	expiresAt, err := time.ParseInLocation("2006-01-02T15:04:05", expiryStr, time.Local)
	if err != nil {
		return time.Time{}, fmt.Errorf("failed to parse certificate expiry: %w", err)
	}

	return expiresAt, nil
}
