use thiserror::Error;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum CodecError {
    #[error("unexpected end of canonical input")]
    UnexpectedEof,
    #[error("canonical length does not fit in u32")]
    LengthOverflow,
    #[error("invalid canonical value: {0}")]
    InvalidValue(&'static str),
    #[error("trailing bytes after canonical value")]
    TrailingBytes,
}

pub trait CanonicalCodec: Sized {
    fn encode_canonical(&self) -> Vec<u8>;
    fn decode_canonical(bytes: &[u8]) -> Result<Self, CodecError>;
}

#[derive(Default)]
pub(crate) struct Writer {
    pub bytes: Vec<u8>,
}

impl Writer {
    pub fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn fixed(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
    }

    pub fn bytes(&mut self, value: &[u8]) -> Result<(), CodecError> {
        let len = u32::try_from(value.len()).map_err(|_| CodecError::LengthOverflow)?;
        self.u32(len);
        self.fixed(value);
        Ok(())
    }

    pub fn count(&mut self, count: usize) -> Result<(), CodecError> {
        self.u32(u32::try_from(count).map_err(|_| CodecError::LengthOverflow)?);
        Ok(())
    }
}

pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], CodecError> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or(CodecError::UnexpectedEof)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(CodecError::UnexpectedEof)?;
        self.offset = end;
        Ok(value)
    }

    pub fn u8(&mut self) -> Result<u8, CodecError> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16, CodecError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    pub fn u32(&mut self) -> Result<u32, CodecError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub fn fixed<const N: usize>(&mut self) -> Result<[u8; N], CodecError> {
        Ok(self.take(N)?.try_into().unwrap())
    }

    pub fn bytes(&mut self) -> Result<Vec<u8>, CodecError> {
        let len = self.u32()? as usize;
        Ok(self.take(len)?.to_vec())
    }

    pub fn vec<T, F>(&mut self, mut decode: F) -> Result<Vec<T>, CodecError>
    where
        F: FnMut(&mut Self) -> Result<T, CodecError>,
    {
        let len = self.u32()? as usize;
        let mut values = Vec::with_capacity(len.min(1024));
        for _ in 0..len {
            values.push(decode(self)?);
        }
        Ok(values)
    }

    pub fn finish(self) -> Result<(), CodecError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(CodecError::TrailingBytes)
        }
    }
}

pub(crate) fn encode_with<F>(f: F) -> Result<Vec<u8>, CodecError>
where
    F: FnOnce(&mut Writer) -> Result<(), CodecError>,
{
    let mut writer = Writer::default();
    f(&mut writer)?;
    Ok(writer.bytes)
}

pub(crate) fn decode_with<T, F>(bytes: &[u8], f: F) -> Result<T, CodecError>
where
    F: FnOnce(&mut Reader<'_>) -> Result<T, CodecError>,
{
    let mut reader = Reader::new(bytes);
    let value = f(&mut reader)?;
    reader.finish()?;
    Ok(value)
}
