/* Flash layout for embassy-boot; bootloader/memory.x has the same partitions.
   0x10000000  boot2 and the bootloader (24K)
   0x10006000  bootloader state (4K)
   0x10007000  this firmware, the active partition (960K)
   0x100F7000  download area for an update (964K: active + one sector) */
MEMORY
{
  BOOT2            : ORIGIN = 0x10000000, LENGTH = 0x100
  BOOTLOADER_STATE : ORIGIN = 0x10006000, LENGTH = 4K
  FLASH            : ORIGIN = 0x10007000, LENGTH = 960K
  DFU              : ORIGIN = 0x100F7000, LENGTH = 964K
  RAM              : ORIGIN = 0x20000000, LENGTH = 264K
}

__bootloader_state_start = ORIGIN(BOOTLOADER_STATE) - ORIGIN(BOOT2);
__bootloader_state_end = ORIGIN(BOOTLOADER_STATE) + LENGTH(BOOTLOADER_STATE) - ORIGIN(BOOT2);
__bootloader_dfu_start = ORIGIN(DFU) - ORIGIN(BOOT2);
__bootloader_dfu_end = ORIGIN(DFU) + LENGTH(DFU) - ORIGIN(BOOT2);
