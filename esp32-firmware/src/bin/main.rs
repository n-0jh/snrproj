#![no_std]
#![no_main]

extern crate alloc;

use embassy_executor::Spawner;
use embassy_net::{
    Runner, Stack, StackResources,
    udp::{PacketMetadata, UdpSocket},
};
use embassy_time::{Duration, Timer};
use esp_hal::time::Instant;
#[path = "../cpu_usage.rs"]
mod cpu_usage;

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

// C3 sends encrypted commands to this port (rascal_cmd_port in the plugin)
const COMMAND_PORT: u16 = 6901;

// encrypted command decode + ACK builder
#[path = "../command.rs"]
mod command;
use command::SecureLink;



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
    static RESOURCES: static_cell::StaticCell<StackResources<5>> = static_cell::StaticCell::new();

    let resources = RESOURCES.init(StackResources::<5>::new());

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


    // start the encrypted command receiver (has its own UDP socket on COMMAND_PORT)
    spawner.spawn(command_task(stack).unwrap());

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
    let mut last_sample = Instant::now();
    

    loop {
        Timer::after(Duration::from_secs(1)).await;

        uptime = uptime.wrapping_add(1);
        // Calculate CPU utilization
        let now = Instant::now();

        let elapsed_us = now.duration_since_epoch().as_micros()
        - last_sample.duration_since_epoch().as_micros();
        last_sample = now;
        
        let idle_us = cpu_usage::take_idle_us() as u64; 

        let busy_us = elapsed_us.saturating_sub(idle_us);

        info!(
        "CPU timing: elapsed={} us, idle={} us, busy={} us",
        elapsed_us,
        idle_us,
        busy_us
        );

        let cpu_usage_percent: u8 = if elapsed_us > 0 {
            ((busy_us * 100) / elapsed_us).min(100) as u8
        } else {
            0
        };

        info!("CPU 0 utilization estimate: {}%", cpu_usage_percent);
        // Construct ESP32 telemetry packet
        let mut packet = [0u8; 8];

        // Packet ID: ESP32 = 5
        packet[0] = 5;

        // Counter: 16-bit unsigned integer, big-endian
        let counter = (uptime as u16).to_be_bytes();

        packet[1..3].copy_from_slice(&counter);

        // Uptime: 32-bit unsigned integer, big-endian
        let uptime_bytes = uptime.to_be_bytes();

        packet[3..7].copy_from_slice(&uptime_bytes);
        // CPU 0 utilization percentage
        packet[7] = cpu_usage_percent;

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

// receives encrypted commands from C3, replies with an encrypted ACK
#[embassy_executor::task]
async fn command_task(stack: Stack<'static>) {
    let link = SecureLink::new();
    let rng = Rng::new();

    let mut rx_meta = [PacketMetadata::EMPTY; 4];
    let mut rx_buffer = [0u8; 512];
    let mut tx_meta = [PacketMetadata::EMPTY; 4];
    let mut tx_buffer = [0u8; 512];

    let mut socket = UdpSocket::new(
        stack,
        &mut rx_meta,
        &mut rx_buffer,
        &mut tx_meta,
        &mut tx_buffer,
    );
    socket
        .bind(COMMAND_PORT)
        .expect("Failed to bind command socket");

    info!("Listening for encrypted commands on UDP port {}", COMMAND_PORT);

    let mut buf = [0u8; 256];

    loop {
        let (len, meta) = match socket.recv_from(&mut buf).await {
            Ok(v) => v,
            Err(e) => {
                error!("Command receive failed: {:?}", e);
                continue;
            }
        };

        info!("Received {} bytes from {}", len, meta.endpoint);

        // Fresh random 12-byte nonce for the ACK
        let mut nonce = [0u8; command::NONCE_LEN];
        for chunk in nonce.chunks_mut(4) {
            chunk.copy_from_slice(&rng.random().to_be_bytes());
        }

        match link.handle(&buf[..len], nonce) {
            Ok((seq, ack)) => {
                info!("AES-GCM authentication successful");
                info!("NOOP received with sequence number {}", seq);

                let destination = (
                    embassy_net::Ipv4Address::from_octets(GROUND_STATION_IP),
                    TELEMETRY_PORT,
                );
                match socket.send_to(&ack, destination).await {
                    Ok(()) => info!(
                        "Encrypted ACK sent with sequence number {} ({} bytes)",
                        seq,
                        ack.len()
                    ),
                    Err(e) => error!("ACK transmission failed: {:?}", e),
                }
            }
            Err(e) => error!("Command rejected: {:?}", e),
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
