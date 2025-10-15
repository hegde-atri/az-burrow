package tui

import (
	"fmt"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/bubbles/table"
	"github.com/charmbracelet/lipgloss"
)

// App represents the main TUI application for Az-Burrow
type App struct {
	version string
	program *tea.Program
}

// New creates and initializes a new Az-Burrow TUI application
// It sets up the bubbletea program with the initial model
func New(version string) *App {
	m := model{
		version: version,
		table:   createTunnelTable(),
	}

	p := tea.NewProgram(m, tea.WithAltScreen())

	return &App{
		version: version,
		program: p,
	}
}

// Run starts the TUI application and blocks until it exits
// Returns an error if the program fails to run
func (a *App) Run() error {
	_, err := a.program.Run()
	return err
}

// model represents the state of the bubbletea application
// It holds the terminal dimensions to enable responsive fullscreen layout
type model struct {
	version string
	table   table.Model
	width   int
	height  int
}

// Init is called when the program starts
// Returns an initial command to run (nil means no command)
func (m model) Init() tea.Cmd {
	return nil
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

	case tea.KeyMsg:
		// Handle keyboard input
		switch msg.String() {
		case "q", "ctrl+c":
			// Quit the application
			return m, tea.Quit
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

	// Footer with navigation hints
	footer := footerStyle.Render("↑/↓: navigate • q: quit")

	// Combine all sections vertically to create fullscreen layout
	// This approach ensures the content adapts to terminal size:
	// 1. Header stays at top with branding
	// 2. Table expands to fill available space
	// 3. Footer stays at bottom with controls
	content := lipgloss.JoinVertical(
		lipgloss.Left,
		header,
		"",
		tableView,
		footer,
	)

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

// createTunnelTable initializes the table with columns and mock data
// Returns a configured table.Model ready to display tunnel information
func createTunnelTable() table.Model {
	// Define table columns
	columns := []table.Column{
		{Title: "Name", Width: 30},
		{Title: "Local Port", Width: 12},
		{Title: "Remote Port", Width: 13},
		{Title: "Status", Width: 15},
	}

	// Mock data for testing - replace with real tunnel data later
	rows := []table.Row{
		{"vm-uk-experiment-01", "2022", "22", "Active"},
		{"vm-api-dev", "8080", "80", "Inactive"},
		{"vm-db-prod", "5432", "5432", "Connecting..."},
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