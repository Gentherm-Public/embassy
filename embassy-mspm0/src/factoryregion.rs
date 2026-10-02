//! Factory region
//!
//! This part of flash contains read only data describing the device and any trim values.

// TODO: Complete this, using a proper pac definitions rather than hardcoded addresses (which are constant
// across families).

/// We want the SRAMFLASH with exposed provenance since this is a read only part of flash.
const SRAMFLASH_ADDRESS: *const u32 = core::ptr::with_exposed_provenance(0x41C4_0018);

const MAINFLASH_MASK: u32 = 0x0000_0FFF;
const MAINNUMBANKS_MASK: u32 = 0x0000_3000;
const MAINNUMBANKS_SHIFT: u32 = 12;

/// Get the size of main flash in kB.
pub fn main_flash_size() -> u32 {
    // SAFETY:
    // - src is always valid, since this is a read only part of flash
    // - src is properly aligned
    // - u32 is Copy
    // - volatile read since we cannot determine this at compile time.
    let sramflash = unsafe { SRAMFLASH_ADDRESS.read_volatile() };
    let main_flash_kb = sramflash & MAINFLASH_MASK;
    main_flash_kb
}

/// Get the number of main flash banks.
pub fn main_num_banks() -> u8 {
    // SAFETY:
    // - src is always valid, since this is a read only part of flash
    // - src is properly aligned
    // - u32 is Copy
    // - volatile read since we cannot determine this at compile time.
    let sramflash = unsafe { SRAMFLASH_ADDRESS.read_volatile() };
    let main_num_banks = (sramflash & MAINNUMBANKS_MASK) >> MAINNUMBANKS_SHIFT;

    // MAINNUMBANKS is always + 1
    // 0: 1 bank
    // 1: 2 banks
    // 2: 3 banks
    // and etc.
    (main_num_banks as u8) + 1
}
