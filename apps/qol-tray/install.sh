#!/usr/bin/env bash

set -euo pipefail

echo "Installing QoL Tray..."

# Build release binary
echo "Building release binary..."
cargo build --release

# Install binary
echo "Installing to /usr/bin/qol-tray..."
sudo cp target/release/qol-tray /usr/bin/qol-tray
sudo chmod +x /usr/bin/qol-tray

# Create config directory
CONFIG_DIR="$HOME/.config/qol-tray"
PLUGINS_DIR="$CONFIG_DIR/plugins"

echo "Creating config directory at $CONFIG_DIR..."
mkdir -p "$PLUGINS_DIR"

# Install example screen-recorder plugin
echo "Installing screen-recorder example plugin..."
cp -r examples/plugins/screen-recorder "$PLUGINS_DIR/"
chmod +x "$PLUGINS_DIR/screen-recorder/run.sh"

# Create desktop entry for autostart
AUTOSTART_DIR="$HOME/.config/autostart"
mkdir -p "$AUTOSTART_DIR"

cat > "$AUTOSTART_DIR/qol-tray.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=QoL Tray
Comment=Quality of Life Tray daemon for utility scripts
Exec=/usr/bin/qol-tray
Icon=applications-utilities
Terminal=false
Categories=Utility;
StartupNotify=false
X-GNOME-Autostart-enabled=true
EOF

echo "Created autostart entry at $AUTOSTART_DIR/qol-tray.desktop"

echo ""
echo "✅ Installation complete!"
echo ""
echo "To start QoL Tray now, run:"
echo "  qol-tray"
echo ""
echo "Or log out and log back in for autostart."
echo ""
echo "Plugins are located at: $PLUGINS_DIR"
echo "Add more plugins there and reload from the tray menu."
