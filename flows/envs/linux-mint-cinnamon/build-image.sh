#!/usr/bin/env bash

set -euo pipefail

asset_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
workspace_root="$(cd "$asset_root/../../.." && pwd -P)"
expected_iso_sha256='a081ab202cfda17f6924128dbd2de8b63518ac0531bcfe3f1a1b88097c459bd4'
iso="${1:-}"
output="${2:-}"

if [[ "$#" -ne 2 || ! -f "$iso" || "$output" != /* || -e "$output" ]]; then
  echo "usage: ./build-image.sh /path/to/linuxmint-22.3-cinnamon-64bit.iso /absolute/output.qcow2" >&2
  exit 1
fi

for command in 7z cargo cpio genisoimage qemu-img qemu-system-x86_64 sha256sum timeout unmkinitramfs zstd; do
  if ! command -v "$command" >/dev/null; then
    echo "required command is unavailable: $command" >&2
    exit 1
  fi
done

actual_iso_sha256="$(sha256sum "$iso" | cut -d' ' -f1)"
if [[ "$actual_iso_sha256" != "$expected_iso_sha256" ]]; then
  echo "Linux Mint ISO checksum mismatch" >&2
  exit 1
fi

ovmf_code='/usr/share/OVMF/OVMF_CODE_4M.fd'
ovmf_vars='/usr/share/OVMF/OVMF_VARS_4M.fd'
if [[ ! -r "$ovmf_code" || ! -r "$ovmf_vars" || ! -r /dev/kvm || ! -w /dev/kvm ]]; then
  echo "usable KVM and OVMF firmware are required" >&2
  exit 1
fi

output_parent="$(dirname "$output")"
mkdir -p "$output_parent"
output_parent="$(cd "$output_parent" && pwd -P)"
output="$output_parent/$(basename "$output")"
partial="$output.partial-$$"
work="$(mktemp -d "$output_parent/.qol-mint-image-build-XXXXXX")"

cleanup() {
  rm -rf "$work"
  rm -f "$partial"
}
trap cleanup EXIT

cargo build --locked --release --manifest-path "$workspace_root/Cargo.toml" -p qol-guest-runner
mkdir -p "$work/payload/linux-mint-cinnamon" "$work/initrd-unpacked"
cp -a "$asset_root/image-identity.json" "$asset_root/packages.txt" "$asset_root/provision.sh" "$asset_root/qol-sandbox-payload" "$asset_root/rootfs" "$work/payload/linux-mint-cinnamon/"
cp "$workspace_root/target/release/qol-guest-runner" "$work/payload/qol-guest-runner"
genisoimage -quiet -R -J -V QOL_PROVISION -o "$work/qol-provision.iso" "$work/payload"
7z e -y -o"$work" "$iso" casper/vmlinuz casper/initrd.lz >/dev/null
unmkinitramfs "$work/initrd.lz" "$work/initrd-unpacked"
for early in "$work/initrd-unpacked"/early*; do
  if [[ -d "$early" ]]; then
    cp -a "$early/." "$work/initrd-unpacked/main/"
  fi
done
cp "$asset_root/preseed.cfg" "$work/initrd-unpacked/main/preseed.cfg"
(cd "$work/initrd-unpacked/main" && find . -print0 | cpio --null --create --format=newc --owner=0:0 2>/dev/null | zstd -q -T0 -19 -o "$work/initrd-qol")
cp "$ovmf_vars" "$work/OVMF_VARS.fd"
qemu-img create -q -f qcow2 "$partial" 40G

if ! timeout --signal=TERM 90m qemu-system-x86_64 \
  -name qol-mint-image-build \
  -machine q35,accel=kvm \
  -cpu host \
  -smp 4 \
  -m 6144 \
  -drive "if=pflash,format=raw,readonly=on,file=$ovmf_code" \
  -drive "if=pflash,format=raw,file=$work/OVMF_VARS.fd" \
  -drive "file=$partial,if=virtio,format=qcow2" \
  -drive "file=$iso,media=cdrom,readonly=on" \
  -drive "file=$work/qol-provision.iso,media=cdrom,readonly=on" \
  -kernel "$work/vmlinuz" \
  -initrd "$work/initrd-qol" \
  -append 'boot=casper uuid=6e72f523-dc09-4880-8910-93ffa64401c5 automatic-ubiquity noninteractive noprompt quiet splash console=tty0 console=ttyS0,115200n8 ---' \
  -netdev user,id=qolnet \
  -device virtio-net-pci,netdev=qolnet \
  -device virtio-vga \
  -display none \
  -serial "file:$work/serial.log" \
  -no-reboot; then
  cp "$work/serial.log" "$output.build.log"
  echo "Mint image build process failed: $output.build.log" >&2
  exit 1
fi

if ! grep -Fq 'QOL_IMAGE_BUILD_COMPLETE' "$work/serial.log"; then
  cp "$work/serial.log" "$output.build.log"
  echo "Mint image build did not report successful provisioning: $output.build.log" >&2
  exit 1
fi

qemu-img check -q "$partial"
mv "$partial" "$output"
trap - EXIT
rm -rf "$work"
echo "$output"
