//! Command framing for the ESP32 on the Pimoroni Wireless Pack, which runs
//! the NINA firmware (Arduino/Adafruit `nina-fw`) as an SPI slave.
//!
//! A command is `E0 cmd n`, then each parameter as length + bytes, then `EE`,
//! padded with filler to a multiple of four bytes for the ESP32's SPI DMA.
//! Parameters carry a one-byte length, except for the socket data commands,
//! which use two (big-endian). The reply has the same shape with `cmd | 0x80`.
//! Layouts checked against `CommandHandler.cpp` in nina-fw.

use crate::Cursor;

pub const START: u8 = 0xE0;
pub const END: u8 = 0xEE;
pub const ERROR: u8 = 0xEF;
pub const REPLY: u8 = 0x80;

pub const SET_PASSPHRASE: u8 = 0x11;
pub const GET_CONN_STATUS: u8 = 0x20;
pub const START_CLIENT_TCP: u8 = 0x2D;
pub const REQ_HOST_BY_NAME: u8 = 0x34;
pub const GET_HOST_BY_NAME: u8 = 0x35;
pub const GET_FW_VERSION: u8 = 0x37;
pub const GET_SOCKET: u8 = 0x3F;
pub const SEND_DATA_TCP: u8 = 0x44;
pub const GET_DATABUF_TCP: u8 = 0x45;

/// `GET_CONN_STATUS`: associated and holding an address.
pub const WL_CONNECTED: u8 = 3;
/// `GET_SOCKET` when every socket is taken.
pub const NO_SOCKET: u8 = 255;

pub enum Param<'a> {
    /// One-byte length prefix.
    Short(&'a [u8]),
    /// Two-byte length prefix, for `SEND_DATA_TCP` and `GET_DATABUF_TCP`.
    Long(&'a [u8]),
}

/// Writes the padded command into `out` and returns its length.
pub fn frame(cmd: u8, params: &[Param], out: &mut [u8]) -> Option<usize> {
    let mut w = Cursor::new(out);
    w.put(&[START, cmd & !REPLY, u8::try_from(params.len()).ok()?])?;
    for param in params {
        let data = match param {
            Param::Short(data) => {
                w.put(&[u8::try_from(data.len()).ok()?])?;
                data
            }
            Param::Long(data) => {
                w.put(&u16::try_from(data.len()).ok()?.to_be_bytes())?;
                data
            }
        };
        w.put(data)?;
    }
    w.put(&[END])?;
    while !w.at.is_multiple_of(4) {
        w.put(&[0xFF])?;
    }
    Some(w.at)
}

#[cfg(test)]
mod tests {
    use super::Param::{Long, Short};
    use super::*;

    fn framed(cmd: u8, params: &[Param]) -> Vec<u8> {
        let mut buf = [0; 64];
        let n = frame(cmd, params, &mut buf).unwrap();
        assert_eq!(n % 4, 0);
        buf[..n].to_vec()
    }

    #[test]
    fn no_parameters() {
        assert_eq!(framed(GET_CONN_STATUS, &[]), [0xE0, 0x20, 0, 0xEE]);
    }

    #[test]
    fn short_parameters_are_padded() {
        assert_eq!(
            framed(SET_PASSPHRASE, &[Short(b"ab"), Short(b"cdef")]),
            [
                0xE0, 0x11, 2, 2, b'a', b'b', 4, b'c', b'd', b'e', b'f', 0xEE
            ]
        );
        assert_eq!(
            framed(REQ_HOST_BY_NAME, &[Short(b"h")]),
            [0xE0, 0x34, 1, 1, b'h', 0xEE, 0xFF, 0xFF]
        );
    }

    #[test]
    fn start_client_matches_nina_fw_offsets() {
        // startClientTcp, four-parameter form: ip at [4], port (network
        // order) at [9], socket at [12], type at [14].
        let c = framed(
            START_CLIENT_TCP,
            &[
                Short(&[192, 168, 1, 67]),
                Short(&1883u16.to_be_bytes()),
                Short(&[2]),
                Short(&[0]),
            ],
        );
        assert_eq!(c[2], 4);
        assert_eq!(&c[4..8], &[192, 168, 1, 67]);
        assert_eq!(u16::from_be_bytes([c[9], c[10]]), 1883);
        assert_eq!((c[12], c[14], c[15]), (2, 0, END));
    }

    #[test]
    fn socket_data_commands_match_nina_fw_offsets() {
        // getDataBufTcp: socket at [5], requested length little-endian at [8].
        let c = framed(GET_DATABUF_TCP, &[Long(&[1]), Long(&512u16.to_le_bytes())]);
        assert_eq!(
            (c[5], u16::from_le_bytes([c[8], c[9]]), c[10]),
            (1, 512, END)
        );

        // sendDataTcp: socket at [5], length big-endian at [6], data from [8].
        let c = framed(SEND_DATA_TCP, &[Long(&[1]), Long(b"hello")]);
        assert_eq!((c[5], u16::from_be_bytes([c[6], c[7]])), (1, 5));
        assert_eq!(&c[8..13], b"hello");
        assert_eq!(c[13], END);
    }

    #[test]
    fn oversized_parameters_fail() {
        let mut buf = [0; 600];
        assert!(frame(SET_PASSPHRASE, &[Short(&[0; 256])], &mut buf).is_none());
        assert!(frame(SET_PASSPHRASE, &[Short(&[0; 8])], &mut [0; 8]).is_none());
    }
}
