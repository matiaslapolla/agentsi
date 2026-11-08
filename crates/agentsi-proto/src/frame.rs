//! Framed mode: `[channel u32 LE][kind u8][len u32 LE][payload]`.
//!
//! The layout copies Herdr's `herdr-core` wire format (Apache-2.0).
//! Channel 0 carries one JSON-RPC message in each DATA frame. The other
//! channels carry terminal streams.

use std::io::{self, Read};

pub const HEADER_LEN: usize = 9;
pub const MAX_PAYLOAD: usize = 16 << 20;
/// The channel of JSON-RPC messages.
pub const CONTROL: u32 = 0;
/// The core sends at most this many bytes on a stream channel before the
/// client returns a CREDIT.
pub const STREAM_WINDOW: usize = 1 << 20;

/// A frame kind. A receiver ignores a kind that it does not know, so a new
/// kind does not break an old peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Kind(pub u8);

impl Kind {
    /// Channel 0: one JSON-RPC message. Other channels: stream bytes.
    pub const DATA: Kind = Kind(0);
    /// u32 LE: the bytes that the receiver consumed.
    pub const CREDIT: Kind = Kind(1);
    /// The channel ends. From the core, the payload is a JSON reason.
    pub const CLOSE: Kind = Kind(2);
    /// u16 LE cols, u16 LE rows. From a pane, two more u16 LE may give the
    /// width and the height of a cell in pixels. From the core, the size of
    /// the terminal, before the bytes at that size.
    pub const RESIZE: Kind = Kind(3);
    /// 1 or 0, from a pane to the core.
    pub const FOCUS: Kind = Kind(4);
    /// Reply bytes that the pane's screen made, not typed keys.
    pub const AUTO: Kind = Kind(5);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub channel: u32,
    pub kind: Kind,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TooLarge(pub usize);

impl std::fmt::Display for TooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "a frame of {} bytes is larger than {MAX_PAYLOAD}",
            self.0
        )
    }
}

impl std::error::Error for TooLarge {}

impl Frame {
    pub fn new(channel: u32, kind: Kind, payload: Vec<u8>) -> Frame {
        Frame {
            channel,
            kind,
            payload,
        }
    }

    /// A JSON-RPC message on channel 0.
    pub fn message(json: &str) -> Frame {
        Frame::new(CONTROL, Kind::DATA, json.as_bytes().to_vec())
    }

    pub fn credit(channel: u32, bytes: u32) -> Frame {
        Frame::new(channel, Kind::CREDIT, bytes.to_le_bytes().to_vec())
    }

    pub fn resize(channel: u32, cols: u16, rows: u16) -> Frame {
        let mut payload = cols.to_le_bytes().to_vec();
        payload.extend_from_slice(&rows.to_le_bytes());
        Frame::new(channel, Kind::RESIZE, payload)
    }

    /// A RESIZE frame with the size of a cell in pixels.
    pub fn resize_px(channel: u32, cols: u16, rows: u16, cell_w: u16, cell_h: u16) -> Frame {
        let mut f = Frame::resize(channel, cols, rows);
        f.payload.extend_from_slice(&cell_w.to_le_bytes());
        f.payload.extend_from_slice(&cell_h.to_le_bytes());
        f
    }

    pub fn focus(channel: u32, focused: bool) -> Frame {
        Frame::new(channel, Kind::FOCUS, vec![u8::from(focused)])
    }

    /// A CLOSE frame with a JSON reason.
    pub fn close(channel: u32, reason: &str) -> Frame {
        Frame::new(channel, Kind::CLOSE, reason.as_bytes().to_vec())
    }

    /// The value of a FOCUS frame, or `None` for a bad payload.
    pub fn as_focus(&self) -> Option<bool> {
        match (self.kind, self.payload.as_slice()) {
            (Kind::FOCUS, [0]) => Some(false),
            (Kind::FOCUS, [1]) => Some(true),
            _ => None,
        }
    }

    /// The cell size in pixels of a RESIZE frame with 8 bytes.
    pub fn as_cell_size(&self) -> Option<(u16, u16)> {
        if self.kind != Kind::RESIZE || self.payload.len() != 8 {
            return None;
        }
        let p = &self.payload;
        Some((
            u16::from_le_bytes([p[4], p[5]]),
            u16::from_le_bytes([p[6], p[7]]),
        ))
    }

    /// The bytes of a CREDIT frame, or `None` for a bad payload.
    pub fn as_credit(&self) -> Option<u32> {
        if self.kind != Kind::CREDIT {
            return None;
        }
        Some(u32::from_le_bytes(self.payload.as_slice().try_into().ok()?))
    }

    /// `(cols, rows)` of a RESIZE frame, or `None` for a bad payload.
    pub fn as_resize(&self) -> Option<(u16, u16)> {
        if self.kind != Kind::RESIZE || !matches!(self.payload.len(), 4 | 8) {
            return None;
        }
        let p = &self.payload;
        Some((
            u16::from_le_bytes([p[0], p[1]]),
            u16::from_le_bytes([p[2], p[3]]),
        ))
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.payload.len());
        out.extend_from_slice(&self.channel.to_le_bytes());
        out.push(self.kind.0);
        out.extend_from_slice(&(self.payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.payload);
        out
    }
}

/// Reads `(channel, kind, payload length)` from a header.
pub fn decode_header(h: &[u8; HEADER_LEN]) -> Result<(u32, Kind, usize), TooLarge> {
    let channel = u32::from_le_bytes([h[0], h[1], h[2], h[3]]);
    let len = u32::from_le_bytes([h[5], h[6], h[7], h[8]]) as usize;
    if len > MAX_PAYLOAD {
        return Err(TooLarge(len));
    }
    Ok((channel, Kind(h[4]), len))
}

/// Reads one frame. `Ok(None)` is a clean end of the stream between frames.
/// A stream that ends inside a frame gives `UnexpectedEof`.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<Frame>> {
    let mut header = [0u8; HEADER_LEN];
    let mut got = 0;
    while got < HEADER_LEN {
        match r.read(&mut header[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the stream ended inside a frame header",
                ));
            }
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    let (channel, kind, len) =
        decode_header(&header).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut payload = vec![0; len];
    r.read_exact(&mut payload)?;
    Ok(Some(Frame {
        channel,
        kind,
        payload,
    }))
}

/// Splits a byte stream into frames, for a client that reads with its own
/// event loop.
#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
}

impl Decoder {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The bytes of a frame that is not whole yet. At the end of the stream,
    /// a value other than 0 means that the stream ended inside a frame.
    pub fn buffered(&self) -> usize {
        self.buf.len()
    }

    /// The next whole frame, or `Ok(None)` until more bytes come.
    pub fn next_frame(&mut self) -> Result<Option<Frame>, TooLarge> {
        let Some(header) = self.buf.first_chunk::<HEADER_LEN>() else {
            return Ok(None);
        };
        let (channel, kind, len) = decode_header(header)?;
        if self.buf.len() < HEADER_LEN + len {
            return Ok(None);
        }
        let payload = self.buf[HEADER_LEN..HEADER_LEN + len].to_vec();
        self.buf.drain(..HEADER_LEN + len);
        Ok(Some(Frame {
            channel,
            kind,
            payload,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_round_trips_through_the_reader_and_the_decoder() {
        let frames = [
            Frame::message(r#"{"jsonrpc":"2.0","id":1,"method":"core.hello"}"#),
            Frame::credit(3, 65536),
            Frame::resize(3, 120, 40),
            Frame::focus(3, true),
            Frame::new(7, Kind(42), b"unknown kinds pass".to_vec()),
        ];
        let bytes: Vec<u8> = frames.iter().flat_map(Frame::encode).collect();

        let mut reader = bytes.as_slice();
        for f in &frames {
            assert_eq!(read_frame(&mut reader).unwrap().as_ref(), Some(f));
        }
        assert_eq!(read_frame(&mut reader).unwrap(), None);

        let mut decoder = Decoder::default();
        let mut got = Vec::new();
        for byte in &bytes {
            decoder.push(std::slice::from_ref(byte));
            while let Some(f) = decoder.next_frame().unwrap() {
                got.push(f);
            }
        }
        assert_eq!(got, frames);
        assert_eq!(got[1].as_credit(), Some(65536));
        assert_eq!(got[2].as_resize(), Some((120, 40)));
        assert_eq!(got[2].as_cell_size(), None);
        assert_eq!(got[3].as_focus(), Some(true));
        let px = Frame::resize_px(3, 120, 40, 9, 18);
        assert_eq!(px.as_resize(), Some((120, 40)));
        assert_eq!(px.as_cell_size(), Some((9, 18)));
        assert_eq!(Frame::new(3, Kind::RESIZE, vec![1, 0, 2]).as_resize(), None);
        assert_eq!(Frame::new(3, Kind::FOCUS, vec![2]).as_focus(), None);
    }

    #[test]
    fn the_header_layout_is_little_endian_with_the_kind_in_byte_four() {
        let bytes = Frame::new(0x0102_0304, Kind::CLOSE, b"ab".to_vec()).encode();
        assert_eq!(bytes, [4, 3, 2, 1, 2, 2, 0, 0, 0, b'a', b'b']);
    }

    #[test]
    fn a_stream_that_ends_inside_a_header_is_an_error() {
        let bytes = Frame::message("{}").encode();
        let err = read_frame(&mut &bytes[..5]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        assert_eq!(read_frame(&mut &bytes[..0]).unwrap(), None);
    }

    #[test]
    fn a_payload_over_16_mib_is_refused() {
        let mut header = [0u8; HEADER_LEN];
        header[5..9].copy_from_slice(&((MAX_PAYLOAD + 1) as u32).to_le_bytes());
        assert_eq!(decode_header(&header), Err(TooLarge(MAX_PAYLOAD + 1)));
        let mut decoder = Decoder::default();
        decoder.push(&header);
        assert!(decoder.next_frame().is_err());
    }
}
