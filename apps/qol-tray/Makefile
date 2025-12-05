.PHONY: build release install uninstall run clean test deb

build:
	cargo build

release:
	cargo build --release

install: release
	@echo "Installing QoL Tray..."
	sudo cp target/release/qol-tray /usr/bin/qol-tray
	sudo chmod +x /usr/bin/qol-tray
	@mkdir -p ~/.config/qol-tray/plugins
	@cp -r examples/plugins/screen-recorder ~/.config/qol-tray/plugins/ 2>/dev/null || true
	@chmod +x ~/.config/qol-tray/plugins/screen-recorder/run.sh 2>/dev/null || true
	@mkdir -p ~/.config/autostart
	@echo "[Desktop Entry]" > ~/.config/autostart/qol-tray.desktop
	@echo "Type=Application" >> ~/.config/autostart/qol-tray.desktop
	@echo "Name=QoL Tray" >> ~/.config/autostart/qol-tray.desktop
	@echo "Comment=Quality of Life Tray daemon" >> ~/.config/autostart/qol-tray.desktop
	@echo "Exec=/usr/bin/qol-tray" >> ~/.config/autostart/qol-tray.desktop
	@echo "Icon=applications-utilities" >> ~/.config/autostart/qol-tray.desktop
	@echo "Terminal=false" >> ~/.config/autostart/qol-tray.desktop
	@echo "Categories=Utility;" >> ~/.config/autostart/qol-tray.desktop
	@echo "X-GNOME-Autostart-enabled=true" >> ~/.config/autostart/qol-tray.desktop
	@echo "✅ Installation complete!"
	@echo "Run 'qol-tray' to start the daemon"

uninstall:
	@echo "Uninstalling QoL Tray..."
	sudo rm -f /usr/bin/qol-tray
	rm -f ~/.config/autostart/qol-tray.desktop
	@echo "Config and plugins remain at ~/.config/qol-tray/"
	@echo "Remove manually if desired: rm -rf ~/.config/qol-tray/"

run: build
	RUST_LOG=info cargo run

run-debug: build
	RUST_LOG=debug cargo run

clean:
	cargo clean

test:
	cargo test

deb: release
	cargo deb --no-build
