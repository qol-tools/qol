#!/usr/bin/env bash

set -euo pipefail

export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

asset_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
runner_source="${1:-}"
mount_unit='run-qol\x2dpayload.mount'
mount_unit_source='run-qol-payload.mount'
automount_unit='run-qol\x2dpayload.automount'
automount_unit_source='run-qol-payload.automount'
run_id_unit='qol-guest-run-id.service'

if [[ "${EUID}" -ne 0 ]]; then
  echo "provision.sh must run as root" >&2
  exit 1
fi

if [[ "$#" -ne 1 || ! -f "$runner_source" || ! -x "$runner_source" ]]; then
  echo "usage: sudo ./provision.sh /path/to/qol-guest-runner" >&2
  exit 1
fi

mapfile -t packages < "$asset_root/packages.txt"
if [[ "${#packages[@]}" -eq 0 ]]; then
  echo "packages.txt must contain at least one package" >&2
  exit 1
fi
for package in "${packages[@]}"; do
  if [[ ! "$package" =~ ^[a-z0-9][a-z0-9+.-]*$ ]]; then
    echo "invalid package name: $package" >&2
    exit 1
  fi
done

if [[ "$(dpkg --print-architecture)" != "amd64" ]] \
  || ! grep -Fxq 'RELEASE=22.3' /etc/linuxmint/info \
  || ! grep -Fxq 'EDITION="Cinnamon"' /etc/linuxmint/info; then
  echo "base OS is not Linux Mint 22.3 Cinnamon amd64" >&2
  exit 1
fi

if ! virtualization="$(systemd-detect-virt --vm)" \
  || [[ "$virtualization" != "qemu" && "$virtualization" != "kvm" ]]; then
  echo "provisioning is restricted to a disposable QEMU/KVM guest" >&2
  exit 1
fi

if getent passwd qol >/dev/null; then
  existing_home="$(getent passwd qol | cut -d: -f6)"
  if [[ "$existing_home" != "/home/qol" || ! -d "$existing_home" || -L "$existing_home" ]]; then
    echo "existing user qol must have a real /home/qol directory" >&2
    exit 1
  fi
  if [[ -n "$(find "$existing_home" -mindepth 1 -print -quit)" ]]; then
    echo "refusing to provision over nonempty existing user home /home/qol" >&2
    exit 1
  fi
fi

export DEBIAN_FRONTEND=noninteractive
apt-get update
echo 'lightdm shared/default-x-display-manager select lightdm' | debconf-set-selections
apt-get install --yes --no-install-recommends "${packages[@]}"

if [[ "$(cinnamon --version)" != "Cinnamon 6.6.7" ]]; then
  echo "base OS does not match mint-22.3-cinnamon-6.6.7-qol-2" >&2
  exit 1
fi

if ! "$runner_source" --help | grep -Fq -- '--run-id-path PATH'; then
  echo "qol-guest-runner does not support the fw_cfg run identity contract" >&2
  exit 1
fi

getent group qol-guest >/dev/null || groupadd --system qol-guest
if ! id qol >/dev/null 2>&1; then
  useradd --home-dir /home/qol --no-create-home --shell /bin/bash qol
fi
install -d -m 0700 -o qol -g qol /home/qol
usermod --append --groups qol-guest qol
if getent group nopasswdlogin >/dev/null; then
  usermod --append --groups nopasswdlogin qol
fi
passwd --lock qol >/dev/null

install -d -m 0755 /usr/local/libexec
install -m 0755 "$runner_source" /usr/local/libexec/qol-guest-runner
install -m 0755 "$asset_root/qol-sandbox-payload" /usr/local/libexec/qol-sandbox-payload
install -m 0444 "$asset_root/image-identity.json" /etc/qol-dev-image.json
install -d -m 0755 /etc/lightdm/lightdm.conf.d
install -m 0644 "$asset_root/rootfs/etc/lightdm/lightdm.conf.d/90-qol-dev.conf" /etc/lightdm/lightdm.conf.d/90-qol-dev.conf
install -d -m 0755 /etc/modules-load.d
install -m 0644 "$asset_root/rootfs/etc/modules-load.d/qol-qemu-fw-cfg.conf" /etc/modules-load.d/qol-qemu-fw-cfg.conf
install -d -m 0755 /etc/xdg/autostart
for desktop_entry in "$asset_root/rootfs/etc/xdg/autostart/"*.desktop; do
  install -m 0644 "$desktop_entry" "/etc/xdg/autostart/$(basename "$desktop_entry")"
done
install -d -m 0755 /etc/udev/rules.d
install -m 0644 "$asset_root/rootfs/etc/udev/rules.d/70-qol-guest-control.rules" /etc/udev/rules.d/70-qol-guest-control.rules
install -d -m 0755 /etc/modules-load.d
install -m 0644 "$asset_root/rootfs/etc/modules-load.d/qol-bluetooth-vhci.conf" /etc/modules-load.d/qol-bluetooth-vhci.conf
install -d -m 0755 /etc/tmpfiles.d
install -m 0644 "$asset_root/rootfs/etc/tmpfiles.d/qol-bluetooth-vhci.conf" /etc/tmpfiles.d/qol-bluetooth-vhci.conf
install -d -m 0755 /etc/systemd/system
install -m 0644 "$asset_root/rootfs/etc/systemd/system/$mount_unit_source" "/etc/systemd/system/$mount_unit"
install -m 0644 "$asset_root/rootfs/etc/systemd/system/$automount_unit_source" "/etc/systemd/system/$automount_unit"
install -m 0644 "$asset_root/rootfs/etc/systemd/system/$run_id_unit" "/etc/systemd/system/$run_id_unit"

systemctl daemon-reload
systemctl enable lightdm.service "$automount_unit" "$run_id_unit"
systemctl set-default graphical.target
udevadm control --reload-rules
modprobe qemu_fw_cfg

home="$(getent passwd qol | cut -d: -f6)"
if [[ "$home" != "/home/qol" || ! -d "$home" || -L "$home" ]]; then
  echo "user qol must have a real /home/qol directory" >&2
  exit 1
fi
chown qol:qol "$home"
chmod 0700 "$home"
rm -rf /var/lib/qol-tray /var/cache/qol-tray /var/log/qol-tray /tmp/qol-*
apt-get clean
rm -rf /var/lib/apt/lists/*
rm -f /var/lib/systemd/random-seed
truncate --size 0 /etc/machine-id
rm -f /var/lib/dbus/machine-id

test -x /usr/local/libexec/qol-guest-runner
test -x /usr/local/libexec/qol-sandbox-payload
test -x /usr/bin/btvirt
test -r /etc/qol-dev-image.json
test -d /sys/firmware/qemu_fw_cfg/by_name
grep -Fq -- '--run-id-path /run/qol-guest-run-id' /etc/xdg/autostart/qol-guest-runner.desktop
grep -Fxq 'vhci_hcd' /etc/modules-load.d/qol-bluetooth-vhci.conf
grep -Fxq 'z /dev/vhci 0660 root qol-guest - -' /etc/tmpfiles.d/qol-bluetooth-vhci.conf
grep -Fxq 'Hidden=true' /etc/xdg/autostart/mintwelcome.desktop
test -z "$(find "$home" -mindepth 1 -print -quit)"
systemctl is-enabled --quiet lightdm.service
systemctl is-enabled --quiet "$automount_unit"
systemctl is-enabled --quiet "$run_id_unit"
sync
