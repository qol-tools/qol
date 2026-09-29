use std::collections::BTreeSet;

use zeroize::{Zeroize, Zeroizing};

use super::FrameError;

const MAX_DEPTH: usize = 32;
const MAX_VALUES: usize = 4096;

pub(super) fn validate(bytes: &[u8]) -> Result<(), FrameError> {
    validate_profile(bytes, false)
}

pub(crate) fn validate_operation(bytes: &[u8]) -> Result<(), FrameError> {
    validate_profile(bytes, true)
}

fn validate_profile(bytes: &[u8], exact_integers: bool) -> Result<(), FrameError> {
    std::str::from_utf8(bytes).map_err(|_| FrameError::InvalidUtf8)?;
    let mut parser = Validator {
        bytes,
        offset: 0,
        values: 0,
        exact_integers,
    };
    parser.value(0)?;
    parser.whitespace();
    if parser.offset != bytes.len() {
        return Err(FrameError::InvalidJson);
    }
    Ok(())
}

struct Validator<'a> {
    bytes: &'a [u8],
    offset: usize,
    values: usize,
    exact_integers: bool,
}

impl Validator<'_> {
    fn value(&mut self, depth: usize) -> Result<(), FrameError> {
        self.values += 1;
        if self.values > MAX_VALUES {
            return Err(FrameError::TooManyValues);
        }
        self.whitespace();
        match self.peek() {
            Some(b'{' | b'[') => self.container(depth + 1),
            Some(b'"') => self.string().map(|_| ()),
            Some(b't') => self.literal(b"true"),
            Some(b'f') => self.literal(b"false"),
            Some(b'n') => self.literal(b"null"),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(FrameError::InvalidJson),
        }
    }

    fn container(&mut self, depth: usize) -> Result<(), FrameError> {
        if depth > MAX_DEPTH {
            return Err(FrameError::TooDeep);
        }
        let object = self.peek() == Some(b'{');
        let end = if object { b'}' } else { b']' };
        self.offset += 1;
        self.whitespace();
        if self.consume(end) {
            return Ok(());
        }
        let mut keys = ObjectKeys(BTreeSet::new());
        loop {
            if object {
                let mut key = self.string()?;
                if keys.0.contains(key.as_str()) {
                    return Err(FrameError::DuplicateKey);
                }
                keys.0.insert(std::mem::take(&mut *key));
                self.whitespace();
                self.require(b':')?;
            }
            self.value(depth)?;
            self.whitespace();
            if self.consume(end) {
                return Ok(());
            }
            self.require(b',')?;
            self.whitespace();
        }
    }

    fn string(&mut self) -> Result<Zeroizing<String>, FrameError> {
        let start = self.offset;
        self.require(b'"')?;
        loop {
            match self.peek() {
                Some(b'"') => {
                    self.offset += 1;
                    return serde_json::from_slice(&self.bytes[start..self.offset])
                        .map(Zeroizing::new)
                        .map_err(|_| FrameError::InvalidJson);
                }
                Some(b'\\') => self.offset += 2,
                Some(0..=31) | None => return Err(FrameError::InvalidJson),
                Some(_) => self.offset += 1,
            }
        }
    }

    fn number(&mut self) -> Result<(), FrameError> {
        let start = self.offset;
        self.consume(b'-');
        match self.peek() {
            Some(b'0') => self.offset += 1,
            Some(b'1'..=b'9') => self.digits()?,
            _ => return Err(FrameError::InvalidJson),
        }
        if self.consume(b'.') {
            self.digits()?;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.offset += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.offset += 1;
            }
            self.digits()?;
        }
        if self.exact_integers {
            let token = std::str::from_utf8(&self.bytes[start..self.offset])
                .map_err(|_| FrameError::InvalidJson)?;
            let number = token.parse::<i64>().map_err(|_| FrameError::InvalidJson)?;
            if token == "-0" || !(-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&number)
            {
                return Err(FrameError::InvalidJson);
            }
        }
        Ok(())
    }

    fn digits(&mut self) -> Result<(), FrameError> {
        let start = self.offset;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.offset += 1;
        }
        if self.offset == start {
            return Err(FrameError::InvalidJson);
        }
        Ok(())
    }

    fn literal(&mut self, literal: &[u8]) -> Result<(), FrameError> {
        if !self.bytes[self.offset..].starts_with(literal) {
            return Err(FrameError::InvalidJson);
        }
        self.offset += literal.len();
        Ok(())
    }

    fn require(&mut self, byte: u8) -> Result<(), FrameError> {
        if !self.consume(byte) {
            return Err(FrameError::InvalidJson);
        }
        Ok(())
    }

    fn consume(&mut self, byte: u8) -> bool {
        if self.peek() != Some(byte) {
            return false;
        }
        self.offset += 1;
        true
    }

    fn whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.offset += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.offset).copied()
    }
}

struct ObjectKeys(BTreeSet<String>);

impl Drop for ObjectKeys {
    fn drop(&mut self) {
        while let Some(mut key) = self.0.pop_first() {
            key.zeroize();
        }
    }
}
