#!/bin/bash -e
PROJ_ROOT="$(dirname "$(dirname "${BASH_SOURCE:-$0}")")"
cd "${PROJ_ROOT}"

PATH_TO_EFI="$1"
shift

rm -rf mnt
mkdir -p mnt/EFI/BOOT/
cp "${PATH_TO_EFI}" mnt/EFI/BOOT/BOOTX64.EFI
set +e
qemu-system-x86_64 \
  -m 4G \
  -bios third_party/ovmf/RELEASEX64_OVMF.fd \
  -drive format=raw,file=fat:rw:mnt \
  -device isa-debug-exit,iobase=0xf4,iosize=0x01 \
  -serial stdio \
  "$@"
RETCODE=$?
set -e
# QEMU exits with (isa-debug-exit value << 1) | 1, so our QemuExitCode::Success
# (0x1) shows up as 3. The boot may also end with the firmware's own status.
if [ $RETCODE -eq 0 ]; then
  exit 0
elif [ $RETCODE -eq 3 ]; then
  printf "\nPASS\n"
  exit 0
else
  printf "\nFAIL: QEMU returned %s\n" "$RETCODE"
  exit 1
fi
