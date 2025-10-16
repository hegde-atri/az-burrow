package tui

import (
	"fmt"
	"time"

	"github.com/charmbracelet/bubbles/table"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/hegde-atri/az-burrow/internal/azure"
	"github.com/hegde-atri/az-burrow/internal/config"
	"github.com/hegde-atri/az-burrow/internal/types"
)

// TunnelStatusMsg is sent when a tunnel's status changes
type TunnelStatusMsg struct {
	TunnelID int
	Status   string
}

// TunnelErrorMsg is sent when a tunnel encounters an error
type TunnelErrorMsg struct {
	TunnelID int
	Error    error
}

// CertStatusMsg is sent when a certificate's status changes
type CertStatusMsg struct {
	Status azure.CertStatus
}

// TickMsg is sent every minute to update certificate expiry times
type TickMsg time.Time

// ClearNotificationMsg is sent to clear the notification
type ClearNotificationMsg struct{}

// App represents the main TUI application for Az-Burrow
type App struct {
	version            string
	program            *tea.Program
	tunnelManager      *azure.TunnelManager
	certificateManager *azure.CertificateManager
}

// New creates and initializes a new Az-Burrow TUI application
// It sets up the bubbletea program with the initial model
func New(version string, configPath string) (*App, error) {
	// Load machine configurations from YAML
	cfg, err := config.LoadOrPrompt(configPath)
	if err != nil {
		return nil, err
	}

	// Convert config to machine types
	machines := make([]types.Machine, len(cfg.Machines))
	for i, mc := range cfg.Machines {
		machines[i] = types.Machine{
			Name:                 mc.Name,
			ResourceGroup:        mc.ResourceGroup,
			TargetResourceID:     mc.TargetResourceID,
			BastionName:          mc.BastionName,
			BastionResourceGroup: mc.BastionResourceGroup,
			SSHConfigPath:        mc.SSHConfigPath,
		}
	}

	tunnelManager := azure.NewTunnelManager()
	certificateManager := azure.NewCertificateManager()

	// Register certificates for machines that have SSH config
	for _, machine := range machines {
		if machine.SSHConfigPath != "" {
			// Register certificate (will be tracked even if doesn't exist yet)
			if err := certificateManager.RegisterCertificate(machine.Name, machine.SSHConfigPath); err != nil {
				// Log but don't fail - certificate might not exist yet
				fmt.Printf("Note: Certificate not found for %s: %v\n", machine.Name, err)
			}
		}
	}

	// Start monitoring certificates
	certificateManager.StartMonitoring()

	m := model{
		version:              version,
		machines:             machines,
		tunnels:              []types.Tunnel{}, // Start with no active tunnels
		tunnelManager:        tunnelManager,
		certificateManager:   certificateManager,
		table:                createTunnelTable([]types.Tunnel{}),
		tunnelStatusChannels: make(map[int]chan string),
		tunnelErrorChannels:  make(map[int]chan error),
		nextTunnelID:         1, // Start tunnel IDs from 1
	}

	p := tea.NewProgram(m, tea.WithAltScreen())

	return &App{
		version:            version,
		program:            p,
		tunnelManager:      tunnelManager,
		certificateManager: certificateManager,
	}, nil
}

// Run starts the TUI application and blocks until it exits
// Returns an error if the program fails to run
func (a *App) Run() error {
	_, err := a.program.Run()
	// Clean up all tunnels and stop certificate monitoring on exit
	a.tunnelManager.StopAll()
	a.certificateManager.Stop()
	return err
}

// model represents the state of the bubbletea application
// It holds the terminal dimensions to enable responsive fullscreen layout
type model struct {
	version            string
	machines           []types.Machine           // Available VMs from config
	tunnels            []types.Tunnel            // Active tunnels
	tunnelManager      *azure.TunnelManager      // Manages tunnel processes
	certificateManager *azure.CertificateManager // Manages SSH certificates
	table              table.Model
	width              int
	height             int
	// prompt state for creating a new tunnel
	showingCreate      bool
	selectedMachineIdx int    // Index of machine being configured in create flow
	createStep         int    // 0:select machine, 1:local port, 2:remote port
	createLocalPort    string // Temporary input for local port
	createRemotePort   string // Temporary input for remote port
	// confirm dialogs
	showingConfirmQuit   bool
	showingConfirmDelete bool
	deleteTargetIdx      int // Index of tunnel to delete
	// log viewer
	showingLogs   bool
	logsForTunnel int      // Index of tunnel whose logs we're viewing
	tunnelLogs    []string // Current logs being displayed
	// notification
	notification     string    // Current notification message
	notificationTime time.Time // When notification was shown
	// tunnel update channels (indexed by tunnel ID, not slice index)
	tunnelStatusChannels map[int]chan string
	tunnelErrorChannels  map[int]chan error
	nextTunnelID         int // Counter for unique tunnel IDs
}

// Init is called when the program starts
// Returns an initial command to run (nil means no command)
func (m model) Init() tea.Cmd {
	return tea.Batch(
		tickEveryMinute(),
		listenForCertUpdates(m.certificateManager),
	)
}

// Update handles incoming messages and updates the model state
// This implements responsive fullscreen layout by tracking terminal size
// and delegating table navigation to the bubbles/table component
func (m model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	var cmd tea.Cmd

	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		// Track terminal dimensions for responsive layout
		m.width = msg.Width
		m.height = msg.Height

		// Resize table to fit within the terminal
		// Reserve space for header (~6 lines), footer (~2 lines), and padding
		headerHeight := 8
		footerHeight := 3
		availableHeight := m.height - headerHeight - footerHeight

		m.table.SetWidth(m.width - 4)
		m.table.SetHeight(availableHeight)

	case TickMsg:
		// Update certificate expiry times for all tunnels
		m.updateCertificateStatuses()
		m.table = createTunnelTable(m.tunnels)
		return m, tickEveryMinute()

	case ClearNotificationMsg:
		// Clear the notification
		m.notification = ""
		return m, nil

	case CertStatusMsg:
		// Update certificate status for matching tunnel
		for i := range m.tunnels {
			if m.tunnels[i].Machine.Name == msg.Status.VMName {
				m.tunnels[i].CertStatus = msg.Status.Status
				expiresIn := msg.Status.ExpiresIn.Round(time.Second)
				if expiresIn < 0 {
					m.tunnels[i].CertExpiresIn = "expired"
				} else {
					m.tunnels[i].CertExpiresIn = formatDuration(expiresIn)
				}
			}
		}
		m.table = createTunnelTable(m.tunnels)
		return m, listenForCertUpdates(m.certificateManager)

	case TunnelStatusMsg:
		// Update tunnel status - find tunnel by ID
		for i := range m.tunnels {
			if m.tunnels[i].ID == msg.TunnelID {
				m.tunnels[i].Status = msg.Status
				m.table = createTunnelTable(m.tunnels)

				// Continue listening for more updates if channels exist
				if statusCh, ok := m.tunnelStatusChannels[msg.TunnelID]; ok {
					if errorCh, ok := m.tunnelErrorChannels[msg.TunnelID]; ok {
						return m, listenForTunnelUpdates(msg.TunnelID, statusCh, errorCh)
					}
				}
				break
			}
		}
		// If tunnel ID not found, stop listening (tunnel was deleted)

	case TunnelErrorMsg:
		// Handle tunnel error - find tunnel by ID
		for i := range m.tunnels {
			if m.tunnels[i].ID == msg.TunnelID {
				// Check if this tunnel still has active channels (not deleted)
				if _, ok := m.tunnelStatusChannels[msg.TunnelID]; ok {
					m.tunnels[i].Status = "Error"
					m.table = createTunnelTable(m.tunnels)

					// Continue listening for more updates if channels exist
					if statusCh, ok := m.tunnelStatusChannels[msg.TunnelID]; ok {
						if errorCh, ok := m.tunnelErrorChannels[msg.TunnelID]; ok {
							return m, listenForTunnelUpdates(msg.TunnelID, statusCh, errorCh)
						}
					}
				}
				break
			}
		}
		// If tunnel ID not found or channels don't exist, stop listening

	case tea.KeyMsg:
		// Handle keyboard input
		switch msg.String() {
		case "r":
			// Regenerate certificate for selected tunnel
			if !m.showingCreate && !m.showingConfirmQuit && !m.showingConfirmDelete && !m.showingLogs && len(m.tunnels) > 0 {
				selectedIdx := m.table.Cursor()
				if selectedIdx >= 0 && selectedIdx < len(m.tunnels) {
					tunnel := &m.tunnels[selectedIdx]
					if tunnel.Machine.SSHConfigPath != "" {
						// Show notification that regeneration is starting
						m.notification = fmt.Sprintf("🔄 Regenerating certificate for %s...", tunnel.Machine.Name)
						m.notificationTime = time.Now()

						// Generate certificate
						publicKeyPath := tunnel.Machine.SSHConfigPath + "/id_rsa.pub"
						if err := m.certificateManager.GenerateCertificate(
							tunnel.Machine.Name,
							tunnel.Machine.SSHConfigPath,
							publicKeyPath,
						); err != nil {
							m.notification = fmt.Sprintf("❌ Failed to regenerate certificate: %v", err)
							m.notificationTime = time.Now()
						} else {
							tunnel.CertStatus = "valid"
							m.updateCertificateStatuses()
							m.notification = fmt.Sprintf("✅ Certificate regenerated for %s", tunnel.Machine.Name)
							m.notificationTime = time.Now()
						}
						m.table = createTunnelTable(m.tunnels)
						return m, clearNotificationAfter(3 * time.Second)
					} else {
						m.notification = "⚠️ No SSH config path set for this VM"
						m.notificationTime = time.Now()
						return m, clearNotificationAfter(3 * time.Second)
					}
				}
			}
			return m, nil
		case "c":
			// start create-tunnel flow unless we're already in a prompt
			if !m.showingCreate && !m.showingConfirmQuit && !m.showingLogs && len(m.machines) > 0 {
				m.showingCreate = true
				m.createStep = 0
				m.selectedMachineIdx = 0
				m.createLocalPort = ""
				m.createRemotePort = ""
			}
			return m, nil
		case " ":
			// View logs for the selected tunnel
			if !m.showingCreate && !m.showingConfirmQuit && !m.showingConfirmDelete && !m.showingLogs && len(m.tunnels) > 0 {
				selectedIdx := m.table.Cursor()
				if selectedIdx >= 0 && selectedIdx < len(m.tunnels) {
					m.showingLogs = true
					m.logsForTunnel = selectedIdx
					m.tunnelLogs = m.tunnelManager.GetLogs(selectedIdx)
				}
			}
			return m, nil
		case "d", "delete":
			// Show delete confirmation for the currently selected tunnel
			if !m.showingCreate && !m.showingConfirmQuit && !m.showingConfirmDelete && !m.showingLogs && len(m.tunnels) > 0 {
				selectedIdx := m.table.Cursor()
				if selectedIdx >= 0 && selectedIdx < len(m.tunnels) {
					m.showingConfirmDelete = true
					m.deleteTargetIdx = selectedIdx
				}
			}
			return m, nil
		case "q", "ctrl+c":
			// Handle quit - show confirm dialog or cancel existing dialogs
			if m.showingConfirmDelete {
				// Cancel delete confirmation
				m.showingConfirmDelete = false
			} else if !m.showingConfirmQuit && !m.showingCreate {
				m.showingConfirmQuit = true
			} else if m.showingConfirmQuit {
				// allow pressing 'q' again to cancel the quit dialog
				m.showingConfirmQuit = false
			}
			return m, nil
		case "y":
			// Handle confirmation dialogs
			if m.showingConfirmQuit {
				return m, tea.Quit
			} else if m.showingConfirmDelete {
				// Confirm deletion - remove the tunnel
				if m.deleteTargetIdx >= 0 && m.deleteTargetIdx < len(m.tunnels) {
					tunnel := &m.tunnels[m.deleteTargetIdx]

					// Stop the tunnel if it's running
					if tunnel.Status == "Active" || tunnel.Status == "Connecting..." || tunnel.Status == "Starting" {
						err := m.tunnelManager.StopTunnel(m.deleteTargetIdx)
						if err != nil {
							tunnel.Status = "Error: " + err.Error()
						} else {
							tunnel.Status = "Inactive"
						}
					}

					// Clean up the stored channels using tunnel ID
					delete(m.tunnelStatusChannels, tunnel.ID)
					delete(m.tunnelErrorChannels, tunnel.ID)

					// Remove from the tunnel list
					m.tunnels = append(m.tunnels[:m.deleteTargetIdx], m.tunnels[m.deleteTargetIdx+1:]...)
					m.table = createTunnelTable(m.tunnels)
					// Adjust cursor if needed
					if m.deleteTargetIdx > 0 && m.deleteTargetIdx >= len(m.tunnels) {
						m.table.SetCursor(len(m.tunnels) - 1)
					}
				}
				m.showingConfirmDelete = false
			}
		case "esc":
			// Esc can cancel dialogs
			if m.showingConfirmQuit {
				m.showingConfirmQuit = false
				return m, nil
			} else if m.showingConfirmDelete {
				m.showingConfirmDelete = false
				return m, nil
			} else if m.showingCreate {
				m.showingCreate = false
				return m, nil
			} else if m.showingLogs {
				m.showingLogs = false
				return m, nil
			}
		}

		// If we're showing the create prompt, handle input first
		if m.showingCreate {
			switch m.createStep {
			case 0: // Machine selection
				switch msg.String() {
				case "up", "k":
					if m.selectedMachineIdx > 0 {
						m.selectedMachineIdx--
					}
				case "down", "j":
					if m.selectedMachineIdx < len(m.machines)-1 {
						m.selectedMachineIdx++
					}
				case "enter":
					m.createStep = 1 // Move to local port input
				}
			case 1, 2: // Port input steps
				switch msg.Type {
				case tea.KeyRunes:
					r := msg.String()
					// Only accept numeric input for ports
					if len(r) == 1 && r[0] >= '0' && r[0] <= '9' {
						if m.createStep == 1 {
							m.createLocalPort += r
						} else {
							m.createRemotePort += r
						}
					}
				case tea.KeyBackspace:
					if m.createStep == 1 && len(m.createLocalPort) > 0 {
						m.createLocalPort = m.createLocalPort[:len(m.createLocalPort)-1]
					} else if m.createStep == 2 && len(m.createRemotePort) > 0 {
						m.createRemotePort = m.createRemotePort[:len(m.createRemotePort)-1]
					}
				case tea.KeyEnter:
					if m.createStep == 1 && len(m.createLocalPort) > 0 {
						m.createStep = 2
					} else if m.createStep == 2 && len(m.createRemotePort) > 0 {
						// Create the tunnel with a unique ID
						newTunnel := types.Tunnel{
							ID:         m.nextTunnelID,
							Machine:    m.machines[m.selectedMachineIdx],
							LocalPort:  m.createLocalPort,
							RemotePort: m.createRemotePort,
							Status:     "Inactive",
						}
						m.nextTunnelID++ // Increment for next tunnel
						m.tunnels = append(m.tunnels, newTunnel)

						// Update certificate statuses for the new tunnel
						m.updateCertificateStatuses()

						m.table = createTunnelTable(m.tunnels)
						m.showingCreate = false
					}
				}
			}

			return m, nil
		}

		// Handle Enter key to start/stop tunnels
		if !m.showingConfirmQuit && !m.showingConfirmDelete && !m.showingLogs && len(m.tunnels) > 0 {
			switch msg.String() {
			case "enter":
				cursor := m.table.Cursor()
				if cursor < len(m.tunnels) {
					tunnel := &m.tunnels[cursor]

					switch tunnel.Status {
					case "Inactive":
						tunnel.Status = "Starting"
						m.table = createTunnelTable(m.tunnels)

						// Start the tunnel asynchronously (still using cursor as the manager's index)
						statusCh, errorCh, err := m.tunnelManager.StartTunnel(cursor, *tunnel)
						if err != nil {
							tunnel.Status = "Error: " + err.Error()
							m.table = createTunnelTable(m.tunnels)
							return m, nil
						}

						// Store the channels using tunnel ID for proper tracking
						m.tunnelStatusChannels[tunnel.ID] = statusCh
						m.tunnelErrorChannels[tunnel.ID] = errorCh

						// Start listening for status and error updates using tunnel ID
						return m, listenForTunnelUpdates(tunnel.ID, statusCh, errorCh)

					case "Active":
						// Stop the tunnel
						err := m.tunnelManager.StopTunnel(cursor)
						if err != nil {
							tunnel.Status = "Error: " + err.Error()
						} else {
							tunnel.Status = "Inactive"
						}
						// Clean up the stored channels using tunnel ID
						delete(m.tunnelStatusChannels, tunnel.ID)
						delete(m.tunnelErrorChannels, tunnel.ID)
						m.table = createTunnelTable(m.tunnels)

					default:
						// Do nothing if the tunnel is starting or stopping
					}
				}
			}
		}

		// Delegate arrow key navigation to the table component
		m.table, cmd = m.table.Update(msg)
	}

	return m, cmd
}

// View renders the UI based on the current model state
// Implements a fullscreen adaptive layout with header, content, and footer sections
// The layout responds to terminal size changes for optimal viewing
func (m model) View() string {
	// Define base styles
	var (
		// Purple theme colors
		primaryColor   = lipgloss.Color("#7D56F4")
		secondaryColor = lipgloss.Color("#FF8C00")
		mutedColor     = lipgloss.Color("#626262")

		// Header styles
		asciiStyle = lipgloss.NewStyle().
				Foreground(secondaryColor).
				Bold(true)

		titleStyle = lipgloss.NewStyle().
				Foreground(primaryColor).
				Bold(true)

		subtitleStyle = lipgloss.NewStyle().
				Foreground(primaryColor).
				Italic(true).
				MarginBottom(1)

		// Footer style
		footerStyle = lipgloss.NewStyle().
				Foreground(mutedColor).
				MarginTop(1).
				Align(lipgloss.Center)
	)

	// ASCII art of a cute burrow/badger
	ascii := asciiStyle.Render(`  ___
 (o o)
 (. .)
  \-/  `)

	// Title aligned next to ASCII art
	title := titleStyle.Render(fmt.Sprintf("Burrow v%s ~ hegde-atri", m.version))

	// Combine ASCII and title horizontally with some spacing
	headerTop := lipgloss.JoinHorizontal(
		lipgloss.Top,
		ascii,
		lipgloss.NewStyle().Padding(0, 2).Render(title),
	)

	// Subtitle below the header
	subtitle := subtitleStyle.Render("Your cosy tunnel to Azure VMs")

	// Build header section
	header := lipgloss.JoinVertical(
		lipgloss.Left,
		headerTop,
		subtitle,
	)

	// Render the table (main content area)
	tableView := m.table.View()

	// Show notification if present
	var notificationView string
	if m.notification != "" {
		notificationStyle := lipgloss.NewStyle().
			Foreground(lipgloss.Color("#FFFFFF")).
			Background(lipgloss.Color("#7D56F4")).
			Padding(0, 2).
			Bold(true).
			Align(lipgloss.Center)
		notificationView = notificationStyle.Render(m.notification)
	}

	// Footer with navigation hints - show delete option if tunnels exist
	footerText := "c: create • ↑/↓: navigate • q: quit"
	if len(m.tunnels) > 0 {
		footerText = "c: create • Enter: start/stop • Space: logs • r: regen cert • d: delete • ↑/↓: navigate • q: quit"
	}
	footer := footerStyle.Render(footerText)

	// Combine all sections vertically to create fullscreen layout
	// This approach ensures the content adapts to terminal size:
	// 1. Header stays at top with branding
	// 2. Table expands to fill available space
	// 3. Notification shows if present
	// 4. Footer stays at bottom with controls
	var contentParts []string
	contentParts = append(contentParts, header, "", tableView)
	if notificationView != "" {
		contentParts = append(contentParts, "", notificationView)
	}
	contentParts = append(contentParts, footer)

	content := lipgloss.JoinVertical(
		lipgloss.Left,
		contentParts...,
	)

	// If showing create prompt, overlay a polished prompt dialog
	if m.showingCreate {
		// Define dialog styles
		var (
			dialogBorder = lipgloss.NewStyle().
					Border(lipgloss.RoundedBorder()).
					BorderForeground(primaryColor).
					Padding(1, 2).
					Width(70)

			dialogTitle = lipgloss.NewStyle().
					Foreground(primaryColor).
					Bold(true).
					Align(lipgloss.Center).
					MarginBottom(1)

			fieldLabel = lipgloss.NewStyle().
					Foreground(secondaryColor).
					Bold(true)

			fieldInput = lipgloss.NewStyle().
					Foreground(lipgloss.Color("#FFFFFF")).
					Background(lipgloss.Color("#3C3C3C")).
					Padding(0, 1).
					Width(50)

			fieldMuted = lipgloss.NewStyle().
					Foreground(mutedColor)

			stepIndicator = lipgloss.NewStyle().
					Foreground(primaryColor).
					Bold(true)

			helpText = lipgloss.NewStyle().
					Foreground(mutedColor).
					Italic(true).
					Align(lipgloss.Center).
					MarginTop(1)
		)

		// Build step indicator
		stepText := stepIndicator.Render(fmt.Sprintf("Step %d of 3", m.createStep+1))

		// Build the form based on current step
		var formContent string
		switch m.createStep {
		case 0:
			// Machine selection step
			machineList := ""
			for i, machine := range m.machines {
				prefix := "  "
				if i == m.selectedMachineIdx {
					prefix = "▶ "
				}
				machineList += fieldLabel.Render(prefix+machine.Name) + "\n"
			}

			formContent = lipgloss.JoinVertical(
				lipgloss.Left,
				stepText,
				"",
				fieldLabel.Render("Select Virtual Machine:"),
				"",
				machineList,
				helpText.Render("↑/↓: navigate • Enter: select • Esc: cancel"),
			)
		case 1:
			// Local port input step
			selectedMachine := m.machines[m.selectedMachineIdx]
			summary := fieldMuted.Render(fmt.Sprintf("Machine: %s", selectedMachine.Name))
			formContent = lipgloss.JoinVertical(
				lipgloss.Left,
				stepText,
				summary,
				"",
				fieldLabel.Render("Local Port:"),
				fieldInput.Render(m.createLocalPort+"█"),
				"",
				helpText.Render("The local port to bind (e.g., 2022, 8080)"),
			)
		case 2:
			// Remote port input step
			selectedMachine := m.machines[m.selectedMachineIdx]
			summary := fieldMuted.Render(fmt.Sprintf("Machine: %s • Local: %s", selectedMachine.Name, m.createLocalPort))
			formContent = lipgloss.JoinVertical(
				lipgloss.Left,
				stepText,
				summary,
				"",
				fieldLabel.Render("Remote Port:"),
				fieldInput.Render(m.createRemotePort+"█"),
				"",
				helpText.Render("The remote port on the VM (e.g., 22, 80, 443) • Enter: create tunnel"),
			)
		}

		// Create the dialog
		title := dialogTitle.Render("🚇 Create New SSH Tunnel")
		dialogContent := lipgloss.JoinVertical(lipgloss.Left, title, "", formContent)
		box := dialogBorder.Render(dialogContent)

		// Center the dialog
		content = lipgloss.Place(m.width, m.height, lipgloss.Center, lipgloss.Center, box)
		return content
	}

	// If showing confirm delete dialog, overlay a styled warning dialog
	if m.showingConfirmDelete {
		// Define warning dialog styles
		var (
			warningBorder = lipgloss.NewStyle().
					Border(lipgloss.RoundedBorder()).
					BorderForeground(lipgloss.Color("#FFA500")).
					Padding(2, 4).
					Width(60)

			warningTitle = lipgloss.NewStyle().
					Foreground(lipgloss.Color("#FFA500")).
					Bold(true).
					Align(lipgloss.Center).
					MarginBottom(1)

			warningText = lipgloss.NewStyle().
					Foreground(lipgloss.Color("#FFFFFF")).
					Align(lipgloss.Center).
					MarginBottom(1)

			warningHelp = lipgloss.NewStyle().
					Foreground(mutedColor).
					Italic(true).
					Align(lipgloss.Center).
					MarginTop(1)
		)

		// Get tunnel details
		tunnelToDelete := m.tunnels[m.deleteTargetIdx]
		tunnelInfo := fmt.Sprintf("%s (Local:%s → Remote:%s)",
			tunnelToDelete.Machine.Name,
			tunnelToDelete.LocalPort,
			tunnelToDelete.RemotePort)

		// Build centered content with proper width
		contentWidth := 52 // Width minus padding

		title := lipgloss.NewStyle().Width(contentWidth).Align(lipgloss.Center).Render(
			warningTitle.Render("🗑️  Confirm Delete"),
		)
		message := lipgloss.NewStyle().Width(contentWidth).Align(lipgloss.Center).Render(
			warningText.Render("Are you sure you want to delete this tunnel?"),
		)
		tunnelDetails := lipgloss.NewStyle().Width(contentWidth).Align(lipgloss.Center).Render(
			lipgloss.NewStyle().Foreground(primaryColor).Bold(true).Render(tunnelInfo),
		)
		help := lipgloss.NewStyle().Width(contentWidth).Align(lipgloss.Center).Render(
			warningHelp.Render("Press 'y' to delete • 'q' or Esc to cancel"),
		)

		dialogContent := lipgloss.JoinVertical(
			lipgloss.Left,
			title,
			"",
			message,
			"",
			tunnelDetails,
			"",
			help,
		)

		confirm := warningBorder.Render(dialogContent)
		content = lipgloss.Place(m.width, m.height, lipgloss.Center, lipgloss.Center, confirm)
		return content
	}

	// If showing tunnel logs, overlay a log viewer dialog
	if m.showingLogs {
		// Define log viewer styles
		var (
			logBorder = lipgloss.NewStyle().
					Border(lipgloss.RoundedBorder()).
					BorderForeground(primaryColor).
					Padding(1, 2).
					Width(80).
					Height(30)

			logTitle = lipgloss.NewStyle().
					Foreground(primaryColor).
					Bold(true).
					Align(lipgloss.Center).
					MarginBottom(1)

			logText = lipgloss.NewStyle().
				Foreground(lipgloss.Color("#FFFFFF")).
				MarginBottom(0)

			logHelp = lipgloss.NewStyle().
				Foreground(mutedColor).
				Italic(true).
				Align(lipgloss.Center).
				MarginTop(1)
		)

		// Build log content
		tunnelInfo := "Unknown Tunnel"
		if m.logsForTunnel >= 0 && m.logsForTunnel < len(m.tunnels) {
			tunnel := m.tunnels[m.logsForTunnel]
			tunnelInfo = fmt.Sprintf("%s:%s → %s (Port %s)",
				tunnel.Machine.Name,
				tunnel.RemotePort,
				tunnel.Machine.Name,
				tunnel.LocalPort,
			)
		}

		title := logTitle.Render(fmt.Sprintf("📋 Tunnel Logs: %s", tunnelInfo))

		// Format logs
		logsContent := ""
		if len(m.tunnelLogs) == 0 {
			logsContent = logText.Render("No logs available yet...")
		} else {
			// Show last 20 lines to fit in the dialog
			startIdx := 0
			if len(m.tunnelLogs) > 20 {
				startIdx = len(m.tunnelLogs) - 20
			}
			for _, log := range m.tunnelLogs[startIdx:] {
				logsContent += logText.Render(log) + "\n"
			}
		}

		help := logHelp.Render("Esc: close")

		dialogContent := lipgloss.JoinVertical(
			lipgloss.Left,
			title,
			"",
			logsContent,
			help,
		)

		logViewer := logBorder.Render(dialogContent)
		content = lipgloss.Place(m.width, m.height, lipgloss.Center, lipgloss.Center, logViewer)
		return content
	}

	// If showing confirm quit dialog, overlay a styled warning dialog
	if m.showingConfirmQuit {
		// Define warning dialog styles
		var (
			warningBorder = lipgloss.NewStyle().
					Border(lipgloss.RoundedBorder()).
					BorderForeground(lipgloss.Color("#FF6B6B")).
					Padding(2, 4).
					Width(60)

			warningTitle = lipgloss.NewStyle().
					Foreground(lipgloss.Color("#FF6B6B")).
					Bold(true).
					Align(lipgloss.Center).
					MarginBottom(1)

			warningText = lipgloss.NewStyle().
					Foreground(lipgloss.Color("#FFFFFF")).
					Align(lipgloss.Center).
					MarginBottom(1)

			warningHelp = lipgloss.NewStyle().
					Foreground(mutedColor).
					Italic(true).
					Align(lipgloss.Center).
					MarginTop(1)
		)

		// Build centered content with proper width
		contentWidth := 52 // Width minus padding

		title := lipgloss.NewStyle().Width(contentWidth).Align(lipgloss.Center).Render(
			warningTitle.Render("⚠️  Confirm Quit"),
		)
		message := lipgloss.NewStyle().Width(contentWidth).Align(lipgloss.Center).Render(
			warningText.Render("All active SSH tunnels will be terminated."),
		)
		message2 := lipgloss.NewStyle().Width(contentWidth).Align(lipgloss.Center).Render(
			warningText.Render("Are you sure you want to exit?"),
		)
		help := lipgloss.NewStyle().Width(contentWidth).Align(lipgloss.Center).Render(
			warningHelp.Render("Press 'y' to quit • 'q' or Esc to cancel"),
		)

		dialogContent := lipgloss.JoinVertical(
			lipgloss.Left,
			title,
			"",
			message,
			message2,
			"",
			help,
		)

		confirm := warningBorder.Render(dialogContent)
		content = lipgloss.Place(m.width, m.height, lipgloss.Center, lipgloss.Center, confirm)
		return content
	}

	// Center the entire layout if terminal is wider than content
	if m.width > 0 {
		content = lipgloss.Place(
			m.width,
			m.height,
			lipgloss.Left,
			lipgloss.Top,
			content,
		)
	}

	return content
}

// createTunnelTable initializes the table with columns and active tunnels
// Returns a configured table.Model ready to display tunnel information
func createTunnelTable(tunnels []types.Tunnel) table.Model {
	// Define table columns
	columns := []table.Column{
		{Title: "Name", Width: 25},
		{Title: "Local Port", Width: 12},
		{Title: "Remote Port", Width: 13},
		{Title: "Status", Width: 15},
		{Title: "Cert Status", Width: 15},
		{Title: "Cert Expires", Width: 13},
	}

	// Convert tunnels to table rows
	rows := make([]table.Row, len(tunnels))
	for i, t := range tunnels {
		certStatus := "N/A"
		certExpires := "-"

		// Show certificate info if SSH is configured
		if t.Machine.SSHConfigPath != "" {
			if t.CertStatus != "" {
				certStatus = formatCertStatus(t.CertStatus)
			}
			if t.CertExpiresIn != "" {
				certExpires = t.CertExpiresIn
			}
		}

		rows[i] = table.Row{
			t.Machine.Name,
			t.LocalPort,
			t.RemotePort,
			t.Status,
			certStatus,
			certExpires,
		}
	}

	// Create table with columns and rows
	t := table.New(
		table.WithColumns(columns),
		table.WithRows(rows),
		table.WithFocused(true),
		table.WithHeight(10),
	)

	// Customize table styles to match purple theme
	s := table.DefaultStyles()
	s.Header = s.Header.
		BorderStyle(lipgloss.NormalBorder()).
		BorderForeground(lipgloss.Color("#7D56F4")).
		BorderBottom(true).
		Bold(true).
		Foreground(lipgloss.Color("#7D56F4"))

	s.Selected = s.Selected.
		Foreground(lipgloss.Color("#FFFFFF")).
		Background(lipgloss.Color("#7D56F4")).
		Bold(true)

	t.SetStyles(s)

	return t
}

// listenForTunnelUpdates creates a command that continuously listens for tunnel updates
func listenForTunnelUpdates(tunnelID int, statusCh chan string, errorCh chan error) tea.Cmd {
	return func() tea.Msg {
		select {
		case status, ok := <-statusCh:
			if !ok {
				// Channel closed, tunnel stopped
				return nil
			}
			// Return status message and continue listening in the Update function
			return TunnelStatusMsg{TunnelID: tunnelID, Status: status}
		case err, ok := <-errorCh:
			if !ok {
				// Channel closed
				return nil
			}
			return TunnelErrorMsg{TunnelID: tunnelID, Error: err}
		}
	}
}

// listenForCertUpdates listens for certificate status updates
func listenForCertUpdates(certMgr *azure.CertificateManager) tea.Cmd {
	return func() tea.Msg {
		status := <-certMgr.StatusChannel()
		return CertStatusMsg{Status: status}
	}
}

// tickEveryMinute returns a command that sends a TickMsg every minute
func tickEveryMinute() tea.Cmd {
	return tea.Tick(time.Minute, func(t time.Time) tea.Msg {
		return TickMsg(t)
	})
}

// updateCertificateStatuses updates certificate statuses for all tunnels
func (m *model) updateCertificateStatuses() {
	for i := range m.tunnels {
		tunnel := &m.tunnels[i]
		if tunnel.Machine.SSHConfigPath != "" {
			status, expiresIn, err := m.certificateManager.GetStatus(tunnel.Machine.Name)
			if err == nil {
				tunnel.CertStatus = status
				if expiresIn < 0 {
					tunnel.CertExpiresIn = "expired"
				} else {
					tunnel.CertExpiresIn = formatDuration(expiresIn.Round(time.Second))
				}
			}
		}
	}
}

// formatDuration formats a duration in a human-readable way
func formatDuration(d time.Duration) string {
	if d < 0 {
		return "expired"
	}

	hours := int(d.Hours())
	minutes := int(d.Minutes()) % 60
	seconds := int(d.Seconds()) % 60

	if hours > 0 {
		return fmt.Sprintf("%dh%dm", hours, minutes)
	} else if minutes > 0 {
		return fmt.Sprintf("%dm%ds", minutes, seconds)
	} else {
		return fmt.Sprintf("%ds", seconds)
	}
}

// formatCertStatus formats certificate status with emoji
func formatCertStatus(status string) string {
	switch status {
	case "valid":
		return "🟢 valid"
	case "expiring_soon":
		return "🟡 expiring"
	case "renewing":
		return "🔄 renewing"
	case "renewed":
		return "✅ renewed"
	case "expired":
		return "❌ expired"
	case "renewal_failed":
		return "⚠️ failed"
	default:
		return status
	}
}

// clearNotificationAfter returns a command that clears the notification after a duration
func clearNotificationAfter(d time.Duration) tea.Cmd {
	return tea.Tick(d, func(t time.Time) tea.Msg {
		return ClearNotificationMsg{}
	})
}
