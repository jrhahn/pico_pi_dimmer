//! The parts of the dimmer that are pure computation: the MQTT wire format,
//! the light state and its Home Assistant announcement. No HAL in here, so
//! all of it is tested on the host.

#![cfg_attr(not(test), no_std)]

use core::fmt;

pub mod mqtt;

/// What Home Assistant last asked for.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Light {
    pub on: bool,
    /// Home Assistant's brightness, 1..=255; kept while off.
    pub level: u8,
}

/// One message on the command topic, Home Assistant's JSON light schema:
/// `{"state":"ON","brightness":180,"transition":1800}`, every key optional.
#[derive(Default, PartialEq, Debug)]
pub struct Command {
    pub on: Option<bool>,
    pub level: Option<u8>,
    /// How long the change should take.
    pub fade_ms: u32,
}

impl Command {
    /// None for anything that is not a JSON object.
    pub fn parse(payload: &[u8]) -> Option<Command> {
        let s = core::str::from_utf8(payload).ok()?.trim();
        if !s.starts_with('{') {
            return None;
        }
        let on = match json_value(s, "state") {
            Some("\"ON\"") => Some(true),
            Some("\"OFF\"") => Some(false),
            _ => None,
        };
        let level = json_value(s, "brightness").and_then(|v| v.parse().ok());
        let fade_ms = json_value(s, "transition").map_or(0, seconds_to_ms);
        Some(Command { on, level, fade_ms })
    }
}

impl Light {
    /// Until the retained state arrives from the broker: on at 12.5 % duty,
    /// the level v1 always started at, so the strip works without a network.
    pub const BOOT: Light = Light {
        on: true,
        level: 90,
    };

    pub fn apply(&mut self, command: &Command) {
        if let Some(level) = command.level.filter(|&l| l > 0) {
            self.level = level;
        }
        if let Some(on) = command.on {
            self.on = on;
        }
    }

    /// Where the strip should end up, in 1/256 steps of Home Assistant's
    /// brightness, so a fade passes smoothly between the 255 levels.
    pub fn target(&self) -> u32 {
        if self.on { self.level as u32 * 256 } else { 0 }
    }

    /// The retained state message, in the same JSON schema.
    pub fn state(&self, out: &mut impl fmt::Write) -> fmt::Result {
        let state = if self.on { "ON" } else { "OFF" };
        write!(
            out,
            "{{\"state\":\"{state}\",\"brightness\":{},\"color_mode\":\"brightness\"}}",
            self.level
        )
    }
}

/// PWM compare value for a fine level (see `Light::target`) and a counter
/// that wraps at `full`.
///
/// Squared, because the eye is roughly logarithmic: a linear duty puts all
/// the visible change into the bottom of the slider.
pub fn duty(fine: u32, full: u32) -> u32 {
    const MAX: u64 = 255 * 256;
    let fine = fine.min(MAX as u32) as u64;
    (fine * fine * full as u64 / (MAX * MAX)) as u32
}

/// A linear ramp between two fine levels.
#[derive(Clone, Copy)]
pub struct Fade {
    pub from: u32,
    pub to: u32,
    pub ms: u32,
}

impl Fade {
    pub fn at(&self, elapsed_ms: u32) -> u32 {
        if elapsed_ms >= self.ms {
            return self.to;
        }
        let (from, to) = (self.from as i64, self.to as i64);
        (from + (to - from) * elapsed_ms as i64 / self.ms as i64) as u32
    }
}

/// The raw text of `key`'s value in a flat JSON object: a quoted string with
/// its quotes, or a number. Enough for Home Assistant's light commands.
fn json_value<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let mut rest = json;
    loop {
        let at = rest.find('"')?;
        rest = &rest[at + 1..];
        let close = rest.find('"')?;
        let name = &rest[..close];
        rest = rest[close + 1..].trim_start();
        let Some(after) = rest.strip_prefix(':') else {
            continue; // a string value, not a key
        };
        let value = after.trim_start();
        let len = if let Some(v) = value.strip_prefix('"') {
            v.find('"')? + 2
        } else {
            value.find([',', '}']).unwrap_or(value.len())
        };
        if name == key {
            return Some(value[..len].trim_end());
        }
        rest = &value[len..];
    }
}

/// `"1800"` or `"2.5"` seconds to milliseconds; at most a day.
fn seconds_to_ms(s: &str) -> u32 {
    let (whole, frac) = s.split_once('.').unwrap_or((s, ""));
    let whole: u32 = whole.parse().unwrap_or(0);
    let frac = frac.bytes().chain(core::iter::repeat(b'0')).take(3);
    let ms = frac.fold(0, |ms, d| ms * 10 + d.wrapping_sub(b'0').min(9) as u32);
    whole.min(86_400) * 1000 + ms
}

/// An update order on `smarthome/<id>/ota`:
/// `http://host[:port]/path <size> <signature>`, the signature as 128 hex
/// digits: ed25519 over the image's SHA-512, as embassy-boot checks it.
#[derive(PartialEq, Debug)]
pub struct Ota<'a> {
    pub host: &'a str,
    pub port: u16,
    pub path: &'a str,
    pub size: u32,
    pub signature: [u8; 64],
}

impl<'a> Ota<'a> {
    pub fn parse(payload: &'a [u8]) -> Option<Self> {
        let mut parts = core::str::from_utf8(payload).ok()?.split_ascii_whitespace();
        let url = parts.next()?.strip_prefix("http://")?;
        let size = parts.next()?.parse().ok().filter(|&n| n > 0)?;
        let signature = hex(parts.next()?)?;
        if parts.next().is_some() {
            return None;
        }
        let (authority, path) = url.split_at(url.find('/')?);
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (host, port.parse().ok()?),
            None => (authority, 80),
        };
        if host.is_empty() {
            return None;
        }
        Some(Ota {
            host,
            port,
            path,
            size,
            signature,
        })
    }

    /// The GET request; HTTP/1.0, so the server closes after the body.
    pub fn request(&self, out: &mut impl fmt::Write) -> fmt::Result {
        write!(
            out,
            "GET {} HTTP/1.0\r\nHost: {}\r\n\r\n",
            self.path, self.host
        )
    }
}

/// Where the body starts in an HTTP response, once its header is complete.
/// An error for any status but 200.
pub fn http_body(response: &[u8]) -> Result<Option<usize>, &'static str> {
    let Some(end) = response.windows(4).position(|w| w == b"\r\n\r\n") else {
        return Ok(None);
    };
    match response.split(|&b| b == b' ').nth(1) {
        Some(b"200") => Ok(Some(end + 4)),
        _ => Err("update host answered with an error"),
    }
}

/// Exactly `2 * N` hex digits to bytes. `const`, so the build can check the
/// public key.
pub const fn hex<const N: usize>(s: &str) -> Option<[u8; N]> {
    const fn digit(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }
    let s = s.as_bytes();
    if s.len() != 2 * N {
        return None;
    }
    let mut out = [0; N];
    let mut i = 0;
    while i < N {
        match (digit(s[2 * i]), digit(s[2 * i + 1])) {
            (Some(hi), Some(lo)) => out[i] = hi << 4 | lo,
            _ => return None,
        }
        i += 1;
    }
    Some(out)
}

/// Retained discovery config for `homeassistant/light/<id>/enabled/config`.
///
/// JSON schema, so Home Assistant passes `transition` on and the dimmer fades
/// by itself. Commands come on `~/set`; the dimmer publishes `~/state`
/// retained and reads it back after a restart.
pub fn discovery(out: &mut impl fmt::Write, id: &str, name: &str) -> fmt::Result {
    write!(
        out,
        concat!(
            "{{\"~\":\"smarthome/{id}\",\"name\":null,\"uniq_id\":\"{id}_enabled\",",
            "\"schema\":\"json\",\"cmd_t\":\"~/set\",\"stat_t\":\"~/state\",",
            "\"sup_clrm\":[\"brightness\"],\"bri_scl\":255,\"avty_t\":\"~/status\",",
            "\"dev\":{{\"ids\":[\"{id}\"],\"name\":\"{name}\",",
            "\"mdl\":\"pico_pi_dimmer v4\",\"sw\":\"{sw}\"}}}}"
        ),
        id = id,
        name = name,
        sw = env!("CARGO_PKG_VERSION"),
    )
}

/// Bounded writer over a byte buffer; every `put` fails instead of panicking
/// when the buffer is full.
pub(crate) struct Cursor<'a> {
    buf: &'a mut [u8],
    pub at: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(buf: &'a mut [u8]) -> Self {
        Cursor { buf, at: 0 }
    }

    pub fn put(&mut self, data: &[u8]) -> Option<()> {
        let end = self.at + data.len();
        self.buf.get_mut(self.at..end)?.copy_from_slice(data);
        self.at = end;
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(s: &str) -> Command {
        Command::parse(s.as_bytes()).unwrap()
    }

    #[test]
    fn parses_home_assistant_commands() {
        assert_eq!(
            cmd(r#"{"state":"ON","brightness":180,"transition":1800}"#),
            Command {
                on: Some(true),
                level: Some(180),
                fade_ms: 1_800_000
            }
        );
        assert_eq!(
            cmd(r#"{ "state" : "OFF", "transition": 2.5 }"#),
            Command {
                on: Some(false),
                level: None,
                fade_ms: 2_500
            }
        );
        assert_eq!(cmd(r#"{"state":"ON"}"#).fade_ms, 0);
        // Other keys, even ones holding our key names as values, are skipped.
        let c = cmd(r#"{"color_mode":"state","effect":"x","brightness":7}"#);
        assert_eq!((c.on, c.level), (None, Some(7)));
        assert_eq!(cmd(r#"{"brightness":300}"#).level, None);
        assert_eq!(cmd(r#"{"transition":999999999}"#).fade_ms, 86_400_000);
        assert!(Command::parse(b"ON").is_none());
    }

    #[test]
    fn light_follows_commands() {
        let mut light = Light::BOOT;
        light.apply(&cmd(r#"{"state":"OFF"}"#));
        assert_eq!(light.target(), 0);
        light.apply(&cmd(r#"{"state":"ON","brightness":255}"#));
        assert_eq!(duty(light.target(), 25_000), 25_000);
        // Brightness 0 is not a level to come back to.
        light.apply(&cmd(r#"{"brightness":0}"#));
        assert_eq!(light.level, 255);
    }

    #[test]
    fn state_reads_back_as_a_command() {
        let light = Light {
            on: false,
            level: 42,
        };
        let mut s = String::new();
        light.state(&mut s).unwrap();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(
            (v["state"].as_str(), v["brightness"].as_u64()),
            (Some("OFF"), Some(42))
        );
        let mut back = Light::BOOT;
        back.apply(&cmd(&s));
        assert_eq!(back, light);
    }

    #[test]
    fn boot_level_matches_v1() {
        // v1 started at 12.5 % duty.
        let duty = duty(Light::BOOT.target(), 10_000);
        assert!((1_200..=1_300).contains(&duty), "{duty}");
    }

    #[test]
    fn fade_ramps_and_lands() {
        let f = Fade {
            from: 0,
            to: 1000,
            ms: 100,
        };
        assert_eq!(
            (f.at(0), f.at(50), f.at(100), f.at(5000)),
            (0, 500, 1000, 1000)
        );
        let down = Fade {
            from: 1000,
            to: 0,
            ms: 100,
        };
        assert_eq!(down.at(25), 750);
        assert_eq!(
            Fade {
                from: 3,
                to: 9,
                ms: 0
            }
            .at(0),
            9
        );
    }

    #[test]
    fn discovery_is_valid_json_with_matching_topics() {
        let mut s = String::new();
        discovery(&mut s, "plants", "Pflanzenlicht").unwrap();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["~"], "smarthome/plants");
        assert_eq!(v["uniq_id"], "plants_enabled");
        assert_eq!(v["schema"], "json");
        assert_eq!(
            (v["cmd_t"].as_str(), v["stat_t"].as_str()),
            (Some("~/set"), Some("~/state"))
        );
        assert!(v["name"].is_null());
        assert_eq!(v["dev"]["name"], "Pflanzenlicht");

        // The longest id and name main.rs accepts still fit its 512-byte buffer.
        let mut s = String::new();
        discovery(&mut s, &"i".repeat(32), &"n".repeat(48)).unwrap();
        assert!(s.len() <= 512, "{}", s.len());
    }

    #[test]
    fn parses_ota_orders() {
        let sig = "ab".repeat(64);
        let order = format!("http://192.168.1.10:8000/v4.bin 335280 {sig}");
        let o = Ota::parse(order.as_bytes()).unwrap();
        assert_eq!(
            (o.host, o.port, o.path, o.size),
            ("192.168.1.10", 8000, "/v4.bin", 335280)
        );
        assert_eq!(o.signature, [0xab; 64]);
        let mut r = String::new();
        o.request(&mut r).unwrap();
        assert_eq!(r, "GET /v4.bin HTTP/1.0\r\nHost: 192.168.1.10\r\n\r\n");
        assert_eq!(
            Ota::parse(format!("http://pc/a 1 {sig}").as_bytes())
                .unwrap()
                .port,
            80
        );

        for bad in [
            format!("https://pc/a 1 {sig}"),
            format!("http://pc 1 {sig}"),
            format!("http://:80/a 1 {sig}"),
            format!("http://pc/a 0 {sig}"),
            format!("http://pc/a 1 {}", &sig[1..]),
            format!("http://pc/a 1 {sig} extra"),
        ] {
            assert!(Ota::parse(bad.as_bytes()).is_none(), "{bad}");
        }
    }

    #[test]
    fn finds_the_http_body() {
        assert_eq!(http_body(b"HTTP/1.0 200 OK\r\nServer: x\r\n"), Ok(None));
        assert_eq!(http_body(b"HTTP/1.0 200 OK\r\n\r\nBODY"), Ok(Some(19)));
        assert!(http_body(b"HTTP/1.0 404 Not Found\r\n\r\n").is_err());
    }

    #[test]
    fn hex_decodes_exactly() {
        assert_eq!(hex::<2>("0aFf"), Some([0x0a, 0xff]));
        assert_eq!(hex::<2>("0aF"), None);
        assert_eq!(hex::<1>("g0"), None);
    }
}
