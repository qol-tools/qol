#!/usr/bin/env bash

set -euo pipefail

asset_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
workspace_root="$(cd "$asset_root/../../.." && pwd -P)"
expected_windows_iso_sha256='66b7b4b71763ed6f9b2ce29326ed9284544da6f5283d00329921540c01aaaeea'
expected_virtio_iso_sha256='e14cf2b94492c3e925f0070ba7fdfedeb2048c91eea9c5a5afb30232a3976331'
runner_target='x86_64-pc-windows-gnu'
boot_prompt_seconds=12
windows_iso="${1:-}"
virtio_iso="${2:-}"
output="${3:-}"

if [[ "$#" -ne 3 || ! -f "$windows_iso" || ! -f "$virtio_iso" || "$output" != /* || -e "$output" ]]; then
  echo "usage: ./build-image.sh /path/to/Win11_25H2_EnglishInternational_x64_v2.iso /path/to/virtio-win.iso /absolute/output.qcow2" >&2
  exit 1
fi

for command in cargo cargo-zigbuild zig genisoimage python3 qemu-img qemu-system-x86_64 sha256sum timeout; do
  if ! command -v "$command" >/dev/null; then
    echo "required command is unavailable: $command" >&2
    exit 1
  fi
done

if [[ "$(sha256sum "$windows_iso" | cut -d' ' -f1)" != "$expected_windows_iso_sha256" ]]; then
  echo "Windows 11 ISO checksum mismatch" >&2
  exit 1
fi
if [[ "$(sha256sum "$virtio_iso" | cut -d' ' -f1)" != "$expected_virtio_iso_sha256" ]]; then
  echo "virtio-win ISO checksum mismatch" >&2
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
work="$(mktemp -d "$output_parent/.qol-windows-image-build-XXXXXX")"
qemu_pid=''

cleanup() {
  if [[ -n "$qemu_pid" ]] && kill -0 "$qemu_pid" 2>/dev/null; then
    kill "$qemu_pid" 2>/dev/null || true
    wait "$qemu_pid" 2>/dev/null || true
  fi
  rm -rf "$work"
  rm -f "$partial"
}
trap cleanup EXIT

cargo zigbuild --locked --release --manifest-path "$workspace_root/Cargo.toml" -p qol-guest-runner --target "$runner_target"
mkdir -p "$work/payload"
cp "$asset_root/autounattend.xml" "$asset_root/qol-provision.ps1" "$asset_root/image-identity.json" "$work/payload/"
cp "$workspace_root/target/$runner_target/release/qol-guest-runner.exe" "$work/payload/qol-guest-runner.exe"
genisoimage -quiet -J -R -V QOL_PROVISION -o "$work/qol-provision.iso" "$work/payload"
cp "$ovmf_vars" "$work/OVMF_VARS.fd"
qemu-img create -q -f qcow2 "$partial" 40G

timeout --signal=TERM 150m qemu-system-x86_64 \
  -name qol-windows-image-build \
  -machine q35,accel=kvm \
  -cpu host \
  -smp 4 \
  -m 6144 \
  -drive "if=pflash,format=raw,readonly=on,file=$ovmf_code" \
  -drive "if=pflash,format=raw,file=$work/OVMF_VARS.fd" \
  -drive "file=$partial,if=none,id=qoldisk,format=qcow2" \
  -device virtio-blk-pci,drive=qoldisk,bootindex=1 \
  -drive "file=$windows_iso,if=none,id=wincd,media=cdrom,readonly=on" \
  -device ide-cd,drive=wincd,bus=ide.0,bootindex=2 \
  -drive "file=$virtio_iso,if=none,id=virtiocd,media=cdrom,readonly=on" \
  -device ide-cd,drive=virtiocd,bus=ide.1 \
  -drive "file=$work/qol-provision.iso,if=none,id=provisioncd,media=cdrom,readonly=on" \
  -device ide-cd,drive=provisioncd,bus=ide.2 \
  -nic none \
  -display none \
  -monitor "unix:$work/monitor.sock,server=on,wait=off" \
  -serial "file:$work/serial.log" &
qemu_pid=$!

python3 -I - "$work/monitor.sock" "$boot_prompt_seconds" <<'PY' || true
import socket, sys, time
deadline = time.monotonic() + float(sys.argv[2])
monitor = None
while monitor is None and time.monotonic() < deadline:
    try:
        monitor = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        monitor.connect(sys.argv[1])
    except OSError:
        monitor = None
        time.sleep(0.05)
while monitor is not None and time.monotonic() < deadline:
    monitor.sendall(b"sendkey ret\n")
    time.sleep(0.2)
PY

if ! wait "$qemu_pid"; then
  qemu_pid=''
  cp "$work/serial.log" "$output.build.log" 2>/dev/null || true
  echo "Windows image build process failed: $output.build.log" >&2
  exit 1
fi
qemu_pid=''

if ! grep -Fq 'QOL_IMAGE_BUILD_COMPLETE' "$work/serial.log"; then
  cp "$work/serial.log" "$output.build.log"
  echo "Windows image build did not report successful provisioning: $output.build.log" >&2
  exit 1
fi

qemu-img check -q "$partial"
mv "$partial" "$output"
trap - EXIT
rm -rf "$work"
echo "$output"
