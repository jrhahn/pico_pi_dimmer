# Dimmer for LED strips, Pico W

A Raspberry Pi Pico W dims a 24 V LED strip through a MOSFET module and shows up in Home Assistant as a dimmable
light, over MQTT discovery. Firmware in async `no_std` Rust ([Embassy](https://embassy.dev)) on the Pico W's own
CYW43439 radio, toolchain via a Nix flake.

Version 4, tested on a Pico W with Home Assistant's broker. It is [v3](../v3) without the Wireless Pack, on the
board [v2](../v2) runs MicroPython on.

## Hardware

- Raspberry Pi Pico W
- Dual-MOSFET trigger module, 5–36 V / 15 A, PWM up to 20 kHz
- 24 V → 5 V buck converter (Mini-360 or MP1584)
- 24 V LED strip and power supply

| From | To |
| --- | --- |
| PSU +24 V | MOSFET VIN+, buck IN+ |
| PSU GND | MOSFET VIN−, buck IN− |
| buck OUT+ (**set to 5.0 V before connecting**) | Pico W VSYS (pin 39) |
| buck OUT− | Pico W GND (pin 38) |
| Pico W GP14 (pin 19) | MOSFET trigger/PWM + |
| Pico W GND (pin 18) | MOSFET trigger/PWM − |
| MOSFET OUT+ / OUT− | LED strip + / − |

VSYS takes at most 5.5 V. To have USB plugged in while the 24 V supply is on, put a Schottky diode
(e.g. 1N5819) in the buck's OUT+ line so USB cannot feed the converter backwards.

The [v3 enclosure](../v3/cad) fits; the Pico W is smaller than v3's Pico + pack stack.

## Build and flash

```bash
cp .env.example .env     # Wi-Fi and broker; direnv loads it
direnv allow             # or: nix develop
python ota.py keygen     # once, for both v3 and v4: the key that signs updates
python ota.py usb        # Pico W in BOOTSEL mode: hold the button while plugging it in
```

`keygen` prints `OTA_PUBLIC_KEY`; put it in `.env` before `usb`. `usb` flashes the bootloader and the firmware in
one go. Needed once per board; after that, updates go over Wi-Fi (see below) or, on USB, with `cargo run --release`.

The settings in `.env` are compiled into the firmware. After changing one, flash again.

| Variable | Meaning |
| --- | --- |
| `SSID`, `PASSWORD` | Wi-Fi, 2.4 GHz only |
| `MQTT_BROKER` | broker host name or IPv4 address, port 1883 |
| `MQTT_USER`, `MQTT_PASSWORD` | optional, leave unset for an anonymous broker |
| `NODE_ID` | topic level and MQTT client id, one per dimmer; default `dimmer`, 1–32 of `a-z 0-9 - _` |
| `NODE_NAME` | device name in Home Assistant; default `Dimmer`, at most 48 bytes, no `"` or `\` |
| `OTA_PUBLIC_KEY` | from `python ota.py keygen`; without it the firmware takes no updates over Wi-Fi |

Running next to a v3 dimmer, give each its own `NODE_ID`: two clients with the same id keep throwing each other
off the broker.

Once the dimmer firmware runs, no button is needed any more: writing `bootsel` to its USB serial port reboots it
into the bootloader.

```bash
echo bootsel > /dev/serial/by-id/usb-Embassy_USB-serial_logger-if00
cargo run --release
```

The same port carries the log (`screen /dev/serial/by-id/usb-Embassy_USB-serial_logger-if00`).

Unit tests (MQTT framing, light commands and fades, discovery payload, update orders) run on the host:

```bash
cargo test --lib --target x86_64-unknown-linux-gnu
```

The radio's firmware is in [`cyw43-firmware`](cyw43-firmware), under Infineon's permissive binary license, and is
compiled into the image.

## Home Assistant

Needs the MQTT integration with discovery on (the default). The dimmer announces itself as a device named
`NODE_NAME` with one light entity: on/off and brightness. No YAML. It shows up under Settings → Devices & services
→ MQTT.

| Topic | Payload |
| --- | --- |
| `homeassistant/light/<NODE_ID>/enabled/config` | discovery, retained |
| `smarthome/<NODE_ID>/set` | commands from Home Assistant: `{"state":"ON","brightness":180,"transition":1800}` |
| `smarthome/<NODE_ID>/state` | the dimmer's state, same JSON, retained |
| `smarthome/<NODE_ID>/status` | `online` / `offline` (last will), retained |
| `smarthome/<NODE_ID>/ota` | update orders from `ota.py`, see below |

The dimmer fades by itself: give `light.turn_on` or `light.turn_off` a `transition` in seconds, and it ramps
smoothly over that time, up to a day. A slow sunrise and sunset for plants, for example:

```yaml
- alias: Pflanzenlicht an
  triggers:
    - trigger: time
      at: "07:00:00"
  actions:
    - action: light.turn_on
      target:
        entity_id: light.dimmer
      data:
        brightness: 180
        transition: 1800
- alias: Pflanzenlicht aus
  triggers:
    - trigger: time
      at: "18:30:00"
  actions:
    - action: light.turn_off
      target:
        entity_id: light.dimmer
      data:
        transition: 1800
```

The state is retained on the broker, so the dimmer restores it after a power cut. Until it has, the strip runs at
12.5 % duty, as v1 always did. Brightness maps to duty squared, so the slider looks even to the eye.

If the broker closes the connection, the dimmer is back within a few seconds. If Wi-Fi drops, it joins again.
A broker or access point that vanishes without a word shows after three unanswered pings, 30 seconds. A hardware
watchdog (8 s) and reboot-on-panic cover everything else.

## Updates over Wi-Fi

```bash
python ota.py push       # builds and signs the image, puts it on the home server, starts the update
```

`push` copies the image with `scp` to `OTA_SSH:OTA_DIR` (default `/var/lib/smarthome-firmware`, which the home
server's nginx serves as `http://192.168.1.67/fw/`), tells the dimmer `NODE_ID` over MQTT where to fetch it, waits
until it has checked the signature and restarted, and reports back. Put `OTA_SSH` (e.g. `admin@family-server`) in
`.env`. The dimmer only installs an image signed with the key whose
public half it was built with. Anyone who can publish on your broker can make it try, but nothing unsigned gets
in. Keep `~/.config/pico_pi_dimmer/ota-key.pem` and a backup safe: without it, the next update needs USB again.

A bootloader ([embassy-boot](https://embassy.dev)) swaps the new firmware in. The new firmware confirms itself
once it reaches the broker; if it resets before that, the bootloader swaps the old one back. Flash layout:
[`memory.x`](memory.x).

