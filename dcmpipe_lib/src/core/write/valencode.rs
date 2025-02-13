//! This module contains implementations for encoding values for a DICOM element's value field
//! bytes, based on the element's value representation and transfer syntax.

use std::{iter::once, mem::size_of};

use crate::core::{
    dcmelement::DicomElement,
    defn::vr::{CS_SEPARATOR, CS_SEPARATOR_BYTE},
    read::{ParseError, ParseResult},
    values::{Attribute, RawValue},
};

/// Encodes a RawValue into the binary data for the given element, based on the element's currently
/// set Value Representation, Character Set, and Transfer Syntax.
pub struct ElemAndRawValue<'a>(pub &'a DicomElement, pub RawValue);
impl<'a> TryFrom<ElemAndRawValue<'a>> for Vec<u8> {
    type Error = ParseError;

    fn try_from(value: ElemAndRawValue<'a>) -> Result<Self, Self::Error> {
        let elem = value.0;
        let value = value.1;

        let mut bytes: Vec<u8> = match value {
            RawValue::Attribute(attrs) => ElemAndAttributes(elem, attrs).into(),
            RawValue::Uid(uid) => ElemAndUid(elem, uid).try_into()?,
            RawValue::Strings(strings) => ElemAndStrings(elem, strings).try_into()?,
            RawValue::Shorts(shorts) => ElemAndShorts(elem, shorts).into(),
            RawValue::UnsignedShorts(ushorts) => ElemAndUnsignedShorts(elem, ushorts).into(),
            RawValue::Integers(ints) => ElemAndIntegers(elem, ints).into(),
            RawValue::UnsignedIntegers(uints) => ElemAndUnsignedIntegers(elem, uints).into(),
            RawValue::Longs(longs) => ElemAndLongs(elem, longs).into(),
            RawValue::UnsignedLongs(ulongs) => ElemAndUnsignedLongs(elem, ulongs).into(),
            RawValue::Floats(floats) => ElemAndFloats(elem, floats).into(),
            RawValue::Doubles(doubles) => ElemAndDoubles(elem, doubles).into(),
            RawValue::Bytes(bytes) => bytes,
            RawValue::Words(words) => ElemAndWords(elem, words).into(),
            RawValue::DoubleWords(dwords) => ElemAndDoubleWords(elem, dwords).into(),
            RawValue::QuadWords(qwords) => ElemAndQuadWords(elem, qwords).into(),
        };

        // All fields are required to be of even length, with padding added as necessary. Note
        // that the standard refers to values of "character string" however binary values are
        // expected to always result in even number of bytes.
        //
        // Part 5, Ch 6.4:
        // Each string Value in a multiple valued character string may be of even or odd length,
        // but the length of the entire Value Field (including "\" delimiters) shall be of even
        // length. If padding is required to make the Value Field of even length, a single padding
        // character shall be applied to the end of the Value Field (to the last Value), in which
        // case the length of the last Value may exceed the Length of Value by 1.
        if bytes.len() % 2 != 0 {
            bytes.push(elem.vr().padding);
        }

        Ok(bytes)
    }
}

struct ElemAndAttributes<'a>(&'a DicomElement, Vec<Attribute>);
impl<'a> From<ElemAndAttributes<'a>> for Vec<u8> {
    fn from(value: ElemAndAttributes<'a>) -> Self {
        let elem = value.0;
        let attrs = value.1;

        const U32_SIZE: usize = size_of::<u32>();
        let num_attrs = attrs.len();
        let mut bytes: Vec<u8> = vec![0u8; U32_SIZE * num_attrs];
        for (i, attr) in attrs.iter().enumerate() {
            let Attribute(attr) = attr;
            let group_number: u16 = ((attr >> 16) & 0xFFFF) as u16;
            let elem_number: u16 = (attr & 0xFFFF) as u16;
            let idx = i * U32_SIZE;
            if elem.ts().big_endian() {
                bytes[idx..(idx + 2)].copy_from_slice(&group_number.to_be_bytes());
                bytes[(idx + 2)..(idx + 4)].copy_from_slice(&elem_number.to_be_bytes());
            } else {
                bytes[idx..(idx + 2)].copy_from_slice(&group_number.to_le_bytes());
                bytes[(idx + 2)..(idx + 4)].copy_from_slice(&elem_number.to_le_bytes());
            }
        }
        bytes
    }
}

struct ElemAndUid<'a>(&'a DicomElement, String);
impl<'a> TryFrom<ElemAndUid<'a>> for Vec<u8> {
    type Error = ParseError;

    fn try_from(value: ElemAndUid<'a>) -> Result<Self, Self::Error> {
        let elem = value.0;
        let uid = value.1;
        elem.cs()
            .encode(&uid)
            .map_err(|e| ParseError::CharsetError { source: e })
    }
}

struct ElemAndStrings<'a>(&'a DicomElement, Vec<String>);
impl<'a> TryFrom<ElemAndStrings<'a>> for Vec<u8> {
    type Error = ParseError;

    fn try_from(value: ElemAndStrings<'a>) -> Result<Self, Self::Error> {
        let elem = value.0;
        let strings = value.1;

        type MaybeBytes = Vec<ParseResult<Vec<u8>>>;
        let (values, errs): (MaybeBytes, MaybeBytes) = strings
            .iter()
            .map(|s| {
                elem.cs()
                    .encode(s)
                    // Add the separator after each encoded value. Below the last separator
                    // will be popped off.
                    .map(|mut v| {
                        v.push(CS_SEPARATOR as u8);
                        v
                    })
                    .map_err(|e| ParseError::CharsetError { source: e })
            })
            .partition(ParseResult::is_ok);

        if let Some(Err(e)) = errs.into_iter().last() {
            return Err(e);
        }

        // Flatten the bytes for all strings.
        let mut bytes: Vec<u8> = values
            .into_iter()
            .flat_map(ParseResult::unwrap)
            .collect::<Vec<u8>>();
        // Remove last separator.
        bytes.pop();
        Ok(bytes)
    }
}

struct ElemAndShorts<'a>(&'a DicomElement, Vec<i16>);
impl<'a> From<ElemAndShorts<'a>> for Vec<u8> {
    fn from(value: ElemAndShorts<'a>) -> Self {
        let elem = value.0;
        let shorts = value.1;

        if elem.vr().is_character_string {
            // This should only be the case for a VR of IS.
            let mut encoded = shorts
                .into_iter()
                // In theory this should use the default character set, but this
                // relies on i16::to_string only using ascii which falls under that.
                .map(|short: i16| short.to_string().into_bytes())
                .flat_map(|v| v.into_iter().chain(once(CS_SEPARATOR_BYTE)))
                .collect::<Vec<u8>>();
            encoded.pop();
            encoded
        } else {
            // This should only be the case for a VR of SS
            shorts
                .into_iter()
                .flat_map(|short: i16| {
                    if elem.ts().big_endian() {
                        short.to_be_bytes()
                    } else {
                        short.to_le_bytes()
                    }
                })
                .collect::<Vec<u8>>()
        }
    }
}

struct ElemAndUnsignedShorts<'a>(&'a DicomElement, Vec<u16>);
impl<'a> From<ElemAndUnsignedShorts<'a>> for Vec<u8> {
    fn from(value: ElemAndUnsignedShorts<'a>) -> Self {
        let elem = value.0;
        let ushorts = value.1;

        if elem.vr().is_character_string {
            // This should only be the case for a VR of IS.
            let mut encoded = ushorts
                .into_iter()
                // In theory this should use the default character set, but this
                // relies on u16::to_string only using ascii which falls under that.
                .map(|ushort: u16| ushort.to_string().into_bytes())
                .flat_map(|v| v.into_iter().chain(once(CS_SEPARATOR_BYTE)))
                .collect::<Vec<u8>>();
            encoded.pop();
            encoded
        } else {
            // This should only be the case for a VR of US
            ushorts
                .into_iter()
                .flat_map(|ushort: u16| {
                    if elem.ts().big_endian() {
                        ushort.to_be_bytes()
                    } else {
                        ushort.to_le_bytes()
                    }
                })
                .collect::<Vec<u8>>()
        }
    }
}

struct ElemAndIntegers<'a>(&'a DicomElement, Vec<i32>);
impl<'a> From<ElemAndIntegers<'a>> for Vec<u8> {
    fn from(value: ElemAndIntegers<'a>) -> Self {
        let elem = value.0;
        let ints = value.1;

        if elem.vr().is_character_string {
            // This should only be the case for a VR of IS.
            let mut encoded = ints
                .into_iter()
                // In theory this should use the default character set, but this
                // relies on i32::to_string only using ascii which falls under that.
                .map(|int: i32| int.to_string().into_bytes())
                .flat_map(|v| v.into_iter().chain(once(CS_SEPARATOR_BYTE)))
                .collect::<Vec<u8>>();
            encoded.pop();
            encoded
        } else {
            // This should only be the case for a VR of SL.
            ints.into_iter()
                .flat_map(|int: i32| {
                    if elem.ts().big_endian() {
                        int.to_be_bytes()
                    } else {
                        int.to_le_bytes()
                    }
                })
                .collect::<Vec<u8>>()
        }
    }
}

struct ElemAndUnsignedIntegers<'a>(&'a DicomElement, Vec<u32>);
impl<'a> From<ElemAndUnsignedIntegers<'a>> for Vec<u8> {
    fn from(value: ElemAndUnsignedIntegers<'a>) -> Self {
        let elem = value.0;
        let uints = value.1;

        if elem.vr().is_character_string {
            // XXX: This shouldn't happen. Unsigned integers should only ever be encoded
            // as binary.
            let mut encoded = uints
                .into_iter()
                // In theory this should use the default character set, but this
                // relies on i16::to_string only using ascii which falls under that.
                .map(|uint: u32| uint.to_string().into_bytes())
                .flat_map(|v| v.into_iter().chain(once(CS_SEPARATOR_BYTE)))
                .collect::<Vec<u8>>();
            encoded.pop();
            encoded
        } else {
            // This should only be the case for a VR of UL.
            uints
                .into_iter()
                .flat_map(|uint: u32| {
                    if elem.ts().big_endian() {
                        uint.to_be_bytes()
                    } else {
                        uint.to_le_bytes()
                    }
                })
                .collect::<Vec<u8>>()
        }
    }
}

struct ElemAndLongs<'a>(&'a DicomElement, Vec<i64>);
impl<'a> From<ElemAndLongs<'a>> for Vec<u8> {
    fn from(value: ElemAndLongs<'a>) -> Self {
        let elem = value.0;
        let longs = value.1;

        if elem.vr().is_character_string {
            // This should only be the case for a VR of IS.
            let mut encoded = longs
                .into_iter()
                // In theory this should use the default character set, but this
                // relies on i32::to_string only using ascii which falls under that.
                .map(|long: i64| long.to_string().into_bytes())
                .flat_map(|v| v.into_iter().chain(once(CS_SEPARATOR_BYTE)))
                .collect::<Vec<u8>>();
            encoded.pop();
            encoded
        } else {
            // This should only be the case for a VR of SL.
            longs
                .into_iter()
                .flat_map(|long: i64| {
                    if elem.ts().big_endian() {
                        long.to_be_bytes()
                    } else {
                        long.to_le_bytes()
                    }
                })
                .collect::<Vec<u8>>()
        }
    }
}

struct ElemAndUnsignedLongs<'a>(&'a DicomElement, Vec<u64>);
impl<'a> From<ElemAndUnsignedLongs<'a>> for Vec<u8> {
    fn from(value: ElemAndUnsignedLongs<'a>) -> Self {
        let elem = value.0;
        let ulongs = value.1;

        if elem.vr().is_character_string {
            // XXX: This shouldn't happen. Unsigned integers should only ever be encoded
            // as binary.
            let mut encoded = ulongs
                .into_iter()
                // In theory this should use the default character set, but this
                // relies on i16::to_string only using ascii which falls under that.
                .map(|ulong: u64| ulong.to_string().into_bytes())
                .flat_map(|v| v.into_iter().chain(once(CS_SEPARATOR_BYTE)))
                .collect::<Vec<u8>>();
            encoded.pop();
            encoded
        } else {
            // This should only be the case for a VR of UL.
            ulongs
                .into_iter()
                .flat_map(|ulong: u64| {
                    if elem.ts().big_endian() {
                        ulong.to_be_bytes()
                    } else {
                        ulong.to_le_bytes()
                    }
                })
                .collect::<Vec<u8>>()
        }
    }
}

struct ElemAndFloats<'a>(&'a DicomElement, Vec<f32>);
impl<'a> From<ElemAndFloats<'a>> for Vec<u8> {
    fn from(value: ElemAndFloats<'a>) -> Self {
        let elem = value.0;
        let floats = value.1;

        if elem.vr().is_character_string {
            // This should only be the case for a VR of DS.
            let mut encoded = floats
                .into_iter()
                .filter(|float: &f32| float.is_finite())
                // In theory this should use the default character set, but this
                // relies on f32::to_string only using ascii which falls under that.
                .map(|float: f32| {
                    // Force at least one digit of precision.
                    if float.fract() == 0.0 {
                        format!("{float:.1}").into_bytes()
                    } else {
                        float.to_string().into_bytes()
                    }
                })
                .flat_map(|v| v.into_iter().chain(once(CS_SEPARATOR_BYTE)))
                .collect::<Vec<u8>>();
            encoded.pop();
            encoded
        } else {
            // This should only be the case for a VR of FL.
            floats
                .into_iter()
                .filter(|float: &f32| float.is_finite())
                .flat_map(|float: f32| {
                    if elem.ts().big_endian() {
                        float.to_be_bytes()
                    } else {
                        float.to_le_bytes()
                    }
                })
                .collect::<Vec<u8>>()
        }
    }
}

struct ElemAndDoubles<'a>(&'a DicomElement, Vec<f64>);
impl<'a> From<ElemAndDoubles<'a>> for Vec<u8> {
    fn from(value: ElemAndDoubles<'a>) -> Self {
        let elem = value.0;
        let doubles = value.1;

        if elem.vr().is_character_string {
            // This should only be the case for a VR of DS.
            let mut encoded = doubles
                .into_iter()
                .filter(|double: &f64| double.is_finite())
                // In theory this should use the default character set, but this
                // relies on f64::to_string only using ascii which falls under that.
                .map(|double: f64| {
                    // Force at least one digit of precision.
                    if double.fract() == 0.0 {
                        format!("{double:.1}").into_bytes()
                    } else {
                        double.to_string().into_bytes()
                    }
                })
                .flat_map(|v| v.into_iter().chain(once(CS_SEPARATOR_BYTE)))
                .collect::<Vec<u8>>();
            encoded.pop();
            encoded
        } else {
            // This should only be the case for a VR of FL.
            doubles
                .into_iter()
                .filter(|double: &f64| double.is_finite())
                .flat_map(|double: f64| {
                    if elem.ts().big_endian() {
                        double.to_be_bytes()
                    } else {
                        double.to_le_bytes()
                    }
                })
                .collect::<Vec<u8>>()
        }
    }
}

struct ElemAndWords<'a>(&'a DicomElement, Vec<u16>);
impl<'a> From<ElemAndWords<'a>> for Vec<u8> {
    fn from(value: ElemAndWords<'a>) -> Self {
        let elem = value.0;
        let words = value.1;

        words
            .into_iter()
            .flat_map(|word| {
                if elem.ts().big_endian() {
                    word.to_be_bytes()
                } else {
                    word.to_le_bytes()
                }
            })
            .collect::<Vec<u8>>()
    }
}

struct ElemAndDoubleWords<'a>(&'a DicomElement, Vec<u32>);
impl<'a> From<ElemAndDoubleWords<'a>> for Vec<u8> {
    fn from(value: ElemAndDoubleWords<'a>) -> Self {
        let elem = value.0;
        let dwords = value.1;

        dwords
            .into_iter()
            .flat_map(|dword| {
                if elem.ts().big_endian() {
                    dword.to_be_bytes()
                } else {
                    dword.to_le_bytes()
                }
            })
            .collect::<Vec<u8>>()
    }
}

struct ElemAndQuadWords<'a>(&'a DicomElement, Vec<u64>);
impl<'a> From<ElemAndQuadWords<'a>> for Vec<u8> {
    fn from(value: ElemAndQuadWords<'a>) -> Self {
        let elem = value.0;
        let qwords = value.1;

        qwords
            .into_iter()
            .flat_map(|qword| {
                if elem.ts().big_endian() {
                    qword.to_be_bytes()
                } else {
                    qword.to_le_bytes()
                }
            })
            .collect::<Vec<u8>>()
    }
}
