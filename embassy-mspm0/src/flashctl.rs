use core::fmt;
use core::marker::PhantomData;

use embassy_hal_internal::Peri;
use embedded_storage::nor_flash::{self, NorFlashError, NorFlashErrorKind, ReadNorFlash};

use crate::factoryregion::{main_flash_size, main_num_banks};
use crate::pac;
use crate::pac::common::{RW, Reg};
use crate::pac::flashctl::regs::Cmdweprot;
use crate::pac::flashctl::{regs, vals};
use crate::peripherals::FLASHCTL;

/// Blocking flash mode typestate.
pub enum Blocking {}

/// Async flash mode typestate.
pub enum Async {}

/// Errors related to accessing flash memory.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// The size of a write or erase was not correct.
    ///
    /// ## Write
    ///
    /// This will occur if the write is not a multiple of [`WRITE_SIZE`](nor_flash::NorFlash::WRITE_SIZE).
    ///
    /// ## Erase
    ///
    /// This will occur if the size of an erase is not a multiple of [`ERASE_SIZE`](nor_flash::NorFlash::ERASE_SIZE).
    Size,

    /// A flash address which is out of bounds was used.
    OutOfBounds,

    /// The address being written to/erased is protected.
    ///
    /// This will only occur if dynamic write protection has protected an address. If static write protection is enabled
    /// you will get the [`Error::IllegalAddress`] error instead.
    Protected,

    /// The write/erase is not properly aligned. Reads may use any alignment and will never cause this error.
    ///
    /// ## Write
    ///
    /// This will occur if the alignment of a write is not [`WRITE_SIZE`](nor_flash::NorFlash::WRITE_SIZE).
    ///
    /// ## Erase
    ///
    /// This will occur if the alignment of an erase is not [`ERASE_SIZE`](nor_flash::NorFlash::ERASE_SIZE).
    Unaligned,

    /// Verification error.
    ///
    /// This will occur if the hardware could not verify a write or erase was successful.
    Verify,

    /// A write or erase was tried on a statically write protected flash address.
    ///
    /// This will occur if the NONMAIN region of flash has set static write protection on a part of flash.
    IllegalAddress,

    /// An attempt to write invalid data.
    ///
    /// This occurs if you try to change a `0` bit to a `1` bit without an erase.
    InvalidData,

    /// A flash command failed because a bank was in a mode other than read.
    CommandFail,

    /// Some other unknown flash error.
    Other,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Size => write!(f, "incorrect size"),
            Self::OutOfBounds => write!(f, "a flash access was out of bounds"),
            Self::Protected => write!(f, "sector is protected"),
            Self::Unaligned => write!(f, "address is unaligned"),
            Self::Verify => write!(f, "could not verify"),
            Self::IllegalAddress => write!(f, "write or erase to statically protected address"),
            Self::InvalidData => write!(f, "invalid data write"),
            Self::CommandFail => {
                write!(f, "flash command failed because bank was not in READ mode")
            }
            Self::Other => write!(f, "other flash error"),
        }
    }
}

impl core::error::Error for Error {}

/// Flash controller
pub struct Flashctl<'d, MODE = Async> {
    _inner: Peri<'d, FLASHCTL>,
    _mode: PhantomData<MODE>,
}

impl<'d> Flashctl<'d, Blocking> {
    /// Create a new flash driver, usable in blocking mode.
    pub fn new_blocking(p: Peri<'d, FLASHCTL>) -> Self {
        Self {
            _inner: p,
            _mode: PhantomData,
        }
    }
}

impl<'d, MODE> Flashctl<'d, MODE> {
    /// Blocking read.
    ///
    /// NOTE: `offset` is an offset from the flash start, NOT an absolute address.
    /// For example, to write address `0x0800_1234` you have to use offset `0x1234`.
    pub fn blocking_read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Error> {
        blocking_read_inner(FLASH_BASE, main_flash_size() * SECTOR_SIZE, offset, bytes)
    }

    /// Blocking write.
    ///
    /// NOTE: `offset` is an offset from the flash start, NOT an absolute address.
    /// For example, to write address `0x0800_1234` you have to use offset `0x1234`.
    pub fn blocking_write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Error> {
        blocking_write_inner(FLASH_BASE, main_flash_size() * SECTOR_SIZE, offset, bytes)
    }

    /// Blocking erase.
    ///
    /// NOTE: `from` and `to` are offsets from the flash start, NOT an absolute address.
    /// For example, to erase address `0x0801_0000` you have to use offset `0x1_0000`.
    pub fn blocking_erase(&mut self, from: u32, to: u32) -> Result<(), Error> {
        blocking_erase_inner(FLASH_BASE, main_flash_size() * SECTOR_SIZE, from, to)
    }
}

impl NorFlashError for Error {
    fn kind(&self) -> NorFlashErrorKind {
        match self {
            Error::Size => NorFlashErrorKind::OutOfBounds,
            Error::Unaligned => NorFlashErrorKind::NotAligned,
            _ => NorFlashErrorKind::Other,
        }
    }
}

impl<MODE> nor_flash::ErrorType for Flashctl<'_, MODE> {
    type Error = Error;
}

impl<MODE> ReadNorFlash for Flashctl<'_, MODE> {
    const READ_SIZE: usize = READ_SIZE as usize;

    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        self.blocking_read(offset, bytes)
    }

    fn capacity(&self) -> usize {
        main_flash_size() as usize
    }
}

impl<MODE> nor_flash::NorFlash for Flashctl<'_, MODE> {
    const WRITE_SIZE: usize = WRITE_SIZE as usize;
    const ERASE_SIZE: usize = ERASE_SIZE as usize;

    fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        self.blocking_erase(from, to)
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        self.blocking_write(offset, bytes)
    }
}

const READ_SIZE: u32 = 1;
// 1 flash word
const WRITE_SIZE: u32 = 8;
const SECTOR_SIZE: u32 = 1024;
// 1K sector size.
const ERASE_SIZE: u32 = SECTOR_SIZE;

/// Flash is always at address 0
const FLASH_BASE: u32 = 0x0000_0000;

/// Write a 64-bit value with ECC correction.
const BYTEEN_64BIT_WITH_ECC: u32 = 0x1FF;

/// Verify a flash access does not exceed the bounds of flash.
fn bounds_checks(size: u32, offset: u32, len: u32) -> Result<(), Error> {
    if offset > size {
        return Err(Error::OutOfBounds);
    }

    // offset + length may result in an overflow.
    let Some(end) = offset.checked_add(len) else {
        return Err(Error::OutOfBounds);
    };

    if end > size {
        return Err(Error::OutOfBounds);
    }

    Ok(())
}

fn blocking_read_inner(base: u32, size: u32, offset: u32, bytes: &mut [u8]) -> Result<(), Error> {
    bounds_checks(size, offset, bytes.len() as u32)?;

    let start_address = (base + offset) as *const u8;

    // SAFETY:
    // - src is valid because it is a slice of u8. Size is valid because FACTORYREGION asserts the true flash size in hardware.
    // - dst is valid for bytes.len() writes because the slice guarantees it is valid.
    // - src and dst are properly aligned since align is 1.
    // - src and dst cannot overlap because the dst must be in RAM in order to be written to.
    unsafe { core::ptr::copy_nonoverlapping(start_address, bytes.as_mut_ptr(), bytes.len()) };

    Ok(())
}

/// Blocking wait until the flash controller is available to use again.
fn wait_busy() {
    // Wait until flash controller is available.
    while pac::FLASHCTL.statcmd().read().inprogress() {}
}

fn blocking_write_inner(base: u32, size: u32, offset: u32, bytes: &[u8]) -> Result<(), Error> {
    bounds_checks(size, offset, bytes.len() as u32)?;

    if offset % WRITE_SIZE as u32 != 0 {
        return Err(Error::Unaligned);
    }

    if bytes.len() % WRITE_SIZE as usize != 0 {
        return Err(Error::Unaligned);
    }

    let mut address = base + offset;
    trace!(
        "Writing {} bytes at 0x{:x} (base=0x{:x}, offset=0x{:x})",
        bytes.len(),
        address,
        base,
        offset
    );

    let (chunks, remainder) = bytes.as_chunks::<{ WRITE_SIZE as usize }>();
    debug_assert!(remainder.is_empty(), "Copy was not aligned to WRITE_SIZE");

    for chunk in chunks {
        wait_busy();
        write_chunk(address, chunk)?;
        address += WRITE_SIZE as u32;
    }

    program_noop();

    Ok(())
}

fn write_chunk(address: u32, chunk: &[u8; WRITE_SIZE as usize]) -> Result<(), Error> {
    pac::FLASHCTL.cmdtype().write(|w| {
        w.set_command(vals::Command::Program);
        // All chips support Oneword.
        // TODO: Allow using multi-word
        w.set_size(vals::Size::Oneword);
    });

    pac::FLASHCTL.cmdctl().write(|w| {
        // Verify after write.
        w.set_postveren(true);
        // DATAVEREN must be enabled to detect 0 -> 1 changes.
        w.set_dataveren(true);
        w.set_addrxlateovr(false);
    });

    pac::FLASHCTL.cmdaddr().write_value(address);
    // 0x1FF to write entire word with ECC.
    pac::FLASHCTL
        .cmdbyten()
        .write_value(regs::Cmdbyten(BYTEEN_64BIT_WITH_ECC));

    let ([lower, upper], _) = chunk.as_chunks::<4>() else {
        unreachable!()
    };

    pac::FLASHCTL.cmddata(0).write_value(u32::from_le_bytes(*lower));
    pac::FLASHCTL.cmddata(1).write_value(u32::from_le_bytes(*upper));

    unprotect_sector(address);
    pac::FLASHCTL.cmdexec().write(|w| w.set_val(true));

    wait_busy();
    protect_sector(address);

    get_error()
}

fn blocking_erase_inner(base: u32, size: u32, from: u32, to: u32) -> Result<(), Error> {
    if from >= to {
        return Err(Error::Size);
    }

    bounds_checks(size, from, to - from)?;

    let start_address = base + from;
    let end_address = base + to;

    if (from % SECTOR_SIZE as u32) != 0 {
        return Err(Error::Unaligned);
    }

    if (to % SECTOR_SIZE as u32) != 0 {
        return Err(Error::Unaligned);
    }

    let Some(erase_size) = to.checked_sub(from) else {
        return Err(Error::Size);
    };

    if erase_size > size {
        return Err(Error::Size);
    }

    // Size must be a multiple of sector size because from and to are multiple of sector size.
    trace!("Erasing from 0x{:x} to 0x{:x}", start_address, end_address);

    let mut address = start_address;

    while address < end_address {
        wait_busy();
        blocking_erase_sector(address)?;
        address += SECTOR_SIZE as u32;
    }

    program_noop();
    Ok(())
}

fn blocking_erase_sector(sector_base: u32) -> Result<(), Error> {
    pac::FLASHCTL.cmdtype().write(|w| {
        w.set_command(vals::Command::Erase);
        w.set_size(vals::Size::Sector);
    });

    pac::FLASHCTL.cmdaddr().write_value(sector_base);
    pac::FLASHCTL
        .cmdbyten()
        .write_value(regs::Cmdbyten(BYTEEN_64BIT_WITH_ECC));

    unprotect_sector(sector_base);
    pac::FLASHCTL.cmdexec().write(|w| w.set_val(true));

    // Wait for erase to finish.
    while !pac::FLASHCTL.statcmd().read().done() {}

    protect_sector(sector_base);
    get_error()
}

/// PROTA is 32, 1 sector regions.
const PROTA_SECTOR_END: u32 = 32;

/// For devices with PROTA, PROTB is 28, 8 sector regions.
///
/// Otherwise PROTB is 32, 8 sector regions.
// TODO: Handle devices without PROTA.
const PROTB_SECTOR_END: u32 = 256;

/// PROTC is 32, 8 sector regions.
const PROTC_SECTOR_END: u32 = 512;

fn address_to_physical_sector(address: u32) -> u32 {
    address / SECTOR_SIZE as u32
}

/// Compute the sector number inside of the corresponding bank.
fn address_to_sector_in_bank(address: u32) -> u32 {
    let physical = address_to_physical_sector(address);
    let num_banks = main_num_banks() as u32;

    if num_banks > 1 {
        let sectors_per_bank = main_flash_size() / num_banks;

        // Banks divide the flash use the remainder to "wrap" the sector number around if the
        // sector is in upper banks.
        return physical % sectors_per_bank;
    }

    physical
}

/// Look up which register and the bit in the register sector guards dynamic write protection for the
/// specified address in flash.
///
/// This code is based off of the MSPM0 SDK because of the special cases.
fn main_sector_to_bit(sector_base: u32) -> (Reg<Cmdweprot, RW>, usize) {
    let physical_sector = address_to_physical_sector(sector_base);
    let sector_in_bank = address_to_sector_in_bank(sector_base);

    // TODO: handle devices which do not have PROTA.
    if physical_sector < PROTA_SECTOR_END {
        let bit = physical_sector as usize;

        (pac::FLASHCTL.cmdweprota(), bit)
    } else if sector_in_bank < PROTB_SECTOR_END {
        let main_num_banks = main_num_banks();

        // For devices with a single bank, we
        let bit = if main_num_banks == 1 {
            (sector_in_bank - 32) / 8
        } else {
            sector_in_bank / 8
        };

        (pac::FLASHCTL.cmdweprotb(), bit as usize)
    } else if sector_in_bank < PROTC_SECTOR_END {
        // PROTC uses 8 sector regions, starting at 256KB
        let bit = ((sector_in_bank - 256) / 8) as usize;

        (pac::FLASHCTL.cmdweprotc(), bit)
    } else {
        unreachable!("Invalid flash region")
    }
}

fn unprotect_sector(sector_base: u32) {
    let (reg, bit) = main_sector_to_bit(sector_base);

    // Guard dynamic write protection RMW from multiple threads.
    critical_section::with(|_| {
        reg.modify(|w| w.set_region(bit, false));
    });
}

fn protect_sector(sector_base: u32) {
    let (reg, bit) = main_sector_to_bit(sector_base);

    // Guard dynamic write protection RMW from multiple threads.
    critical_section::with(|_| {
        reg.modify(|w| w.set_region(bit, true));
    });
}

/// Per SLAU847e "6.3.2 NOOP Command" it is suggested to program NOOP when flash controller is not in use.
fn program_noop() {
    pac::FLASHCTL.cmdtype().write(|w| {
        w.set_command(vals::Command::Noop);
    });
}

fn get_error() -> Result<(), Error> {
    let stat = pac::FLASHCTL.statcmd().read();

    if !stat.pass() {
        // Errors are in order of lowest bit to highest bit according to register layout.
        if stat.failweprot() {
            return Err(Error::Protected);
        }

        if stat.failverify() {
            return Err(Error::Verify);
        }

        if stat.faililladdr() {
            return Err(Error::IllegalAddress);
        }

        if stat.failmode() {
            return Err(Error::CommandFail);
        }

        if stat.failinvdata() {
            return Err(Error::InvalidData);
        }

        if stat.failmisc() {
            return Err(Error::Other);
        }
    }

    Ok(())
}
