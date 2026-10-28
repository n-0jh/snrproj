#![no_std]
#![no_main]

extern crate alloc;

use embassy_executor::Spawner;
use embassy_net::{
    Runner, StackResources,
    udp::{PacketMetadata, UdpSocket},
};
use embassy_time::{Duration, Timer};

use esp_hal::clock::CpuClock;
use esp_hal::rng::Rng;
use esp_hal::timer::timg::TimerGroup;

use esp_radio::wifi::{
    AuthenticationMethodConfig, Config, ControllerConfig, Interface, WifiController,
    sta::StationConfig,
};

use log::{error, info};

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

const GROUND_STATION_IP: [u8; 4] = [192, 168, 1, 180];
const TELEMETRY_PORT: u16 = 6900;

#[panic_handler]
fn panic(panic_info: &core::panic::PanicInfo) -> ! {
    error!("{}", panic_info);
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();

    // Initialize ESP32 hardware
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());

    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(
        #[esp_hal::ram(reclaimed)]
        size: 98768
    );

    // Start asynchronous runtime
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    info!("RASCAL ESP32 initialized!");

    // Configure Wi-Fi credentials
    let station_config = Config::Station(
        StationConfig::default()
            .with_ssid(SSID.try_into().unwrap())
            .with_authentication(AuthenticationMethodConfig::Wpa2Personal(
                PASSWORD.try_into().unwrap(),
            )),
    );

    // Initialize Wi-Fi
    let wifi_interface = Interface::station();

    let controller = WifiController::new(
        peripherals.WIFI,
        ControllerConfig::default().with_initial_config(station_config),
    )
    .expect("Failed to initialize Wi-Fi");

    info!("Wi-Fi initialized!");

    // Configure DHCP
    let net_config = embassy_net::Config::dhcpv4(Default::default());

    // Generate a random network seed
    let rng = Rng::new();

    let seed = ((rng.random() as u64) << 32) | (rng.random() as u64);

    // Allocate network stack resources
    static RESOURCES: static_cell::StaticCell<StackResources<3>> = static_cell::StaticCell::new();

    let resources = RESOURCES.init(StackResources::<3>::new());

    let (stack, runner) = embassy_net::new(wifi_interface, net_config, resources, seed);

    // Start Wi-Fi connection and network tasks
    spawner.spawn(wifi_task(controller).unwrap());
    spawner.spawn(network_task(runner).unwrap());
    info!("Waiting for network connection...");

    // Wait until DHCP assigns an address
    stack.wait_config_up().await;

    if let Some(config) = stack.config_v4() {
        info!("Wi-Fi connected!");
        info!("ESP32 IP address: {}", config.address);
    }

    // Allocate UDP receive/transmit buffers
    let mut rx_meta = [PacketMetadata::EMPTY; 1];
    let mut rx_buffer = [0u8; 256];

    let mut tx_meta = [PacketMetadata::EMPTY; 1];
    let mut tx_buffer = [0u8; 256];

    // Create UDP socket
    let mut socket = UdpSocket::new(
        stack,
        &mut rx_meta,
        &mut rx_buffer,
        &mut tx_meta,
        &mut tx_buffer,
    );

    // Bind ESP32 to a local UDP source port
    socket.bind(6902).expect("Failed to bind UDP socket");

    info!("UDP telemetry socket initialized!");
    // Main application loop
    let mut uptime: u32 = 0;

    loop {
        Timer::after(Duration::from_secs(1)).await;

        uptime = uptime.wrapping_add(1);

        // Construct ESP32 telemetry packet
        let mut packet = [0u8; 7];

        // Packet ID: ESP32 = 5
        packet[0] = 5;

        // Counter: 16-bit unsigned integer, big-endian
        let counter = (uptime as u16).to_be_bytes();

        packet[1..3].copy_from_slice(&counter);

        // Uptime: 32-bit unsigned integer, big-endian
        let uptime_bytes = uptime.to_be_bytes();

        packet[3..7].copy_from_slice(&uptime_bytes);

        // Ground station destination
        let destination = (
            embassy_net::Ipv4Address::from_octets(GROUND_STATION_IP),
            TELEMETRY_PORT,
        );

        // Transmit UDP telemetry
        match socket.send_to(&packet, destination).await {
            Ok(()) => {
                info!("ESP32 telemetry sent: uptime = {} seconds", uptime);
            }
            Err(e) => {
                error!("UDP transmission failed: {:?}", e);
            }
        }
    }
}

// Manages connection and reconnection
#[embassy_executor::task]
async fn wifi_task(mut controller: WifiController<'static>) {
    loop {
        info!("Connecting to Wi-Fi...");

        match controller.connect_async().await {
            Ok(_) => {
                info!("Wi-Fi association successful!");

                controller.wait_for_disconnect_async().await.ok();

                error!("Wi-Fi disconnected!");
            }

            Err(e) => {
                error!("Wi-Fi connection failed: {:?}", e);
            }
        }

        Timer::after(Duration::from_secs(5)).await;
    }
}

// Runs the TCP/IP networking stack
#[embassy_executor::task]
async fn network_task(mut runner: Runner<'static, Interface>) {
    runner.run().await;
}
