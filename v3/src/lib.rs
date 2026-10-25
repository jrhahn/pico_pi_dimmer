//! The parts of the dimmer that are pure computation: the MQTT and NINA wire
//! formats, the light state and its Home Assistant announcement. No HAL in
//! here, so all of it is tested on the host.

#![cfg_attr(not(test), no_std)]

use core::fmt;

pub mod mqtt;
pub mod nina;

/// What Home Assistant last asked for.
pub struct Light {
    pub on: bool,
    /// Home Assistant's brightness, 0..=255.
    pub level: u8,
}

impl Light {
    /// Until the retained state arrives from the broker: on at 12.5 % duty,
    /// the level v1 always started at, so the strip works without a network.
    pub const BOOT: Light = Light {
        on: true,
        level: 90,
    };

    /// Applies one of the two retained command topics. Anything else is ignored.
    ///
    /// Home Assistant's default `on_command_type` sends the brightness first and
    /// then `ON`, so a brightness alone never switches the strip on.
    pub fn handle(&mut self, topic: &str, payload: &[u8]) {
        match (topic.rsplit('/').next(), payload) {
            (Some("enabled"), b"ON") => self.on = true,
            (Some("enabled"), b"OFF") => self.on = false,
            (Some("brightness"), level) => {
                if let Some(level) = core::str::from_utf8(level)
                    .ok()
                    .and_then(|s| s.parse().ok())
                {
                    self.level = level;
                }
            }
            _ => {}
        }
    }

    /// PWM compare value for a counter that wraps at `full`.
    ///
    /// Squared, because the eye is roughly logarithmic: a linear duty puts all
    /// the visible change into the bottom of the slider.
    pub fn duty(&self, full: u32) -> u32 {
        if !self.on {
            return 0;
        }
        let level = self.level as u32;
        level * level * full / (255 * 255)
    }
}

/// Retained discovery config for `homeassistant/light/<id>/enabled/config`.
///
/// Home Assistant publishes its commands retained and reads them back as the
/// state, so the broker remembers the light across a power cut and the node
/// never has to publish a state of its own. Topic layout as in
/// rs-smarthome-nodes: `smarthome/<id>/config/{enabled,brightness}`.
pub fn discovery(out: &mut impl fmt::Write, id: &str, name: &str) -> fmt::Result {
    write!(
        out,
        concat!(
            "{{\"~\":\"smarthome/{id}\",\"name\":null,\"uniq_id\":\"{id}_enabled\",",
            "\"cmd_t\":\"~/config/enabled\",\"stat_t\":\"~/config/enabled\",",
            "\"bri_cmd_t\":\"~/config/brightness\",\"bri_stat_t\":\"~/config/brightness\",",
            "\"bri_scl\":255,\"ret\":true,\"avty_t\":\"~/status\",",
            "\"dev\":{{\"ids\":[\"{id}\"],\"name\":\"{name}\",",
            "\"mdl\":\"pico_pi_dimmer v3\",\"sw\":\"{sw}\"}}}}"
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

    #[test]
    fn light_follows_the_command_topics() {
        let mut light = Light::BOOT;
        light.handle("smarthome/d/config/brightness", b"255");
        light.handle("smarthome/d/config/enabled", b"OFF");
        assert!(!light.on);
        assert_eq!(light.level, 255);
        assert_eq!(light.duty(25_000), 0);

        light.handle("smarthome/d/config/enabled", b"ON");
        assert_eq!(light.duty(25_000), 25_000);

        light.handle("smarthome/d/config/brightness", b"0");
        assert_eq!(light.duty(25_000), 0);

        // Junk leaves the state alone.
        light.handle("smarthome/d/config/brightness", b"300");
        light.handle("smarthome/d/config/enabled", b"on");
        light.handle("smarthome/d/other", b"OFF");
        assert!(light.on);
        assert_eq!(light.level, 0);
    }

    #[test]
    fn boot_level_matches_v1() {
        // v1 started at 12.5 % duty.
        let duty = Light::BOOT.duty(10_000);
        assert!((1_200..=1_300).contains(&duty), "{duty}");
    }

    #[test]
    fn discovery_is_valid_json_with_matching_topics() {
        let mut s = String::new();
        discovery(&mut s, "plants", "Pflanzenlicht").unwrap();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["~"], "smarthome/plants");
        assert_eq!(v["uniq_id"], "plants_enabled");
        assert_eq!(v["cmd_t"], v["stat_t"]);
        assert_eq!(v["bri_cmd_t"], v["bri_stat_t"]);
        assert_eq!(v["ret"], true);
        assert!(v["name"].is_null());
        assert_eq!(v["dev"]["ids"][0], "plants");
        assert_eq!(v["dev"]["name"], "Pflanzenlicht");

        // The longest id and name main.rs accepts still fit its 512-byte buffer.
        let mut s = String::new();
        discovery(&mut s, &"i".repeat(32), &"n".repeat(48)).unwrap();
        assert!(s.len() <= 512, "{}", s.len());
    }
}
