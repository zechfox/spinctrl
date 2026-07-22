#!/bin/bash

set -e

INSTALL_DIR="/usr/local/bin"
SERVICE_DIR="/etc/systemd/system"
CONFIG_DIR="/etc/spinctrl"
IPC_DIR="/var/lib/spinctrl"
SERVICE_NAME="spinctrl"

echo "SpinCtrl Uninstallation Script"
echo "=============================="

# Check if running as root
if [[ $EUID -ne 0 ]]; then
    echo "This script must be run as root (use sudo)"
    exit 1
fi

# Stop and disable the service
echo "Stopping and disabling service..."
systemctl stop "$SERVICE_NAME" 2>/dev/null || true
systemctl disable "$SERVICE_NAME" 2>/dev/null || true

# Remove systemd unit
echo "Removing systemd unit..."
rm -f "$SERVICE_DIR/$SERVICE_NAME.service"
systemctl daemon-reload

# Remove binaries
echo "Removing binaries..."
rm -f "$INSTALL_DIR/spinctrl"
rm -f "$INSTALL_DIR/spinctrl-service"

# Remove system configuration
echo "Removing configuration ($CONFIG_DIR)..."
rm -rf "$CONFIG_DIR"

# Remove runtime state
echo "Removing runtime state ($IPC_DIR)..."
rm -rf "$IPC_DIR"

# Remove the spinctrl group (only if no users remain members)
echo "Removing spinctrl group (if no users remain members)..."
if getent group spinctrl >/dev/null 2>&1; then
    if groupdel spinctrl 2>/dev/null; then
        echo "  Removed group 'spinctrl'."
    else
        echo "  Note: group 'spinctrl' was not removed (users are still members)."
        echo "  Remove membership with: sudo gpasswd -d <user> spinctrl"
        echo "  Then re-run: sudo groupdel spinctrl"
    fi
fi

echo ""
echo "Uninstallation complete."
echo "Note: users previously in the 'spinctrl' group retain the group"
echo "membership in their active session until they log out and back in."
