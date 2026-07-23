#!/bin/bash

set -e

INSTALL_DIR="/usr/local/bin"
SERVICE_DIR="/etc/systemd/system"
CONFIG_DIR="/etc/spinctrl"
IPC_DIR="/var/lib/spinctrl"

echo "SpinCtrl Installation Script"
echo "=========================="

# Check if running as root
if [[ $EUID -ne 0 ]]; then
    echo "This script must be run as root (use sudo)"
    exit 1
fi

# Check dependencies
echo "Checking dependencies..."
missing_deps=()

for cmd in ectool cpupower jq inotifywait udevadm; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
        missing_deps+=("$cmd")
    fi
done

if [[ ${#missing_deps[@]} -gt 0 ]]; then
    echo "Missing required dependencies: ${missing_deps[*]}"
    echo "Please install them first:"
    echo "  Ubuntu/Debian: sudo apt install coreutils cpupower jq inotify-tools udev"
    echo "  For ectool: Install chromium-ec package or build from source"
    exit 1
fi

# Build Rust components
echo "Building Rust components..."
if ! command -v cargo >/dev/null 2>&1; then
    echo "Cargo (Rust) is required to build the TUI component"
    echo "Install Rust from: https://rustup.rs/"
    exit 1
fi

cargo build --release --bin spinctrl
cargo build --release --bin spinctrl-service

# Create system user and group
echo "Creating system group..."
if ! getent group spinctrl >/dev/null; then
    groupadd -r spinctrl
    echo "Created group 'spinctrl'"
fi

# Install binaries
echo "Installing binaries..."
install -m 755 target/release/spinctrl "$INSTALL_DIR/"
install -m 755 target/release/spinctrl-service "$INSTALL_DIR/"

# Install systemd service
echo "Installing systemd service..."
install -m 644 spinctrl-service/spinctrl.service "$SERVICE_DIR/"

# Create system configuration directory and default config
echo "Setting up configuration directory ($CONFIG_DIR)..."
mkdir -p "$CONFIG_DIR"
chown root:spinctrl "$CONFIG_DIR"
chmod 750 "$CONFIG_DIR"

if [[ ! -f "$CONFIG_DIR/config.json" ]]; then
    echo "Creating default configuration..."
    cat > "$CONFIG_DIR/config.json" << 'EOF'
{
  "battery": {
    "threshold": 80,
    "force_charge": false
  },
  "cpu": {
    "governor_ac": "performance",
    "governor_battery": "powersave"
  },
  "thermal": {
    "warn_temp": 70,
    "high_temp": 55,
    "shutdown_temp": 80,
    "fan_off_temp": 50,
    "fan_max_temp": 75,
    "profile": "balanced"
  },
  "version": 1
}
EOF
    chown root:spinctrl "$CONFIG_DIR/config.json"
    chmod 640 "$CONFIG_DIR/config.json"
fi

# Create runtime IPC directory (status, FIFO, events)
echo "Setting up runtime IPC directory ($IPC_DIR)..."
mkdir -p "$IPC_DIR"
chown root:spinctrl "$IPC_DIR"
chmod 750 "$IPC_DIR"

# Reload systemd and enable service
echo "Enabling systemd service..."
systemctl daemon-reload
systemctl enable spinctrl

echo ""
echo "Installation completed successfully!"
echo ""
echo "Next steps:"
echo "1. Add users to the 'spinctrl' group to use the TUI:"
echo "   sudo usermod -a -G spinctrl \$USER"
echo "   (then log out and back in)"
echo ""
echo "2. Start the service:"
echo "   sudo systemctl start spinctrl"
echo ""
echo "3. Check service status:"
echo "   sudo systemctl status spinctrl"
echo ""
echo "4. Run the TUI as a regular user:"
echo "   spinctrl"
echo ""
echo "5. Check service status from TUI:"
echo "   spinctrl --check"
echo ""
echo "Configuration:"
echo "  - Boot-time defaults: $CONFIG_DIR/config.json (root:spinctrl, 0640)"
echo "  - Runtime state:      $IPC_DIR/ (status.json, commands.fifo, events.log)"
echo "  - The TUI reads /etc config for display and pushes edits at runtime"
echo "    via the commands FIFO; /etc is NOT written by the TUI or service."
echo "    Edit /etc directly only to change boot-time defaults."