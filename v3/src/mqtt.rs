//! The slice of MQTT 3.1.1 a dimmer needs: connect with a last will, publish
//! and subscribe at QoS 0, ping, and reading what the broker sends back.
//!
//! Encoders write into a caller's buffer and return the length, or `None` if
//! it does not fit.

use crate::Cursor;

pub const PINGREQ: [u8; 2] = [0xC0, 0x00];

pub enum Packet<'a> {
    /// CONNACK with its return code, 0 = accepted.
    ConnAck(u8),
    Publish {
        topic: &'a str,
        payload: &'a [u8],
    },
    /// SUBACK, PINGRESP and anything else that needs no answer.
    Other,
}

/// CONNECT with a clean session and a retained last will.
pub fn connect(
    buf: &mut [u8],
    client_id: &str,
    user: Option<&str>,
    password: Option<&str>,
    will_topic: &str,
    will: &[u8],
    keep_alive_secs: u16,
) -> Option<usize> {
    let field = |s: Option<&str>| s.map_or(0, |s| 2 + s.len());
    let len = 10
        + 2
        + client_id.len()
        + 2
        + will_topic.len()
        + 2
        + will.len()
        + field(user)
        + field(password);
    // clean session | will | will retain
    let mut flags = 0x02 | 0x04 | 0x20;
    if user.is_some() {
        flags |= 0x80;
    }
    if password.is_some() {
        flags |= 0x40;
    }

    let mut w = Cursor::new(buf);
    header(&mut w, 0x10, len)?;
    string(&mut w, b"MQTT")?;
    w.put(&[4, flags])?;
    w.put(&keep_alive_secs.to_be_bytes())?;
    string(&mut w, client_id.as_bytes())?;
    string(&mut w, will_topic.as_bytes())?;
    string(&mut w, will)?;
    for s in [user, password].into_iter().flatten() {
        string(&mut w, s.as_bytes())?;
    }
    Some(w.at)
}

pub fn publish(buf: &mut [u8], topic: &str, payload: &[u8], retain: bool) -> Option<usize> {
    let mut w = Cursor::new(buf);
    header(&mut w, 0x30 | retain as u8, 2 + topic.len() + payload.len())?;
    string(&mut w, topic.as_bytes())?;
    w.put(payload)?;
    Some(w.at)
}

/// SUBSCRIBE to one topic filter at QoS 0.
pub fn subscribe(buf: &mut [u8], packet_id: u16, topic: &str) -> Option<usize> {
    let mut w = Cursor::new(buf);
    header(&mut w, 0x82, 2 + 2 + topic.len() + 1)?;
    w.put(&packet_id.to_be_bytes())?;
    string(&mut w, topic.as_bytes())?;
    w.put(&[0])?;
    Some(w.at)
}

/// The first complete packet in `buf` and how many bytes it took, or `None`
/// while it is still incomplete.
pub fn parse(buf: &[u8]) -> Option<(Packet<'_>, usize)> {
    let (&kind, rest) = buf.split_first()?;
    let (mut len, mut i) = (0usize, 0);
    loop {
        let byte = *rest.get(i)?;
        len |= ((byte & 0x7F) as usize) << (7 * i);
        i += 1;
        if byte & 0x80 == 0 {
            break;
        }
        if i == 4 {
            // A malformed length never completes; the caller's full buffer
            // ends the connection.
            return None;
        }
    }
    let body = rest.get(i..i + len)?;
    let packet = match kind >> 4 {
        2 => body
            .get(1)
            .map_or(Packet::Other, |&code| Packet::ConnAck(code)),
        3 => publish_body(kind, body).unwrap_or(Packet::Other),
        _ => Packet::Other,
    };
    Some((packet, 1 + i + len))
}

fn publish_body(kind: u8, body: &[u8]) -> Option<Packet<'_>> {
    let len = u16::from_be_bytes([*body.first()?, *body.get(1)?]) as usize;
    let topic = core::str::from_utf8(body.get(2..2 + len)?).ok()?;
    // QoS 1 and 2 carry a packet id between topic and payload.
    let id = if kind & 0x06 != 0 { 2 } else { 0 };
    Some(Packet::Publish {
        topic,
        payload: body.get(2 + len + id..)?,
    })
}

fn header(w: &mut Cursor, kind: u8, mut len: usize) -> Option<()> {
    w.put(&[kind])?;
    loop {
        let byte = (len % 128) as u8;
        len /= 128;
        w.put(&[if len > 0 { byte | 0x80 } else { byte }])?;
        if len == 0 {
            return Some(());
        }
    }
}

fn string(w: &mut Cursor, s: &[u8]) -> Option<()> {
    w.put(&u16::try_from(s.len()).ok()?.to_be_bytes())?;
    w.put(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_layout() {
        let mut buf = [0; 128];
        let n = connect(
            &mut buf,
            "d",
            Some("u"),
            Some("p"),
            "s/d/status",
            b"offline",
            30,
        )
        .unwrap();
        let p = &buf[..n];
        assert_eq!(p[0], 0x10);
        assert_eq!(p[1] as usize, n - 2);
        assert_eq!(&p[2..8], b"\x00\x04MQTT");
        assert_eq!(p[8], 4);
        assert_eq!(p[9], 0x80 | 0x40 | 0x20 | 0x04 | 0x02);
        assert_eq!(&p[10..12], &30u16.to_be_bytes());
        assert_eq!(&p[12..15], b"\x00\x01d");
        assert!(p.ends_with(b"\x00\x01u\x00\x01p"));

        let n = connect(&mut buf, "d", None, None, "t", b"x", 30).unwrap();
        assert_eq!(buf[9], 0x20 | 0x04 | 0x02);
        assert_eq!(n, 2 + 10 + 3 + 3 + 3);
    }

    #[test]
    fn publish_round_trips_through_parse() {
        let mut buf = [0; 512];
        let payload = [7u8; 300]; // needs a two-byte remaining length
        let n = publish(&mut buf, "a/b", &payload, true).unwrap();
        assert_eq!(buf[0], 0x31);
        assert_eq!(&buf[1..3], &[(305 % 128) as u8 | 0x80, (305 / 128) as u8]);

        let Some((Packet::Publish { topic, payload: p }, used)) = parse(&buf[..n]) else {
            panic!("no publish");
        };
        assert_eq!((topic, p, used), ("a/b", &payload[..], n));
        assert!(parse(&buf[..n - 1]).is_none(), "incomplete");
    }

    #[test]
    fn parses_connack_qos1_publish_and_others() {
        let Some((Packet::ConnAck(5), 4)) = parse(&[0x20, 2, 0, 5, 0xD0]) else {
            panic!()
        };
        let Some((Packet::Other, 2)) = parse(&[0xD0, 0]) else {
            panic!()
        };
        // QoS 1: topic "t", packet id 0x0102, payload "ON"
        let Some((
            Packet::Publish {
                topic: "t",
                payload: b"ON",
            },
            9,
        )) = parse(&[0x32, 7, 0, 1, b't', 1, 2, b'O', b'N'])
        else {
            panic!()
        };
        assert!(parse(&[]).is_none());
        assert!(parse(&[0x30, 0x80, 0x80, 0x80, 0x80, 1]).is_none());
    }

    #[test]
    fn subscribe_layout() {
        let mut buf = [0; 32];
        let n = subscribe(&mut buf, 1, "a/b").unwrap();
        assert_eq!(&buf[..n], b"\x82\x08\x00\x01\x00\x03a/b\x00");
    }

    #[test]
    fn too_small_buffers_fail() {
        assert!(publish(&mut [0; 4], "a/b", b"x", false).is_none());
    }
}
