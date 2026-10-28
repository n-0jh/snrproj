#![no_std]
#![no_main]

extern crate alloc;

use embassy_executor::Spawner;
use embassy_net::{Runner, StackResources};
use embassy_time::{Duration, Timer};

use esp_hal::clock::CpuClock;
use esp_hal::rng::Rng;
use esp_hal::timer::timg::TimerGroup;

use esp_radio::wifi::{
    AuthenticationMethodConfig,
    Config,
    ControllerConfig,
    Interface,
    WifiController,
    sta::StationConfig,
};

use log::{error, info};

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

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
    let config = esp_hal::Config::default()
        .with_cpu_clock(CpuClock::max());

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
            .with_authentication(
                AuthenticationMethodConfig::Wpa2Personal(
                    PASSWORD.try_into().unwrap()
                )
            ),
    );

    // Initialize Wi-Fi
    let wifi_interface = Interface::station();

    let controller = WifiController::new(
        peripherals.WIFI,
        ControllerConfig::default()
            .with_initial_config(station_config),
    )
    .expect("Failed to initialize Wi-Fi");

    info!("Wi-Fi initialized!");

    // Configure DHCP
    let net_config = embassy_net::Config::dhcpv4(Default::default());

    // Generate a random network seed
    let rng = Rng::new();

    let seed =
        ((rng.random() as u64) << 32)
        | (rng.random() as u64);

    // Allocate network stack resources
    static RESOURCES: static_cell::StaticCell<StackResources<3>> =
        static_cell::StaticCell::new();

    let resources = RESOURCES.init(StackResources::<3>::new());

    let (stack, runner) = embassy_net::new(
        wifi_interface,
        net_config,
        resources,
        seed,
    );

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

    // Main application loop
    let mut uptime: u32 = 0;

    loop {
        Timer::after(Duration::from_secs(1)).await;

        uptime = uptime.wrapping_add(1);

        info!("RASCAL uptime: {} seconds", uptime);
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