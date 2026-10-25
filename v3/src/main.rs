//! Pico dimmer v3: a Raspberry Pi Pico with the Pimoroni Wireless Pack drives
//! an LED strip through a MOSFET module on GP14, and shows up in Home
//! Assistant as a dimmable light over MQTT discovery.
//!
//! On any failure -- Wi-Fi, broker, a confused ESP32 -- the ESP32 is reset and
//! everything is set up again from scratch. The strip keeps its last duty
//! meanwhile.

#![no_std]
#![no_main]

use core::fmt::Write as _;

use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Flex, Input, Level, Output, Pull};
use embassy_rp::peripherals::{SPI0, USB};
use embassy_rp::pwm::{self, Pwm};
use embassy_rp::spi::{self, Blocking, Spi};
use embassy_rp::usb::{self, Driver};
use embassy_rp::watchdog::Watchdog;
use embassy_time::{Duration, Instant, Timer, with_timeout};
use embassy_usb_logger::ReceiverHandler;
use heapless::String;
use log::{info, warn};
use panic_reset as _;

use pico_dimmer::nina::{self, Param::Long, Param::Short};
use pico_dimmer::{Light, mqtt};

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");
/// Host name or dotted IPv4; the ESP32 resolves it.
const BROKER: &str = env!("MQTT_BROKER");
const MQTT_USER: Option<&str> = non_empty(option_env!("MQTT_USER"));
const MQTT_PASSWORD: Option<&str> = non_empty(option_env!("MQTT_PASSWORD"));
const NODE_ID: &str = match non_empty(option_env!("NODE_ID")) {
    Some(id) => id,
    None => "dimmer",
};
const NODE_NAME: &str = match non_empty(option_env!("NODE_NAME")) {
    Some(name) => name,
    None => "Dimmer",
};
const _: () = assert!(valid_id(NODE_ID), "NODE_ID: 1-32 of a-z, 0-9, - and _");
const _: () = assert!(
    valid_name(NODE_NAME),
    "NODE_NAME: at most 48 bytes, no quotes or backslashes"
);

/// PWM frequency, as v1. The MOSFET module is rated to 20 kHz.
const PWM_HZ: u32 = 5_000;
const KEEP_ALIVE_SECS: u16 = 30;
/// Ping interval; three unanswered pings end the session. nina-fw 1.7.3
/// reports a connection the broker closed as still up, and only a write
/// after the broker's reset fails, so this is how a broker restart shows.
const PING: Duration = Duration::from_secs(5);
const WATCHDOG: Duration = Duration::from_secs(8);

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => usb::InterruptHandler<USB>;
});

/// Log over USB serial. Writing `bootsel` to the port reboots into the USB
/// bootloader, so the next `cargo run` needs no button.
#[embassy_executor::task]
async fn logger(driver: Driver<'static, USB>) {
    embassy_usb_logger::run!(1024, log::LevelFilter::Info, driver, Bootsel);
}

struct Bootsel;

impl ReceiverHandler for Bootsel {
    async fn handle_data(&self, data: &[u8]) {
        if data.starts_with(b"bootsel") {
            embassy_rp::rom_data::reset_to_usb_boot(0, 0);
        }
    }

    fn new() -> Self {
        Bootsel
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    spawner.spawn(logger(Driver::new(p.USB, Irqs)).unwrap());

    let mut light = Light::BOOT;
    let mut config = pwm::Config::default();
    config.top = (embassy_rp::clocks::clk_sys_freq() / PWM_HZ - 1) as u16;
    let mut strip = Strip {
        pwm: Pwm::new_output_a(p.PWM_SLICE7, p.PIN_14, config.clone()),
        config,
    };
    strip.show(&light);

    let mut watchdog = Watchdog::new(p.WATCHDOG);
    watchdog.start(WATCHDOG);

    // Pimoroni Wireless Pack pinout.
    let mut spi_config = spi::Config::default();
    spi_config.frequency = 8_000_000;
    let mut esp = Esp {
        spi: Spi::new_blocking(p.SPI0, p.PIN_18, p.PIN_19, p.PIN_16, spi_config),
        cs: Output::new(p.PIN_7, Level::High),
        ack: Input::new(p.PIN_10, Pull::None),
        reset: Output::new(p.PIN_11, Level::Low),
        gpio0: Flex::new(p.PIN_2),
        watchdog,
        buf: [0; 1024],
    };

    loop {
        esp.reset().await;
        if let Err(e) = run(&mut esp, &mut light, &mut strip).await {
            warn!("{e}; resetting the ESP32");
        }
        Timer::after_secs(2).await;
    }
}

async fn run(esp: &mut Esp, light: &mut Light, strip: &mut Strip) -> Result<(), &'static str> {
    let version = esp.command(nina::GET_FW_VERSION, &[], false).await?;
    info!(
        "ESP32 firmware {}",
        core::str::from_utf8(version)
            .unwrap_or("?")
            .trim_end_matches('\0')
    );

    esp.join().await?;
    info!("Wi-Fi up, connecting to {BROKER}");
    let sock = esp.connect(BROKER, 1883).await?;

    let status = topic("status");
    let mut tx = [0u8; 600];
    let n = mqtt::connect(
        &mut tx,
        NODE_ID,
        MQTT_USER,
        MQTT_PASSWORD,
        &status,
        b"offline",
        KEEP_ALIVE_SECS,
    )
    .ok_or("CONNECT too long")?;
    esp.send(sock, &tx[..n]).await?;

    let mut rx = [0u8; 512];
    let mut fill = 0;
    let mut accepted = false;
    let started = Instant::now();
    let (mut last_rx, mut last_ping) = (started, started);
    loop {
        let n = esp.recv(sock, &mut rx[fill..]).await?;
        fill += n;
        while let Some((packet, used)) = mqtt::parse(&rx[..fill]) {
            last_rx = Instant::now();
            match packet {
                mqtt::Packet::ConnAck(0) => {
                    accepted = true;
                    announce(esp, sock, &mut tx).await?;
                    info!("MQTT up as smarthome/{NODE_ID}");
                }
                mqtt::Packet::ConnAck(_) => return Err("broker refused the connection"),
                mqtt::Packet::Publish { topic, payload } => {
                    light.handle(topic, payload);
                    strip.show(light);
                    info!("on={} level={}", light.on, light.level);
                }
                mqtt::Packet::Other => {}
            }
            rx.copy_within(used..fill, 0);
            fill -= used;
        }
        if fill == rx.len() {
            return Err("MQTT packet larger than the receive buffer");
        }
        if !accepted && started.elapsed() > Duration::from_secs(5) {
            return Err("no CONNACK");
        }
        if last_rx.elapsed() > PING * 3 {
            return Err("broker went silent");
        }
        if last_ping.elapsed() > PING {
            esp.send(sock, &mqtt::PINGREQ).await?;
            last_ping = Instant::now();
        }
        if n == 0 {
            Timer::after_millis(50).await;
        }
    }
}

/// Discovery, availability and the two command subscriptions, after CONNACK.
async fn announce(esp: &mut Esp, sock: u8, tx: &mut [u8]) -> Result<(), &'static str> {
    let mut config = String::<512>::new();
    pico_dimmer::discovery(&mut config, NODE_ID, NODE_NAME).map_err(|_| "discovery too long")?;
    let mut discovery = String::<64>::new();
    let _ = write!(discovery, "homeassistant/light/{NODE_ID}/enabled/config");

    let n = mqtt::publish(tx, &discovery, config.as_bytes(), true).ok_or("discovery too long")?;
    esp.send(sock, &tx[..n]).await?;
    let n = mqtt::publish(tx, &topic("status"), b"online", true).ok_or("topic too long")?;
    esp.send(sock, &tx[..n]).await?;
    for (id, key) in [(1, "config/enabled"), (2, "config/brightness")] {
        let n = mqtt::subscribe(tx, id, &topic(key)).ok_or("topic too long")?;
        esp.send(sock, &tx[..n]).await?;
    }
    Ok(())
}

fn topic(key: &str) -> String<64> {
    let mut t = String::new();
    let _ = write!(t, "smarthome/{NODE_ID}/{key}");
    t
}

struct Strip {
    pwm: Pwm<'static>,
    config: pwm::Config,
}

impl Strip {
    fn show(&mut self, light: &Light) {
        self.config.compare_a = light.duty(self.config.top as u32 + 1) as u16;
        self.pwm.set_config(&self.config);
    }
}

/// The ESP32 on the Wireless Pack, spoken to as Pimoroni's and Arduino's
/// drivers do: wait for ACK low (ready), select, wait for ACK high, transfer.
struct Esp {
    spi: Spi<'static, SPI0, Blocking>,
    cs: Output<'static>,
    ack: Input<'static>,
    reset: Output<'static>,
    gpio0: Flex<'static>,
    /// Fed on every exchange with the ESP32; every loop in this firmware makes one.
    watchdog: Watchdog,
    buf: [u8; 1024],
}

impl Esp {
    async fn reset(&mut self) {
        // GPIO0 high through reset selects the firmware, not the ROM loader.
        self.gpio0.set_high();
        self.gpio0.set_as_output();
        self.cs.set_high();
        self.reset.set_low();
        Timer::after_millis(10).await;
        self.reset.set_high();
        Timer::after_millis(750).await;
        self.gpio0.set_as_input();
        self.watchdog.feed(WATCHDOG);
    }

    /// Sends one command and returns the first parameter of the reply.
    /// `wide`: the reply parameter has a two-byte length (`GET_DATABUF_TCP`).
    async fn command(
        &mut self,
        cmd: u8,
        params: &[nina::Param<'_>],
        wide: bool,
    ) -> Result<&[u8], &'static str> {
        let n = nina::frame(cmd, params, &mut self.buf).ok_or("ESP32 command too long")?;
        self.select(2).await?;
        let sent = self.spi.blocking_write(&self.buf[..n]);
        self.cs.set_high();
        sent.map_err(|_| "SPI write failed")?;

        // Long timeout: the replies to REQ_HOST_BY_NAME and START_CLIENT_TCP
        // wait for DNS and the TCP handshake.
        self.select(10).await?;
        let reply = self.read_reply(cmd, wide);
        self.cs.set_high();
        let len = reply?;
        Ok(&self.buf[..len])
    }

    async fn select(&mut self, timeout_secs: u32) -> Result<(), &'static str> {
        let mut ready = false;
        for _ in 0..timeout_secs {
            self.watchdog.feed(WATCHDOG);
            if with_timeout(Duration::from_secs(1), self.ack.wait_for_low())
                .await
                .is_ok()
            {
                ready = true;
                break;
            }
        }
        if !ready {
            return Err("ESP32 not ready");
        }
        self.cs.set_low();
        // The ESP32 raises ACK once it has seen the select. Like Arduino's
        // driver, carry on after a few ms regardless.
        let _ = with_timeout(Duration::from_millis(5), self.ack.wait_for_high()).await;
        Ok(())
    }

    /// Reads a reply into `buf` and returns the length of its first parameter.
    fn read_reply(&mut self, cmd: u8, wide: bool) -> Result<usize, &'static str> {
        let mut byte = [0u8];
        let mut started = false;
        for _ in 0..32 {
            self.spi
                .blocking_read(&mut byte)
                .map_err(|_| "SPI read failed")?;
            match byte[0] {
                nina::START => {
                    started = true;
                    break;
                }
                nina::ERROR => return Err("ESP32 rejected the command"),
                _ => {}
            }
        }
        if !started {
            return Err("no reply from the ESP32");
        }

        let mut header = [0u8; 4];
        let header = &mut header[..if wide { 4 } else { 3 }];
        self.spi
            .blocking_read(header)
            .map_err(|_| "SPI read failed")?;
        // One parameter, or none from a failed START_CLIENT_TCP, whose END
        // was then the last header byte.
        if header[0] != cmd | nina::REPLY || header[1] > 1 {
            warn!("ESP32 answered {header:02x?} to command {cmd:#04x}");
            return Err("unexpected reply from the ESP32");
        }
        if header[1] == 0 {
            return Ok(0);
        }
        let len = if wide {
            u16::from_be_bytes([header[2], header[3]]) as usize
        } else {
            header[2] as usize
        };
        let data = self.buf.get_mut(..len).ok_or("ESP32 reply too long")?;
        self.spi
            .blocking_read(data)
            .map_err(|_| "SPI read failed")?;
        // Clock out the trailing END too, as the reference drivers do.
        self.spi
            .blocking_read(&mut byte)
            .map_err(|_| "SPI read failed")?;
        Ok(len)
    }

    async fn join(&mut self) -> Result<(), &'static str> {
        self.command(
            nina::SET_PASSPHRASE,
            &[Short(SSID.as_bytes()), Short(PASSWORD.as_bytes())],
            false,
        )
        .await?;
        for _ in 0..40 {
            Timer::after_millis(500).await;
            if self.command(nina::GET_CONN_STATUS, &[], false).await? == [nina::WL_CONNECTED] {
                return Ok(());
            }
        }
        Err("Wi-Fi join timed out")
    }

    /// Resolves `host` (a name or dotted IPv4), opens a TCP connection to it
    /// and returns its socket.
    async fn connect(&mut self, host: &str, port: u16) -> Result<u8, &'static str> {
        // Resolved separately: nina-fw 1.7.3's host-name form of
        // START_CLIENT_TCP passes a pointer as the address and never connects.
        if self
            .command(nina::REQ_HOST_BY_NAME, &[Short(host.as_bytes())], false)
            .await?
            != [1]
        {
            return Err("cannot resolve the broker");
        }
        let ip: [u8; 4] = self
            .command(nina::GET_HOST_BY_NAME, &[], false)
            .await?
            .try_into()
            .map_err(|_| "bad address from the ESP32")?;
        let sock = match self.command(nina::GET_SOCKET, &[], false).await? {
            &[sock] if sock != nina::NO_SOCKET => sock,
            _ => return Err("no free socket"),
        };
        let params = [
            Short(&ip),
            Short(&port.to_be_bytes()),
            Short(&[sock]),
            Short(&[0]),
        ];
        if self.command(nina::START_CLIENT_TCP, &params, false).await? != [1] {
            return Err("broker unreachable");
        }
        Ok(sock)
    }

    async fn send(&mut self, sock: u8, data: &[u8]) -> Result<(), &'static str> {
        let written = self
            .command(nina::SEND_DATA_TCP, &[Long(&[sock]), Long(data)], false)
            .await?;
        match written {
            &[lo, hi] if u16::from_le_bytes([lo, hi]) as usize == data.len() => Ok(()),
            _ => Err("short write"),
        }
    }

    /// Whatever has arrived, up to `out.len()` bytes; never waits for more.
    async fn recv(&mut self, sock: u8, out: &mut [u8]) -> Result<usize, &'static str> {
        let want = (out.len().min(512) as u16).to_le_bytes();
        let data = self
            .command(nina::GET_DATABUF_TCP, &[Long(&[sock]), Long(&want)], true)
            .await?;
        let n = data.len().min(out.len());
        out[..n].copy_from_slice(&data[..n]);
        Ok(n)
    }
}

const fn non_empty(s: Option<&'static str>) -> Option<&'static str> {
    match s {
        Some(s) if !s.is_empty() => Some(s),
        _ => None,
    }
}

/// Topic level and unique-id safe.
const fn valid_id(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if !matches!(b[i], b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_') {
            return false;
        }
        i += 1;
    }
    !b.is_empty() && b.len() <= 32
}

/// Goes into the discovery JSON verbatim.
const fn valid_name(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if matches!(b[i], b'"' | b'\\' | 0..=0x1f) {
            return false;
        }
        i += 1;
    }
    b.len() <= 48
}
