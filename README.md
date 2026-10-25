# Dimmer for LED strips

A Raspberry Pi Pico with the [Pimoroni Pico Wireless Pack](https://shop.pimoroni.com/products/pico-wireless-pack)
dims a 24 V LED strip through a MOSFET module and shows up in Home Assistant as a dimmable light, over MQTT
discovery. Firmware in async `no_std` Rust ([Embassy](https://embassy.dev)), toolchain via a Nix flake, and a
printable enclosure.

Version 3, tested on the hardware below with Home Assistant. The MicroPython REST versions (v1 with the Wireless
Pack, v2 for the Pico W) are in the git history.

## Hardware

- Raspberry Pi Pico + Pimoroni Pico Wireless Pack. The pack's ESP32 runs the NINA firmware it ships with
  (tested: 1.7.3); the Pico talks to it over SPI.
- Dual-MOSFET trigger module, 5–36 V / 15 A, PWM up to 20 kHz
- 24 V → 5 V buck converter (Mini-360 or MP1584)
- 24 V LED strip and power supply

| From | To |
| --- | --- |
| PSU +24 V | MOSFET VIN+, buck IN+ |
| PSU GND | MOSFET VIN−, buck IN− |
| buck OUT+ (**set to 5.0 V before connecting**) | Pico VSYS (pin 39) |
| buck OUT− | Pico GND (pin 38) |
| Pico GP14 (pin 19) | MOSFET trigger/PWM + |
| Pico GND (pin 18) | MOSFET trigger/PWM − |
| MOSFET OUT+ / OUT− | LED strip + / − |

VSYS takes at most 5.5 V. To have USB plugged in while the 24 V supply is on, put a Schottky diode
(e.g. 1N5819) in the buck's OUT+ line so USB cannot feed the converter backwards.

## Build and flash

```bash
cp .env.example .env     # Wi-Fi and broker; direnv loads it
direnv allow             # or: nix develop
cargo run --release      # Pico in BOOTSEL mode: hold the button while plugging it in
```

The settings in `.env` are compiled into the firmware. After changing one, flash again.

| Variable | Meaning |
| --- | --- |
| `SSID`, `PASSWORD` | Wi-Fi, 2.4 GHz only |
| `MQTT_BROKER` | broker host name or IPv4 address, port 1883 |
| `MQTT_USER`, `MQTT_PASSWORD` | optional, leave unset for an anonymous broker |
| `NODE_ID` | topic level and MQTT client id, one per dimmer; default `dimmer`, 1–32 of `a-z 0-9 - _` |
| `NODE_NAME` | device name in Home Assistant; default `Dimmer`, at most 48 bytes, no `"` or `\` |

Once the dimmer firmware runs, no button is needed any more: writing `bootsel` to its USB serial port reboots it
into the bootloader.

```bash
echo bootsel > /dev/serial/by-id/usb-Embassy_USB-serial_logger-if00
cargo run --release
```

The same port carries the log (`screen /dev/serial/by-id/usb-Embassy_USB-serial_logger-if00`).

Unit tests (MQTT and NINA framing, light, discovery payload) run on the host:

```bash
cargo test --lib --target x86_64-unknown-linux-gnu
```

## Home Assistant

Needs the MQTT integration with discovery on (the default). The dimmer announces itself as a device named
`NODE_NAME` with one light entity: on/off and brightness. No YAML. It shows up under Settings → Devices & services
→ MQTT.

| Topic | Payload |
| --- | --- |
| `homeassistant/light/<NODE_ID>/enabled/config` | discovery, retained |
| `smarthome/<NODE_ID>/config/enabled` | `ON` / `OFF`, retained |
| `smarthome/<NODE_ID>/config/brightness` | `0`–`255`, retained |
| `smarthome/<NODE_ID>/status` | `online` / `offline` (last will), retained |

Home Assistant publishes its commands retained and reads them back as the state, so the broker remembers the
light across a power cut and the dimmer restores it on connect. Until then the strip runs at 12.5 % duty, as v1
always did. Brightness maps to duty squared, so the slider looks even to the eye.

If Wi-Fi or the broker goes away, the Pico resets the ESP32 and connects again. After the broker drops the
connection, the dimmer is back within about 20 seconds. A hardware watchdog (8 s) and reboot-on-panic cover
everything else.

Two quirks of nina-fw 1.7.3 shape the code. Its host-name connect is broken, so the dimmer resolves
`MQTT_BROKER` first and connects by address. And it reports a connection the broker closed as still open, so the
dimmer pings every 5 s and starts over after three unanswered pings.

## Enclosure v1

80 × 67 × 45 mm, the lid on top. Both cables (24 V in, LED strip out) leave through slots in the same end face.
The slots are open at the rim, so cables with their plugs on go in from above; the lid closes them. Two parts,
both print without support: the body floor down, the lid plate down. STL and STEP are in
[`cad/export`](cad/export).

- Four M2.5 × 6 screws (pan or socket head) hold the lid. They go into M2.5 heat-set inserts, which you melt
  into the bosses in the body's corners. The holes are 4.0 mm wide and 6.7 mm deep, for ruthex M2.5×5.7 or CNC
  Kitchen M2.5×4. For other inserts, set `INSERT_D` and `INSERT_L` in [`cad/enclosure.py`](cad/enclosure.py).
- Everything lies flat on the floor, on double-sided foam tape: the Pico + pack stack, the MOSFET module with
  its terminals up, and the buck converter. Keep the corners free for the bosses.
- Cinch a cable tie round each cable inside the wall as strain relief. Set `CABLE_D` to your thicker cable +
  0.5 mm.

```bash
nix develop .#cad
python cad/enclosure.py      # rewrites cad/export/
```
