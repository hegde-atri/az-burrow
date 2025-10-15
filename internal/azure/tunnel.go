package azure

import (
	"bufio"
	"context"
	"fmt"
	"os/exec"
	"regexp"
	"runtime"
	"strconv"
	"strings"
	"sync"

	"github.com/hegde-atri/az-burrow/internal/types"
)

// TunnelManager handles Azure Bastion tunnel lifecycle
type TunnelManager struct {
	tunnels map[int]*TunnelProcess // Map of tunnel index to process
	mu      sync.RWMutex           // Protect concurrent access
}

// TunnelProcess represents a running tunnel process
type TunnelProcess struct {
	Cmd       *exec.Cmd
	Cancel    context.CancelFunc
	StatusCh  chan string  // Channel for status updates
	ErrorCh   chan error   // Channel for errors
	Logs      []string     // Store output logs
	LocalPort string       // Local port for Windows cleanup
	mu        sync.RWMutex // Protect log access
}

// NewTunnelManager creates a new tunnel manager
func NewTunnelManager() *TunnelManager {
	return &TunnelManager{
		tunnels: make(map[int]*TunnelProcess),
	}
}

// StartTunnel starts an Azure Bastion tunnel for the given configuration
func (tm *TunnelManager) StartTunnel(index int, tunnel types.Tunnel) (chan string, chan error, error) {
	tm.mu.Lock()
	// Check if already running
	if _, exists := tm.tunnels[index]; exists {
		tm.mu.Unlock()
		return nil, nil, fmt.Errorf("tunnel already running")
	}
	tm.mu.Unlock()

	// Create context for cancellation
	ctx, cancel := context.WithCancel(context.Background())

	// Build the az command
	args := []string{
		"network", "bastion", "tunnel",
		"--name", tunnel.Machine.BastionName,
		"--resource-group", tunnel.Machine.BastionResourceGroup,
		"--target-resource-id", tunnel.Machine.TargetResourceID,
		"--resource-port", tunnel.RemotePort,
		"--port", tunnel.LocalPort,
	}

	cmd := exec.CommandContext(ctx, "az", args...)

	// Create channels for status and error reporting
	statusCh := make(chan string, 10)
	errorCh := make(chan error, 10)

	// Capture stdout for status monitoring
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		cancel()
		return nil, nil, fmt.Errorf("failed to create stdout pipe: %w", err)
	}

	// Capture stderr for error monitoring
	stderr, err := cmd.StderrPipe()
	if err != nil {
		cancel()
		return nil, nil, fmt.Errorf("failed to create stderr pipe: %w", err)
	}

	// Start the command
	if err := cmd.Start(); err != nil {
		cancel()
		return nil, nil, fmt.Errorf("failed to start tunnel: %w", err)
	}

	// Store the process
	tp := &TunnelProcess{
		Cmd:       cmd,
		Cancel:    cancel,
		StatusCh:  statusCh,
		ErrorCh:   errorCh,
		Logs:      make([]string, 0, 100),
		LocalPort: tunnel.LocalPort,
	}

	tm.mu.Lock()
	tm.tunnels[index] = tp
	tm.mu.Unlock()

	// Initial status
	statusCh <- "Connecting..."

	// Monitor stdout in a goroutine
	go func() {
		scanner := bufio.NewScanner(stdout)
		for scanner.Scan() {
			line := scanner.Text()

			// Store log
			tp.mu.Lock()
			tp.Logs = append(tp.Logs, "[OUT] "+line)
			// Keep only last 100 lines
			if len(tp.Logs) > 100 {
				tp.Logs = tp.Logs[len(tp.Logs)-100:]
			}
			tp.mu.Unlock()

			// Parse output for status indicators
			if strings.Contains(line, "Tunnel is ready") || strings.Contains(line, "connect on port") {
				statusCh <- "Active"
			} else if strings.Contains(line, "Opening tunnel") {
				statusCh <- "Connecting..."
			}
		}
	}()

	// Note: Azure CLI writes normal output to stderr, not just errors!
	go func() {
		scanner := bufio.NewScanner(stderr)
		for scanner.Scan() {
			line := scanner.Text()
			if line != "" {
				// Store log (Azure CLI uses stderr for normal output)
				tp.mu.Lock()
				tp.Logs = append(tp.Logs, line)
				if len(tp.Logs) > 100 {
					tp.Logs = tp.Logs[len(tp.Logs)-100:]
				}
				tp.mu.Unlock()

				// Parse stderr for status indicators (Azure CLI outputs here)
				if strings.Contains(line, "Tunnel is ready") || strings.Contains(line, "connect on port") {
					statusCh <- "Active"
				} else if strings.Contains(line, "Opening tunnel") {
					statusCh <- "Connecting..."
				}
				// Only send actual errors to error channel
				if strings.Contains(strings.ToLower(line), "error") || strings.Contains(strings.ToLower(line), "failed") {
					errorCh <- fmt.Errorf("%s", line)
				}
			}
		}
	}()

	// Wait for process completion in a goroutine
	go func() {
		err := cmd.Wait()
		if err != nil {
			tp.mu.Lock()
			tp.Logs = append(tp.Logs, fmt.Sprintf("[ERR] Process exited: %v", err))
			tp.mu.Unlock()
			errorCh <- fmt.Errorf("tunnel process exited: %w", err)
		}
		// Clean up
		tm.mu.Lock()
		delete(tm.tunnels, index)
		tm.mu.Unlock()
		close(statusCh)
		close(errorCh)
	}()

	return statusCh, errorCh, nil
}

// StopTunnel stops a running tunnel
func (tm *TunnelManager) StopTunnel(index int) error {
	tm.mu.Lock()
	tp, exists := tm.tunnels[index]
	tm.mu.Unlock()

	if !exists {
		return fmt.Errorf("tunnel not running")
	}

	// Cancel the context (this will kill the process)
	tp.Cancel()
	if tp.Cmd.Process != nil {
		if err := tp.Cmd.Process.Kill(); err != nil {
			tp.mu.Lock()
			tp.Logs = append(tp.Logs, fmt.Sprintf("[WARN] Failed to kill process: %v", err))
			tp.mu.Unlock()
		}
	}

	// On Windows, also kill by port since process might not die properly
	if runtime.GOOS == "windows" {
		if err := killWindowsProcessByPort(tp.LocalPort); err != nil {
			// Log but don't fail - process might already be dead
			tp.mu.Lock()
			tp.Logs = append(tp.Logs, fmt.Sprintf("[WARN] Failed to kill Windows process by port: %v", err))
			tp.mu.Unlock()
		}
	}

	// Clean up
	tm.mu.Lock()
	delete(tm.tunnels, index)
	tm.mu.Unlock()

	return nil
}

// killWindowsProcessByPort finds and kills the process listening on a specific port on Windows
func killWindowsProcessByPort(port string) error {
	// Run netstat to find the PID listening on the port
	cmd := exec.Command("netstat", "-ano")
	output, err := cmd.Output()
	if err != nil {
		return fmt.Errorf("failed to run netstat: %w", err)
	}

	// Parse output to find the PID
	// Looking for lines like: TCP    127.0.0.1:2222         0.0.0.0:0              LISTENING       23216
	lines := strings.Split(string(output), "\n")
	re := regexp.MustCompile(`\s+TCP\s+127\.0\.0\.1:` + port + `\s+.*LISTENING\s+(\d+)`)

	var pid string
	for _, line := range lines {
		matches := re.FindStringSubmatch(line)
		if len(matches) > 1 {
			pid = matches[1]
			break
		}
	}

	if pid == "" {
		// Process might already be dead
		return nil
	}

	// Validate PID is a number
	if _, err := strconv.Atoi(pid); err != nil {
		return fmt.Errorf("invalid PID: %s", pid)
	}

	// Kill the process using taskkill
	killCmd := exec.Command("taskkill", "/PID", pid, "/F")
	if err := killCmd.Run(); err != nil {
		return fmt.Errorf("failed to kill process %s: %w", pid, err)
	}

	return nil
}

// GetLogs retrieves the logs for a tunnel
func (tm *TunnelManager) GetLogs(index int) []string {
	tm.mu.RLock()
	tp, exists := tm.tunnels[index]
	tm.mu.RUnlock()

	if !exists {
		return []string{"Tunnel not running"}
	}

	tp.mu.RLock()
	defer tp.mu.RUnlock()

	// Return a copy of the logs
	logsCopy := make([]string, len(tp.Logs))
	copy(logsCopy, tp.Logs)
	return logsCopy
}

// IsRunning checks if a tunnel is currently running
func (tm *TunnelManager) IsRunning(index int) bool {
	_, exists := tm.tunnels[index]
	return exists
}

// StopAll stops all running tunnels
func (tm *TunnelManager) StopAll() {
	for index := range tm.tunnels {
		if err := tm.StopTunnel(index); err != nil {
			fmt.Printf("Error stopping tunnel %d: %v\n", index, err)
		}
	}
}
