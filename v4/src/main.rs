//! Pico dimmer v4: a Raspberry Pi Pico W drives an LED strip through a MOSFET
//! module on GP14, and shows up in Home Assistant as a dimmable light over
//! MQTT discovery. v3 with the Pico W's own CYW43439 radio in place of the
//! Wireless Pack.
//!
//! If Wi-Fi or the broker goes away, it joins and connects again. The strip
//! keeps its last duty meanwhile.

#![no_std]
#![no_main]

use core::fmt::Write as _;

use cyw43::{JoinOptions, aligned_bytes};
use cyw43_pio::{DEFAULT_CLOCK_DIVIDER, PioSpi};
use embassy_executor::Spawner;
use embassy_net::dns::DnsQueryType;
use embassy_net::tcp::TcpSocket;
use embassy_net::{Config, Stack, StackResources};
use embassy_rp::clocks::RoscRng;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIO0, USB};
use embassy_rp::pio::{self, Pio};
use embassy_rp::pwm::{self, Pwm};
use embassy_rp::usb::{self, Driver};
use embassy_rp::watchdog::Watchdog;
use embassy_rp::{bind_interrupts, dma};
use embassy_time::{Duration, Instant, Timer, with_timeout};
use embassy_usb_logger::ReceiverHandler;
use heapless::String;
use log::{info, warn};
use panic_reset as _;
use static_cell::StaticCell;

use pico_dimmer::{Light, mqtt};

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");
/// Host name or dotted IPv4.
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

/// PWM frequency, as v2. The MOSFET module is rated to 20 kHz.
const PWM_HZ: u32 = 5_000;
const KEEP_ALIVE_SECS: u16 = 30;
/// Ping interval; three unanswered pings end the session. Catches a broker or
/// an access point that vanished without closing the connection.
const PING: Duration = Duration::from_secs(10);
const WATCHDOG: Duration = Duration::from_secs(8);

type Radio = cyw43::Runner<'static, cyw43::SpiBus<Output<'static>, PioSpi<'static, PIO0, 0>>>;

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => usb::InterruptHandler<USB>;
    PIO0_IRQ_0 => pio::InterruptHandler<PIO0>;
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>;
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

/// Feeds the watchdog while the executor runs, so a fault or a task that never
/// yields reboots the board. The network steps have their own timeouts.
#[embassy_executor::task]
async fn feed(mut watchdog: Watchdog) {
    watchdog.start(WATCHDOG);
    loop {
        watchdog.feed(WATCHDOG);
        Timer::after_secs(1).await;
    }
}

#[embassy_executor::task]
async fn radio(runner: Radio) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn net(mut runner: embassy_net::Runner<'static, cyw43::NetDriver<'static>>) -> ! {
    runner.run().await
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

    spawner.spawn(feed(Watchdog::new(p.WATCHDOG)).unwrap());

    // The Pico W's radio, on its half-duplex SPI through PIO.
    let fw = aligned_bytes!("../cyw43-firmware/43439A0.bin");
    let clm = aligned_bytes!("../cyw43-firmware/43439A0_clm.bin");
    let nvram = aligned_bytes!("../cyw43-firmware/nvram_rp2040.bin");
    let mut pio = Pio::new(p.PIO0, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        DEFAULT_CLOCK_DIVIDER,
        pio.irq0,
        Output::new(p.PIN_25, Level::High),
        p.PIN_24,
        p.PIN_29,
        dma::Channel::new(p.DMA_CH0, Irqs),
    );
    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let pwr = Output::new(p.PIN_23, Level::Low);
    let (device, mut control, runner) =
        cyw43::new(STATE.init(cyw43::State::new()), pwr, spi, fw, nvram).await;
    spawner.spawn(radio(runner).unwrap());
    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::PowerSave)
        .await;

    // Sockets: DHCP, DNS and the one to the broker.
    static RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(
        device,
        Config::dhcpv4(Default::default()),
        RESOURCES.init(StackResources::new()),
        RoscRng.next_u64(),
    );
    spawner.spawn(net(runner).unwrap());

    loop {
        if let Err(e) = run(stack, &mut control, &mut light, &mut strip).await {
            warn!("{e}; starting over");
        }
        Timer::after_secs(2).await;
    }
}

async fn run(
    stack: Stack<'static>,
    control: &mut cyw43::Control<'static>,
    light: &mut Light,
    strip: &mut Strip,
) -> Result<(), &'static str> {
    if !stack.is_link_up() {
        info!("joining {SSID}");
        let join = control.join(SSID, JoinOptions::new(PASSWORD.as_bytes()));
        with_timeout(Duration::from_secs(30), join)
            .await
            .map_err(|_| "Wi-Fi join timed out")?
            .map_err(|_| "Wi-Fi join failed")?;
    }
    with_timeout(Duration::from_secs(30), stack.wait_config_up())
        .await
        .map_err(|_| "no address from DHCP")?;
    info!("Wi-Fi up, connecting to {BROKER}");
    let addr = *with_timeout(
        Duration::from_secs(15),
        stack.dns_query(BROKER, DnsQueryType::A),
    )
    .await
    .map_err(|_| "DNS timed out")?
    .map_err(|_| "cannot resolve the broker")?
    .first()
    .ok_or("cannot resolve the broker")?;

    let (mut rx_buf, mut tx_buf) = ([0u8; 1024], [0u8; 1024]);
    let mut socket = TcpSocket::new(stack, &mut rx_buf, &mut tx_buf);
    socket.set_timeout(Some(PING * 3));
    with_timeout(Duration::from_secs(15), socket.connect((addr, 1883)))
        .await
        .map_err(|_| "broker unreachable")?
        .map_err(|_| "broker unreachable")?;

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
    send(&mut socket, &tx[..n]).await?;

    let mut rx = [0u8; 512];
    let mut fill = 0;
    let mut accepted = false;
    let started = Instant::now();
    let (mut last_rx, mut last_ping) = (started, started);
    loop {
        match with_timeout(Duration::from_secs(1), socket.read(&mut rx[fill..])).await {
            Ok(Ok(0)) => return Err("broker closed the connection"),
            Ok(Ok(n)) => fill += n,
            Ok(Err(_)) => return Err("connection lost"),
            Err(_) => {}
        }
        while let Some((packet, used)) = mqtt::parse(&rx[..fill]) {
            last_rx = Instant::now();
            match packet {
                mqtt::Packet::ConnAck(0) => {
                    accepted = true;
                    announce(&mut socket, &mut tx).await?;
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
            send(&mut socket, &mqtt::PINGREQ).await?;
            last_ping = Instant::now();
        }
    }
}

/// Discovery, availability and the two command subscriptions, after CONNACK.
async fn announce(socket: &mut TcpSocket<'_>, tx: &mut [u8]) -> Result<(), &'static str> {
    let mut config = String::<512>::new();
    pico_dimmer::discovery(&mut config, NODE_ID, NODE_NAME).map_err(|_| "discovery too long")?;
    let mut discovery = String::<64>::new();
    let _ = write!(discovery, "homeassistant/light/{NODE_ID}/enabled/config");

    let n = mqtt::publish(tx, &discovery, config.as_bytes(), true).ok_or("discovery too long")?;
    send(socket, &tx[..n]).await?;
    let n = mqtt::publish(tx, &topic("status"), b"online", true).ok_or("topic too long")?;
    send(socket, &tx[..n]).await?;
    for (id, key) in [(1, "config/enabled"), (2, "config/brightness")] {
        let n = mqtt::subscribe(tx, id, &topic(key)).ok_or("topic too long")?;
        send(socket, &tx[..n]).await?;
    }
    Ok(())
}

async fn send(socket: &mut TcpSocket<'_>, mut data: &[u8]) -> Result<(), &'static str> {
    while !data.is_empty() {
        match with_timeout(Duration::from_secs(5), socket.write(data)).await {
            Ok(Ok(n)) if n > 0 => data = &data[n..],
            Ok(_) => return Err("connection lost"),
            Err(_) => return Err("send timed out"),
        }
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
