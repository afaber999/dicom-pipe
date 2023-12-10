use std::io::{ErrorKind, Read};

use super::parser::Result;
use crate::core::read::error::ParseError;
use crate::defn::constants::tags;
use crate::defn::ts::TSRef;
use crate::defn::vl;
use crate::defn::vl::ValueLength;
use crate::defn::vr::{self, VRRef, VR};

/// Whether the element is a non-standard parent-able element. These are non-SQ, non-ITEM elements
/// with a VR of `UN`, `OB`, `OF`, or `OW` and have a value length of `UndefinedLength`. These
/// types of elements are considered either private-tag sequences or otherwise whose contents are
/// encoded as IVRLE.
pub(crate) fn is_non_standard_seq(tag: u32, vr: VRRef, vl: ValueLength) -> bool {
    tag != tags::ITEM
        && (vr == &vr::UN || vr == &vr::OB || vr == &vr::OF || vr == &vr::OW)
        && vl == ValueLength::UndefinedLength
}

/// This is a variation of `Read::read_exact` however if zero bytes are read instead of returning
/// an error with `ErrorKind::UnexpectedEof` it will return an error with `ParseError::ExpectedEOF`.
fn read_exact_expect_eof(dataset: &mut impl Read, mut buf: &mut [u8]) -> Result<()> {
    let mut bytes_read: usize = 0;
    while !buf.is_empty() {
        match dataset.read(buf) {
            Ok(0) => break,
            Ok(n) => {
                bytes_read += n;
                let tmp = buf;
                buf = &mut tmp[n..];
            }
            Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    if !buf.is_empty() {
        if bytes_read == 0 {
            Err(ParseError::ExpectedEOF)
        } else {
            Err(ParseError::IOError {
                source: std::io::Error::new(
                    ErrorKind::UnexpectedEof,
                    format!("failed to fill whole buffer, read {} bytes", bytes_read),
                ),
            })
        }
    } else {
        Ok(())
    }
}

/// Reads a tag attribute from a given dataset
pub(crate) fn read_tag_from_dataset(dataset: &mut impl Read, big_endian: bool) -> Result<u32> {
    let mut buf: [u8; 2] = [0; 2];

    read_exact_expect_eof(dataset, &mut buf)?;
    let group_number: u32 = if big_endian {
        u32::from(u16::from_be_bytes(buf)) << 16
    } else {
        u32::from(u16::from_le_bytes(buf)) << 16
    };

    dataset.read_exact(&mut buf)?;
    let element_number: u32 = if big_endian {
        u32::from(u16::from_be_bytes(buf))
    } else {
        u32::from(u16::from_le_bytes(buf))
    };

    let tag: u32 = group_number + element_number;
    Ok(tag)
}

/// Reads a VR from a given dataset.
pub(crate) fn read_vr_from_dataset(dataset: &mut impl Read) -> Result<VRRef> {
    let mut buf: [u8; 2] = [0; 2];
    dataset.read_exact(&mut buf)?;
    let first_char: u8 = buf[0];
    let second_char: u8 = buf[1];

    let code: u16 = (u16::from(first_char) << 8) + u16::from(second_char);
    let vr: VRRef = match VR::from_code(code) {
        Some(found_vr) => {
            if found_vr.has_explicit_2byte_pad {
                dataset.read_exact(&mut buf)?;
            }
            found_vr
        }
        None => return Err(ParseError::UnknownExplicitVR(code)),
    };

    Ok(vr)
}

/// Reads a Value Length from a given dataset.
/// `dataset` The dataset to read bytes from.
/// `ts` The transfer syntax of the element being read from.
/// `vr` The VR of the current element the value length is being read for.
pub(crate) fn read_value_length_from_dataset(
    dataset: &mut impl Read,
    ts: TSRef,
    vr: VRRef,
) -> Result<ValueLength> {
    let value_length: u32 = if !ts.is_explicit_vr() || vr.has_explicit_2byte_pad {
        let mut buf: [u8; 4] = [0; 4];
        dataset.read_exact(&mut buf)?;
        if ts.is_big_endian() {
            u32::from_be_bytes(buf)
        } else {
            u32::from_le_bytes(buf)
        }
    } else {
        let mut buf: [u8; 2] = [0; 2];
        dataset.read_exact(&mut buf)?;
        if ts.is_big_endian() {
            u16::from_be_bytes(buf) as u32
        } else {
            u16::from_le_bytes(buf) as u32
        }
    };
    Ok(vl::from_value_length(value_length))
}
