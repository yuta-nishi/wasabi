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
# (0x1) shows up as 3.
if [ $RETCODE -eq 3 ]; then
  printf "\nPASS\n"
  exit 0
fi

# Test binaries always end through isa-debug-exit, so any other status means the
# verdict never arrived. The OS binary itself has no verdict to report: a clean
# QEMU exit (0) is fine there.
if [ $RETCODE -eq 0 ] && [[ "$PATH_TO_EFI" != */deps/* ]]; then
  exit 0
fi

printf "\nFAIL: QEMU returned %s (expected 3)\n" "$RETCODE"
exit 1
