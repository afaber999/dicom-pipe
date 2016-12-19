#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

use byteorder::{BigEndian, ByteOrder, LittleEndian, ReadBytesExt};

use core::dict::file_meta_elements as fme;
use core::dict::lookup::{TAG_BY_VALUE, TS_BY_ID};
use core::dict::transfer_syntaxes as ts;
use core::tag::Tag;
use core::ts::TransferSyntax;
use core::vr;

use std::collections::hash_map::HashMap;
use std::fmt;
use std::fs::File;
use std::io::{Cursor, Error, ErrorKind, Seek};
use std::path::Path;
use std::string;


pub const FILE_PREAMBLE_LENGTH: usize = 128;
pub const DICOM_PREFIX_LENGTH: usize = 4;

pub static DICOM_PREFIX: [u8;DICOM_PREFIX_LENGTH] = ['D' as u8, 'I' as u8, 'C' as u8, 'M' as u8];


pub struct DicomStream<StreamType> {
    stream: StreamType,

    file_preamble: [u8;FILE_PREAMBLE_LENGTH],
    dicom_prefix: [u8;DICOM_PREFIX_LENGTH],
    
    file_meta: HashMap<u32, DicomElement>,
    ts: &'static TransferSyntax,

    // To allow peeking the next tag without fully reading the next element 
    tag_peek: Option<u32>,

    bytes_read: usize,
}

pub struct DicomElement {
    pub tag: u32,
    pub vr: &'static vr::VR,
    pub vl: u32,
    pub bytes: Vec<u8>,
}

impl fmt::Debug for DicomElement {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "DicomElement {{ tag: {:08X}  vr: {}  vl: {} }}", self.tag, self.vr.ident, self.vl)
    }
}

impl DicomStream<File> {
    pub fn new_from_path(path: &Path) -> Result<DicomStream<File>, Error> {
        if !path.is_file() {
            return Err(Error::new(ErrorKind::InvalidData,
                                          format!("Invalid path: {:?}", path)));
        }

        let file: File = File::open(path)?;
        Ok::<DicomStream<File>, Error>(DicomStream::new(file))
    }
}

impl<StreamType: ReadBytesExt + Seek> DicomStream<StreamType> {
    pub fn new(stream: StreamType) -> DicomStream<StreamType> {
        DicomStream {
            stream: stream,
            file_preamble: [0u8;FILE_PREAMBLE_LENGTH],
            dicom_prefix: [0u8;DICOM_PREFIX_LENGTH],
            file_meta: HashMap::with_capacity(12),
            ts: &ts::ExplicitVRLittleEndian,
            tag_peek: None,
            bytes_read: 0usize,
        }
    }

    pub fn get_stream(&self) -> &StreamType {
        &self.stream
    }

    pub fn get_file_preamble(&self) -> &[u8;FILE_PREAMBLE_LENGTH] {
        &self.file_preamble
    }

    pub fn get_dicom_prefix(&self) -> &[u8;DICOM_PREFIX_LENGTH] {
        &self.dicom_prefix
    }

    pub fn read_file_preamble(&mut self) -> Result<(), Error> {
        self.stream.read_exact(&mut self.file_preamble)?;
        self.bytes_read += self.file_preamble.len();
        Ok(())
    }

    pub fn read_dicom_prefix(&mut self) -> Result<(), Error> {
        self.stream.read_exact(&mut self.dicom_prefix)?;
        self.bytes_read += self.dicom_prefix.len();

        for n in 0..DICOM_PREFIX.len() {
            if self.dicom_prefix[n] != DICOM_PREFIX[n] {
                return Err(Error::new(ErrorKind::InvalidData,
                                    format!("Invalid DICOM Prefix: {:?}", self.dicom_prefix)));
            }
        }

        Ok(())
    }

    /// Reads the next tag using the selected Endian. To allow for peeking
    /// what the next tag is without fully parsing it, the read tag value
    /// is stored in `self.tag_peek`. Calls to this method will repeatedly
    /// return the previously peek'd value until `self.tag_peek` is cleared.
    pub fn read_tag<Endian: ByteOrder>(&mut self) -> Result<u32, Error> {
        if self.tag_peek != None {
            return self.tag_peek.ok_or(Error::new(ErrorKind::InvalidData, format!("Unable to read next tag")));
        }
        let first: u32 = (self.stream.read_u16::<Endian>()? as u32) << 16;
        let second: u32 = self.stream.read_u16::<Endian>()? as u32;
        let result: u32 = first + second;
        self.tag_peek = Some(result);
        self.bytes_read += 4;
        Ok(result)
    }

    pub fn read_vr(&mut self) -> Result<&'static vr::VR, Error> {
        let first_char: u8 = self.stream.read_u8()?;
        let second_char: u8 = self.stream.read_u8()?;
        let code: u16 = ((first_char as u16) << 8) + second_char as u16;
        self.bytes_read += 2;
        match vr::VR::code_to_vr(code) {
            Some(vr) => Ok(vr),
            None => Err(Error::new(ErrorKind::InvalidData, format!("Unable to interpret VR: {:?}", code)))
        }
    }

    pub fn read_value_length<Endian: ByteOrder>(&mut self, vr: &vr::VR) -> Result<u32, Error> {
        let vl: u32 = match vr.explicit_vr_header_bytes {
            8 => self.stream.read_u16::<Endian>().map(|n| n as u32),
            12 => {
                self.stream.read_u16::<Endian>()?;
                self.stream.read_u32::<Endian>()
            },
            n => Err(Error::new(ErrorKind::InvalidData, format!("Invalid VR Header Length: {:?}", n))),
        }?;
        self.bytes_read += vr.explicit_vr_header_bytes as usize;
        Ok(vl)
    }

    pub fn read_value_field(&mut self, value_length: u32) -> Result<Vec<u8>, Error> {
        let mut bytes: Vec<u8> = vec![0;value_length as usize];
        self.stream.read_exact(bytes.as_mut_slice())?;
        self.bytes_read += value_length as usize;
        Ok(bytes)
    }

    pub fn read_dicom_element(&mut self) -> Result<DicomElement, Error> {
        if self.ts.big_endian {
            self._read_dicom_element::<BigEndian>()
        } else {
            self._read_dicom_element::<LittleEndian>()
        }
    }

    fn _read_dicom_element<Endian: ByteOrder>(&mut self) -> Result<DicomElement, Error> {
        let tag: u32;
        match self.tag_peek {
            Some(read_tag) => tag = read_tag,
            None => tag = self.read_tag::<Endian>()?,
        };

        // Clear `self.tag_peek` so subsequent calls will read the next tag value
        self.tag_peek = None;
        
        let vr: &vr::VR = if self.ts.explicit_vr {
            self.read_vr()?
        } else {
            TAG_BY_VALUE.get(&tag)
                .and_then(|read_tag: &&Tag| read_tag.implicit_vr)
                .unwrap_or(&vr::UN)
        };

        let vl: u32 = self.read_value_length::<Endian>(vr)?;
        let bytes: Vec<u8> = self.read_value_field(vl)?;

        Ok(DicomElement {
            tag: tag,
            vr: vr,
            vl: vl,
            bytes: bytes,
        })
    }

    pub fn read_file_meta(&mut self) -> Result<(), Error> {
        // This is required for "well-formed" DICOM files however it's not 100% required
        // so somehow detect reading of FileMetaInformationGroupLength maybe?
        self.read_file_preamble()?;
        self.read_dicom_prefix()?;

        let bytes_read_before_fme: usize = self.bytes_read;

        // All FileMetaInformation fields are encoded as LittleEndian

        // The FileMetaInformationGroupLength is required first element and
        // tells us how many bytes to reach end of FileMetaInformation
        let fmi_grouplength: DicomElement = self.read_dicom_element()?;
        if fmi_grouplength.tag != fme::FileMetaInformationGroupLength.tag {
            return Err(Error::new(ErrorKind::InvalidData, format!("Expected FileMetaInformationGroupLength but read: {:?}", fmi_grouplength)))
        }

        // TODO: this reading of bytes as u32 should be part of VR (and remove padding)
        let mut fmi_grouplength_rdr: Cursor<Vec<u8>> = Cursor::new(fmi_grouplength.bytes);
        let fme_bytes: usize = fmi_grouplength_rdr.read_u32::<LittleEndian>()? as usize;

        let mut transfer_syntax: &TransferSyntax = &ts::ImplicitVRLittleEndian;
        while self.bytes_read - bytes_read_before_fme < fme_bytes {    
            let element: DicomElement = self.read_dicom_element()?;
            if element.tag == fme::TransferSyntaxUID.tag {
                // strip out the padding bytes for the tag being read
                // TODO: this filtering is generally not correct as it's only padded
                // at the end of the value. Need to find a fast/easy way to remove trailing 0's
                let ts_uid_bytes: Vec<u8> = element.bytes.iter()
                    .filter(|b: &&u8| **b != vr::UI.padding)
                    .map(|b: &u8| *b)
                    .collect::<Vec<u8>>();

                let ts_uid: String = String::from_utf8(ts_uid_bytes)
                    .map_err(|e: string::FromUtf8Error| Error::new(ErrorKind::InvalidData, e))?;
                
                let ts_uid_str: &str = ts_uid.as_ref();
                if let Some(ts) = TS_BY_ID.get(ts_uid_str) {
                    transfer_syntax = ts;
                }
            }
            self.file_meta.insert(element.tag, element);
        }

        // don't set the transfer syntax until after reading all FileMeta, otherwise it 
        // will attempt to read remaining FME tags as different syntax than ExplicitVRLittleEndian (which is required)
        self.ts = transfer_syntax;

        Ok(())
    }
}
